//! The per-version table set of the modern (26.x) inventory.
//!
//! The click simulation, the component readers, the hash schemas and the menu logic exist once;
//! everything that differs between modern versions is a table chosen by [`Mv`]: item rows,
//! registry name tables, menu facts, the component kind <-> wire id maps, and the protocol's
//! slot reader. Every accessor is a `match` returning a `&'static` table (code addresses, no
//! pointer table: nothing to relocate at load time), so where the version is a constant (the
//! `window_items` loops) the match folds away.
//!
//! Components are handled as *kinds* ([`kind`]): the 26.1 type ids, then every kind a later
//! version added. A version's wire ids map to kinds by name, computed at compile time from the
//! generated name tables.
//!
//! Adding a version: its generated `items_<v>.rs` / `menus_<v>.rs` (see `examples/`), one
//! `Mv` variant and one arm per accessor below, and `Version` / `PACKET_IDS` rows.

use bytes::Bytes;
use ghast_protocol::DecodeError;
use ghast_protocol::codec::Reader;
use ghast_protocol::types::slot::v775::{ComponentPatch, ItemStack};

use crate::rules::Version;

/// Canonical component kinds (the 26.1 type ids, then later additions).
#[allow(dead_code)]
pub(crate) mod kind {
    include!("generated/component_kinds.rs");
}

#[cfg(feature = "v26_1")]
#[allow(dead_code)]
pub(crate) mod g261 {
    use crate::item26::{Equip, Row};
    include!("generated/items_26_1.rs");
}
#[cfg(feature = "v26_1")]
#[allow(dead_code)]
mod g261m {
    include!("generated/menus_26_1.rs");
}
#[cfg(feature = "v26_2")]
#[allow(dead_code)]
pub(crate) mod g262 {
    use crate::item26::{Equip, Row};
    include!("generated/items_26_2.rs");
}
#[cfg(feature = "v26_2")]
#[allow(dead_code)]
mod g262m {
    include!("generated/menus_26_2.rs");
}

// The menu fact bits are the same in every version's table (one generator).
#[cfg(feature = "v26_1")]
pub(crate) use g261m::{
    BANNER, BEACON_PAYMENT, BREWING_FUEL, BREWING_INGREDIENT, FITS_CONTAINER, FUEL, HAS_DYE,
    HAS_PATTERNS, LOOM_DYE_TAG, LOOM_PATTERN_TAG,
};
#[cfg(not(feature = "v26_1"))]
pub(crate) use g262m::{
    BANNER, BEACON_PAYMENT, BREWING_FUEL, BREWING_INGREDIENT, FITS_CONTAINER, FUEL, HAS_DYE,
    HAS_PATTERNS, LOOM_DYE_TAG, LOOM_PATTERN_TAG,
};
#[cfg(all(feature = "v26_1", feature = "v26_2"))]
const _: () = assert!(
    g261m::FUEL == g262m::FUEL
        && g261m::BREWING_INGREDIENT == g262m::BREWING_INGREDIENT
        && g261m::BREWING_FUEL == g262m::BREWING_FUEL
        && g261m::BEACON_PAYMENT == g262m::BEACON_PAYMENT
        && g261m::LOOM_DYE_TAG == g262m::LOOM_DYE_TAG
        && g261m::LOOM_PATTERN_TAG == g262m::LOOM_PATTERN_TAG
        && g261m::BANNER == g262m::BANNER
        && g261m::FITS_CONTAINER == g262m::FITS_CONTAINER
        && g261m::HAS_DYE == g262m::HAS_DYE
        && g261m::HAS_PATTERNS == g262m::HAS_PATTERNS
);

/// A modern version: which table set the shared code reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Mv {
    /// 26.1 (protocol 775).
    #[cfg(feature = "v26_1")]
    V26_1,
    /// 26.2 (protocol 776).
    #[cfg(feature = "v26_2")]
    V26_2,
}

