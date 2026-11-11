//! 26.1 enchantment value effects (`EnchantmentValueEffect` over `LevelBasedValue`): the
//! number an enchantment adds to, multiplies or sets for one effect component, such as
//! `minecraft:trident_spin_attack_strength`.
//!
//! Read from a `registry_data` entry's NBT, or from the compact text form the built-in vanilla
//! table stores (`add(linear(1.5,0.75))`; [`Fx`]'s `Display` writes it, [`Fx::parse`] reads it).
//! The arithmetic is the game's: `f32`, with the `double` steps (`Math.pow`) where vanilla
//! has them.

use std::fmt;

use ghast_protocol::nbt::{Compound, Tag};

/// `LevelBasedValue`.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Lvl {
    Constant(f32),
    Linear(f32, f32),
    Clamped(Box<Lvl>, f32, f32),
    Exponent(Box<Lvl>, Box<Lvl>),
    Fraction(Box<Lvl>, Box<Lvl>),
    LevelsSquared(f32),
    Lookup(Vec<f32>, Box<Lvl>),
}

/// `EnchantmentValueEffect`.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Fx {
    Add(Lvl),
    Multiply(Lvl),
    Set(Lvl),
    /// `exponential`: input × base ^ exponent.
    Exponential(Lvl, Lvl),
    AllOf(Vec<Fx>),
    /// `remove_binomial` (uses the holder's random: not computable).
    Random,
}

impl Lvl {
    pub(crate) fn calculate(&self, level: i32) -> f32 {
        match self {
            Lvl::Constant(v) => *v,
            Lvl::Linear(base, per) => base + per * (level - 1) as f32,
            Lvl::Clamped(v, min, max) => {
                let v = v.calculate(level);
                // Mth.clamp(float): v < min ? min : Math.min(v, max)
                if v < *min { *min } else { v.min(*max) }
            }
            Lvl::Exponent(b, p) => {
                f64::from(b.calculate(level)).powf(f64::from(p.calculate(level))) as f32
            }
            Lvl::Fraction(n, d) => {
                let d = d.calculate(level);
                if d == 0.0 {
                    0.0
                } else {
                    n.calculate(level) / d
                }
            }
            Lvl::LevelsSquared(added) => level.wrapping_mul(level) as f32 + added,
            Lvl::Lookup(values, fallback) => {
                match usize::try_from(level - 1)
                    .ok()
                    .filter(|_| level as usize <= values.len())
                {
                    Some(i) => values[i],
                    None => fallback.calculate(level),
                }
            }
        }
    }

    fn from_tag(t: &Tag) -> Option<Lvl> {
        if let Some(v) = number(t) {
            return Some(Lvl::Constant(v));
        }
        let c = t.as_compound()?;
        let f = |k: &str| get(c, k).and_then(number);
        let sub = |k: &str| get(c, k).and_then(Lvl::from_tag).map(Box::new);
        Some(match kind(c)? {
            "constant" => Lvl::Constant(f("value")?),
            "linear" => Lvl::Linear(f("base")?, f("per_level_above_first")?),
            "clamped" => Lvl::Clamped(sub("value")?, f("min")?, f("max")?),
            "exponent" => Lvl::Exponent(sub("base")?, sub("power")?),
            "fraction" => Lvl::Fraction(sub("numerator")?, sub("denominator")?),
            "levels_squared" => Lvl::LevelsSquared(f("added")?),
            "lookup" => {
                let Tag::List(l) = get(c, "values")? else {
                    return None;
                };
                let values = l.items.iter().map(number).collect::<Option<Vec<f32>>>()?;
                Lvl::Lookup(values, sub("fallback")?)
            }
            _ => return None,
        })
    }
}

impl Fx {
    /// `process(level, random, input)`; `None` for an effect that needs the holder's random.
    pub(crate) fn process(&self, level: i32, input: f32) -> Option<f32> {
        Some(match self {
            Fx::Add(v) => input + v.calculate(level),
            Fx::Multiply(v) => input * v.calculate(level),
            Fx::Set(v) => v.calculate(level),
            Fx::Exponential(b, e) => {
                (f64::from(input)
                    * f64::from(b.calculate(level)).powf(f64::from(e.calculate(level))))
                    as f32
            }
            Fx::AllOf(list) => {
                let mut v = input;
                for fx in list {
                    v = fx.process(level, v)?;
                }
                v
            }
            Fx::Random => return None,
        })
    }

