//! The 1.12.2 and 1.8.9 anvils like their client (`AnvilScreenHandler.updateResult`, taking the result,
//! the screen's rename field and `MC|ItemName`): the 1.16.5 rules over 1.12 / 1.8 stacks (1.8: weights for the enchantment cost, no all-clash
//! or stack rules, the work penalty always grows, a 30-character field)
//! (damage values, `ench` / `StoredEnchantments` with numeric ids, plain-string names). The
//! enchantment and repair data were read off the jar (unit-tested against the dump).

#![cfg_attr(not(any(feature = "v1_8", feature = "v1_12")), allow(dead_code))]

use ghast_protocol::nbt::{Compound, Tag};

use crate::anvil16::Anvil;
use crate::nbt16 as n;
use crate::rules::Version;
use crate::sim::{R, Sim, Unpredictable};
use crate::stack::{Stack, last, number};

/// 1.12.2 enchantments: raw id, max level, the anvil's cost weight by rarity, the ids it cannot
/// combine with (bits by raw id, itself included). Read off the jar (unit-tested).
const ANVIL_ENCHANTMENTS_1_12: &[(i16, i32, i32, u128)] = &[
    (0, 4, 1, 0x1b),
    (1, 4, 2, 0x1b),
    (2, 4, 2, 0x4),
    (3, 4, 4, 0x1b),
    (4, 4, 2, 0x1b),
    (5, 3, 4, 0x20),
    (6, 1, 4, 0x40),
    (7, 3, 8, 0x80),
    (8, 3, 4, 0x300),
    (9, 2, 4, 0x300),
    (10, 1, 8, 0x400),
    (16, 5, 1, 0x70000),
    (17, 5, 2, 0x70000),
    (18, 5, 2, 0x70000),
    (19, 2, 2, 0x80000),
    (20, 2, 4, 0x100000),
    (21, 3, 4, 0x200200000),
    (22, 3, 4, 0x400000),
    (32, 5, 1, 0x100000000),
    (33, 1, 8, 0x2000000a00200000),
    (34, 3, 2, 0x400000000),
    (35, 3, 4, 0xa00000000),
    (48, 5, 1, 0x1000000000000),
    (49, 2, 4, 0x2000000000000),
    (50, 1, 4, 0x4000000000000),
    (51, 1, 8, 0x400008000000000000),
    (61, 3, 4, 0x2000000200000000),
    (62, 3, 4, 0x4000000000000000),
    (70, 1, 4, 0x400008000000000000),
    (71, 1, 8, 0x800000000000000000),
];

