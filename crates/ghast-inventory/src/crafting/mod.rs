//! Crafting: what the vanilla client computes for a crafting grid.
//!
//! - **1.8.9**: the client computes the result itself (`RecipeDispatcher`, first matching recipe in
//!   its sorted list) every time a grid slot is set or taken from, and asks the matching recipe
//!   for the remainders when the result is taken. Shaped / shapeless recipes and the special
//!   recipes armor dye, fireworks, book cloning, map cloning and repairing are reproduced; banner
//!   recipes and map upscaling (which needs the map's state) report [`Craft::Unknown`].
//! - **1.16.5**: the server computes results (`set_slot`); the client only needs the recipes it got
//!   in `declare_recipes` for the remainders when a result is taken ([`RecipeBook`]).

use std::sync::Arc;

use ghast_protocol::nbt::{Compound, List, NamedTag, RawNbt, Tag, tag};

use ghast_data::v1_8::{RECIPE_ANY_DATA, Recipe, SpecialRecipe};

use crate::rules::Version;
use crate::stack::Stack;

/// The result a grid gives, as far as ghast knows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Craft {
    /// The result vanilla's client would show (`None`: no result).
    Known(Option<Stack>),
    /// A recipe ghast does not model may decide it.
    Unknown,
}

const WILDCARD: u16 = RECIPE_ANY_DATA;

// 1.8 item ids used by the special recipes
const DYE: i32 = 351;
const GUNPOWDER: i32 = 289;
const FIREWORK_CHARGE: i32 = 402;
const PAPER: i32 = 339;
const GLOWSTONE_DUST: i32 = 348;
const DIAMOND: i32 = 264;
const FIRE_CHARGE: i32 = 385;
const FEATHER: i32 = 288;
const GOLD_NUGGET: i32 = 371;
const SKULL: i32 = 397;
const FIREWORKS: i32 = 401;
const WRITTEN_BOOK: i32 = 387;
const WRITABLE_BOOK: i32 = 386;
const FILLED_MAP: i32 = 358;
const MAP: i32 = 395;
const BANNER: i32 = 425;
const BUCKET: i32 = 325;

/// `DyeItem.COLORS` (by dye damage).
const DYE_COLORS: [i32; 16] = [
    1973019, 11743532, 3887386, 5320730, 2437522, 8073150, 2651799, 11250603, 4408131, 14188952,
    4312372, 14602026, 6719955, 12801229, 15435844, 15790320,
];

/// `SheepEntity.getDyedColor` by wool colour id.
const SHEEP_COLORS: [[f32; 3]; 16] = [
    [1.0, 1.0, 1.0],
    [0.85, 0.5, 0.2],
    [0.7, 0.3, 0.85],
    [0.4, 0.6, 0.85],
    [0.9, 0.9, 0.2],
    [0.5, 0.8, 0.1],
    [0.95, 0.5, 0.65],
    [0.3, 0.3, 0.3],
    [0.6, 0.6, 0.6],
    [0.3, 0.5, 0.6],
    [0.5, 0.25, 0.7],
    [0.2, 0.3, 0.7],
    [0.4, 0.3, 0.2],
    [0.4, 0.5, 0.2],
    [0.6, 0.2, 0.2],
    [0.1, 0.1, 0.1],
];

/// A grid: row-major stacks and the grid width (2 or 3).
pub(crate) struct Grid<'a> {
    pub stacks: &'a [Option<Stack>],
    pub width: usize,
}

impl Grid<'_> {
    fn height(&self) -> usize {
        self.stacks.len() / self.width
    }

    fn at(&self, x: usize, y: usize) -> Option<&Stack> {
        if x < self.width && y < self.height() {
            self.stacks[x + y * self.width].as_ref()
        } else {
            None
        }
    }

    fn items(&self) -> impl Iterator<Item = &Stack> {
        self.stacks.iter().flatten()
    }
}

// ---------------------------------------------------------------- 1.8.9

fn ing_matches(ing: (u16, u16), s: &Stack) -> bool {
    s.id == i32::from(ing.0) && (ing.1 == WILDCARD || i32::from(s.damage) == i32::from(ing.1))
}

