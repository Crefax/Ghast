//! Version-independent item facts: the item kind and enchantment levels by name on every
//! version (1.8 / 1.12 numeric ids, 1.16.5 names, 26.x registry ids).

use ghast_inventory::{Inventory, ItemKind, Stack, Version};
use ghast_protocol::nbt::{Compound, List, NamedTag, RawNbt, Tag};

fn id(v: Version, name: &str) -> i32 {
    (0..2000i32)
        .find(|&i| Stack::new(i, 1, 0).item_name(v) == Some(name))
        .unwrap_or_else(|| panic!("{name} on {v:?}"))
}

#[test]
fn item_kinds_on_every_version() {
    for v in [
        Version::V1_8,
        Version::V1_12,
        Version::V1_16_5,
        Version::V26_1,
        Version::V26_2,
    ] {
        for (name, kind) in [
            ("diamond_sword", ItemKind::Sword),
            ("golden_sword", ItemKind::Sword),
            ("iron_axe", ItemKind::Axe),
            ("stone_pickaxe", ItemKind::Pickaxe),
            ("diamond_helmet", ItemKind::Helmet),
            ("bow", ItemKind::Bow),
            ("fishing_rod", ItemKind::FishingRod),
            ("golden_apple", ItemKind::Other),
        ] {
            let s = Stack::new(id(v, name), 1, 0);
            assert_eq!(s.kind(v), kind, "{name} {v:?}");
        }
        assert!(Stack::new(id(v, "wooden_sword"), 1, 0).is_sword(v));
    }
    assert_eq!(
        ItemKind::of_name("minecraft:netherite_leggings"),
        ItemKind::Leggings
    );
    assert_eq!(ItemKind::of_name("shield"), ItemKind::Shield);
}

/// A sword with enchantments as 1.8 / 1.12 (`ench`, numeric ids) or 1.16.5 (`Enchantments`,
/// names) NBT.
fn legacy_sword(v: Version, list: &[(i16, &str, i16)]) -> Stack {
    let modern_names = v == Version::V1_16_5;
    let items = list
        .iter()
        .map(|&(num, name, lvl)| {
            let id = if modern_names {
                Tag::String(format!("minecraft:{name}"))
            } else {
                Tag::Short(num)
            };
            Tag::Compound(Compound(vec![
                ("id".into(), id),
                ("lvl".into(), Tag::Short(lvl)),
            ]))
        })
        .collect();
    let key = if modern_names { "Enchantments" } else { "ench" };
    let nbt = RawNbt::from_named(&NamedTag {
        name: String::new(),
        tag: Tag::Compound(Compound(vec![(
            key.into(),
            Tag::List(List {
                elem_type: 10,
                items,
            }),
        )])),
    })
    .unwrap();
    Stack::new(id(v, "diamond_sword"), 1, 0).with_nbt(Some(nbt))
}

#[test]
fn enchantment_levels_by_name_legacy() {
    for v in [Version::V1_8, Version::V1_12, Version::V1_16_5] {
        let inv = Inventory::new(v).unwrap();
        let s = legacy_sword(v, &[(16, "sharpness", 5), (19, "knockback", 2)]);
        assert_eq!(inv.enchantment_level(&s, "sharpness"), 5, "{v:?}");
        assert_eq!(inv.enchantment_level(&s, "minecraft:knockback"), 2, "{v:?}");
        assert_eq!(inv.enchantment_level(&s, "fire_aspect"), 0, "{v:?}");
        assert_eq!(
            inv.enchantment_level(&Stack::new(1, 1, 0), "sharpness"),
            0,
            "{v:?}"
        );
    }
}