/// The items each enchantment accepts (as [`ANVIL_ENCHANTMENTS_1_12`]): item id and damage bits.
const ENCHANTABLE_1_12: &[&[(i32, u16)]] = &[
    &[
        (298, 0xffff),
        (299, 0xffff),
        (300, 0xffff),
        (301, 0xffff),
        (302, 0xffff),
        (303, 0xffff),
        (304, 0xffff),
        (305, 0xffff),
        (306, 0xffff),
        (307, 0xffff),
        (308, 0xffff),
        (309, 0xffff),
        (310, 0xffff),
        (311, 0xffff),
        (312, 0xffff),
        (313, 0xffff),
        (314, 0xffff),
        (315, 0xffff),
        (316, 0xffff),
        (317, 0xffff),
    ],
    &[
        (298, 0xffff),
        (299, 0xffff),
        (300, 0xffff),
        (301, 0xffff),
        (302, 0xffff),
        (303, 0xffff),
        (304, 0xffff),
        (305, 0xffff),
        (306, 0xffff),
        (307, 0xffff),
        (308, 0xffff),
        (309, 0xffff),
        (310, 0xffff),
        (311, 0xffff),
        (312, 0xffff),
        (313, 0xffff),
        (314, 0xffff),
        (315, 0xffff),
        (316, 0xffff),
        (317, 0xffff),
    ],
    &[
        (301, 0xffff),
        (305, 0xffff),
        (309, 0xffff),
        (313, 0xffff),
        (317, 0xffff),
    ],
    &[
        (298, 0xffff),
        (299, 0xffff),
        (300, 0xffff),
        (301, 0xffff),
        (302, 0xffff),
        (303, 0xffff),
        (304, 0xffff),
        (305, 0xffff),
        (306, 0xffff),
        (307, 0xffff),
        (308, 0xffff),
        (309, 0xffff),
        (310, 0xffff),
        (311, 0xffff),
        (312, 0xffff),
        (313, 0xffff),
        (314, 0xffff),
        (315, 0xffff),
        (316, 0xffff),
        (317, 0xffff),
    ],
    &[
        (298, 0xffff),
        (299, 0xffff),
        (300, 0xffff),
        (301, 0xffff),
        (302, 0xffff),
        (303, 0xffff),
        (304, 0xffff),
        (305, 0xffff),
        (306, 0xffff),
        (307, 0xffff),
        (308, 0xffff),
        (309, 0xffff),
        (310, 0xffff),
        (311, 0xffff),
        (312, 0xffff),
        (313, 0xffff),
        (314, 0xffff),
        (315, 0xffff),
        (316, 0xffff),
        (317, 0xffff),
    ],
    &[
        (298, 0xffff),
        (302, 0xffff),
        (306, 0xffff),
        (310, 0xffff),
        (314, 0xffff),
    ],
    &[
        (298, 0xffff),
        (302, 0xffff),
        (306, 0xffff),
        (310, 0xffff),
        (314, 0xffff),
    ],
    &[
        (298, 0xffff),
        (299, 0xffff),
        (300, 0xffff),
        (301, 0xffff),
        (302, 0xffff),
        (303, 0xffff),
        (304, 0xffff),
        (305, 0xffff),
        (306, 0xffff),
        (307, 0xffff),
        (308, 0xffff),
        (309, 0xffff),
        (310, 0xffff),
        (311, 0xffff),
        (312, 0xffff),
        (313, 0xffff),
        (314, 0xffff),
        (315, 0xffff),
        (316, 0xffff),
        (317, 0xffff),
    ],
    &[
        (301, 0xffff),
        (305, 0xffff),
        (309, 0xffff),
        (313, 0xffff),
        (317, 0xffff),
    ],
    &[
        (301, 0xffff),
        (305, 0xffff),
        (309, 0xffff),
        (313, 0xffff),
        (317, 0xffff),
    ],
    &[
        (86, 0xffff),
        (91, 0xffff),
        (298, 0xffff),
        (299, 0xffff),
        (300, 0xffff),
        (301, 0xffff),
        (302, 0xffff),
        (303, 0xffff),
        (304, 0xffff),
        (305, 0xffff),
        (306, 0xffff),
        (307, 0xffff),
        (308, 0xffff),
        (309, 0xffff),
        (310, 0xffff),
        (311, 0xffff),
        (312, 0xffff),
        (313, 0xffff),
        (314, 0xffff),
        (315, 0xffff),
        (316, 0xffff),
        (317, 0xffff),
        (397, 0xffff),
        (443, 0xffff),
    ],
    &[
        (258, 0xffff),
        (267, 0xffff),
        (268, 0xffff),
        (271, 0xffff),
        (272, 0xffff),
        (275, 0xffff),
        (276, 0xffff),
        (279, 0xffff),
        (283, 0xffff),
        (286, 0xffff),
    ],
    &[
        (258, 0xffff),
        (267, 0xffff),
        (268, 0xffff),
        (271, 0xffff),
        (272, 0xffff),
        (275, 0xffff),
        (276, 0xffff),
        (279, 0xffff),
        (283, 0xffff),
        (286, 0xffff),
    ],
    &[
        (258, 0xffff),
        (267, 0xffff),
        (268, 0xffff),
        (271, 0xffff),
        (272, 0xffff),
        (275, 0xffff),
        (276, 0xffff),
        (279, 0xffff),
        (283, 0xffff),
        (286, 0xffff),
    ],
    &[
        (267, 0xffff),
        (268, 0xffff),
        (272, 0xffff),
        (276, 0xffff),
        (283, 0xffff),
    ],
    &[
        (267, 0xffff),
        (268, 0xffff),
        (272, 0xffff),
        (276, 0xffff),
        (283, 0xffff),
    ],
    &[
        (267, 0xffff),
        (268, 0xffff),
        (272, 0xffff),
        (276, 0xffff),
        (283, 0xffff),
    ],
    &[
        (267, 0xffff),
        (268, 0xffff),
        (272, 0xffff),
        (276, 0xffff),
        (283, 0xffff),
    ],
    &[
        (256, 0xffff),
        (257, 0xffff),
        (258, 0xffff),
        (269, 0xffff),
        (270, 0xffff),
        (271, 0xffff),
        (273, 0xffff),
        (274, 0xffff),
        (275, 0xffff),
        (277, 0xffff),
        (278, 0xffff),
        (279, 0xffff),
        (284, 0xffff),
        (285, 0xffff),
        (286, 0xffff),
        (359, 0xffff),
    ],
    &[
        (256, 0xffff),
        (257, 0xffff),
        (258, 0xffff),
        (269, 0xffff),
        (270, 0xffff),
        (271, 0xffff),
        (273, 0xffff),
        (274, 0xffff),
        (275, 0xffff),
        (277, 0xffff),
        (278, 0xffff),
        (279, 0xffff),
        (284, 0xffff),
        (285, 0xffff),
        (286, 0xffff),
    ],
    &[
        (256, 0xffff),
        (257, 0xffff),
        (258, 0xffff),
        (259, 0xffff),
        (261, 0xffff),
        (267, 0xffff),
        (268, 0xffff),
        (269, 0xffff),
        (270, 0xffff),
        (271, 0xffff),
        (272, 0xffff),
        (273, 0xffff),
        (274, 0xffff),
        (275, 0xffff),
        (276, 0xffff),
        (277, 0xffff),
        (278, 0xffff),
        (279, 0xffff),
        (283, 0xffff),
        (284, 0xffff),
        (285, 0xffff),
        (286, 0xffff),
        (290, 0xffff),
        (291, 0xffff),
        (292, 0xffff),
        (293, 0xffff),
        (294, 0xffff),
        (298, 0xffff),
        (299, 0xffff),
        (300, 0xffff),
        (301, 0xffff),
        (302, 0xffff),
        (303, 0xffff),
        (304, 0xffff),
        (305, 0xffff),
        (306, 0xffff),
        (307, 0xffff),
        (308, 0xffff),
        (309, 0xffff),
        (310, 0xffff),
        (311, 0xffff),
        (312, 0xffff),
        (313, 0xffff),
        (314, 0xffff),
        (315, 0xffff),
        (316, 0xffff),
        (317, 0xffff),
        (346, 0xffff),
        (359, 0xffff),
        (398, 0xffff),
        (442, 0xffff),
        (443, 0xffff),
    ],
    &[
        (256, 0xffff),
        (257, 0xffff),
        (258, 0xffff),
        (269, 0xffff),
        (270, 0xffff),
        (271, 0xffff),
        (273, 0xffff),
        (274, 0xffff),
        (275, 0xffff),
        (277, 0xffff),
        (278, 0xffff),
        (279, 0xffff),
        (284, 0xffff),
        (285, 0xffff),
        (286, 0xffff),
    ],
    &[(261, 0xffff)],
    &[(261, 0xffff)],
    &[(261, 0xffff)],
    &[(261, 0xffff)],
    &[(346, 0xffff)],
    &[(346, 0xffff)],
    &[
        (256, 0xffff),
        (257, 0xffff),
        (258, 0xffff),
        (259, 0xffff),
        (261, 0xffff),
        (267, 0xffff),
        (268, 0xffff),
        (269, 0xffff),
        (270, 0xffff),
        (271, 0xffff),
        (272, 0xffff),
        (273, 0xffff),
        (274, 0xffff),
        (275, 0xffff),
        (276, 0xffff),
        (277, 0xffff),
        (278, 0xffff),
        (279, 0xffff),
        (283, 0xffff),
        (284, 0xffff),
        (285, 0xffff),
        (286, 0xffff),
        (290, 0xffff),
        (291, 0xffff),
        (292, 0xffff),
        (293, 0xffff),
        (294, 0xffff),
        (298, 0xffff),
        (299, 0xffff),
        (300, 0xffff),
        (301, 0xffff),
        (302, 0xffff),
        (303, 0xffff),
        (304, 0xffff),
        (305, 0xffff),
        (306, 0xffff),
        (307, 0xffff),
        (308, 0xffff),
        (309, 0xffff),
        (310, 0xffff),
        (311, 0xffff),
        (312, 0xffff),
        (313, 0xffff),
        (314, 0xffff),
        (315, 0xffff),
        (316, 0xffff),
        (317, 0xffff),
        (346, 0xffff),
        (359, 0xffff),
        (398, 0xffff),
        (442, 0xffff),
        (443, 0xffff),
    ],
    &[
        (86, 0xffff),
        (91, 0xffff),
        (256, 0xffff),
        (257, 0xffff),
        (258, 0xffff),
        (259, 0xffff),
        (261, 0xffff),
        (267, 0xffff),
        (268, 0xffff),
        (269, 0xffff),
        (270, 0xffff),
        (271, 0xffff),
        (272, 0xffff),
        (273, 0xffff),
        (274, 0xffff),
        (275, 0xffff),
        (276, 0xffff),
        (277, 0xffff),
        (278, 0xffff),
        (279, 0xffff),
        (283, 0xffff),
        (284, 0xffff),
        (285, 0xffff),
        (286, 0xffff),
        (290, 0xffff),
        (291, 0xffff),
        (292, 0xffff),
        (293, 0xffff),
        (294, 0xffff),
        (298, 0xffff),
        (299, 0xffff),
        (300, 0xffff),
        (301, 0xffff),
        (302, 0xffff),
        (303, 0xffff),
        (304, 0xffff),
        (305, 0xffff),
        (306, 0xffff),
        (307, 0xffff),
        (308, 0xffff),
        (309, 0xffff),
        (310, 0xffff),
        (311, 0xffff),
        (312, 0xffff),
        (313, 0xffff),
        (314, 0xffff),
        (315, 0xffff),
        (316, 0xffff),
        (317, 0xffff),
        (346, 0xffff),
        (359, 0xffff),
        (397, 0xffff),
        (398, 0xffff),
        (442, 0xffff),
        (443, 0xffff),
    ],
];