fn shaped_matches(g: &Grid<'_>, w: usize, h: usize, ins: &[Option<(u16, u16)>]) -> bool {
    for ox in 0..=3usize.saturating_sub(w) {
        for oy in 0..=3usize.saturating_sub(h) {
            for mirror in [true, false] {
                let ok = (0..3).all(|x| {
                    (0..3).all(|y| {
                        let ing = if x >= ox && y >= oy && x - ox < w && y - oy < h {
                            let (k, l) = (x - ox, y - oy);
                            if mirror {
                                ins[w - k - 1 + l * w]
                            } else {
                                ins[k + l * w]
                            }
                        } else {
                            None
                        };
                        match (ing, g.at(x, y)) {
                            (None, None) => true,
                            (Some(i), Some(s)) => ing_matches(i, s),
                            _ => false,
                        }
                    })
                });
                if ok {
                    return true;
                }
            }
        }
    }
    false
}

fn shapeless_matches(g: &Grid<'_>, ins: &[(u16, u16)]) -> bool {
    let mut left: Vec<(u16, u16)> = ins.to_vec();
    for y in 0..g.height() {
        for x in 0..g.width {
            if let Some(s) = g.at(x, y) {
                match left.iter().position(|&i| ing_matches(i, s)) {
                    Some(p) => {
                        left.remove(p);
                    }
                    None => return false,
                }
            }
        }
    }
    left.is_empty()
}

fn output(o: (u16, u8, u16)) -> Stack {
    Stack::new(i32::from(o.0), i32::from(o.1), o.2 as i16)
}

/// Whether a special recipe matches; `Err` when ghast cannot tell.
fn special_matches(sp: SpecialRecipe, g: &Grid<'_>) -> Result<bool, ()> {
    match sp {
        SpecialRecipe::BannerPattern | SpecialRecipe::BannerDuplicate => {
            if g.items().any(|s| s.id == BANNER) {
                Err(())
            } else {
                Ok(false)
            }
        }
        SpecialRecipe::MapUpscale => {
            const P: Option<(u16, u16)> = Some((PAPER as u16, 0));
            let ins = [P, P, P, P, Some((FILLED_MAP as u16, WILDCARD)), P, P, P, P];
            // the shape matches: whether it crafts depends on the map's state
            if shaped_matches(g, 3, 3, &ins) {
                Err(())
            } else {
                Ok(false)
            }
        }
        SpecialRecipe::ArmorDye => {
            let mut armor = false;
            let mut dyes = false;
            for s in g.items() {
                if (298..=317).contains(&s.id) {
                    if !(298..=301).contains(&s.id) || armor {
                        return Ok(false);
                    }
                    armor = true;
                } else if s.id == DYE {
                    dyes = true;
                } else {
                    return Ok(false);
                }
            }
            Ok(armor && dyes)
        }
        SpecialRecipe::Firework => Ok(firework(g).is_some()),
        SpecialRecipe::BookCloning => {
            let mut book = false;
            let mut n = 0;
            for s in g.items() {
                if s.id == WRITTEN_BOOK {
                    if book {
                        return Ok(false);
                    }
                    book = true;
                } else if s.id == WRITABLE_BOOK {
                    n += 1;
                } else {
                    return Ok(false);
                }
            }
            Ok(book && n > 0)
        }
        SpecialRecipe::MapCloning => {
            let mut filled = false;
            let mut n = 0;
            for s in g.items() {
                if s.id == FILLED_MAP {
                    if filled {
                        return Ok(false);
                    }
                    filled = true;
                } else if s.id == MAP {
                    n += 1;
                } else {
                    return Ok(false);
                }
            }
            Ok(filled && n > 0)
        }
        SpecialRecipe::Repairing => {
            let list: Vec<&Stack> = g.items().collect();
            for (k, s) in list.iter().enumerate().skip(1) {
                let first = list[0];
                let _ = k;
                if s.id != first.id
                    || first.count != 1
                    || s.count != 1
                    || !damageable_item(first.id)
                {
                    return Ok(false);
                }
            }
            Ok(list.len() == 2)
        }
    }
}

