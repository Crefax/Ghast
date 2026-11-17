//! Furnaces (1.16.5 also blast furnaces and smokers) on 1.8.9, 1.12.2 and 1.16.5: the fuel slot
//! and the shift-click routes into the input and fuel slots.
//!
//! The tables were read off the real client (the fuel slot's `canInsert` and, for 1.8 / 1.12,
//! `SmeltingRecipeRegistry` over every item id and damage value, dumped by the oracle's
//! `MenuClickDump`); the unit tests check them against the dump. 1.16.5 smelting inputs come
//! from the server's recipes (`declare_recipes`, [`RecipeBook`](crate::RecipeBook)).

use crate::rules::Version;
use crate::stack::Stack;

/// 1.8.9 furnace fuels (`FurnaceBlockEntity.isFuel`; the fuel slot also takes the bucket): item
/// id and the damage values 0..15 as bits (all 16: any damage). Sorted.
const FUEL_1_8: &[(i32, u16)] = &[
    (5, 0xffff),
    (6, 0xffff),
    (17, 0xffff),
    (25, 0xffff),
    (47, 0xffff),
    (53, 0xffff),
    (54, 0xffff),
    (58, 0xffff),
    (72, 0xffff),
    (84, 0xffff),
    (85, 0xffff),
    (96, 0xffff),
    (99, 0xffff),
    (100, 0xffff),
    (107, 0xffff),
    (126, 0xffff),
    (134, 0xffff),
    (135, 0xffff),
    (136, 0xffff),
    (146, 0xffff),
    (151, 0xffff),
    (162, 0xffff),
    (163, 0xffff),
    (164, 0xffff),
    (173, 0xffff),
    (183, 0xffff),
    (184, 0xffff),
    (185, 0xffff),
    (186, 0xffff),
    (187, 0xffff),
    (188, 0xffff),
    (189, 0xffff),
    (190, 0xffff),
    (191, 0xffff),
    (192, 0xffff),
    (263, 0xffff),
    (268, 0xffff),
    (269, 0xffff),
    (270, 0xffff),
    (271, 0xffff),
    (280, 0xffff),
    (290, 0xffff),
    (327, 0xffff),
    (369, 0xffff),
    (425, 0xffff),
];

/// 1.8.9 smelting inputs (`SmeltingRecipeRegistry.getResult`): item id and damage bits as
/// above. Sorted.
const SMELTABLE_1_8: &[(i32, u16)] = &[
    (4, 0xffff),
    (12, 0xffff),
    (14, 0xffff),
    (15, 0xffff),
    (16, 0xffff),
    (17, 0xffff),
    (19, 0x0002),
    (21, 0xffff),
    (56, 0xffff),
    (73, 0xffff),
    (81, 0xffff),
    (82, 0xffff),
    (87, 0xffff),
    (98, 0x0001),
    (129, 0xffff),
    (153, 0xffff),
    (162, 0xffff),
    (319, 0xffff),
    (337, 0xffff),
    (349, 0x0003),
    (363, 0xffff),
    (365, 0xffff),
    (392, 0xffff),
    (411, 0xffff),
    (423, 0xffff),
];

/// 1.12.2 furnace fuels, as [`FUEL_1_8`].
const FUEL_1_12: &[(i32, u16)] = &[
    (5, 0xffff),
    (6, 0xffff),
    (17, 0xffff),
    (25, 0xffff),
    (35, 0xffff),
    (47, 0xffff),
    (53, 0xffff),
    (54, 0xffff),
    (58, 0xffff),
    (65, 0xffff),
    (72, 0xffff),
    (84, 0xffff),
    (85, 0xffff),
    (96, 0xffff),
    (99, 0xffff),
    (100, 0xffff),
    (107, 0xffff),
    (126, 0xffff),
    (134, 0xffff),
    (135, 0xffff),
    (136, 0xffff),
    (143, 0xffff),
    (146, 0xffff),
    (151, 0xffff),
    (162, 0xffff),
    (163, 0xffff),
    (164, 0xffff),
    (171, 0xffff),
    (173, 0xffff),
    (183, 0xffff),
    (184, 0xffff),
    (185, 0xffff),
    (186, 0xffff),
    (187, 0xffff),
    (188, 0xffff),
    (189, 0xffff),
    (190, 0xffff),
    (191, 0xffff),
    (192, 0xffff),
    (261, 0xffff),
    (263, 0xffff),
    (268, 0xffff),
    (269, 0xffff),
    (270, 0xffff),
    (271, 0xffff),
    (280, 0xffff),
    (281, 0xffff),
    (290, 0xffff),
    (323, 0xffff),
    (324, 0xffff),
    (327, 0xffff),
    (333, 0xffff),
    (346, 0xffff),
    (369, 0xffff),
    (425, 0xffff),
    (427, 0xffff),
    (428, 0xffff),
    (429, 0xffff),
    (430, 0xffff),
    (431, 0xffff),
    (444, 0xffff),
    (445, 0xffff),
    (446, 0xffff),
    (447, 0xffff),
    (448, 0xffff),
];