    /// An effect from its NBT (`{type: "minecraft:add", value: ...}`).
    pub(crate) fn from_tag(t: &Tag) -> Option<Fx> {
        let c = t.as_compound()?;
        let lvl = |k: &str| get(c, k).and_then(Lvl::from_tag);
        Some(match kind(c)? {
            "add" => Fx::Add(lvl("value")?),
            "multiply" => Fx::Multiply(lvl("factor")?),
            "set" => Fx::Set(lvl("value")?),
            "exponential" => Fx::Exponential(lvl("base")?, lvl("exponent")?),
            "all_of" => {
                let Tag::List(l) = get(c, "effects")? else {
                    return None;
                };
                Fx::AllOf(
                    l.items
                        .iter()
                        .map(Fx::from_tag)
                        .collect::<Option<Vec<_>>>()?,
                )
            }
            "remove_binomial" => Fx::Random,
            _ => return None,
        })
    }

    /// The text form (`Display`) back.
    pub(crate) fn parse(s: &str) -> Option<Fx> {
        let mut p = Parser {
            s: s.as_bytes(),
            i: 0,
        };
        let fx = p.fx()?;
        (p.i == p.s.len()).then_some(fx)
    }
}

fn get<'a>(c: &'a Compound, k: &str) -> Option<&'a Tag> {
    c.0.iter().rev().find(|(n, _)| n == k).map(|(_, v)| v)
}

fn kind(c: &Compound) -> Option<&str> {
    match get(c, "type")? {
        Tag::String(s) => Some(s.strip_prefix("minecraft:").unwrap_or(s)),
        _ => None,
    }
}

/// `Codec.FLOAT` over NBT: any numeric tag, as `Number.floatValue()`.
fn number(t: &Tag) -> Option<f32> {
    Some(match t {
        Tag::Byte(v) => f32::from(*v),
        Tag::Short(v) => f32::from(*v),
        Tag::Int(v) => *v as f32,
        Tag::Long(v) => *v as f32,
        Tag::Float(v) => *v,
        Tag::Double(v) => *v as f32,
        _ => return None,
    })
}

impl fmt::Display for Lvl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Lvl::Constant(v) => write!(f, "{v:?}"),
            Lvl::Linear(b, p) => write!(f, "linear({b:?},{p:?})"),
            Lvl::Clamped(v, min, max) => write!(f, "clamped({v},{min:?},{max:?})"),
            Lvl::Exponent(b, p) => write!(f, "exponent({b},{p})"),
            Lvl::Fraction(n, d) => write!(f, "fraction({n},{d})"),
            Lvl::LevelsSquared(a) => write!(f, "levels_squared({a:?})"),
            Lvl::Lookup(values, fallback) => {
                write!(f, "lookup([")?;
                for (i, v) in values.iter().enumerate() {
                    write!(f, "{}{v:?}", if i > 0 { "," } else { "" })?;
                }
                write!(f, "],{fallback})")
            }
        }
    }
}

impl fmt::Display for Fx {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Fx::Add(v) => write!(f, "add({v})"),
            Fx::Multiply(v) => write!(f, "multiply({v})"),
            Fx::Set(v) => write!(f, "set({v})"),
            Fx::Exponential(b, e) => write!(f, "exponential({b},{e})"),
            Fx::AllOf(list) => {
                write!(f, "all_of(")?;
                for (i, fx) in list.iter().enumerate() {
                    write!(f, "{}{fx}", if i > 0 { "," } else { "" })?;
                }
                write!(f, ")")
            }
            Fx::Random => write!(f, "random"),
        }
    }
}

struct Parser<'a> {
    s: &'a [u8],
    i: usize,
}

