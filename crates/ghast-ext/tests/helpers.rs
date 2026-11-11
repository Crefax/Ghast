//! Every helper against an in-process 1.8 fake server (`support`): real login, real ticks
//! (50 ms), real packets. The fake server answers digs, places, clicks and chat like a
//! vanilla server would.

mod support;

use std::sync::{Arc, Mutex};

use ghast_client::inventory::WindowKind;
use ghast_client::{BlockPos, Face, Interests, SessionEnd, Vec3};
use ghast_ext::{Error, Event, Ext, WhenDone};
use support::{BREAD, CHEST, COBBLESTONE, DIAMOND_PICKAXE, DIRT, STONE, Setup};

/// The main task finished without an error.
fn ok(ext: &Ext) {
    match ext.result() {
        Some(Ok(())) => {}
        other => panic!("main task: {other:?}"),
    }
}

fn is_move(name: &str) -> bool {
    matches!(name, "flying" | "position" | "look" | "position_look")
}

#[tokio::test]
async fn look_turns_in_mouse_steps() {
    let mut ext = Ext::new(|bot| async move {
        bot.look(90.0, 30.0).await?;
        let s = bot.sent();
        assert!(
            (s.yaw - 90.0).abs() <= 0.15 && (s.pitch - 30.0).abs() <= 0.15,
            "{s:?}"
        );
        // A point straight below the eyes (+X of the bot, at the floor).
        bot.look_at(Vec3::new(10.5, 63.0, 8.5)).await?;
        let s = bot.sent();
        assert!((s.yaw + 90.0).abs() < 1.0, "{s:?}");
        Ok(())
    });
    let (end, log) = support::run(Setup::flat(), &mut ext, 20).await;
    ok(&ext);
    assert!(matches!(end, SessionEnd::Quit(_)), "{end:?}");
    // The server has the final look; every change is a whole number of mouse steps
    // (sensitivity 0.5: 0.15 degrees).
    assert!((log.look.0 + 90.0).abs() < 1.0, "{:?}", log.look);
}

#[tokio::test]
async fn look_at_block_puts_the_crosshair_on_the_face() {
    let mut ext = Ext::new(|bot| async move {
        let pos = BlockPos::new(8, 64, 10);
        let hit = bot.look_at_block(pos, Some(Face::North)).await?;
        assert_eq!((hit.pos, hit.face), (pos, Face::North));
        // A face turned away from the eyes cannot be hit.
        let far = bot.look_at_block(pos, Some(Face::South)).await;
        assert_eq!(far, Err(Error::OutOfReach(pos)));
        let unloaded = bot.look_at_block(BlockPos::new(200, 64, 0), None).await;
        assert_eq!(unloaded, Err(Error::NotLoaded(BlockPos::new(200, 64, 0))));
        Ok(())
    });
    support::run(Setup::flat().block([8, 64, 10], DIRT), &mut ext, 20).await;
    ok(&ext);
}

#[tokio::test]
async fn dig_breaks_the_block_before_the_movement_packet() {
    let mut ext = Ext::new(|bot| async move {
        let pos = BlockPos::new(8, 64, 10);
        bot.dig(pos).await?;
        let b = bot.block_at(pos).await?.expect("loaded");
        assert!(b.is_air(), "{}", b.name());
        assert_eq!(bot.dig(pos).await, Err(Error::NothingToDig(pos)));
        let bedrock = BlockPos::new(9, 64, 10);
        assert_eq!(bot.dig(bedrock).await, Err(Error::Unbreakable(bedrock)));
        let far = BlockPos::new(200, 64, 0);
        assert_eq!(bot.dig(far).await, Err(Error::NotLoaded(far)));
        Ok(())
    });
    let setup = Setup::flat()
        .block([8, 64, 10], DIRT)
        .block([9, 64, 10], 7 << 4);
    let (_, log) = support::run(setup, &mut ext, 30).await;
    ok(&ext);
    let pos = BlockPos::new(8, 64, 10);
    assert_eq!(log.digs.first(), Some(&(0, pos)));
    assert!(log.digs.contains(&(2, pos)), "{:?}", log.digs);
    assert_eq!(log.blocks[&[8, 64, 10]], 0);
    // Tick order: the dig start and its swing go out before that tick's movement packet.
    let i = log.packets.iter().position(|n| *n == "block_dig").unwrap();
    assert_eq!(log.packets[i + 1], "arm_animation");
    assert!(is_move(log.packets[i + 2]), "{:?}", &log.packets[i..i + 3]);
}