/// Damageable items and what repairs them (item id, damage bits), sorted.
const REPAIR_1_12: &[(i32, &[(i32, u16)])] = &[
    (256, &[(265, 0xffff)]),
    (257, &[(265, 0xffff)]),
    (258, &[(265, 0xffff)]),
    (259, &[]),
    (261, &[]),
    (267, &[(265, 0xffff)]),
    (268, &[(5, 0xffff)]),
    (269, &[(5, 0xffff)]),
    (270, &[(5, 0xffff)]),
    (271, &[(5, 0xffff)]),
    (272, &[(4, 0xffff)]),
    (273, &[(4, 0xffff)]),
    (274, &[(4, 0xffff)]),
    (275, &[(4, 0xffff)]),
    (276, &[(264, 0xffff)]),
    (277, &[(264, 0xffff)]),
    (278, &[(264, 0xffff)]),
    (279, &[(264, 0xffff)]),
    (283, &[(266, 0xffff)]),
    (284, &[(266, 0xffff)]),
    (285, &[(266, 0xffff)]),
    (286, &[(266, 0xffff)]),
    (290, &[]),
    (291, &[]),
    (292, &[]),
    (293, &[]),
    (294, &[]),
    (298, &[(334, 0xffff)]),
    (299, &[(334, 0xffff)]),
    (300, &[(334, 0xffff)]),
    (301, &[(334, 0xffff)]),
    (302, &[(265, 0xffff)]),
    (303, &[(265, 0xffff)]),
    (304, &[(265, 0xffff)]),
    (305, &[(265, 0xffff)]),
    (306, &[(265, 0xffff)]),
    (307, &[(265, 0xffff)]),
    (308, &[(265, 0xffff)]),
    (309, &[(265, 0xffff)]),
    (310, &[(264, 0xffff)]),
    (311, &[(264, 0xffff)]),
    (312, &[(264, 0xffff)]),
    (313, &[(264, 0xffff)]),
    (314, &[(266, 0xffff)]),
    (315, &[(266, 0xffff)]),
    (316, &[(266, 0xffff)]),
    (317, &[(266, 0xffff)]),
    (346, &[]),
    (359, &[]),
    (398, &[]),
    (442, &[(5, 0xffff)]),
    (443, &[(334, 0xffff)]),
];

