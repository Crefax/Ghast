//! `Bot::with` releases its result slot once the request ran (0.4.6 kept every slot alive:
//! a bot looping over failing `goto`s grew by about a kilobyte per call).

mod support;

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicIsize, Ordering};

use ghast_ext::Ext;
use support::Setup;

/// Counts the live heap bytes of the test process.
struct Counting;

static LIVE: AtomicIsize = AtomicIsize::new(0);

#[allow(clippy::cast_possible_wrap)]
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        LIVE.fetch_add(l.size() as isize, Ordering::Relaxed);
        // SAFETY: forwarded unchanged.
        unsafe { System.alloc(l) }
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        LIVE.fetch_sub(l.size() as isize, Ordering::Relaxed);
        // SAFETY: forwarded unchanged.
        unsafe { System.dealloc(p, l) }
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 {
        LIVE.fetch_add(n as isize - l.size() as isize, Ordering::Relaxed);
        // SAFETY: forwarded unchanged.
        unsafe { System.realloc(p, l, n) }
    }
}

#[global_allocator]
static A: Counting = Counting;

#[tokio::test]
async fn with_releases_its_slot() {
    let mut ext = Ext::new(|bot| async move {
        bot.wait_ticks(5).await;
        // A large result: each leaked slot would keep its space.
        let call = || bot.with(|ctx| [ctx.tick() as u8; 4096]);
        for _ in 0..50 {
            assert_eq!(call().await?.len(), 4096);
        }
        let before = LIVE.load(Ordering::Relaxed);
        for _ in 0..500 {
            assert_eq!(call().await?.len(), 4096);
        }
        let grown = LIVE.load(Ordering::Relaxed) - before;
        assert!(grown < 200_000, "500 calls kept {grown} bytes");
        Ok(())
    });
    let _ = support::run(Setup::flat(), &mut ext, 30).await;
    match ext.result() {
        Some(Ok(())) => {}
        other => panic!("main task: {other:?}"),
    }
}
