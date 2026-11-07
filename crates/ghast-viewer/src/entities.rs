//! What the page needs to know about an entity: its 1.8 name, kind, size, the stack it shows,
//! its model variant, looks (baby, armour, armour stand pose) and state flags.
//!
//! Names are the 1.8 entity list names (`"PigZombie"`, `"MinecartChest"`, …), which the page's
//! models are keyed by. Angles on the wire are in the page's convention: radians, yaw 0 looks
//! towards −z and grows to the left (see [`yaw_rad`]).

use std::f64::consts::{PI, TAU};

use ghast_entity::{Entity, EntityClass, EntityType, MetaValue};

use crate::protocol::{Looks, NO_COLOR, kind, look};

/// Yaw in degrees (Minecraft) → the page's radians: `(π − yaw·π/180) mod 2π`.
#[must_use]
pub fn yaw_rad(deg: f32) -> f32 {
    (PI - f64::from(deg).to_radians()).rem_euclid(TAU) as f32
}

/// Pitch in degrees (Minecraft, down positive) → the page's radians (up positive) in `[−π, π)`.
#[must_use]
pub fn pitch_rad(deg: f32) -> f32 {
    ((-f64::from(deg)).to_radians() + PI).rem_euclid(TAU) as f32 - PI as f32
}

/// 1.8 object type → entity list name, width, height (each class's `setSize`).
const OBJECTS: &[(u8, &str, f32, f32)] = &[
    (1, "Boat", 1.5, 0.6),
    (2, "Item", 0.25, 0.25),
    (10, "MinecartRideable", 0.98, 0.7),
    (50, "PrimedTnt", 0.98, 0.98),
    (51, "EnderCrystal", 2.0, 2.0),
    (60, "Arrow", 0.5, 0.5),
    (61, "Snowball", 0.25, 0.25),
    (62, "ThrownEgg", 0.25, 0.25),
    (63, "Fireball", 1.0, 1.0),
    (64, "SmallFireball", 0.3125, 0.3125),
    (65, "ThrownEnderpearl", 0.25, 0.25),
    (66, "WitherSkull", 0.3125, 0.3125),
    (70, "FallingSand", 0.98, 0.98),
    (71, "ItemFrame", 0.5, 0.5),
    (72, "EyeOfEnderSignal", 0.25, 0.25),
    (73, "ThrownPotion", 0.25, 0.25),
    (75, "ThrownExpBottle", 0.25, 0.25),
    (76, "FireworksRocketEntity", 0.25, 0.25),
    (77, "LeashKnot", 0.5, 0.5),
    (78, "ArmorStand", 0.5, 1.975),
    (90, "FishingHook", 0.25, 0.25),
];

/// Minecart object data → class (the minecart type enum).
const MINECARTS: [&str; 7] = [
    "MinecartRideable",
    "MinecartChest",
    "MinecartFurnace",
    "MinecartTNT",
    "MinecartSpawner",
    "MinecartHopper",
    "MinecartCommandBlock",
];

/// A 1.8 object's name and size.
#[derive(Debug, Clone, PartialEq)]
pub struct ObjectInfo {
    pub name: String,
    pub width: f32,
    pub height: f32,
    pub ty: u8,
    pub data: i32,
}

/// The object an entity was spawned as (1.8 `spawn_entity`), `None` for anything else.
#[must_use]
pub fn object_info(e: &Entity) -> Option<ObjectInfo> {
    let EntityType::LegacyObject(ty) = e.entity_type() else {
        return None;
    };
    let data = e.data();
    Some(match OBJECTS.iter().find(|o| o.0 == ty) {
        Some(&(_, name, w, h)) => ObjectInfo {
            name: if ty == 10 {
                usize::try_from(data)
                    .ok()
                    .and_then(|i| MINECARTS.get(i))
                    .copied()
                    .unwrap_or(name)
                    .to_string()
            } else {
                name.to_string()
            },
            width: w,
            height: h,
            ty,
            data,
        },
        None => ObjectInfo {
            name: format!("object{ty}"),
            width: 0.5,
            height: 0.5,
            ty,
            data,
        },
    })
}

/// The entity's 1.8 name: an object's class name, a mob's entity list name, `"player"`,
/// `"XPOrb"`, `"Painting"`.
#[must_use]
pub fn name_of(e: &Entity) -> String {
    if let Some(o) = object_info(e) {
        return o.name;
    }
    match e.entity_type() {
        EntityType::Player => "player".into(),
        EntityType::ExperienceOrb => "XPOrb".into(),
        EntityType::Painting => "Painting".into(),
        t => ghast_entity::v1_8::info(t).map_or_else(String::new, |i| i.name.to_string()),
    }
}