/// 1.8.9 enchantments: id, max level, the anvil's cost by weight (10 → 1, 5 → 2, 2 → 4, 1 → 8,
/// others 0), the ids it cannot combine with (bits by id, itself included). Read off the jar.
const ANVIL_ENCHANTMENTS_1_8: &[(i16, i32, i32, u128)] = &[
    (0, 4, 1, 0x1b),
    (1, 4, 2, 0x1b),
    (2, 4, 2, 0x4),
    (3, 4, 4, 0x1b),
    (4, 4, 2, 0x1b),
    (5, 3, 4, 0x20),
    (6, 1, 4, 0x40),
    (7, 3, 8, 0x80),
    (8, 3, 4, 0x100),
    (16, 5, 1, 0x70000),
    (17, 5, 2, 0x70000),
    (18, 5, 2, 0x70000),
    (19, 2, 2, 0x80000),
    (20, 2, 4, 0x100000),
    (21, 3, 4, 0x200200000),
    (32, 5, 1, 0x100000000),
    (33, 1, 8, 0xa00000000),
    (34, 3, 2, 0x400000000),
    (35, 3, 4, 0xa00000000),
    (48, 5, 1, 0x1000000000000),
    (49, 2, 4, 0x2000000000000),
    (50, 1, 4, 0x4000000000000),
    (51, 1, 8, 0x8000000000000),
    (61, 3, 4, 0x2000000200000000),
    (62, 3, 4, 0x4000000000000000),
];

/// The items each 1.8.9 enchantment accepts: item id and damage bits.
const ENCHANTABLE_1_8: &[&[(i32, u16)]] = &[
    &[
        (298, 0xffff),
        (299, 0xffff),
        (300, 0xffff),
        (301, 0xffff),
        (302, 0xffff),
        (303, 0xffff),
        (304, 0xffff),
        (305, 0xffff),
        (306, 0xffff),
        (307, 0xffff),
        (308, 0xffff),
        (309, 0xffff),
        (310, 0xffff),
        (311, 0xffff),
        (312, 0xffff),
        (313, 0xffff),
        (314, 0xffff),
        (315, 0xffff),
        (316, 0xffff),
        (317, 0xffff),
    ],
    &[
        (298, 0xffff),
        (299, 0xffff),
        (300, 0xffff),
        (301, 0xffff),
        (302, 0xffff),
        (303, 0xffff),
        (304, 0xffff),
        (305, 0xffff),
        (306, 0xffff),
        (307, 0xffff),
        (308, 0xffff),
        (309, 0xffff),
        (310, 0xffff),
        (311, 0xffff),
        (312, 0xffff),
        (313, 0xffff),
        (314, 0xffff),
        (315, 0xffff),
        (316, 0xffff),
        (317, 0xffff),
    ],
    &[
        (301, 0xffff),
        (305, 0xffff),
        (309, 0xffff),
        (313, 0xffff),
        (317, 0xffff),
    ],
    &[
        (298, 0xffff),
        (299, 0xffff),
        (300, 0xffff),
        (301, 0xffff),
        (302, 0xffff),
        (303, 0xffff),
        (304, 0xffff),
        (305, 0xffff),
        (306, 0xffff),
        (307, 0xffff),
        (308, 0xffff),
        (309, 0xffff),
        (310, 0xffff),
        (311, 0xffff),
        (312, 0xffff),
        (313, 0xffff),
        (314, 0xffff),
        (315, 0xffff),
        (316, 0xffff),
        (317, 0xffff),
    ],
    &[
        (298, 0xffff),
        (299, 0xffff),
        (300, 0xffff),
        (301, 0xffff),
        (302, 0xffff),
        (303, 0xffff),
        (304, 0xffff),
        (305, 0xffff),
        (306, 0xffff),
        (307, 0xffff),
        (308, 0xffff),
        (309, 0xffff),
        (310, 0xffff),
        (311, 0xffff),
        (312, 0xffff),
        (313, 0xffff),
        (314, 0xffff),
        (315, 0xffff),
        (316, 0xffff),
        (317, 0xffff),
    ],
    &[
        (298, 0xffff),
        (302, 0xffff),
        (306, 0xffff),
        (310, 0xffff),
        (314, 0xffff),
    ],
    &[
        (298, 0xffff),
        (302, 0xffff),
        (306, 0xffff),
        (310, 0xffff),
        (314, 0xffff),
    ],
    &[
        (298, 0xffff),
        (299, 0xffff),
        (300, 0xffff),
        (301, 0xffff),
        (302, 0xffff),
        (303, 0xffff),
        (304, 0xffff),
        (305, 0xffff),
        (306, 0xffff),
        (307, 0xffff),
        (308, 0xffff),
        (309, 0xffff),
        (310, 0xffff),
        (311, 0xffff),
        (312, 0xffff),
        (313, 0xffff),
        (314, 0xffff),
        (315, 0xffff),
        (316, 0xffff),
        (317, 0xffff),
    ],
    &[
        (301, 0xffff),
        (305, 0xffff),
        (309, 0xffff),
        (313, 0xffff),
        (317, 0xffff),
    ],
    &[
        (258, 0xffff),
        (267, 0xffff),
        (268, 0xffff),
        (271, 0xffff),
        (272, 0xffff),
        (275, 0xffff),
        (276, 0xffff),
        (279, 0xffff),
        (283, 0xffff),
        (286, 0xffff),
    ],
    &[
        (258, 0xffff),
        (267, 0xffff),
        (268, 0xffff),
        (271, 0xffff),
        (272, 0xffff),
        (275, 0xffff),
        (276, 0xffff),
        (279, 0xffff),
        (283, 0xffff),
        (286, 0xffff),
    ],
    &[
        (258, 0xffff),
        (267, 0xffff),
        (268, 0xffff),
        (271, 0xffff),
        (272, 0xffff),
        (275, 0xffff),
        (276, 0xffff),
        (279, 0xffff),
        (283, 0xffff),
        (286, 0xffff),
    ],
    &[
        (267, 0xffff),
        (268, 0xffff),
        (272, 0xffff),
        (276, 0xffff),
        (283, 0xffff),
    ],
    &[
        (267, 0xffff),
        (268, 0xffff),
        (272, 0xffff),
        (276, 0xffff),
        (283, 0xffff),
    ],
    &[
        (267, 0xffff),
        (268, 0xffff),
        (272, 0xffff),
        (276, 0xffff),
        (283, 0xffff),
    ],
    &[
        (256, 0xffff),
        (257, 0xffff),
        (258, 0xffff),
        (269, 0xffff),
        (270, 0xffff),
        (271, 0xffff),
        (273, 0xffff),
        (274, 0xffff),
        (275, 0xffff),
        (277, 0xffff),
        (278, 0xffff),
        (279, 0xffff),
        (284, 0xffff),
        (285, 0xffff),
        (286, 0xffff),
        (359, 0xffff),
    ],
    &[
        (256, 0xffff),
        (257, 0xffff),
        (258, 0xffff),
        (269, 0xffff),
        (270, 0xffff),
        (271, 0xffff),
        (273, 0xffff),
        (274, 0xffff),
        (275, 0xffff),
        (277, 0xffff),
        (278, 0xffff),
        (279, 0xffff),
        (284, 0xffff),
        (285, 0xffff),
        (286, 0xffff),
        (359, 0xffff),
    ],
    &[
        (256, 0xffff),
        (257, 0xffff),
        (258, 0xffff),
        (259, 0xffff),
        (261, 0xffff),
        (267, 0xffff),
        (268, 0xffff),
        (269, 0xffff),
        (270, 0xffff),
        (271, 0xffff),
        (272, 0xffff),
        (273, 0xffff),
        (274, 0xffff),
        (275, 0xffff),
        (276, 0xffff),
        (277, 0xffff),
        (278, 0xffff),
        (279, 0xffff),
        (283, 0xffff),
        (284, 0xffff),
        (285, 0xffff),
        (286, 0xffff),
        (290, 0xffff),
        (291, 0xffff),
        (292, 0xffff),
        (293, 0xffff),
        (294, 0xffff),
        (298, 0xffff),
        (299, 0xffff),
        (300, 0xffff),
        (301, 0xffff),
        (302, 0xffff),
        (303, 0xffff),
        (304, 0xffff),
        (305, 0xffff),
        (306, 0xffff),
        (307, 0xffff),
        (308, 0xffff),
        (309, 0xffff),
        (310, 0xffff),
        (311, 0xffff),
        (312, 0xffff),
        (313, 0xffff),
        (314, 0xffff),
        (315, 0xffff),
        (316, 0xffff),
        (317, 0xffff),
        (346, 0xffff),
        (359, 0xffff),
        (398, 0xffff),
    ],
    &[
        (256, 0xffff),
        (257, 0xffff),
        (258, 0xffff),
        (269, 0xffff),
        (270, 0xffff),
        (271, 0xffff),
        (273, 0xffff),
        (274, 0xffff),
        (275, 0xffff),
        (277, 0xffff),
        (278, 0xffff),
        (279, 0xffff),
        (284, 0xffff),
        (285, 0xffff),
        (286, 0xffff),
    ],
    &[(261, 0xffff)],
    &[(261, 0xffff)],
    &[(261, 0xffff)],
    &[(261, 0xffff)],
    &[(346, 0xffff)],
    &[(346, 0xffff)],
];