/// A generated registry name table (`*_TEXT` + `*_ENDS`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Names {
    Component,
    Menu,
    Potion,
    Attribute,
    SoundEvent,
    VillagerType,
    MobEffect,
    EntityType,
    BlockEntityType,
}

macro_rules! names_of {
    ($g:ident, $n:expr) => {
        match $n {
            Names::Component => ($g::COMPONENT_NAMES_TEXT, &$g::COMPONENT_NAMES_ENDS[..]),
            Names::Menu => ($g::MENUS_TEXT, &$g::MENUS_ENDS[..]),
            Names::Potion => ($g::POTIONS_TEXT, &$g::POTIONS_ENDS[..]),
            Names::Attribute => ($g::ATTRIBUTES_TEXT, &$g::ATTRIBUTES_ENDS[..]),
            Names::SoundEvent => ($g::SOUND_EVENTS_TEXT, &$g::SOUND_EVENTS_ENDS[..]),
            Names::VillagerType => ($g::VILLAGER_TYPES_TEXT, &$g::VILLAGER_TYPES_ENDS[..]),
            Names::MobEffect => ($g::MOB_EFFECTS_TEXT, &$g::MOB_EFFECTS_ENDS[..]),
            Names::EntityType => ($g::ENTITY_TYPES_TEXT, &$g::ENTITY_TYPES_ENDS[..]),
            Names::BlockEntityType => (
                $g::BLOCK_ENTITY_TYPES_TEXT,
                &$g::BLOCK_ENTITY_TYPES_ENDS[..],
            ),
        }
    };
}

impl Mv {
    /// Some enabled modern version (a placeholder where the version cannot matter).
    #[cfg(feature = "v26_1")]
    pub(crate) const ANY: Mv = Mv::V26_1;
    /// Some enabled modern version (a placeholder where the version cannot matter).
    #[cfg(not(feature = "v26_1"))]
    pub(crate) const ANY: Mv = Mv::V26_2;

    /// The table set of a version (`None` for the legacy versions and disabled features).
    #[inline]
    pub(crate) const fn of(v: Version) -> Option<Mv> {
        match v {
            #[cfg(feature = "v26_1")]
            Version::V26_1 => Some(Mv::V26_1),
            #[cfg(feature = "v26_2")]
            Version::V26_2 => Some(Mv::V26_2),
            _ => None,
        }
    }

