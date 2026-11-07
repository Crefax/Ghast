//! Block entities the page draws: sign text, skulls, banners and flower pots, converted from the
//! block entities ghast keeps per column (`WorldConfig::with_block_entities(true)`).

use ghast_protocol::nbt::{Compound, RawNbt, Tag};

use crate::protocol::{SignText, Tile, TileKind};
use crate::skins::parse_textures;
use ghast_world::{BlockEntity, Column};

/// One block entity as the page needs it.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Entry {
    Sign([String; 4]),
    /// Skull type, floor rotation and a player skull's skin URL.
    Skull {
        skull: u8,
        rot: u8,
        url: Option<String>,
    },
    Banner {
        base: u8,
        patterns: Vec<(String, u8)>,
    },
    Pot {
        contents: String,
    },
}

/// The signs (with text) and tiles of a column. `block_name(id)` names a numeric flower pot
/// item; `skin(url)` turns a player skull's skin URL into its served path ('' while not here).
pub fn column_tiles(
    col: &Column,
    block_name: impl Fn(i32) -> Option<&'static str>,
    mut skin: impl FnMut(&str) -> String,
) -> (Vec<SignText>, Vec<Tile>) {
    let (mut signs, mut tiles) = (Vec::new(), Vec::new());
    for ([x, y, z], be) in col.block_entities() {
        let entry = match be {
            BlockEntity::Sign(lines) => Some(Entry::Sign(lines.clone().map(|l| chat_text(&l)))),
            BlockEntity::Nbt { nbt, .. } => RawNbt::from_bytes(nbt.clone())
                .ok()
                .and_then(|n| entry_from_nbt(&n, &block_name)),
            #[allow(unreachable_patterns)]
            _ => None,
        };
        let Some(entry) = entry else { continue };
        let (x, y, z) = (x & 15, y, z & 15);
        match entry {
            Entry::Sign(lines) => {
                if lines.iter().any(|l| !l.is_empty()) {
                    signs.push(SignText { x, y, z, lines });
                }
            }
            Entry::Skull { skull, rot, url } => tiles.push(Tile {
                x,
                y,
                z,
                kind: TileKind::Skull {
                    skull,
                    rot,
                    skin: url.as_deref().map(&mut skin).unwrap_or_default(),
                },
            }),
            Entry::Banner { base, patterns } => tiles.push(Tile {
                x,
                y,
                z,
                kind: TileKind::Banner { base, patterns },
            }),
            Entry::Pot { contents } => tiles.push(Tile {
                x,
                y,
                z,
                kind: TileKind::Pot { contents },
            }),
        }
    }
    (signs, tiles)
}

/// Plain text of a 1.8 sign line (a chat component as JSON, or plain), formatting codes removed.
#[must_use]
pub fn chat_text(raw: &str) -> String {
    fn walk(v: &serde_json::Value, out: &mut String) {
        match v {
            serde_json::Value::String(s) => out.push_str(s),
            serde_json::Value::Number(n) => out.push_str(&n.to_string()),
            serde_json::Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
            serde_json::Value::Array(a) => a.iter().for_each(|c| walk(c, out)),
            serde_json::Value::Object(o) => {
                if let Some(t) = o.get("text") {
                    match t {
                        serde_json::Value::String(s) => out.push_str(s),
                        serde_json::Value::Null => {}
                        other => out.push_str(&other.to_string()),
                    }
                }
                if let Some(serde_json::Value::Array(e)) = o.get("extra") {
                    e.iter().for_each(|c| walk(c, out));
                }
            }
            serde_json::Value::Null => {}
        }
    }
    let text = match serde_json::from_str::<serde_json::Value>(raw) {
        Ok(v) => {
            let mut s = String::new();
            walk(&v, &mut s);
            s
        }
        Err(_) => raw.to_string(),
    };
    strip_codes(&text)
}

/// Removes `§x` formatting codes.
#[must_use]
pub fn strip_codes(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars();
    while let Some(c) = it.next() {
        if c == '§' {
            it.next();
        } else {
            out.push(c);
        }
    }
    out
}

/// The wood names in planks / sapling order.
pub const WOODS: [&str; 6] = ["oak", "spruce", "birch", "jungle", "acacia", "dark_oak"];