/// 1.8 `Item.isDamageable()`: has durability and no subtypes.
fn damageable_item(id: i32) -> bool {
    max_damage(id) > 0 && !Version::V1_8.has_subtypes(id)
}

fn max_damage(id: i32) -> i32 {
    u16::try_from(id)
        .ok()
        .and_then(ghast_data::v1_8::Item::from_id)
        .and_then(|i| i.info().max_durability)
        .map_or(0, i32::from)
}

/// The result of a special recipe that matched.
fn special_result(sp: SpecialRecipe, g: &Grid<'_>) -> Craft {
    match sp {
        SpecialRecipe::ArmorDye => Craft::Known(armor_dye(g)),
        SpecialRecipe::Firework => match firework(g) {
            Some(r) => Craft::Known(Some(r)),
            None => Craft::Unknown,
        },
        SpecialRecipe::BookCloning => book_cloning(g),
        SpecialRecipe::MapCloning => {
            let mut src = None;
            let mut n = 0;
            for s in g.items() {
                if s.id == FILLED_MAP {
                    src = Some(s);
                } else {
                    n += 1;
                }
            }
            let Some(src) = src else {
                return Craft::Known(None);
            };
            let mut out = Stack::new(FILLED_MAP, n + 1, src.damage);
            if let Some(name) = custom_name(src) {
                out.nbt = build(|c| {
                    let mut d = Compound::default();
                    put(&mut d, "Name", Tag::String(name));
                    put(c, "display", Tag::Compound(d));
                });
            }
            Craft::Known(Some(out))
        }
        SpecialRecipe::Repairing => {
            let list: Vec<&Stack> = g.items().collect();
            let (a, b) = (list[0], list[1]);
            let max = max_damage(a.id);
            let j = max - i32::from(a.damage);
            let k = max - i32::from(b.damage);
            let l = j + k + max * 5 / 100;
            let m = (max - l).max(0);
            Craft::Known(Some(Stack::new(a.id, 1, m as i16)))
        }
        _ => Craft::Unknown,
    }
}

/// 1.8 `RecipeDispatcher.matches` (the result for a grid).
pub(crate) fn result_1_8(g: &Grid<'_>) -> Craft {
    for r in ghast_data::v1_8::recipes() {
        let hit = match r {
            Recipe::Shaped {
                width: w,
                height: h,
                inputs: ins,
                output: out,
            } => {
                if shaped_matches(g, usize::from(*w), usize::from(*h), ins) {
                    return Craft::Known(Some(output(*out)));
                }
                false
            }
            Recipe::Shapeless {
                inputs: ins,
                output: out,
            } => {
                if shapeless_matches(g, ins) {
                    return Craft::Known(Some(output(*out)));
                }
                false
            }
            Recipe::Special(sp) => match special_matches(*sp, g) {
                Ok(m) => m,
                Err(()) => return Craft::Unknown,
            },
        };
        if hit {
            let Recipe::Special(sp) = r else {
                unreachable!()
            };
            return special_result(*sp, g);
        }
    }
    Craft::Known(None)
}

/// What a grid slot gets back when the result is taken.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Rem {
    None,
    /// A new stack (the recipe remainder item).
    Item(Stack),
    /// The grid stack itself (book cloning keeps the written book).
    Same,
}