impl Parser<'_> {
    fn eat(&mut self, c: u8) -> Option<()> {
        (self.s.get(self.i) == Some(&c)).then(|| self.i += 1)
    }

    fn word(&mut self) -> &str {
        let start = self.i;
        while self
            .s
            .get(self.i)
            .is_some_and(|c| c.is_ascii_lowercase() || *c == b'_')
        {
            self.i += 1;
        }
        std::str::from_utf8(&self.s[start..self.i]).unwrap_or("")
    }

    fn num(&mut self) -> Option<f32> {
        let start = self.i;
        while self.s.get(self.i).is_some_and(|c| {
            c.is_ascii_digit()
                || matches!(
                    c,
                    b'-' | b'+' | b'.' | b'e' | b'E' | b'i' | b'n' | b'f' | b'N' | b'a'
                )
        }) {
            self.i += 1;
        }
        std::str::from_utf8(&self.s[start..self.i])
            .ok()?
            .parse()
            .ok()
    }

    fn lvl(&mut self) -> Option<Lvl> {
        let save = self.i;
        let w = self.word().to_string();
        if w.is_empty() || w == "inf" || w == "nan" {
            self.i = save;
            return self.num().map(Lvl::Constant);
        }
        self.eat(b'(')?;
        let v = match w.as_str() {
            "linear" => {
                let b = self.num()?;
                self.eat(b',')?;
                Lvl::Linear(b, self.num()?)
            }
            "clamped" => {
                let v = self.lvl()?;
                self.eat(b',')?;
                let min = self.num()?;
                self.eat(b',')?;
                Lvl::Clamped(Box::new(v), min, self.num()?)
            }
            "exponent" | "fraction" => {
                let a = self.lvl()?;
                self.eat(b',')?;
                let b = self.lvl()?;
                if w == "exponent" {
                    Lvl::Exponent(Box::new(a), Box::new(b))
                } else {
                    Lvl::Fraction(Box::new(a), Box::new(b))
                }
            }
            "levels_squared" => Lvl::LevelsSquared(self.num()?),
            "lookup" => {
                self.eat(b'[')?;
                let mut values = Vec::new();
                if self.eat(b']').is_none() {
                    loop {
                        values.push(self.num()?);
                        if self.eat(b']').is_some() {
                            break;
                        }
                        self.eat(b',')?;
                    }
                }
                self.eat(b',')?;
                Lvl::Lookup(values, Box::new(self.lvl()?))
            }
            _ => return None,
        };
        self.eat(b')')?;
        Some(v)
    }

    fn fx(&mut self) -> Option<Fx> {
        let w = self.word().to_string();
        if w == "random" {
            return Some(Fx::Random);
        }
        self.eat(b'(')?;
        let fx = match w.as_str() {
            "add" => Fx::Add(self.lvl()?),
            "multiply" => Fx::Multiply(self.lvl()?),
            "set" => Fx::Set(self.lvl()?),
            "exponential" => {
                let b = self.lvl()?;
                self.eat(b',')?;
                Fx::Exponential(b, self.lvl()?)
            }
            "all_of" => {
                let mut list = vec![self.fx()?];
                while self.eat(b',').is_some() {
                    list.push(self.fx()?);
                }
                Fx::AllOf(list)
            }
            _ => return None,
        };
        self.eat(b')')?;
        Some(fx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_round_trips_and_vanilla_riptide() {
        let riptide = Fx::Add(Lvl::Linear(1.5, 0.75));
        assert_eq!(riptide.to_string(), "add(linear(1.5,0.75))");
        assert_eq!(Fx::parse("add(linear(1.5,0.75))"), Some(riptide.clone()));
        assert_eq!(riptide.process(1, 0.0), Some(1.5));
        assert_eq!(riptide.process(3, 0.0), Some(3.0));
        let all = Fx::AllOf(vec![
            Fx::Multiply(Lvl::Clamped(Box::new(Lvl::LevelsSquared(-0.5)), 0.1, 7.0)),
            Fx::Set(Lvl::Lookup(
                vec![1.0, 2.5],
                Box::new(Lvl::Fraction(
                    Box::new(Lvl::Constant(1.0)),
                    Box::new(Lvl::Exponent(
                        Box::new(Lvl::Constant(2.0)),
                        Box::new(Lvl::Constant(-3.0)),
                    )),
                )),
            )),
            Fx::Exponential(Lvl::Constant(2.0), Lvl::Linear(0.0, 1.0)),
            Fx::Random,
        ]);
        assert_eq!(Fx::parse(&all.to_string()), Some(all.clone()));
        assert_eq!(all.process(1, 3.0), None);
        let lookup = Lvl::Lookup(vec![1.0, 2.5], Box::new(Lvl::Constant(9.0)));
        assert_eq!((lookup.calculate(2), lookup.calculate(3)), (2.5, 9.0));
    }
}