/// The flower pot's blockstates "contents" from its block entity (the item's block name, with
/// or without `minecraft:`, and its data value). Unknown plants: `"empty"`.
#[must_use]
pub fn pot_contents(item: &str, data: i32) -> String {
    let name = item.strip_prefix("minecraft:").unwrap_or(item);
    let flowers = [
        "rose",
        "blue_orchid",
        "allium",
        "houstonia",
        "red_tulip",
        "orange_tulip",
        "white_tulip",
        "pink_tulip",
        "oxeye_daisy",
    ];
    let s = match name {
        "red_flower" => usize::try_from(data)
            .ok()
            .and_then(|i| flowers.get(i))
            .copied()
            .unwrap_or("empty")
            .to_string(),
        "yellow_flower" => "dandelion".into(),
        "sapling" => format!(
            "{}_sapling",
            WOODS.get((data & 7) as usize).copied().unwrap_or("oak")
        ),
        "tallgrass" => (if data == 2 { "fern" } else { "empty" }).into(),
        "deadbush" => "dead_bush".into(),
        "cactus" => "cactus".into(),
        "red_mushroom" => "mushroom_red".into(),
        "brown_mushroom" => "mushroom_brown".into(),
        _ => "empty".into(),
    };
    if name == "sapling" && WOODS.get((data & 7) as usize).is_none() {
        return "empty".into();
    }
    s
}

fn num(t: Option<&Tag>) -> i32 {
    match t {
        Some(Tag::Byte(v)) => i32::from(*v),
        Some(Tag::Short(v)) => i32::from(*v),
        Some(Tag::Int(v)) => *v,
        Some(Tag::Long(v)) => *v as i32,
        Some(Tag::Float(v)) => *v as i32,
        Some(Tag::Double(v)) => *v as i32,
        Some(Tag::String(s)) => s.trim().parse().unwrap_or(0),
        _ => 0,
    }
}

fn string(t: Option<&Tag>) -> Option<&str> {
    match t {
        Some(Tag::String(s)) => Some(s),
        _ => None,
    }
}

fn list(t: Option<&Tag>) -> &[Tag] {
    match t {
        Some(Tag::List(l)) => &l.items,
        _ => &[],
    }
}

/// The skin URL of a player skull's `Owner.Properties.textures`.
fn skull_url(c: &Compound) -> Option<String> {
    let owner = c.get("Owner")?.as_compound()?;
    let props = owner.get("Properties")?.as_compound()?;
    list(props.get("textures"))
        .iter()
        .filter_map(Tag::as_compound)
        .find_map(|p| parse_textures(string(p.get("Value"))?)?.skin)
}

/// The entry for a 1.8 `tile_entity_data` NBT (skull, banner, flower pot), `None` for other
/// block entities. `block_name(id)` names a numeric flower pot item.
#[must_use]
pub fn entry_from_nbt(
    nbt: &RawNbt,
    block_name: &impl Fn(i32) -> Option<&'static str>,
) -> Option<Entry> {
    let root = nbt.parse().ok()??;
    let c = root.tag.as_compound()?;
    let id = string(c.get("id")).unwrap_or("");
    if id == "Skull" || c.get("SkullType").is_some() {
        let skull = num(c.get("SkullType")).clamp(0, 255) as u8;
        let url = if skull == 3 { skull_url(c) } else { None };
        return Some(Entry::Skull {
            skull,
            rot: (num(c.get("Rot")) & 15) as u8,
            url,
        });
    }
    if id == "Banner" || c.get("Base").is_some() || c.get("Patterns").is_some() {
        let patterns = list(c.get("Patterns"))
            .iter()
            .filter_map(Tag::as_compound)
            .filter_map(|p| {
                let name = string(p.get("Pattern"))?;
                (!name.is_empty()).then(|| (name.to_string(), (num(p.get("Color")) & 15) as u8))
            })
            .collect();
        return Some(Entry::Banner {
            base: (num(c.get("Base")) & 15) as u8,
            patterns,
        });
    }
    if id == "FlowerPot" || c.get("Item").is_some() {
        let name = match c.get("Item") {
            Some(Tag::String(s)) => s.clone(),
            Some(t) => block_name(num(Some(t))).unwrap_or("").to_string(),
            None => String::new(),
        };
        let contents = pot_contents(&name, num(c.get("Data")));
        return (contents != "empty").then_some(Entry::Pot { contents });
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sign_lines_are_plain_text() {
        assert_eq!(
            chat_text(r#"{"text":"§aHi ","extra":[{"text":"there"},"!"]}"#),
            "Hi there!"
        );
        assert_eq!(chat_text("\"plain\""), "plain");
        assert_eq!(chat_text("not json §cred"), "not json red");
        assert_eq!(chat_text(r#"{"extra":[{"text":"a"}],"text":""}"#), "a");
    }

    #[test]
    fn pot_contents_match_the_blockstates() {
        assert_eq!(pot_contents("minecraft:red_flower", 4), "red_tulip");
        assert_eq!(pot_contents("red_flower", 20), "empty");
        assert_eq!(pot_contents("minecraft:sapling", 5), "dark_oak_sapling");
        assert_eq!(pot_contents("tallgrass", 2), "fern");
        assert_eq!(pot_contents("tallgrass", 1), "empty");
        assert_eq!(pot_contents("minecraft:stone", 0), "empty");
    }
}