/// 1.8 `RecipeDispatcher.getRemainders` for the matching recipe; `Err` when ghast cannot tell
/// (unmodelled recipe, no recipe matching, a written book stack above 1).
pub(crate) fn remainders_1_8(g: &Grid<'_>) -> Result<Vec<Rem>, ()> {
    if g.items().next().is_none() {
        return Ok(vec![Rem::None; g.stacks.len()]); // nothing to consume or hand back
    }
    let default = || {
        g.stacks
            .iter()
            .map(|s| match s {
                Some(s) if matches!(s.id, 326 | 327 | 335) => Rem::Item(Stack::new(BUCKET, 1, 0)),
                _ => Rem::None,
            })
            .collect()
    };
    for r in ghast_data::v1_8::recipes() {
        match r {
            Recipe::Shaped {
                width: w,
                height: h,
                inputs: ins,
                ..
            } => {
                if shaped_matches(g, usize::from(*w), usize::from(*h), ins) {
                    return Ok(default());
                }
            }
            Recipe::Shapeless { inputs: ins, .. } => {
                if shapeless_matches(g, ins) {
                    return Ok(default());
                }
            }
            Recipe::Special(sp) => {
                if special_matches(*sp, g)? {
                    if *sp == SpecialRecipe::BookCloning {
                        let mut v: Vec<Rem> = vec![Rem::None; g.stacks.len()];
                        if let Some(i) = g
                            .stacks
                            .iter()
                            .position(|s| s.as_ref().is_some_and(|s| s.id == WRITTEN_BOOK))
                        {
                            if g.stacks[i].as_ref().is_some_and(|s| s.count != 1) {
                                return Err(());
                            }
                            v[i] = Rem::Same;
                        }
                        return Ok(v);
                    }
                    return Ok(default());
                }
            }
        }
    }
    // No recipe: vanilla hands back the grid stacks themselves (an aliasing path ghast does
    // not reproduce).
    Err(())
}

// ---------------------------------------------------------------- NBT helpers

fn tree(s: &Stack) -> Option<Compound> {
    match s.nbt.as_ref()?.parse().ok()?? {
        NamedTag {
            tag: Tag::Compound(c),
            ..
        } => Some(c),
        _ => None,
    }
}

fn get<'a>(c: &'a Compound, key: &str) -> Option<&'a Tag> {
    c.0.iter().rev().find(|(k, _)| k == key).map(|(_, v)| v)
}

fn put(c: &mut Compound, key: &str, v: Tag) {
    if let Some(e) = c.0.iter_mut().rev().find(|(k, _)| k == key) {
        e.1 = v;
    } else {
        c.0.push((key.to_string(), v));
    }
}

fn build(f: impl FnOnce(&mut Compound)) -> Option<RawNbt> {
    let mut c = Compound::default();
    f(&mut c);
    write(c)
}

/// Serialises a compound the 1.8 client built. Vanilla writes compounds in `HashMap` iteration
/// order, so the keys are put in that order (byte-identical to vanilla; servers compare trees).
fn write(c: Compound) -> Option<RawNbt> {
    let mut t = Tag::Compound(c);
    java_order(&mut t);
    RawNbt::from_named(&NamedTag {
        name: String::new(),
        tag: t,
    })
    .ok()
}

/// Java `String.hashCode` over UTF-16 code units.
fn java_hash(s: &str) -> i32 {
    s.encode_utf16()
        .fold(0i32, |h, c| h.wrapping_mul(31).wrapping_add(i32::from(c)))
}

/// Reorders every compound into `java.util.HashMap` iteration order: by bucket
/// (`(h ^ h >>> 16) & (capacity - 1)`, capacity 16 doubling past a 0.75 load), insertion order
/// within a bucket.
fn java_order(t: &mut Tag) {
    match t {
        Tag::Compound(c) => {
            for (_, v) in &mut c.0 {
                java_order(v);
            }
            let mut cap = 16usize;
            while c.0.len() * 4 > cap * 3 {
                cap *= 2;
            }
            let bucket = |k: &str| {
                let h = java_hash(k);
                ((h ^ ((h as u32) >> 16) as i32) as u32 as usize) & (cap - 1)
            };
            c.0.sort_by_key(|(k, _)| bucket(k)); // stable: insertion order within a bucket
        }
        Tag::List(l) => {
            for v in &mut l.items {
                java_order(v);
            }
        }
        _ => {}
    }
}

/// `display.Name` if it is a string.
fn custom_name(s: &Stack) -> Option<String> {
    let c = tree(s)?;
    let Some(Tag::Compound(d)) = get(&c, "display") else {
        return None;
    };
    match get(d, "Name") {
        Some(Tag::String(n)) => Some(n.clone()),
        _ => None,
    }
}