#[tokio::test]
async fn place_and_equip_from_the_hotbar() {
    let mut ext = Ext::new(|bot| async move {
        assert_eq!(bot.equip("cobblestone").await?, 2);
        bot.place(BlockPos::new(9, 63, 10), Face::Up).await?;
        let b = bot.block_at(BlockPos::new(9, 64, 10)).await?.unwrap();
        assert_eq!(b.name(), "cobblestone");
        // Refusals that never reach the server.
        let air = BlockPos::new(9, 66, 10);
        assert_eq!(bot.place(air, Face::Up).await, Err(Error::NoSupport(air)));
        assert_eq!(
            bot.place(BlockPos::new(9, 63, 10), Face::Up).await,
            Err(Error::Occupied(BlockPos::new(9, 64, 10)))
        );
        assert_eq!(
            bot.equip("diamond_sword").await,
            Err(Error::NoSuchItem("diamond_sword".into()))
        );
        assert_eq!(
            bot.equip("no_such_thing").await,
            Err(Error::UnknownItem("no_such_thing".into()))
        );
        Ok(())
    });
    let setup = Setup::flat().item(38, COBBLESTONE, 64);
    let (_, log) = support::run(setup, &mut ext, 30).await;
    ok(&ext);
    assert_eq!(log.held.first(), Some(&2));
    assert_eq!(log.places.len(), 1, "{:?}", log.places);
    assert_eq!(log.places[0], (BlockPos::new(9, 63, 10), 1));
    assert_eq!(log.blocks[&[9, 64, 10]], (COBBLESTONE as u16) << 4);
}

#[tokio::test]
async fn equip_from_the_main_inventory_and_item_queries() {
    let mut ext = Ext::new(|bot| async move {
        assert_eq!(bot.count("bread").await?, 5);
        assert_eq!(bot.find_item("bread").await?.map(|(k, _)| k), Some(4));
        // The pickaxe is in main slot 20: swapped into the first empty hotbar slot (0).
        assert_eq!(bot.equip("diamond_pickaxe").await?, 0);
        let held = bot.held_item().await?.expect("held");
        assert_eq!(held.id(), i32::from(DIAMOND_PICKAXE));
        assert_eq!(bot.held_slot(), 0);
        // Toss the bread (a Q click with ctrl).
        assert_eq!(bot.toss("bread").await?, 5);
        assert_eq!(bot.count("bread").await?, 0);
        Ok(())
    });
    let setup = Setup::flat()
        .item(20, DIAMOND_PICKAXE, 1)
        .item(40, BREAD, 5);
    let (_, log) = support::run(setup, &mut ext, 30).await;
    ok(&ext);
    // Number-key swap of window slot 20 with hotbar 0, then the drop click of slot 40.
    assert_eq!(log.clicks[0], (0, 20, 0, 2), "{:?}", log.clicks);
    assert_eq!(log.clicks[1], (0, 40, 1, 4), "{:?}", log.clicks);
    assert!(log.packets.contains(&"close_window"));
}

#[tokio::test]
async fn containers_withdraw_and_deposit() {
    let mut ext = Ext::new(|bot| async move {
        let pos = bot.find_block("chest", 6.0).await?.expect("chest");
        assert_eq!(pos, BlockPos::new(10, 64, 8));
        let c = bot.open_container(pos).await?;
        assert_eq!((c.kind, c.slots), (WindowKind::Chest { rows: 3 }, 27));
        let items = bot.container_items().await?;
        assert_eq!(items.len(), 1);
        assert_eq!(bot.withdraw("bread", None).await?, 10);
        assert_eq!(bot.count("bread").await?, 10);
        assert_eq!(bot.deposit("cobblestone", Some(1)).await?, 64);
        assert_eq!(bot.count("cobblestone").await?, 0);
        bot.close_window().await?;
        assert_eq!(bot.container_items().await, Err(Error::NoWindow));
        Ok(())
    });
    let mut setup = Setup::flat()
        .block([10, 64, 8], CHEST)
        .item(36, COBBLESTONE, 64);
    setup.chest = Some(([10, 64, 8], vec![(0, BREAD, 10)]));
    let (_, log) = support::run(setup, &mut ext, 30).await;
    ok(&ext);
    // Shift clicks in window 1: the chest's slot 0, then the hotbar's cobblestone (54).
    assert_eq!(log.clicks, vec![(1, 0, 0, 1), (1, 54, 0, 1)]);
    assert!(
        log.places
            .iter()
            .any(|(p, _)| *p == BlockPos::new(10, 64, 8))
    );
}