/// 1.8.9 damageable items and what repairs them (item id, damage bits), sorted.
const REPAIR_1_8: &[(i32, &[(i32, u16)])] = &[
    (256, &[(265, 0xffff)]),
    (257, &[(265, 0xffff)]),
    (258, &[(265, 0xffff)]),
    (259, &[]),
    (261, &[]),
    (267, &[(265, 0xffff)]),
    (268, &[(5, 0xffff)]),
    (269, &[(5, 0xffff)]),
    (270, &[(5, 0xffff)]),
    (271, &[(5, 0xffff)]),
    (272, &[(4, 0xffff)]),
    (273, &[(4, 0xffff)]),
    (274, &[(4, 0xffff)]),
    (275, &[(4, 0xffff)]),
    (276, &[(264, 0xffff)]),
    (277, &[(264, 0xffff)]),
    (278, &[(264, 0xffff)]),
    (279, &[(264, 0xffff)]),
    (283, &[(266, 0xffff)]),
    (284, &[(266, 0xffff)]),
    (285, &[(266, 0xffff)]),
    (286, &[(266, 0xffff)]),
    (290, &[]),
    (291, &[]),
    (292, &[]),
    (293, &[]),
    (294, &[]),
    (298, &[(334, 0xffff)]),
    (299, &[(334, 0xffff)]),
    (300, &[(334, 0xffff)]),
    (301, &[(334, 0xffff)]),
    (302, &[(265, 0xffff)]),
    (303, &[(265, 0xffff)]),
    (304, &[(265, 0xffff)]),
    (305, &[(265, 0xffff)]),
    (306, &[(265, 0xffff)]),
    (307, &[(265, 0xffff)]),
    (308, &[(265, 0xffff)]),
    (309, &[(265, 0xffff)]),
    (310, &[(264, 0xffff)]),
    (311, &[(264, 0xffff)]),
    (312, &[(264, 0xffff)]),
    (313, &[(264, 0xffff)]),
    (314, &[(266, 0xffff)]),
    (315, &[(266, 0xffff)]),
    (316, &[(266, 0xffff)]),
    (317, &[(266, 0xffff)]),
    (346, &[]),
    (359, &[]),
    (398, &[]),
];