// ---------------------------------------------------------------- 1.8 special recipes

fn armor_dye(g: &Grid<'_>) -> Option<Stack> {
    let mut armor: Option<Stack> = None;
    let mut rgb = [0i32; 3];
    let mut i: i32 = 0;
    let mut j: i32 = 0;
    for s in g.items() {
        if (298..=317).contains(&s.id) {
            if !(298..=301).contains(&s.id) || armor.is_some() {
                return None;
            }
            let mut a = s.clone();
            a.count = 1;
            if let Some(l) = leather_color(s) {
                let f = (l >> 16 & 255) as f32 / 255.0;
                let gg = (l >> 8 & 255) as f32 / 255.0;
                let h = (l & 255) as f32 / 255.0;
                i = (i as f32 + f.max(gg.max(h)) * 255.0) as i32;
                rgb[0] = (rgb[0] as f32 + f * 255.0) as i32;
                rgb[1] = (rgb[1] as f32 + gg * 255.0) as i32;
                rgb[2] = (rgb[2] as f32 + h * 255.0) as i32;
                j += 1;
            }
            armor = Some(a);
        } else if s.id == DYE {
            let d = i32::from(s.damage);
            let wool = if (0..16).contains(&d) { 15 - d } else { 15 };
            let fs = SHEEP_COLORS[wool as usize];
            let m = (fs[0] * 255.0) as i32;
            let n = (fs[1] * 255.0) as i32;
            let o = (fs[2] * 255.0) as i32;
            i += m.max(n.max(o));
            rgb[0] += m;
            rgb[1] += n;
            rgb[2] += o;
            j += 1;
        } else {
            return None;
        }
    }
    let mut a = armor?;
    let p = rgb[0] / j;
    let q = rgb[1] / j;
    let r = rgb[2] / j;
    let s = i as f32 / j as f32;
    let t = p.max(q.max(r)) as f32;
    let p = (p as f32 * s / t) as i32;
    let q = (q as f32 * s / t) as i32;
    let r = (r as f32 * s / t) as i32;
    let color = (p.wrapping_shl(8).wrapping_add(q))
        .wrapping_shl(8)
        .wrapping_add(r);
    // ArmorItem.setColor
    let mut c = tree(&a).unwrap_or_default();
    let mut d = match get(&c, "display") {
        Some(Tag::Compound(d)) => d.clone(),
        _ => Compound::default(),
    };
    put(&mut d, "color", Tag::Int(color));
    put(&mut c, "display", Tag::Compound(d));
    a.nbt = write(c);
    Some(a)
}

/// `ArmorItem.hasColor` + `getColor` for leather armor.
fn leather_color(s: &Stack) -> Option<i32> {
    let c = tree(s)?;
    let Some(Tag::Compound(d)) = get(&c, "display") else {
        return None;
    };
    match get(d, "color") {
        Some(Tag::Int(v)) => Some(*v),
        _ => None,
    }
}

