//! Process-wide sharing of immutable data that every connection to the same server receives
//! identically (registry names, tags, recipes, the command tree). Many clients in one process
//! then hold one copy instead of one each. Values are matched by content when they are stored
//! (once per join, under a lock); reads go through the `Arc` / [`Bytes`] taken then and never
//! touch the pool. An entry lives as long as something holds it.

use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::hash::{BuildHasher, Hash, RandomState};
use std::sync::{Arc, Mutex, OnceLock, PoisonError, Weak};

use bytes::Bytes;

type Key = (TypeId, u64);

struct Pool {
    hasher: RandomState,
    entries: HashMap<Key, Weak<dyn Any + Send + Sync>>,
    /// Dead entries are dropped when the map reaches this size.
    prune_at: usize,
}

fn pool() -> &'static Mutex<Pool> {
    static POOL: OnceLock<Mutex<Pool>> = OnceLock::new();
    POOL.get_or_init(|| {
        Mutex::new(Pool {
            hasher: RandomState::new(),
            entries: HashMap::new(),
            prune_at: 64,
        })
    })
}

/// The process's shared copy of `value` (an equal value stored earlier and still alive), or
/// `value` itself, which later equal values then share. Costs a hash of `value` and a lock:
/// meant for data stored once per connection, not per packet.
pub fn shared<T: Hash + Eq + Send + Sync + 'static>(value: T) -> Arc<T> {
    let mut pool = pool().lock().unwrap_or_else(PoisonError::into_inner);
    let key = (TypeId::of::<T>(), pool.hasher.hash_one(&value));
    if let Some(old) = pool.entries.get(&key).and_then(Weak::upgrade)
        && let Ok(old) = old.downcast::<T>()
        && *old == value
    {
        return old;
    }
    // New, dead or a hash collision (then the newer value is the one shared from now on).
    let new = Arc::new(value);
    let weak: Weak<dyn Any + Send + Sync> = Arc::downgrade(&new) as _;
    pool.entries.insert(key, weak);
    if pool.entries.len() >= pool.prune_at {
        pool.entries.retain(|_, w| w.strong_count() > 0);
        pool.prune_at = (pool.entries.len() * 2).max(64);
    }
    new
}

/// Bytes owned by a shared allocation.
struct Owner(Arc<Box<[u8]>>);

impl AsRef<[u8]> for Owner {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

/// `data` as [`Bytes`] backed by the process's shared copy of these bytes.
pub fn bytes(data: &[u8]) -> Bytes {
    if data.is_empty() {
        return Bytes::new();
    }
    Bytes::from_owner(Owner(shared(Box::<[u8]>::from(data))))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn equal_values_share_one_allocation() {
        let a = shared(vec!["minecraft:a".to_string(), "minecraft:b".to_string()]);
        let b = shared(vec!["minecraft:a".to_string(), "minecraft:b".to_string()]);
        assert!(Arc::ptr_eq(&a, &b));
        let c = shared(vec!["minecraft:a".to_string()]);
        assert!(!Arc::ptr_eq(&a, &c));
        // Same content, other type: not shared.
        let d = shared(vec![String::from("minecraft:a")].into_boxed_slice());
        assert_eq!(d.len(), 1);

        let x = bytes(b"tags of one server");
        let y = bytes(b"tags of one server");
        assert_eq!(x.as_ptr(), y.as_ptr());
        assert_eq!(&x[..], b"tags of one server");
        assert_ne!(bytes(b"other tags").as_ptr(), x.as_ptr());
    }

    #[test]
    fn an_entry_ends_with_its_last_holder() {
        let v = vec![7u64, 8, 9];
        let a = shared(v.clone());
        let weak = Arc::downgrade(&a);
        drop(a);
        assert!(weak.upgrade().is_none());
        // Built again from scratch.
        let b = shared(v);
        assert_eq!(*b, [7, 8, 9]);
    }
}
