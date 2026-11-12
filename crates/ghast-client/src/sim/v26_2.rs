//! 26.2 (protocol 776): the row of the modern world and movement (`modern.rs`, mounted below as
//! `imp`) — the version's crates, physics rules and tables. Everything else is shared with the
//! other modern versions.

pub(crate) use ghast_data::v26_2 as data;
pub(crate) use ghast_entity::v26_2 as entity;
pub(crate) use ghast_physics::modern::V26_2 as Rules;
// The 26.1 collision and push rules hold for 26.2 by name: the new sulfur cube overrides neither
// canBeCollidedWith nor isPushable (its playerTouch push moves the cube, not the player).
pub(crate) use ghast_physics::modern::{collides_26_1 as collides, push_kind_26_1 as push_kind};
pub(crate) use ghast_physics::pick::v26_2 as pick;
pub(crate) use ghast_protocol::v26_2 as proto;
pub(crate) use ghast_world::v26_2 as world;

use crate::version::GameVersion;

/// The run-time tag of this version.
pub(crate) const GAME: GameVersion = GameVersion::V26_2;
/// The order of the team parameters in `set_player_team`.
pub(crate) const TEAM_LAYOUT: super::teams::TeamLayout = super::teams::TeamLayout::AffixesFirst;

/// Entity data indices (`SynchedEntityData` ids, read from the 26.2 classes, unchanged from 26.1).
pub(crate) mod meta {
    /// `Entity.DATA_SHARED_FLAGS_ID`.
    pub(crate) const SHARED_FLAGS: u8 = 0;
    /// `LivingEntity.DATA_LIVING_ENTITY_FLAGS` (bit value 4: the riptide spin).
    pub(crate) const LIVING_FLAGS: u8 = 8;
    /// `FireworkRocketEntity.DATA_ATTACHED_TO_TARGET`.
    pub(crate) const ATTACHED_TO_TARGET: u8 = 9;
    /// `Avatar.DATA_PLAYER_MAIN_HAND`.
    pub(crate) const PLAYER_MAIN_HAND: u8 = 15;
    /// `AgeableMob.DATA_BABY_ID`.
    pub(crate) const BABY: u8 = 16;
    /// `AbstractHorse.DATA_ID_FLAGS` (tamed = 2) / `TamableAnimal.DATA_FLAGS_ID` (tame = 4).
    pub(crate) const MOUNT_FLAGS: u8 = 18;
}

/// The effects movement and digging read, as the engine's classic 1-based numbers (26.2
/// registries report: `minecraft:mob_effect` protocol id + 1).
pub(crate) mod effects {
    pub(crate) const JUMP_BOOST: i8 = 8;
    pub(crate) const BLINDNESS: i8 = 15;
    pub(crate) const LEVITATION: i8 = 25;
    pub(crate) const SLOW_FALLING: i8 = 28;
    pub(crate) const CONDUIT_POWER: i8 = 29;
    pub(crate) const DOLPHINS_GRACE: i8 = 30;
    pub(crate) const WEAVING: i8 = 37;
}

/// This version's modern item functions (`ghast_inventory::modern` with the version fixed).
pub(crate) mod inv {
    use ghast_inventory::Stack;
    use ghast_inventory::modern as m;
    pub(crate) use m::Equip;

    const V: ghast_inventory::Version = ghast_inventory::Version::V26_2;

    #[inline]
    pub(crate) fn item_name(id: i32) -> Option<&'static str> {
        m::item_name(V, id)
    }
    #[inline]
    pub(crate) fn item_use(s: &Stack) -> Option<m::ItemUse> {
        m::item_use(V, s)
    }
    #[inline]
    pub(crate) fn next_damage_will_break(s: &Stack) -> bool {
        m::next_damage_will_break(V, s)
    }
    #[inline]
    pub(crate) fn can_glide_using(s: &Stack, slot: Equip) -> bool {
        m::can_glide_using(V, s, slot)
    }
}

// One body, mounted once per modern version (see CONTRIBUTING.md).
#[allow(clippy::duplicate_mod)]
#[path = "modern.rs"]
mod imp;

pub(crate) use imp::{ModernSim as Sim262, dig_progress, held_item_facts};

/// The exact destroy progress of this version.
pub(crate) fn exact_dig(m: crate::hands::ModernDig) -> crate::hands::ExactDig {
    crate::hands::ExactDig::V26_2(m)
}

/// `EnchantmentHelper.getTridentSpinAttackStrength` through the synced registry.
pub(crate) fn inv_trident_strength(
    inv: &mut ghast_inventory::Inventory,
    s: &ghast_inventory::Stack,
) -> Option<f32> {
    inv.trident_spin_attack_strength_modern(s)
}

#[cfg(test)]
mod tests {
    /// Every mount / firework entity type resolves by name in the 26.2 registry.
    #[test]
    fn entity_ids_by_name() {
        let (h, l, n, f) = super::imp::entity_ids_for_test();
        assert!(
            h.iter().chain(&l).chain(&n).all(|&i| i >= 0),
            "{h:?} {l:?} {n:?}"
        );
        let id = |s: &str| i32::from(super::data::entity_by_name(s).unwrap().id);
        assert_eq!(
            (h[0], l[0], n[0], f),
            (
                id("horse"),
                id("llama"),
                id("nautilus"),
                id("firework_rocket")
            )
        );
    }
}