/// `FireworkRecipeType.matches`: the crafted stack when the grid matches.
fn firework(g: &Grid<'_>) -> Option<Stack> {
    let (mut paper, mut gunpowder, mut dyes, mut charges, mut effects, mut shapes) =
        (0, 0, 0, 0, 0, 0);
    for s in g.items() {
        match s.id {
            GUNPOWDER => gunpowder += 1,
            FIREWORK_CHARGE => charges += 1,
            DYE => dyes += 1,
            PAPER => paper += 1,
            GLOWSTONE_DUST | DIAMOND => effects += 1,
            FIRE_CHARGE | FEATHER | GOLD_NUGGET | SKULL => shapes += 1,
            _ => return None,
        }
    }
    let m = effects + dyes + shapes;
    if gunpowder > 3 || paper > 1 {
        return None;
    }
    if gunpowder >= 1 && paper == 1 && m == 0 {
        let mut out = Stack::new(FIREWORKS, 1, 0);
        if charges > 0 {
            let explosions: Vec<Tag> = g
                .items()
                .filter(|s| s.id == FIREWORK_CHARGE)
                .filter_map(
                    |s| match tree(s).as_ref().and_then(|c| get(c, "Explosion")) {
                        Some(Tag::Compound(e)) => Some(Tag::Compound(e.clone())),
                        _ => None,
                    },
                )
                .collect();
            out.nbt = build(|c| {
                let mut f = Compound::default();
                let elem_type = if explosions.is_empty() {
                    tag::END
                } else {
                    tag::COMPOUND
                };
                put(
                    &mut f,
                    "Explosions",
                    Tag::List(List {
                        elem_type,
                        items: explosions,
                    }),
                );
                put(&mut f, "Flight", Tag::Byte(gunpowder as i8));
                put(c, "Fireworks", Tag::Compound(f));
            });
        }
        return Some(out);
    }
    if gunpowder == 1 && paper == 0 && charges == 0 && dyes > 0 && shapes <= 1 {
        let mut e = Compound::default();
        let mut kind: i8 = 0;
        let mut colors = Vec::new();
        for s in g.items() {
            match s.id {
                DYE => colors.push(DYE_COLORS[(s.damage & 15) as usize]),
                GLOWSTONE_DUST => put(&mut e, "Flicker", Tag::Byte(1)),
                DIAMOND => put(&mut e, "Trail", Tag::Byte(1)),
                FIRE_CHARGE => kind = 1,
                FEATHER => kind = 4,
                GOLD_NUGGET => kind = 2,
                SKULL => kind = 3,
                _ => {}
            }
        }
        put(&mut e, "Colors", Tag::IntArray(colors));
        put(&mut e, "Type", Tag::Byte(kind));
        let mut out = Stack::new(FIREWORK_CHARGE, 1, 0);
        out.nbt = build(|c| put(c, "Explosion", Tag::Compound(e)));
        return Some(out);
    }
    if gunpowder == 0 && paper == 0 && charges == 1 && dyes > 0 && dyes == m {
        let mut fade = Vec::new();
        let mut charge: Option<Stack> = None;
        for s in g.items() {
            if s.id == DYE {
                fade.push(DYE_COLORS[(s.damage & 15) as usize]);
            } else if s.id == FIREWORK_CHARGE {
                let mut c = s.clone();
                c.count = 1;
                charge = Some(c);
            }
        }
        let mut charge = charge?;
        let mut c = tree(&charge)?; // a charge without NBT does not match
        if let Some(Tag::Compound(e)) = get(&c, "Explosion") {
            let mut e = e.clone();
            put(&mut e, "FadeColors", Tag::IntArray(fade));
            put(&mut c, "Explosion", Tag::Compound(e));
            charge.nbt = write(c);
        }
        // (without an Explosion compound vanilla edits a detached compound: no change)
        return Some(charge);
    }
    None
}

fn book_cloning(g: &Grid<'_>) -> Craft {
    let mut src = None;
    let mut n = 0;
    for s in g.items() {
        if s.id == WRITTEN_BOOK {
            if src.is_some() {
                return Craft::Known(None);
            }
            src = Some(s);
        } else if s.id == WRITABLE_BOOK {
            n += 1;
        } else {
            return Craft::Known(None);
        }
    }
    let Some(src) = src else {
        return Craft::Known(None);
    };
    // getGeneration reads the NBT unconditionally (a written book without NBT crashes vanilla)
    let Some(mut c) = tree(src) else {
        return Craft::Unknown;
    };
    let generation = match get(&c, "generation") {
        Some(Tag::Byte(v)) => i32::from(*v),
        Some(Tag::Short(v)) => i32::from(*v),
        Some(Tag::Int(v)) => *v,
        Some(Tag::Long(v)) => *v as i32,
        Some(_) => return Craft::Unknown,
        None => 0,
    };
    if n < 1 || generation >= 2 {
        return Craft::Known(None);
    }
    put(&mut c, "generation", Tag::Int(generation + 1));
    let mut out = Stack::new(WRITTEN_BOOK, n, 0);
    out.nbt = write(c);
    Craft::Known(Some(out))
}

// ---------------------------------------------------------------- 1.16.5

