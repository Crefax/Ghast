//! The behaviour wrapper that feeds a viewer from the bot's tick.

use std::borrow::Cow;

use ghast_client::{
    Behavior, GameVersion, Health, Interests, Pose, RawInbound, TickContext, TickView,
};
use ghast_entity::Entities;
use ghast_inventory::Stack;
use ghast_protocol::BlockPos;

use crate::Viewer;
use crate::feed::{BotView, RAW_ALWAYS_1_8, RAW_WATCHED_1_8};

/// A behaviour that also feeds a [`Viewer`]. It only reads the tick context: it never writes to
/// the connection and does not change the bot's timing. While nobody watches it adds a handful
/// of rare packets to the filter (weather, experience, block entities) and does nothing else.
///
/// Side effects on the wrapped behaviour: while a page is open, `TickEvents::block_updates` and
/// `TickEvents::raw` may hold entries it did not ask for (filter `raw` by packet id), and with
/// [`crate::ViewerConfig::entities`] on, entity tracking (and the tab list) is enabled for the
/// session. A wrapped behaviour that finishes stops being ticked, but the viewer keeps showing
/// the bot standing there.
///
/// The viewer reads the bot in [`Behavior::after_tick`] (the pose after the movement step). For
/// light, biomes and block entities (sign text, skulls …) the client must keep them: build it
/// with `Client::world_config(ghast_viewer::world_config())`.
pub struct Watched<B> {
    inner: B,
    viewer: Viewer,
    inner_done: bool,
}

impl<B> Watched<B> {
    pub(crate) fn new(inner: B, viewer: Viewer) -> Self {
        Watched {
            inner,
            viewer,
            inner_done: false,
        }
    }

    /// The wrapped behaviour.
    pub fn inner(&self) -> &B {
        &self.inner
    }

    /// The wrapped behaviour.
    pub fn inner_mut(&mut self) -> &mut B {
        &mut self.inner
    }

    /// Unwraps.
    pub fn into_inner(self) -> B {
        self.inner
    }
}

impl<B: Behavior> Behavior for Watched<B> {
    fn name(&self) -> &str {
        self.inner.name()
    }

    fn interests(&self) -> Interests {
        let mut i = if self.inner_done {
            Interests::NONE
        } else {
            self.inner.interests()
        };
        let watched = self.viewer.hub().active();
        match self.viewer.config().entities {
            crate::EntityTracking::Always => {
                i |= Interests::ENTITIES | Interests::PLAYER_LIST | Interests::PLAYER_SKINS;
            }
            crate::EntityTracking::WhileWatched if watched => {
                i |= Interests::ENTITIES | Interests::PLAYER_LIST | Interests::PLAYER_SKINS;
            }
            _ => {}
        }
        if self.viewer.hub().active() {
            i |= Interests::BLOCK_UPDATES;
        }
        i
    }

    fn raw_packets(&self, version: GameVersion) -> Cow<'static, [i32]> {
        let inner = if self.inner_done {
            Cow::Borrowed(&[][..])
        } else {
            self.inner.raw_packets(version)
        };
        if version != GameVersion::V1_8_9 {
            return inner;
        }
        let ours = if self.viewer.hub().active() {
            RAW_WATCHED_1_8
        } else {
            RAW_ALWAYS_1_8
        };
        if inner.is_empty() {
            return Cow::Borrowed(ours);
        }
        let mut all: Vec<i32> = inner.into_owned();
        all.extend_from_slice(ours);
        all.sort_unstable();
        all.dedup();
        Cow::Owned(all)
    }

    fn start(&mut self, ctx: &mut TickContext<'_>) {
        self.inner.start(ctx);
        self.inner_done = !self.inner.running();
    }

    fn tick(&mut self, ctx: &mut TickContext<'_>) {
        if !self.inner_done {
            self.inner.tick(ctx);
            self.inner_done = !self.inner.running();
        }
    }

    fn after_tick(&mut self, view: &TickView<'_>) {
        // the engine calls this also once the inner behaviour has finished, like for any behaviour
        self.inner.after_tick(view);
        self.viewer.hub().tick(&View(view));
    }

    fn running(&self) -> bool {
        // a finished behaviour is not ticked any more, but the viewer keeps its packets
        true
    }

    fn stop(&mut self, reason: &str) {
        self.inner.stop(reason);
        self.viewer
            .hub()
            .session_ended(&format!("The bot left the game ({reason})"));
    }
}

/// [`BotView`] over ghast's after-tick view (the pose after this tick's movement step).
struct View<'a, 'b>(&'a TickView<'b>);

impl BotView for View<'_, '_> {
    fn tick(&self) -> u64 {
        self.0.tick()
    }
    fn version(&self) -> GameVersion {
        self.0.version()
    }
    fn pose(&self) -> Pose {
        *self.0.pose()
    }
    fn entity_id(&self) -> i32 {
        self.0.entity_id()
    }
    fn health(&self) -> Health {
        self.0.health()
    }
    fn held_slot(&self) -> u8 {
        self.0.held_slot()
    }
    fn hotbar(&self, k: usize) -> Option<&Stack> {
        self.0.inventory().hotbar(k)
    }
    fn armor(&self, a: usize) -> Option<&Stack> {
        self.0.inventory().armor(a)
    }
    fn mining(&self) -> Option<(BlockPos, f32)> {
        self.0.mining()
    }
    fn broken(&self) -> u64 {
        self.0.broken()
    }
    fn swings(&self) -> Option<u64> {
        Some(self.0.swings())
    }
    fn using_item(&self) -> bool {
        self.0.using_item()
    }
    fn dimension(&self) -> &str {
        self.0.dimension()
    }
    fn is_column_loaded(&self, cx: i32, cz: i32) -> bool {
        self.0.world().is_column_loaded(cx, cz)
    }
    fn raw_block(&self, x: i32, y: i32, z: i32) -> Option<u32> {
        self.0.world().raw_block(x, y, z)
    }
    fn column(&self, cx: i32, cz: i32) -> Option<&ghast_world::Column> {
        self.0.world().column(cx, cz)
    }
    fn entities(&self) -> Option<&Entities> {
        self.0.entities()
    }
    fn block_updates(&self) -> &[(BlockPos, u32)] {
        &self.0.events().block_updates
    }
    fn raw(&self) -> &[RawInbound] {
        &self.0.events().raw
    }
    fn joined(&self) -> bool {
        self.0.events().joined
    }
    fn respawned(&self) -> bool {
        self.0.events().respawned
    }
    fn build_height(&self) -> Option<(i32, i32)> {
        self.0.world().build_height()
    }
    #[cfg(any(feature = "v26_1", feature = "v26_2"))]
    fn enchantment_name(&self, id: i32) -> Option<&str> {
        self.0.inventory().enchantment_name_modern(id)
    }
}
