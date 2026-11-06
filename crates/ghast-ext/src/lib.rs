//! Sequential, mineflayer-style helpers for ghast bots.
//!
//! ghast's bot API is a per-tick state machine ([`ghast_client::Behavior::tick`]): exact and
//! cheap, but every multi-step job ("turn, then dig, then wait for the block") becomes an
//! explicit state enum. ghast-ext lets you write the same job as straight-line async code,
//! without giving up the tick model:
//!
//! ```no_run
//! use ghast_client::{Account, BlockPos, Client, Face, GameVersion, Interests};
//! use ghast_ext::{Event, Ext};
//!
//! # async fn run() -> Result<(), Box<dyn std::error::Error>> {
//! let mut bot = Ext::new(|bot| async move {
//!     let below = BlockPos::new(0, 63, 2);
//!     bot.look_at_block(below, None).await?;
//!     bot.dig(below).await?;                       // tool choice + vanilla dig time
//!     bot.equip("cobblestone").await?;
//!     bot.place(BlockPos::new(0, 62, 2), Face::Up).await?;
//!     let ores = bot.find_blocks(|b: ghast_ext::Block| b.is_ore(), 32.0, 10).await?;
//!     bot.wait_for(|ev| matches!(ev, Event::Chat(m) if m.plain().contains("hi"))).await?;
//!     bot.chat("hello");
//!     Ok(())
//! })
//! .interests(Interests::CHAT | Interests::ENTITIES);
//!
//! let client = Client::new("localhost:25565".parse()?, GameVersion::V1_8_9, Account::offline("Bot")?);
//! client.run(&mut bot).await?;
//! # Ok(()) }
//! ```
//!
//! # How it works
//!
//! [`Ext`] is a [`ghast_client::Behavior`] with a tiny single-threaded executor inside.
//! Every tick it polls the tasks that are ready. A task reaches the game only through
//! [`Bot::with`]: it hands over a closure, the executor runs it with the tick's
//! [`ghast_client::TickContext`] and resumes the task in the same tick. So:
//!
//! - every action is written inside [`ghast_client::Behavior::tick`], before the tick's
//!   single movement packet, in call order, exactly as hand-written behaviours do;
//! - the core's guarantees hold unchanged: dig / place are refused unless the **sent**
//!   crosshair is on the block (the helpers aim first and act on a later tick), clicks are
//!   refused while turning and while another click is in flight (the helpers wait and
//!   retry), rotation is whole mouse counts;
//! - nothing runs between ticks, and while the server is silent the behaviour (and with it
//!   every task) is on hold;
//! - no `unsafe`: the context never leaves the tick.
//!
//! Helpers fail with [`Error`] when the world changes under them: a teleport / setback or a
//! death interrupts what is running ([`Ext::interrupt_on_teleport`]), and the end of the
//! session drops (cancels) every task. Dropping a helper's future releases the buttons and
//! keys it held.
//!
//! One task should own the look and the movement keys at a time: two tasks aiming in the
//! same tick override each other (the last `set_aim` wins) and a turning look blocks clicks.

#![forbid(unsafe_code)]
#![warn(missing_docs, rust_2018_idioms, unreachable_pub, unnameable_types)]

mod actions;
mod error;
mod events;
mod executor;
mod find;
#[cfg(feature = "pathfinder")]
mod goto;
mod items;
mod look;
mod nav;

pub use actions::DigOptions;
pub use error::Error;
pub use events::{Event, Events, NextEvent};
pub use executor::{Bot, Ext, JoinHandle, Timeout, WaitTicks, WhenDone, With};
pub use find::{Block, BlockMatch, EntityInfo};
/// The path finder ([`Bot::goto`]), re-exported (feature `pathfinder`).
#[cfg(feature = "pathfinder")]
pub use ghast_pathfinder as pathfinder;
pub use items::{Container, ItemMatch, item_id, item_name};
pub use look::yaw_pitch;