/// A crafting recipe from `declare_recipes`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum R16 {
    Shaped {
        w: usize,
        h: usize,
        /// Row-major ingredients: each the item ids it accepts (empty: only an empty slot).
        ins: Vec<Vec<i32>>,
    },
    Shapeless {
        ins: Vec<Vec<i32>>,
    },
}

/// The crafting recipes the server declared (1.16.5 `declare_recipes`), in packet order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RecipeBook {
    pub(crate) recipes: Arc<Vec<R16>>,
    /// Number of special (`crafting_special_*`) recipes declared.
    pub(crate) specials: usize,
}

impl RecipeBook {
    /// Number of crafting recipes (shaped, shapeless, special).
    #[must_use]
    pub fn len(&self) -> usize {
        self.recipes.len() + self.specials
    }

    /// Whether no recipe was declared.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Builds the book from a 1.16.5 `declare_recipes` packet.
    #[must_use]
    pub fn from_v1_16_5(
        p: &ghast_protocol::v1_16_5::play::clientbound::DeclareRecipes,
    ) -> RecipeBook {
        use ghast_protocol::v1_16_5::play::clientbound::DeclareRecipesRecipesData as D;
        let ing = |options: &[ghast_protocol::types::slot::modern::Slot]| -> Vec<i32> {
            options
                .iter()
                .filter_map(|s| {
                    s.0.as_ref()
                        .filter(|i| i.id != 0 && i.count > 0)
                        .map(|i| i.id)
                })
                .collect()
        };
        let mut recipes = Vec::new();
        let mut specials = 0;
        for r in &p.recipes {
            match &r.data {
                D::CraftingShaped(s) => recipes.push(R16::Shaped {
                    w: usize::try_from(s.width).unwrap_or(0),
                    h: usize::try_from(s.height).unwrap_or(0),
                    ins: s.ingredients.iter().flatten().map(|o| ing(o)).collect(),
                }),
                D::CraftingShapeless(s) => recipes.push(R16::Shapeless {
                    ins: s.ingredients.iter().map(|o| ing(o)).collect(),
                }),
                D::CraftingSpecialArmordye
                | D::CraftingSpecialBookcloning
                | D::CraftingSpecialMapcloning
                | D::CraftingSpecialMapextending
                | D::CraftingSpecialFireworkRocket
                | D::CraftingSpecialFireworkStar
                | D::CraftingSpecialFireworkStarFade
                | D::CraftingSpecialRepairitem
                | D::CraftingSpecialTippedarrow
                | D::CraftingSpecialBannerduplicate
                | D::CraftingSpecialBanneraddpattern
                | D::CraftingSpecialShielddecoration
                | D::CraftingSpecialShulkerboxcoloring
                | D::CraftingSpecialSuspiciousstew => specials += 1,
                _ => {}
            }
        }
        RecipeBook {
            recipes: Arc::new(recipes),
            specials,
        }
    }
}

fn test16(options: &[i32], s: Option<&Stack>) -> bool {
    match s {
        None => options.is_empty(),
        Some(s) => options.contains(&s.id),
    }
}

fn shaped16(g: &Grid<'_>, w: usize, h: usize, ins: &[Vec<i32>]) -> bool {
    if w == 0 || h == 0 || ins.len() != w * h || w > g.width || h > g.height() {
        return false;
    }
    for ox in 0..=g.width - w {
        for oy in 0..=g.height() - h {
            for mirror in [true, false] {
                let ok = (0..g.width).all(|x| {
                    (0..g.height()).all(|y| {
                        let empty: &[i32] = &[];
                        let opts: &[i32] = if x >= ox && y >= oy && x - ox < w && y - oy < h {
                            let (k, l) = (x - ox, y - oy);
                            if mirror {
                                &ins[w - k - 1 + l * w]
                            } else {
                                &ins[k + l * w]
                            }
                        } else {
                            empty
                        };
                        test16(opts, g.at(x, y))
                    })
                });
                if ok {
                    return true;
                }
            }
        }
    }
    false
}

