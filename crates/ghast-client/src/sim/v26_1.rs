//! 26.1 (protocol 775): the row of the modern world and movement (`modern.rs`, mounted below as
//! `imp`) — the version's crates, physics rules and tables. Everything else is shared with the
//! other modern versions.

pub(crate) use ghast_data::v26_1 as data;
pub(crate) use ghast_entity::v26_1 as entity;
pub(crate) use ghast_inventory::v26_1 as inv;
pub(crate) use ghast_physics::modern::V26_1 as Rules;
pub(crate) use ghast_physics::modern::{collides_26_1 as collides, push_kind_26_1 as push_kind};
pub(crate) use ghast_physics::pick::v26_1 as pick;
pub(crate) use ghast_protocol::v26_1 as proto;
pub(crate) use ghast_world::v26_1 as world;

use crate::version::GameVersion;

/// The run-time tag of this version.
pub(crate) const GAME: GameVersion = GameVersion::V26_1;
/// The order of the team parameters in `set_player_team`.
pub(crate) const TEAM_LAYOUT: super::teams::TeamLayout =
    super::teams::TeamLayout::RuleBeforeAffixes;

/// Entity data indices (`SynchedEntityData` ids, read from the 26.1 classes).
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

/// The effects movement and digging read, as the engine's classic 1-based numbers (26.1.2
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

// One body, mounted once per modern version (see CONTRIBUTING.md).
#[allow(clippy::duplicate_mod)]
#[path = "modern.rs"]
mod imp;

pub(crate) use imp::{ModernSim as Sim26, dig_progress, held_item_facts};

/// The exact destroy progress of this version.
pub(crate) fn exact_dig(m: crate::hands::ModernDig) -> crate::hands::ExactDig {
    crate::hands::ExactDig::V26_1(m)
}

/// `EnchantmentHelper.getTridentSpinAttackStrength` through the synced registry.
pub(crate) fn inv_trident_strength(
    inv: &mut ghast_inventory::Inventory,
    s: &ghast_inventory::Stack,
) -> Option<f32> {
    inv.trident_spin_attack_strength_26_1(s)
}

#[cfg(test)]
mod tests {
    /// The name-resolved entity ids are the 26.1 registry ids the movement used before.
    #[test]
    fn entity_ids_by_name() {
        let ids = super::imp::entity_ids_for_test();
        assert_eq!(ids.0, [66, 36, 87, 116, 151, 19, 20]);
        assert_eq!(ids.1, [78, 134]);
        assert_eq!(ids.2, [88, 152]);
        assert_eq!(ids.3, 53);
    }
}