/// The English display name of a mob (the page's fallback label).
#[must_use]
pub fn display_name(e: &Entity) -> &'static str {
    match e.entity_type() {
        EntityType::LegacyMob(_) => {
            ghast_entity::v1_8::info(e.entity_type()).map_or("", |i| i.display_name)
        }
        _ => "",
    }
}

/// The `SPAWN` kind.
#[must_use]
pub fn kind_of(e: &Entity) -> u8 {
    match e.class() {
        EntityClass::Player => kind::PLAYER,
        EntityClass::ExperienceOrb => kind::ORB,
        EntityClass::Item => kind::ITEM,
        EntityClass::Mob => kind::MOB,
        EntityClass::Object => kind::OBJECT,
        EntityClass::Painting => kind::OTHER,
        // Classes added for newer versions: drawn as a plain box.
        _ => kind::OTHER,
    }
}

/// Width and height for the page (the page draws a box of this size when it has no model).
#[must_use]
pub fn size_of_entity(e: &Entity) -> (f32, f32) {
    if let Some(o) = object_info(e) {
        return (o.width, o.height);
    }
    if e.class() == EntityClass::Player {
        return (0.6, 1.8);
    }
    e.size()
        .or_else(|| {
            ghast_entity::v1_8::info(e.entity_type()).and_then(|i| Some((i.width?, i.height?)))
        })
        .unwrap_or((0.6, 1.8))
}

/// A metadata value as a number (0 when missing or not numeric).
fn num(v: Option<&MetaValue>) -> i32 {
    match v {
        Some(MetaValue::Byte(b)) => i32::from(*b),
        Some(MetaValue::Short(s)) => i32::from(*s),
        Some(MetaValue::Int(i) | MetaValue::VarInt(i)) => *i,
        Some(MetaValue::Float(f)) if f.is_finite() => *f as i32,
        Some(MetaValue::Bool(b)) => i32::from(*b),
        _ => 0,
    }
}

/// The shown stack of an entity: a dropped item's stack (`id, meta`), a falling block's block,
/// primed TNT, an orb's value (as the meta), else the held item.
#[must_use]
pub fn stack_of(e: &Entity) -> (u16, u16) {
    match e.class() {
        EntityClass::ExperienceOrb => return (0, e.data().clamp(0, 0xffff) as u16),
        EntityClass::Item => {
            return e
                .item()
                .filter(|it| it.id > 0)
                .map_or((0, 0), |it| (it.id as u16, it.damage as u16));
        }
        _ => {}
    }
    if let Some(o) = object_info(e) {
        match o.ty {
            70 => return ((o.data & 0xfff) as u16, ((o.data >> 12) & 15) as u16),
            50 => return (46, 0),
            _ => {}
        }
    }
    e.equipment(0)
        .filter(|it| it.id > 0)
        .map_or((0, 0), |it| (it.id as u16, it.damage as u16))
}

/// Minecart display blocks by class (each class's default display tile, facing north).
fn cart_block(name: &str) -> u16 {
    match name {
        "MinecartChest" => (54 << 4) | 2,
        "MinecartFurnace" => (61 << 4) | 2,
        "MinecartTNT" => 46 << 4,
        "MinecartSpawner" => 52 << 4,
        "MinecartHopper" => 154 << 4,
        "MinecartCommandBlock" => 137 << 4,
        _ => 0,
    }
}

/// The `SPAWN` variant of a mob or object from its 1.8 metadata (`at(index)` as a number):
/// slime size, villager profession, sheep fleece | 16 sheared, horse type << 8 | colour,
/// wither skeleton, zombie villager, wolf flags | collar << 4, ocelot / rabbit type, hanging bat,
/// shooting ghast, elder guardian, armour stand flags, a minecart's block. 0 otherwise.
#[must_use]
pub fn mob_variant(name: &str, at: impl Fn(u8) -> i32) -> u16 {
    let v = match name {
        "Slime" | "LavaSlime" | "Villager" => at(16).clamp(0, 255),
        "Sheep" => at(16) & 31,
        "EntityHorse" => ((at(19) & 7) << 8) | (at(20) & 0xff),
        "Skeleton" | "Zombie" => at(13) & 1,
        "Wolf" => (at(16) & 7) | ((at(20) & 15) << 4),
        "Ozelot" => at(18) & 3,
        "Rabbit" => {
            if at(18) == 99 {
                99
            } else {
                at(18) & 7
            }
        }
        "Bat" | "Ghast" => at(16) & 1,
        "Guardian" => i32::from(at(16) & 4 != 0),
        "ArmorStand" => at(10) & 31,
        _ if name.starts_with("Minecart") => {
            if at(22) != 0 {
                let s = at(20);
                (((s & 0xfff) << 4) | ((s >> 12) & 15)) & 0xffff
            } else if name == "MinecartFurnace" && at(16) & 1 != 0 {
                (62 << 4) | 2
            } else {
                i32::from(cart_block(name))
            }
        }
        _ => 0,
    };
    v as u16
}