/// Shapeless: as many stacks as ingredients and an assignment of stacks to ingredients
/// (vanilla's `RecipeMatcher`).
fn shapeless16(g: &Grid<'_>, ins: &[Vec<i32>]) -> bool {
    let items: Vec<&Stack> = g.items().collect();
    if items.len() != ins.len() {
        return false;
    }
    // bipartite matching: ingredient -> stack
    let mut owner: Vec<Option<usize>> = vec![None; items.len()];
    fn augment(
        ing: usize,
        ins: &[Vec<i32>],
        items: &[&Stack],
        owner: &mut [Option<usize>],
        seen: &mut [bool],
    ) -> bool {
        for (s, it) in items.iter().enumerate() {
            if seen[s] || !ins[ing].contains(&it.id) {
                continue;
            }
            seen[s] = true;
            if owner[s].is_none_or(|o| augment(o, ins, items, owner, seen)) {
                owner[s] = Some(ing);
                return true;
            }
        }
        false
    }
    (0..ins.len()).all(|i| augment(i, ins, &items, &mut owner, &mut vec![false; items.len()]))
}

/// 1.16.5 `RecipeManager.getRemainingStacks` as the client computes it when a result is taken.
/// `Err` when ghast cannot tell: a written book or banner in the grid (book cloning / banner
/// duplication keep them), or no shaped / shapeless recipe matching (a special recipe, or none —
/// vanilla then hands back the grid stacks themselves).
pub(crate) fn remainders_1_16(book: &RecipeBook, g: &Grid<'_>) -> Result<Vec<Rem>, ()> {
    if g.items().next().is_none() {
        return Ok(vec![Rem::None; g.stacks.len()]); // nothing to consume or hand back
    }
    let name = |id: i32| {
        u16::try_from(id)
            .ok()
            .and_then(ghast_data::v1_16_5::Item::from_id)
            .map_or("", |i| i.info().name)
    };
    if g.items()
        .any(|s| name(s.id) == "written_book" || name(s.id).ends_with("_banner"))
    {
        return Err(());
    }
    let hit = book.recipes.iter().any(|r| match r {
        R16::Shaped { w, h, ins } => shaped16(g, *w, *h, ins),
        R16::Shapeless { ins } => shapeless16(g, ins),
    });
    if !hit {
        return Err(());
    }
    let id = |n: &str| i32::from(ghast_data::v1_16_5::Item::from_name(n).map_or(0, |i| i.id()));
    Ok(g.stacks
        .iter()
        .map(|s| match s.as_ref().map(|s| name(s.id)) {
            Some("water_bucket" | "lava_bucket" | "milk_bucket") => {
                Rem::Item(Stack::new(id("bucket"), 1, 0))
            }
            Some("honey_bottle" | "dragon_breath") => {
                Rem::Item(Stack::new(id("glass_bottle"), 1, 0))
            }
            _ => Rem::None,
        })
        .collect::<Vec<_>>())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grid(v: &[Option<Stack>]) -> Grid<'_> {
        Grid {
            stacks: v,
            width: if v.len() == 4 { 2 } else { 3 },
        }
    }

    #[test]
    fn planks_and_sticks() {
        let log = Some(Stack::new(17, 3, 2));
        let g = [None, log, None, None];
        assert_eq!(
            result_1_8(&grid(&g)),
            Craft::Known(Some(Stack::new(5, 4, 2)))
        );
        let p = Some(Stack::new(5, 1, 1));
        let g = [p.clone(), None, p, None];
        assert_eq!(
            result_1_8(&grid(&g)),
            Craft::Known(Some(Stack::new(280, 4, 0)))
        );
        let g = [None, None, None, None];
        assert_eq!(result_1_8(&grid(&g)), Craft::Known(None));
    }

    #[test]
    fn repair() {
        let a = Some(Stack::new(276, 1, 1000));
        let b = Some(Stack::new(276, 1, 1500));
        let g = [a, None, None, b];
        // 1561 max: (561 + 61 + 78) = 700 left -> damage 861
        assert_eq!(
            result_1_8(&grid(&g)),
            Craft::Known(Some(Stack::new(276, 1, 861)))
        );
    }
}