    /// Item rows by registry id.
    #[inline]
    pub(crate) fn item_rows(self) -> &'static [crate::item26::Row] {
        match self {
            #[cfg(feature = "v26_1")]
            Mv::V26_1 => &g261::ITEM_ROWS,
            #[cfg(feature = "v26_2")]
            Mv::V26_2 => &g262::ITEM_ROWS,
        }
    }

    /// The strings of the item rows.
    #[inline]
    pub(crate) fn item_text(self) -> &'static str {
        match self {
            #[cfg(feature = "v26_1")]
            Mv::V26_1 => g261::ITEM_TEXT,
            #[cfg(feature = "v26_2")]
            Mv::V26_2 => g262::ITEM_TEXT,
        }
    }

    /// A registry name table.
    #[inline]
    pub(crate) fn names(self, n: Names) -> (&'static str, &'static [u32]) {
        match self {
            #[cfg(feature = "v26_1")]
            Mv::V26_1 => names_of!(g261, n),
            #[cfg(feature = "v26_2")]
            Mv::V26_2 => names_of!(g262, n),
        }
    }

    /// Name `i` of a registry name table.
    pub(crate) fn name(self, n: Names, i: usize) -> Option<&'static str> {
        let (text, ends) = self.names(n);
        crate::item26::nth(text, ends, i)
    }

    /// The id of `name` in a registry name table.
    pub(crate) fn position(self, n: Names, name: &str) -> Option<usize> {
        let (text, ends) = self.names(n);
        crate::item26::position(text, ends, name)
    }

    /// Menu slot facts by item id.
    #[inline]
    pub(crate) fn menu_facts(self) -> &'static [u16] {
        match self {
            #[cfg(feature = "v26_1")]
            Mv::V26_1 => &g261m::MENU_FACTS,
            #[cfg(feature = "v26_2")]
            Mv::V26_2 => &g262m::MENU_FACTS,
        }
    }

    /// `(item, remainder item, count)` of the items with a crafting remainder, by item id.
    #[inline]
    pub(crate) fn remainders(self) -> &'static [(i32, i32, i32)] {
        match self {
            #[cfg(feature = "v26_1")]
            Mv::V26_1 => &g261m::REMAINDERS,
            #[cfg(feature = "v26_2")]
            Mv::V26_2 => &g262m::REMAINDERS,
        }
    }

    /// The wire id of component kind `k` (an unknown kind maps to -1, which no patch holds).
    #[inline]
    pub(crate) fn wire(self, k: i32) -> i32 {
        match self {
            #[cfg(feature = "v26_1")]
            Mv::V26_1 => k,
            #[cfg(feature = "v26_2")]
            Mv::V26_2 => usize::try_from(k)
                .ok()
                .and_then(|k| WIRE_OF_KIND_262.get(k))
                .map_or(-1, |&w| if w == NONE { -1 } else { i32::from(w) }),
        }
    }

    /// The kind of wire component id `t` (-1 when out of range).
    #[inline]
    pub(crate) fn kind(self, t: i32) -> i32 {
        match self {
            #[cfg(feature = "v26_1")]
            Mv::V26_1 => t,
            #[cfg(feature = "v26_2")]
            Mv::V26_2 => usize::try_from(t)
                .ok()
                .and_then(|t| KIND_OF_WIRE_262.get(t))
                .map_or(-1, |&k| i32::from(k)),
        }
    }

    /// The name of block registry id `i`.
    pub(crate) fn block_name(self, i: u16) -> Option<&'static str> {
        match self {
            #[cfg(feature = "v26_1")]
            Mv::V26_1 => ghast_data::v26_1::Block::from_id(i).map(|b| b.name()),
            #[cfg(feature = "v26_2")]
            Mv::V26_2 => ghast_data::v26_2::Block::from_id(i).map(|b| b.name()),
        }
    }

    /// `ItemStack.OPTIONAL_STREAM_CODEC` with a structural component skip.
    #[inline]
    pub(crate) fn read_stack(self, r: &mut Reader<'_>) -> Result<Option<ItemStack>, DecodeError> {
        match self {
            #[cfg(feature = "v26_1")]
            Mv::V26_1 => ItemStack::read(r),
            #[cfg(feature = "v26_2")]
            Mv::V26_2 => ItemStack::read_776(r),
        }
    }

    /// Skips one `type id + value` component (`DataComponentPatch` entry layout).
    pub(crate) fn skip_component(self, r: &mut Reader<'_>) -> Result<(), DecodeError> {
        match self {
            #[cfg(feature = "v26_1")]
            Mv::V26_1 => ghast_protocol::defs::p775::types::SlotComponent::skip(r),
            #[cfg(feature = "v26_2")]
            Mv::V26_2 => ghast_protocol::defs::p776::types::SlotComponent::skip(r),
        }
    }

    /// A component patch body (`added` values, `removed` types) with a structural skip.
    pub(crate) fn read_patch(
        self,
        r: &mut Reader<'_>,
        added: i32,
        removed: i32,
    ) -> Result<ComponentPatch, DecodeError> {
        match self {
            #[cfg(feature = "v26_1")]
            Mv::V26_1 => ComponentPatch::read(r, added, removed),
            #[cfg(feature = "v26_2")]
            Mv::V26_2 => ComponentPatch::read_776(r, added, removed),
        }
    }

    /// A patch from wire ids and value bytes.
    pub(crate) fn patch_from_parts(
        self,
        added: &[(i32, &[u8])],
        removed: &[i32],
    ) -> ComponentPatch {
        match self {
            #[cfg(feature = "v26_1")]
            Mv::V26_1 => ComponentPatch::from_parts(added, removed),
            #[cfg(feature = "v26_2")]
            Mv::V26_2 => ComponentPatch::from_parts_776(added, removed),
        }
    }

    /// The value of component kind `k` in a patch.
    #[inline]
    pub(crate) fn get(self, p: &ComponentPatch, k: i32) -> Option<Bytes> {
        p.get(self.wire(k))
    }
}