/// The variant of a tracked entity (mobs and objects only).
#[must_use]
pub fn variant_of(e: &Entity) -> u16 {
    if !matches!(e.class(), EntityClass::Mob) && object_info(e).is_none() {
        return 0;
    }
    let m = e.metadata();
    mob_variant(&name_of(e), |i| num(m.get(i)))
}

/// Mobs that are babies when the byte at 12 (the growing age) is negative.
const AGEABLE: [&str; 10] = [
    "Pig",
    "Sheep",
    "Cow",
    "MushroomCow",
    "Chicken",
    "Wolf",
    "Ozelot",
    "EntityHorse",
    "Rabbit",
    "Villager",
];

/// Leather armour colour from an entity item's raw NBT bytes (`display.color`), else
/// [`NO_COLOR`].
#[must_use]
pub fn leather_color(nbt: Option<&bytes::Bytes>) -> u32 {
    nbt.and_then(|b| ghast_protocol::nbt::RawNbt::from_bytes(b.clone()).ok())
        .map_or(NO_COLOR, |n| leather_color_nbt(&n))
}

/// Leather armour colour from an inventory stack's NBT, else [`NO_COLOR`].
#[must_use]
pub fn leather_color_nbt(nbt: &ghast_protocol::nbt::RawNbt) -> u32 {
    use ghast_protocol::nbt::Tag;
    let Ok(Some(root)) = nbt.parse() else {
        return NO_COLOR;
    };
    let color = root
        .tag
        .as_compound()
        .and_then(|c| c.get("display"))
        .and_then(Tag::as_compound)
        .and_then(|d| d.get("color"));
    match color {
        Some(Tag::Int(c)) => (*c as u32) & 0xff_ffff,
        _ => NO_COLOR,
    }
}

/// An armour stand's default rotations when the server sent none.
const STAND_POSE: [[f32; 3]; 6] = [
    [0.0, 0.0, 0.0],
    [0.0, 0.0, 0.0],
    [-10.0, 0.0, -10.0],
    [-15.0, 0.0, 10.0],
    [-1.0, 0.0, -1.0],
    [1.0, 0.0, 1.0],
];

/// The `LOOKS` of an entity, `None` when plain: baby, the armour it wears (equipment 1..4) and an
/// armour stand's pose (metadata 11..16).
#[must_use]
pub fn looks_of(e: &Entity) -> Option<Looks> {
    let name = name_of(e);
    let m = e.metadata();
    let at = |i: u8| num(m.get(i));
    let baby = if AGEABLE.contains(&name.as_str()) {
        at(12) < 0
    } else if name == "Zombie" || name == "PigZombie" {
        at(12) == 1
    } else {
        name == "ArmorStand" && at(10) & 1 != 0
    };
    let mut armour = Vec::new();
    for slot in 1..=4u8 {
        if let Some(it) = e.equipment(usize::from(slot)).filter(|it| it.id > 0) {
            armour.push((
                slot,
                it.id as u16,
                it.damage as u16,
                leather_color(it.nbt.as_ref()),
            ));
        }
    }
    let pose = (name == "ArmorStand").then(|| {
        let mut p = [0f32; 18];
        for (i, def) in STAND_POSE.iter().enumerate() {
            let r = match m.get(11 + i as u8) {
                Some(MetaValue::Rotation(r)) => *r,
                _ => *def,
            };
            p[i * 3..i * 3 + 3].copy_from_slice(&r);
        }
        p
    });
    let l = Looks {
        flags: if baby { look::BABY } else { 0 },
        armour,
        pose,
    };
    (!l.is_plain()).then_some(l)
}

/// The `ESTATE` flags byte of an entity (metadata 0 bits plus [`crate::protocol::eflag::SLEEP`]).
#[must_use]
pub fn estate_flags(e: &Entity, sleeping: bool) -> u8 {
    use crate::protocol::eflag;
    let meta = ghast_entity::v1_8::flags(e);
    (meta & (eflag::FIRE | eflag::SNEAK | eflag::SPRINT | eflag::USE | eflag::INVISIBLE))
        | if sleeping { eflag::SLEEP } else { 0 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn angles_match_the_page_convention() {
        // yaw 0 (south, +z) is π on the page; yaw 90 (west) is π/2; pitch 90 (down) is −π/2
        assert!((yaw_rad(0.0) - PI as f32).abs() < 1e-6);
        assert!((yaw_rad(90.0) - (PI / 2.0) as f32).abs() < 1e-6);
        assert!((yaw_rad(180.0)).abs() < 1e-6);
        assert!((pitch_rad(90.0) + (PI / 2.0) as f32).abs() < 1e-6);
        assert!((pitch_rad(-30.0) - (PI / 6.0) as f32).abs() < 1e-6);
    }
}