/// An enchantment row: raw id, max level, cost weight, incompatible ids (bits).
type EnchRow = (i16, i32, i32, u128);
/// Item ids with damage bits.
type ItemMasks = &'static [(i32, u16)];

/// The version's tables (1.8.9 or 1.12.2).
fn tables(v: Version) -> (&'static [EnchRow], &'static [ItemMasks]) {
    if v == Version::V1_8 {
        (ANVIL_ENCHANTMENTS_1_8, ENCHANTABLE_1_8)
    } else {
        (ANVIL_ENCHANTMENTS_1_12, ENCHANTABLE_1_12)
    }
}

fn ench_index(v: Version, id: i16) -> Option<usize> {
    tables(v).0.iter().position(|e| e.0 == id)
}

fn in_masks(list: &[(i32, u16)], s: &Stack) -> bool {
    let Ok(k) = list.binary_search_by_key(&s.id, |e| e.0) else {
        return false;
    };
    match u16::try_from(s.damage) {
        Ok(d) if d < 16 => list[k].1 & (1 << d) != 0,
        _ => list[k].1 == 0xffff,
    }
}

fn acceptable(v: Version, e: usize, s: &Stack) -> bool {
    in_masks(tables(v).1[e], s)
}

fn can_combine(v: Version, e: usize, o: usize) -> bool {
    let t = tables(v).0;
    t[e].3 & (1u128 << t[o].0) == 0
}

fn can_repair(v: Version, id: i32, with: &Stack) -> bool {
    let r = if v == Version::V1_8 {
        REPAIR_1_8
    } else {
        REPAIR_1_12
    };
    r.binary_search_by_key(&id, |x| x.0)
        .is_ok_and(|k| in_masks(r[k].1, with))
}

/// `EnchantmentHelper.get`: `StoredEnchantments` of a book, else `ench` (numeric ids; an id
/// without an enchantment is a `null` key vanilla goes on with: not modelled).
fn enchantments(v: Version, s: &Stack, t: Option<&Compound>) -> R<Vec<(usize, i32)>> {
    let key = if s.id == 403 {
        "StoredEnchantments"
    } else {
        "ench"
    };
    let mut out: Vec<(usize, i32)> = Vec::new();
    let Some(t) = t else { return Ok(out) };
    // a list of something else (1.8 reads a book's entries as id 0): not modelled
    if let Some(Tag::List(l)) = last(t, key)
        && !l.items.is_empty()
        && l.elem_type != ghast_protocol::nbt::tag::COMPOUND
    {
        return Err(Unpredictable);
    }
    for e in n::compound_list(t, key) {
        let short = |k: &str| last(&e, k).and_then(number).unwrap_or(0) as i16;
        let id = ench_index(v, short("id")).ok_or(Unpredictable)?;
        let lvl = i32::from(short("lvl"));
        match out.iter_mut().find(|x| x.0 == id) {
            Some(x) => x.1 = lvl,
            None => out.push((id, lvl)),
        }
    }
    Ok(out)
}

fn entry(v: Version, e: usize, lvl: i32) -> Compound {
    Compound(vec![
        ("id".into(), Tag::Short(tables(v).0[e].0)),
        ("lvl".into(), Tag::Short(lvl as i16)),
    ])
}

/// `EnchantedBookItem.addEnchantment`.
fn add_stored(v: Version, t: &mut Option<Compound>, e: usize, lvl: i32) {
    let mut list = t
        .as_ref()
        .map(|c| n::compound_list(c, "StoredEnchantments"))
        .unwrap_or_default();
    let same =
        |c: &Compound| last(c, "id").and_then(number).map(|x| x as i16) == Some(tables(v).0[e].0);
    match list.iter_mut().find(|c| same(c)) {
        Some(c) => {
            if i32::from(last(c, "lvl").and_then(number).unwrap_or(0) as i16) < lvl {
                n::put(c, "lvl", Tag::Short(lvl as i16));
            }
        }
        None => list.push(entry(v, e, lvl)),
    }
    n::put(
        t.get_or_insert_with(Compound::default),
        "StoredEnchantments",
        n::list_tag(list),
    );
}

fn custom_name(t: Option<&Compound>) -> Option<&str> {
    match t.and_then(|c| last(c, "display")) {
        Some(Tag::Compound(d)) => match last(d, "Name") {
            Some(Tag::String(s)) => Some(s),
            _ => None,
        },
        _ => None,
    }
}

/// `getCustomName()` when ghast can know it (`Ok(None)`: the translated item name; `Err`: a
/// `LocName`, translated by the client).
fn shown_name(t: Option<&Compound>, v12: bool) -> R<Option<String>> {
    if let Some(n) = custom_name(t) {
        return Ok(Some(n.to_owned()));
    }
    if let Some(Tag::Compound(d)) = t.and_then(|c| last(c, "display"))
        && matches!(last(d, "LocName"), Some(Tag::String(_)))
        && v12
    {
        return Err(Unpredictable);
    }
    Ok(None)
}

fn damageable(v: Version, id: i32, t: Option<&Compound>) -> bool {
    v.max_durability(id).is_some()
        && !t.is_some_and(|c| {
            last(c, "Unbreakable")
                .and_then(number)
                .is_some_and(|x| x as i8 != 0)
        })
}

fn blank(s: Option<&str>) -> bool {
    s.is_none_or(|s| s.chars().all(char::is_whitespace))
}

struct Outcome {
    out: Option<Stack>,
    cost: i32,
    usage: i32,
    synced: bool,
}