#[tokio::test]
async fn chat_commands_and_events() {
    let mut ext = Ext::new(|bot| async move {
        let mut events = bot.events();
        bot.chat("hello");
        let m = bot.wait_for_chat(|m| m.plain().contains("hello")).await?;
        assert_eq!(m.plain(), "<Helper> hello");
        let r = bot
            .command("/ping", |m| m.plain().starts_with("pong"), 40)
            .await?;
        assert_eq!(r.plain(), "pong ping");
        // A reply that never comes.
        let none = bot.command("/silence", |m| m.plain() == "never", 10).await;
        assert_eq!(none, Err(Error::Timeout));
        // The subscription saw the chat lines.
        let mut chats = 0;
        while let Some(ev) = events.try_next() {
            if matches!(ev, Event::Chat(_)) {
                chats += 1;
            }
        }
        assert!(chats >= 2, "{chats}");
        // A teleport event.
        bot.chat("/tp");
        let ev = bot.wait_for(|e| matches!(e, Event::Teleport(_))).await?;
        let Event::Teleport(t) = ev else {
            unreachable!()
        };
        assert!((t.after.pos.x - 11.5).abs() < 1e-9, "{t:?}");
        Ok(())
    });
    let (_, log) = support::run(Setup::flat(), &mut ext, 30).await;
    ok(&ext);
    assert_eq!(log.chats[..3], ["hello", "/ping", "/silence"]);
}

#[tokio::test]
async fn teleports_interrupt_running_helpers() {
    let mut ext = Ext::new(|bot| async move {
        let walker = {
            let b = bot.clone();
            bot.spawn(async move { b.walk_straight_to(Vec3::new(20.5, 64.0, 8.5), 0.3).await })
        };
        bot.wait_ticks(5).await;
        bot.chat("/tp");
        assert_eq!(walker.await?, Err(Error::Teleported));
        // The keys were released.
        bot.wait_ticks(2).await;
        let c = bot.with(|ctx| ctx.controls()).await?;
        assert_eq!(c, ghast_client::Controls::NONE);
        Ok(())
    });
    support::run(Setup::flat(), &mut ext, 30).await;
    ok(&ext);
}

#[tokio::test]
async fn walk_straight_and_find() {
    let mut ext = Ext::new(|bot| async move {
        let found = bot.find_blocks("dirt", 16.0, 10).await?;
        assert_eq!(
            found,
            vec![BlockPos::new(8, 64, 12), BlockPos::new(14, 64, 14)]
        );
        let pred = bot
            .find_blocks(
                |b: ghast_ext::Block| b.name() == "dirt" && b.meta() == 0,
                5.0,
                10,
            )
            .await?;
        assert_eq!(pred, vec![BlockPos::new(8, 64, 12)]);
        assert_eq!(bot.find_block("diamond_ore", 16.0).await?, None);
        bot.walk_straight_to(Vec3::new(4.5, 64.0, 4.5), 0.4).await?;
        let p = bot.pose().pos;
        assert!((p.x - 4.5).hypot(p.z - 4.5) < 0.6, "{p:?}");
        // Entities (the experience orb).
        let orbs = bot.find_entities(|_| true, 16.0).await?;
        assert_eq!(orbs.len(), 1, "{orbs:?}");
        assert_eq!(orbs[0].id, 77);
        assert!(bot.entity(77).await?.is_some());
        assert_eq!(bot.entity(78).await?, None);
        Ok(())
    })
    .interests(Interests::CHAT | Interests::ENTITIES);
    let mut setup = Setup::flat()
        .block([8, 64, 12], DIRT)
        .block([14, 64, 14], DIRT)
        .block([14, 65, 14], STONE);
    setup.orbs.push((77, [10.5, 64.0, 8.5]));
    let (_, log) = support::run(setup, &mut ext, 30).await;
    ok(&ext);
    assert!(
        (log.pos[0] - 4.5).hypot(log.pos[2] - 4.5) < 0.6,
        "{:?}",
        log.pos
    );
}