// ------------------------------------------------------------------ kind <-> wire maps

const NONE: u8 = u8::MAX;

const fn same(text: &[u8], start: usize, end: usize, name: &[u8]) -> bool {
    if end - start != name.len() {
        return false;
    }
    let mut i = 0;
    while i < name.len() {
        if text[start + i] != name[i] {
            return false;
        }
        i += 1;
    }
    true
}

/// The kind of each wire id of a version's component name table; fails the build when a name
/// is not a known kind (append it to `generated/component_kinds.rs`).
#[allow(dead_code)]
const fn kinds_of<const N: usize>(text: &str, ends: &[u32; N]) -> [u8; N] {
    assert!(kind::KINDS < NONE as usize);
    let text = text.as_bytes();
    let mut out = [NONE; N];
    let mut i = 0;
    while i < N {
        let start = if i == 0 { 0 } else { ends[i - 1] as usize };
        let end = ends[i] as usize;
        let mut k = 0;
        while k < kind::KINDS {
            if same(text, start, end, kind::KIND_NAMES[k].as_bytes()) {
                out[i] = k as u8;
                break;
            }
            k += 1;
        }
        assert!(
            out[i] != NONE,
            "a component name is missing from component_kinds.rs"
        );
        i += 1;
    }
    out
}

/// The inverse of [`kinds_of`]: the wire id of each kind ([`NONE`] when the version lacks it).
#[allow(dead_code)]
const fn wires_of<const N: usize>(kinds: &[u8; N]) -> [u8; kind::KINDS] {
    let mut out = [NONE; kind::KINDS];
    let mut i = 0;
    while i < N {
        out[kinds[i] as usize] = i as u8;
        i += 1;
    }
    out
}

// 26.1 is the canonical numbering: its map must be the identity.
#[cfg(feature = "v26_1")]
const _: () = {
    let k = kinds_of(g261::COMPONENT_NAMES_TEXT, &g261::COMPONENT_NAMES_ENDS);
    let mut i = 0;
    while i < k.len() {
        assert!(
            k[i] as usize == i,
            "26.1 component ids are the canonical kinds"
        );
        i += 1;
    }
};

#[cfg(feature = "v26_2")]
static KIND_OF_WIRE_262: [u8; g262::COMPONENT_NAMES_ENDS.len()] =
    kinds_of(g262::COMPONENT_NAMES_TEXT, &g262::COMPONENT_NAMES_ENDS);
#[cfg(feature = "v26_2")]
static WIRE_OF_KIND_262: [u8; kind::KINDS] = wires_of(&KIND_OF_WIRE_262);

#[cfg(all(test, feature = "v26_2"))]
mod tests {
    use super::*;

    #[test]
    fn kinds_26_2() {
        // sulfur_cube_content was inserted at 78: later ids moved up by one
        assert_eq!(Mv::V26_2.wire(kind::SULFUR_CUBE_CONTENT), 78);
        assert_eq!(Mv::V26_2.kind(78), kind::SULFUR_CUBE_CONTENT);
        assert_eq!(Mv::V26_2.wire(kind::BEES), 77);
        assert_eq!(Mv::V26_2.wire(kind::LOCK), 79);
        assert_eq!(Mv::V26_2.wire(kind::SHULKER_COLOR), 110);
        for t in 0..111 {
            assert_eq!(Mv::V26_2.wire(Mv::V26_2.kind(t)), t);
            assert_eq!(
                Mv::V26_2.name(Names::Component, t as usize),
                Some(kind::KIND_NAMES[Mv::V26_2.kind(t) as usize])
            );
        }
        assert_eq!(Mv::V26_2.kind(111), -1);
    }
}