/// 1.12.2 smelting inputs, as [`SMELTABLE_1_8`].
const SMELTABLE_1_12: &[(i32, u16)] = &[
    (4, 0xffff),
    (12, 0xffff),
    (14, 0xffff),
    (15, 0xffff),
    (16, 0xffff),
    (17, 0xffff),
    (19, 0x0002),
    (21, 0xffff),
    (56, 0xffff),
    (73, 0xffff),
    (81, 0xffff),
    (82, 0xffff),
    (87, 0xffff),
    (98, 0x0001),
    (129, 0xffff),
    (153, 0xffff),
    (159, 0xffff),
    (162, 0xffff),
    (256, 0xffff),
    (257, 0xffff),
    (258, 0xffff),
    (267, 0xffff),
    (283, 0xffff),
    (284, 0xffff),
    (285, 0xffff),
    (286, 0xffff),
    (292, 0xffff),
    (294, 0xffff),
    (302, 0xffff),
    (303, 0xffff),
    (304, 0xffff),
    (305, 0xffff),
    (306, 0xffff),
    (307, 0xffff),
    (308, 0xffff),
    (309, 0xffff),
    (314, 0xffff),
    (315, 0xffff),
    (316, 0xffff),
    (317, 0xffff),
    (319, 0xffff),
    (337, 0xffff),
    (349, 0x0003),
    (363, 0xffff),
    (365, 0xffff),
    (392, 0xffff),
    (411, 0xffff),
    (417, 0xffff),
    (418, 0xffff),
    (423, 0xffff),
    (432, 0xffff),
];

/// 1.16.5 furnace fuel slot (`AbstractFurnaceBlockEntity.canUseAsFuel` with the vanilla item
/// tags, and the bucket): item registry ids. Sorted.
const FUEL_SLOT_1_16: &[i32] = &[
    15, 16, 17, 18, 19, 20, 23, 24, 25, 26, 27, 28, 37, 38, 39, 40, 41, 42, 45, 46, 47, 48, 49, 50,
    53, 54, 55, 56, 57, 58, 61, 62, 63, 64, 65, 66, 84, 91, 95, 96, 97, 98, 99, 100, 101, 102, 103,
    104, 105, 106, 107, 108, 109, 110, 135, 138, 139, 140, 141, 142, 143, 168, 179, 180, 183, 186,
    191, 192, 193, 194, 195, 196, 207, 208, 209, 210, 211, 212, 213, 226, 227, 228, 229, 230, 231,
    252, 253, 254, 255, 256, 257, 280, 281, 282, 305, 306, 307, 308, 309, 310, 317, 320, 350, 351,
    352, 353, 354, 355, 356, 357, 358, 359, 360, 361, 362, 363, 364, 365, 367, 369, 370, 556, 558,
    559, 560, 561, 562, 563, 574, 576, 577, 583, 584, 585, 586, 587, 613, 614, 652, 653, 654, 655,
    656, 657, 660, 662, 667, 676, 684, 745, 870, 871, 872, 873, 874, 875, 876, 877, 878, 879, 880,
    881, 882, 883, 884, 885, 899, 900, 901, 902, 903, 926, 928, 935, 936, 939, 940, 942, 943,
];

/// An item and damage in an `(id, damage bits)` table (damage 16 and above: only an item that
/// matches every damage 0..15).
fn in_table(t: &[(i32, u16)], s: &Stack) -> bool {
    let Ok(k) = t.binary_search_by_key(&s.id, |e| e.0) else {
        return false;
    };
    let mask = t[k].1;
    match u16::try_from(s.damage) {
        Ok(d) if d < 16 => mask & (1 << d) != 0,
        _ => mask == 0xffff,
    }
}

impl Version {
    /// The bucket (the fuel slot takes it, one at a time).
    pub(crate) fn is_bucket(self, s: &Stack) -> bool {
        match self {
            Version::V1_8 | Version::V1_12 => s.id == 325,
            Version::V1_16_5 => self.name(s.id) == "bucket",
            _ => false,
        }
    }

    /// `isFuel` (a burn time above 0; the bucket is none).
    pub(crate) fn is_fuel(self, s: &Stack) -> bool {
        match self {
            Version::V1_8 => in_table(FUEL_1_8, s),
            Version::V1_12 => in_table(FUEL_1_12, s),
            Version::V1_16_5 => !self.is_bucket(s) && FUEL_SLOT_1_16.binary_search(&s.id).is_ok(),
            _ => false,
        }
    }

    /// 1.8 / 1.12 `SmeltingRecipeRegistry.getResult(stack) != null` (1.16.5: the recipes).
    pub(crate) fn is_smeltable_legacy(self, s: &Stack) -> bool {
        match self {
            Version::V1_8 => in_table(SMELTABLE_1_8, s),
            Version::V1_12 => in_table(SMELTABLE_1_12, s),
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tables_sorted() {
        for t in [FUEL_1_8, FUEL_1_12, SMELTABLE_1_8, SMELTABLE_1_12] {
            assert!(t.windows(2).all(|w| w[0].0 < w[1].0));
        }
        assert!(FUEL_SLOT_1_16.windows(2).all(|w| w[0] < w[1]));
    }
}