fn update_result(
    v: Version,
    a: Option<&Stack>,
    b: Option<&Stack>,
    name: Option<&str>,
    creative: bool,
) -> R<Outcome> {
    let empty = |usage| Outcome {
        out: None,
        cost: 0,
        usage,
        synced: false,
    };
    let Some(a) = a else {
        return Ok(empty(0));
    };
    let ta = n::tree(a)?;
    let tb = b.map(n::tree).transpose()?.flatten();
    let max = v.max_durability(a.id).unwrap_or(0);
    let mut t2 = ta.clone();
    let mut dmg = i32::from(a.damage);
    let mut map = enchantments(v, a, ta.as_ref())?;
    let mut i = 0i32;
    let j = n::repair_cost(ta.as_ref()) + b.map_or(0, |_| n::repair_cost(tb.as_ref()));
    let mut k = 0;
    let mut usage = 0;
    if let Some(b) = b {
        let book = b.id == 403
            && tb
                .as_ref()
                .is_some_and(|t| !n::compound_list(t, "StoredEnchantments").is_empty());
        if damageable(v, a.id, t2.as_ref()) && can_repair(v, a.id, b) {
            let mut l = dmg.min(max / 4);
            if l <= 0 {
                return Ok(empty(0));
            }
            let mut m = 0;
            while l > 0 && m < b.count {
                dmg = (dmg - l).max(0);
                i += 1;
                l = dmg.min(max / 4);
                m += 1;
            }
            usage = m;
        } else {
            if !book && (a.id != b.id || !damageable(v, a.id, t2.as_ref())) {
                return Ok(empty(0));
            }
            if damageable(v, a.id, t2.as_ref()) && !book {
                let o = max - i32::from(a.damage);
                let p = v.max_durability(b.id).unwrap_or(0) - i32::from(b.damage);
                let q = p + max * 12 / 100;
                let s = (max - (o + q)).max(0);
                if s < dmg {
                    dmg = s;
                    i += 2;
                }
            }
            let mut any_ok = false;
            let mut any_bad = false;
            for (e, lvl) in enchantments(v, b, tb.as_ref())? {
                let t = map.iter().find(|x| x.0 == e).map_or(0, |x| x.1);
                let mut u = if t == lvl { lvl + 1 } else { lvl.max(t) };
                let mut ok = acceptable(v, e, a) || creative || a.id == 403;
                for &(e2, _) in &map {
                    if e2 != e && !can_combine(v, e, e2) {
                        ok = false;
                        i += 1;
                    }
                }
                if !ok {
                    any_bad = true;
                } else {
                    any_ok = true;
                    let (_, maxl, weight, _) = tables(v).0[e];
                    u = u.min(maxl);
                    match map.iter_mut().find(|x| x.0 == e) {
                        Some(x) => x.1 = u,
                        None => map.push((e, u)),
                    }
                    let w = if book { (weight / 2).max(1) } else { weight };
                    i += w * u;
                    // (1.12: a stack of several items can only be renamed)
                    if a.count > 1 && v == Version::V1_12 {
                        i = 40;
                    }
                }
            }
            // (1.12: a book or item whose enchantments all clash gives nothing)
            if any_bad && !any_ok && v == Version::V1_12 {
                return Ok(empty(usage));
            }
        }
    }
    if blank(name) {
        if custom_name(ta.as_ref()).is_some() {
            k = 1;
            i += 1;
            // removeCustomName
            if let Some(c) = &mut t2
                && let Some(Tag::Compound(d)) =
                    c.0.iter_mut()
                        .rev()
                        .find(|x| x.0 == "display")
                        .map(|x| &mut x.1)
            {
                n::remove(d, "Name");
                if d.0.is_empty() {
                    n::remove(c, "display");
                }
            }
            if t2.as_ref().is_some_and(|c| c.0.is_empty()) {
                t2 = None;
            }
        }
    } else {
        let name = name.unwrap_or_default();
        // getCustomName(): the custom name, or the translated item name (taken as different)
        if shown_name(ta.as_ref(), v == Version::V1_12)?.as_deref() != Some(name) {
            k = 1;
            i += 1;
            let c = t2.get_or_insert_with(Compound::default);
            let mut d = match last(c, "display") {
                Some(Tag::Compound(d)) => d.clone(),
                _ => Compound::default(),
            };
            n::put(&mut d, "Name", Tag::String(name.to_owned()));
            n::put(c, "display", Tag::Compound(d));
        }
    }
    let mut cost = j + i;
    let mut keep = i > 0;
    if k == i && k > 0 && cost >= 40 {
        cost = 39;
    }
    if cost >= 40 && !creative {
        keep = false;
    }
    let out = if keep {
        let mut w = n::repair_cost(t2.as_ref());
        if b.is_some() && w < n::repair_cost(tb.as_ref()) {
            w = n::repair_cost(tb.as_ref());
        }
        // (1.8: the prior work penalty grows on every use)
        if k != i || k == 0 || v == Version::V1_8 {
            w = w * 2 + 1;
        }
        n::put(
            t2.get_or_insert_with(Compound::default),
            "RepairCost",
            Tag::Int(w),
        );
        // EnchantmentHelper.set(map, stack)
        let book = a.id == 403;
        let mut list = Vec::new();
        for &(e, lvl) in &map {
            list.push(entry(v, e, lvl));
            if book {
                add_stored(v, &mut t2, e, lvl);
            }
        }
        if list.is_empty() {
            if let Some(c) = &mut t2 {
                n::remove(c, "ench");
            }
        } else if !book {
            n::put(
                t2.get_or_insert_with(Compound::default),
                "ench",
                n::list_tag(list),
            );
        }
        // 1.8's compounds are hash maps: written in their iteration order
        let t2 = match t2 {
            Some(c) if v == Version::V1_8 => {
                let mut t = Tag::Compound(c);
                crate::crafting::java_order(&mut t);
                match t {
                    Tag::Compound(c) => Some(c),
                    _ => unreachable!(),
                }
            }
            t => t,
        };
        Some(n::with_tree(a.id, a.count, t2)?.with_damage(dmg as i16))
    } else {
        None
    };
    Ok(Outcome {
        out,
        cost,
        usage,
        synced: true,
    })
}