#[tokio::test]
async fn executor_ticks_timeouts_and_tasks() {
    let order = Arc::new(Mutex::new(Vec::new()));
    let o = order.clone();
    let mut ext = Ext::new(move |bot| {
        let o = o.clone();
        async move {
            let t = bot.tick();
            bot.wait_ticks(10).await;
            assert!(bot.tick() >= t + 10);
            // Several `with` calls run in one tick.
            let a = bot.with(|ctx| ctx.tick()).await?;
            let b = bot.with(|ctx| ctx.tick()).await?;
            assert_eq!(a, b);
            assert_eq!(
                bot.timeout(5, bot.wait_ticks(100)).await,
                Err(Error::Timeout)
            );
            assert_eq!(bot.timeout(100, async { 7 }).await, Ok(7));
            let h = bot.spawn(async { 3 });
            assert_eq!(h.await, Ok(3));
            let b2 = bot.clone();
            let forever = bot.spawn(async move {
                loop {
                    b2.next_tick().await;
                }
            });
            bot.wait_ticks(3).await;
            assert!(!forever.is_finished());
            forever.abort();
            assert_eq!(forever.await, Err(Error::Cancelled));
            // A panicking task reports it.
            let p = bot.spawn(async { panic!("boom") });
            let r = p.await;
            assert!(
                matches!(&r, Err(Error::Panicked(m)) if m == "boom"),
                "{r:?}"
            );
            // `act` requests run in order.
            for i in 0..3 {
                let o = o.clone();
                bot.act(move |_| o.lock().unwrap().push(i));
            }
            bot.next_tick().await;
            assert_eq!(bot.version(), ghast_client::GameVersion::V1_8_9);
            assert_eq!(bot.entity_id(), 42);
            Ok(())
        }
    })
    .when_done(WhenDone::Disconnect);
    let (end, _) = support::run(Setup::flat(), &mut ext, 30).await;
    ok(&ext);
    assert!(
        matches!(end, SessionEnd::Quit(ref r) if r == "done"),
        "{end:?}"
    );
    assert_eq!(*order.lock().unwrap(), vec![0, 1, 2]);
}

#[tokio::test]
async fn goto_walks_a_planned_path_around_a_wall() {
    use ghast_ext::pathfinder::Goal;
    let mut ext = Ext::new(|bot| async move {
        bot.goto(Goal::Block([16, 64, 8])).await?;
        let p = bot.pose().pos;
        assert_eq!((p.x.floor() as i32, p.z.floor() as i32), (16, 8), "{p:?}");
        // Unreachable: inside the wall.
        assert_eq!(bot.goto(Goal::Block([12, 66, 8])).await, Err(Error::NoPath));
        Ok(())
    });
    let mut setup = Setup::flat();
    for z in 0..=14 {
        for y in 64..=65 {
            setup.blocks.insert([12, y, z], STONE);
        }
    }
    let (_, log) = support::run(setup, &mut ext, 40).await;
    ok(&ext);
    assert_eq!(
        (log.pos[0].floor() as i32, log.pos[2].floor() as i32),
        (16, 8)
    );
}

/// A plan spread over ticks: the player stands still (keys released, no turning) while the
/// follower plans, and the client sends its movement packet every tick meanwhile.
#[tokio::test]
async fn planning_over_several_ticks_holds_still() {
    use ghast_ext::pathfinder::{
        Budget, Config, FollowConfig, FollowStatus, Follower, Goal, Pathfinder,
    };
    let ticks = Arc::new(Mutex::new((0u64, 0u64)));
    let t = ticks.clone();
    let mut ext = Ext::new(move |bot| {
        let t = t.clone();
        async move {
            bot.wait_ticks(5).await;
            let start = bot.tick();
            let cfg = FollowConfig::default().with_budget(Budget::nodes(15));
            let mut f = Follower::new(Pathfinder::new(Config::default()), cfg);
            f.goto(Goal::Block([16, 64, 8]));
            let p0 = bot.pose();
            let mut planning = 0u32;
            loop {
                let (ff, status, still) = bot
                    .with(move |ctx| {
                        let mut f = f;
                        let s = f.tick(ctx);
                        let still = f
                            .is_planning()
                            .then(|| (*ctx.pose(), ctx.controls(), ctx.planned_look()));
                        (f, s, still)
                    })
                    .await?;
                f = ff;
                if let Some((pose, controls, look)) = still {
                    planning += 1;
                    assert_eq!(pose.pos, p0.pos);
                    assert_eq!(controls, ghast_client::Controls::NONE);
                    assert_eq!(look, (p0.yaw, p0.pitch));
                }
                match status {
                    FollowStatus::Arrived => break,
                    FollowStatus::Failed(e) => panic!("{e}"),
                    _ => {}
                }
                bot.next_tick().await;
            }
            assert!(planning >= 3, "planned in {planning} ticks");
            assert_eq!(f.plan_ticks(), planning + 1);
            let p = bot.pose().pos;
            assert_eq!((p.x.floor() as i32, p.z.floor() as i32), (16, 8), "{p:?}");
            *t.lock().unwrap() = (start, bot.tick());
            Ok(())
        }
    });
    let mut setup = Setup::flat();
    for z in 0..=14 {
        for y in 64..=65 {
            setup.blocks.insert([12, y, z], STONE);
        }
    }
    let (_, log) = support::run(setup, &mut ext, 40).await;
    ok(&ext);
    // One movement packet per tick, planning ticks included.
    let (start, end) = *ticks.lock().unwrap();
    let moves = log.packets.iter().filter(|n| is_move(n)).count() as u64;
    assert!(
        moves >= end,
        "{moves} movement packets for {end} ticks ({start} before planning)"
    );
}