impl Sim<'_> {
    fn anvil12_state(&self) -> Anvil {
        match self.m.station.as_deref() {
            Some(crate::station::Station::Anvil(a)) => a.clone(),
            _ => Anvil::default(),
        }
    }

    fn set_anvil12_state(&mut self, a: Anvil) {
        if let Some(crate::station::Station::Anvil(s)) = self.m.station.as_deref_mut() {
            *s = a;
        }
    }

    /// 1.12.2 `updateResult`, then (when it syncs) the screen's slot-0 listener.
    pub(crate) fn anvil12_update(&mut self) {
        let mut st = self.anvil12_state();
        let r = update_result(
            self.v,
            self.m.container[0].as_ref(),
            self.m.container[1].as_ref(),
            st.name.as_deref(),
            self.m.creative,
        );
        let Ok(o) = r else {
            self.m.container_result_known = false;
            return;
        };
        st.cost = o.cost;
        st.usage = o.usage;
        self.m.container[2] = o.out;
        self.m.container_result_known = true;
        self.set_anvil12_state(st);
        if o.synced {
            self.anvil12_sync();
        }
    }

    /// `sendContentUpdates` as the 1.12 anvil screen sees it: slot 0 changed → the field gets
    /// `getCustomName()` (35 characters) → `sendRenameUpdates` (also for an empty name).
    pub(crate) fn anvil12_sync(&mut self) {
        let mut st = self.anvil12_state();
        let cur = self.m.container[0].clone();
        if crate::stack::stacks_equal(cur.as_ref(), st.tracked.as_ref()) {
            return;
        }
        st.tracked.clone_from(&cur);
        self.set_anvil12_state(st);
        let Some(cur) = cur else { return };
        let Ok(t) = n::tree(&cur) else {
            self.m.container_result_known = false;
            return;
        };
        // the field's length: 35 (1.12), 30 (1.8)
        let max = if self.v == Version::V1_8 { 30 } else { 35 };
        let text = match shown_name(t.as_ref(), self.v == Version::V1_12) {
            // the translated item name equals itself: "" (taken as fitting the field)
            Ok(None) => String::new(),
            Ok(Some(s)) => s.chars().take(max).collect(),
            Err(_) => {
                self.m.container_result_known = false;
                return;
            }
        };
        self.anvil12_set_name(text);
    }

    /// `rename(string)` + `MC|ItemName`.
    pub(crate) fn anvil12_set_name(&mut self, name: String) {
        let mut st = self.anvil12_state();
        st.name = Some(name.clone());
        self.set_anvil12_state(st);
        self.m.screen_out.push(crate::Outgoing::RenameItem(name));
        self.anvil12_update();
    }
}

#[cfg(all(test, feature = "v1_8", feature = "v1_12"))]
mod tests {
    use super::*;

    /// The tables as the jar has them (the anvil facts lines of the goldens).
    #[test]
    fn anvil_facts_1_12_2() {
        check(Version::V1_12, "1.12.2");
    }

    #[test]
    fn anvil_facts_1_8_9() {
        check(Version::V1_8, "1.8.9");
    }

    fn check(v: Version, dir: &str) {
        use std::io::BufRead;
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/golden")
            .join(dir)
            .join("clicks_menus_seed1.jsonl.gz");
        let f = std::fs::File::open(&path).unwrap();
        let r = std::io::BufReader::new(flate2::read::GzDecoder::new(f));
        for line in r.lines() {
            let line = line.unwrap();
            if !line.starts_with("{\"facts\":\"anvil\"") {
                continue;
            }
            let c: serde_json::Value = serde_json::from_str(&line).unwrap();
            let es = c["enchantments"].as_array().unwrap();
            assert_eq!(es.len(), tables(v).0.len());
            for (k, e) in es.iter().enumerate() {
                assert_eq!(i64::from(tables(v).0[k].0), e[0].as_i64().unwrap());
                for (o, oe) in es.iter().enumerate() {
                    let bad = e[3]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|x| x.as_i64() == oe[0].as_i64());
                    assert_eq!(can_combine(v, k, o), !bad, "{k} {o}");
                }
                let want: std::collections::HashMap<i64, u64> = e[4]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|x| (x[0].as_i64().unwrap(), x[1].as_u64().unwrap()))
                    .collect();
                for id in 1..2300 {
                    for d in 0..16i16 {
                        let w = want.get(&i64::from(id)).is_some_and(|m| m & (1 << d) != 0);
                        assert_eq!(acceptable(v, k, &Stack::new(id, 1, d)), w, "{k} {id}:{d}");
                    }
                }
            }
            for rep in c["repair"].as_array().unwrap() {
                let id = rep[0].as_i64().unwrap() as i32;
                let want: std::collections::HashMap<i64, u64> = rep[1]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|x| (x[0].as_i64().unwrap(), x[1].as_u64().unwrap()))
                    .collect();
                for m in 1..2300 {
                    for d in 0..16i16 {
                        let w = want.get(&i64::from(m)).is_some_and(|x| x & (1 << d) != 0);
                        assert_eq!(can_repair(v, id, &Stack::new(m, 1, d)), w, "{id} {m}:{d}");
                    }
                }
            }
            return;
        }
        panic!("no anvil facts in {}", path.display());
    }
}
