//! Villager trading on 1.8.9, 1.12.2 and 1.16.5: the offers the server sent, the trade the
//! client picks for the two payment slots, the output it shows, and what taking it does.
//!
//! The legacy client computes the output itself: every change of the payment slots (and every
//! `markDirty` of the trading inventory, also the server's own slot packets) runs the trading
//! inventory's offer lookup, which writes the matching offer's result into the output slot (or
//! empties it). Taking the output uses the payments up and counts one use of the offer on the
//! client. Offers come from the server (1.8 / 1.12 `MC|TrList`, 1.16.5 `trade_list`) and are
//! not looked at again when they arrive: the output only follows on the next slot change.

use crate::rules::Version;
use crate::sim::Model;
use crate::stack::{ItemData, Stack, last, number, tag_eq};
use ghast_protocol::nbt::{Compound, RawNbt, Tag};

/// One villager offer as the client holds it (1.8 / 1.12 `TradeOffer` from `MC|TrList`,
/// 1.16.5 from `trade_list`).
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct TradeOffer {
    /// The first payment (`None`: an empty stack, which matches nothing).
    pub first: Option<Stack>,
    /// The second payment, if the offer has one.
    pub second: Option<Stack>,
    /// What the offer sells (`None`: an empty stack).
    pub result: Option<Stack>,
    /// Uses so far (the client counts its own trades; a disabled offer arrives with
    /// `uses = max_uses`).
    pub uses: i32,
    /// Uses before the offer is disabled.
    pub max_uses: i32,
    /// 1.16.5: the villager experience the trade gives.
    pub xp: i32,
    /// 1.16.5: the price change from reputation and effects (added to the first payment).
    pub special_price: i32,
    /// 1.16.5: how strongly demand raises the first payment.
    pub price_multiplier: f32,
    /// 1.16.5: the demand bonus.
    pub demand: i32,
}

impl TradeOffer {
    /// A 1.8 / 1.12 offer (the 1.16.5 price fields zero).
    #[must_use]
    pub fn new(
        first: Option<Stack>,
        second: Option<Stack>,
        result: Option<Stack>,
        uses: i32,
        max_uses: i32,
    ) -> TradeOffer {
        TradeOffer {
            first,
            second,
            result,
            uses,
            max_uses,
            xp: 0,
            special_price: 0,
            price_multiplier: 0.0,
            demand: 0,
        }
    }

    /// `TradeOffer.isDisabled`: no uses left.
    #[must_use]
    pub fn disabled(&self) -> bool {
        self.uses >= self.max_uses
    }

    /// 1.16.5 `getAdjustedFirstBuyItem`: the first payment with demand and the special price
    /// applied, kept within 1 and the item's stack size. 1.8 / 1.12: the first payment.
    pub(crate) fn first_price(&self, v: Version) -> Option<Stack> {
        let first = self.first.as_ref()?;
        if v != Version::V1_16_5 {
            return Some(first.clone());
        }
        let i = first.count;
        // Java: (i * demand) as int, then times the float, floored
        let f = i.wrapping_mul(self.demand) as f32 * self.price_multiplier;
        let fl = {
            let t = f as i32;
            if f < t as f32 { t - 1 } else { t }
        };
        let j = fl.max(0);
        let n = i.wrapping_add(j).wrapping_add(self.special_price);
        Some(first.with_count(n.clamp(1, v.max_count(first))))
    }
}

/// The open trading screen's state on the client: the server's offers, the selected offer and
/// the offer the output slot currently belongs to.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Merchant {
    /// The offers (`None` until the server sent them; 1.16.5 treats an empty list the same).
    pub offers: Option<Vec<TradeOffer>>,
    /// The screen's selected offer (`setRecipeIndex`; 0 = the first offer that matches).
    pub index: i32,
    /// The offer of the last lookup (`tradeOffer`), which taking the output uses.
    pub current: Option<usize>,
}

/// `TradeOffer`'s item test for one payment slot (`given`) against an offer's stack
/// (`sample`): 1.8 / 1.12 same item and damage (a missing stack matches nothing); 1.16.5 two
/// empties match, else the same item. Then the sample's NBT, if any, must be contained in the
/// given stack's (`NbtHelper.matches`; 1.16.5 reads a damageable stack's NBT with its
/// `Damage` written, as its copy does).
fn accepts(v: Version, given: Option<&Stack>, sample: Option<&Stack>) -> bool {
    let (g, s) = match (given, sample) {
        (Some(g), Some(s)) => (g, s),
        (None, None) => return v == Version::V1_16_5,
        _ => return false,
    };
    if g.id != s.id || (v != Version::V1_16_5 && g.damage != s.damage) {
        return false;
    }
    let Some(want) = s.nbt() else {
        return true;
    };
    let Ok(Some(want)) = want.parse() else {
        // an unreadable sample matches nothing ghast can show; vanilla never sends one
        return false;
    };
    let got = if v == Version::V1_16_5 && v.is_damageable(g) {
        let mut c = match g.nbt().map(RawNbt::parse) {
            Some(Ok(Some(t))) => match t.tag {
                Tag::Compound(c) => c,
                _ => return false,
            },
            None => Compound::default(),
            Some(_) => return false,
        };
        // getDamage() then setDamage(max(0, damage)): the int replaces any `Damage` entry
        let d = last(&c, "Damage").and_then(number).unwrap_or(0) as i32;
        c.0.retain(|(k, _)| k != "Damage");
        c.0.push(("Damage".into(), Tag::Int(d.max(0))));
        Tag::Compound(c)
    } else {
        match g.nbt().map(RawNbt::parse) {
            Some(Ok(Some(t))) => t.tag,
            _ => return false,
        }
    };
    nbt_contains(&want.tag, &got)
}

/// `NbtHelper.matches(standard, subject, false)`: compounds are compared key by key (the
/// subject may have more keys), everything else must be equal.
fn nbt_contains(standard: &Tag, subject: &Tag) -> bool {
    if standard.type_id() != subject.type_id() {
        return false;
    }
    match (standard, subject) {
        (Tag::Compound(a), Tag::Compound(b)) => a.0.iter().enumerate().all(|(i, (k, v))| {
            // Java's map: the last entry of a key is the one that counts
            if a.0[i + 1..].iter().any(|(k2, _)| k2 == k) {
                return true;
            }
            last(b, k).is_some_and(|w| nbt_contains(v, w))
        }),
        _ => tag_eq(standard, subject),
    }
}

fn count(s: Option<&Stack>) -> i32 {
    s.map_or(0, |s| s.count)
}

/// 1.8 / 1.12 `TraderOfferList.getValidRecipe` and 1.16.5 `TradeOfferList.getValidOffer`:
/// the offer the two payments (in this order) buy.
fn lookup(
    v: Version,
    offers: &[TradeOffer],
    first: Option<&Stack>,
    second: Option<&Stack>,
    index: i32,
) -> Option<usize> {
    let fits = |o: &TradeOffer| -> bool {
        if v == Version::V1_16_5 {
            let price = o.first_price(v);
            return accepts(v, first, price.as_ref())
                && count(first) >= count(price.as_ref())
                && accepts(v, second, o.second.as_ref())
                && count(second) >= count(o.second.as_ref());
        }
        accepts(v, first, o.first.as_ref())
            && count(first) >= count(o.first.as_ref())
            && match &o.second {
                None => second.is_none(),
                Some(b) => accepts(v, second, Some(b)) && count(second) >= b.count,
            }
    };
    match usize::try_from(index) {
        Ok(k) if k > 0 && k < offers.len() => fits(&offers[k]).then_some(k),
        _ => offers.iter().position(fits),
    }
}

/// The trading inventory's offer lookup (1.8 / 1.12 `updateRecipes`, 1.16.5 `updateOffers`):
/// writes the output slot (container slot 2) and remembers the offer. With no offers from the
/// server and a payment present, the output is left as it is.
pub(crate) fn update(v: Version, m: &mut Model) {
    let Some(mer) = m.merchant.as_deref_mut() else {
        return;
    };
    if m.container.len() != 3 {
        return;
    }
    mer.current = None;
    let (a, b) = if m.container[0].is_none() {
        (m.container[1].as_ref(), None)
    } else {
        (m.container[0].as_ref(), m.container[1].as_ref())
    };
    if a.is_none() {
        m.container[2] = None;
        return;
    }
    let Some(offers) = mer.offers.as_deref().filter(|o| !o.is_empty()) else {
        return;
    };
    let idx = mer.index;
    let usable = |k: Option<usize>| k.filter(|&k| !offers[k].disabled());
    let found = if v == Version::V1_16_5 {
        let t = lookup(v, offers, a, b, idx);
        match usable(t) {
            Some(k) => Some(k),
            None => {
                // a disabled first match stays the remembered offer unless the swap finds one
                mer.current = t;
                usable(lookup(v, offers, b, a, idx))
            }
        }
    } else {
        match usable(lookup(v, offers, a, b, idx)) {
            Some(k) => Some(k),
            None if b.is_some() => usable(lookup(v, offers, b, a, idx)),
            None => None,
        }
    };
    let out = match found {
        Some(k) => {
            mer.current = Some(k);
            let mut r = offers[k].result.clone();
            crate::sim::clamp(&mut r);
            r
        }
        None => None,
    };
    m.container[2] = out;
}

/// `TradeOutputSlot.depleteBuyItems(first, second)`: takes the offer's payments out of the two
/// stacks; false when they do not pay for it (1.8: same items only; 1.12: and enough of them;
/// 1.16.5: the whole price test).
fn deplete(
    v: Version,
    o: &TradeOffer,
    first: &mut Option<Stack>,
    second: &mut Option<Stack>,
) -> bool {
    match v {
        Version::V1_16_5 => {
            let price = o.first_price(v);
            let ok = accepts(v, first.as_ref(), price.as_ref())
                && count(first.as_ref()) >= count(price.as_ref())
                && accepts(v, second.as_ref(), o.second.as_ref())
                && count(second.as_ref()) >= count(o.second.as_ref());
            if !ok {
                return false;
            }
            if let Some(f) = first {
                f.count -= count(price.as_ref());
            }
            if let (Some(s), Some(b)) = (second.as_mut(), &o.second) {
                s.count -= b.count;
            }
            true
        }
        _ => {
            let (Some(f), Some(a)) = (first.as_mut(), &o.first) else {
                return false;
            };
            if f.id != a.id || (v == Version::V1_12 && f.count < a.count) {
                return false;
            }
            match (&o.second, second.as_mut()) {
                (Some(b), Some(s))
                    if s.id == b.id && (v != Version::V1_12 || s.count >= b.count) =>
                {
                    f.count -= a.count;
                    s.count -= b.count;
                    true
                }
                (None, None) => {
                    f.count -= a.count;
                    true
                }
                _ => false,
            }
        }
    }
}

/// Whether taking `out` runs item code ghast does not model (`Item.onCraft`: a filled map
/// with NBT may be rescaled or get a tracking flag, which needs the world's map data).
pub(crate) fn take_needs_world(v: Version, out: &Stack) -> bool {
    let map = match v {
        Version::V1_8 | Version::V1_12 => out.id == 358,
        _ => v.name(out.id) == "filled_map",
    };
    map && matches!(out.data, ItemData::Nbt(_))
}

/// `TradeOutputSlot.onTakeItem`: with an offer remembered, the payments pay for it (in slot
/// order, else swapped), the client counts a use of the offer and the payment slots are set
/// again (which looks the offer up again). Nothing happens without a remembered offer.
pub(crate) fn take_output(v: Version, m: &mut Model) {
    let Some(mer) = m.merchant.as_deref_mut() else {
        return;
    };
    let (Some(k), Some(offers)) = (mer.current, mer.offers.as_mut()) else {
        return;
    };
    let o = offers[k].clone();
    let (mut a, mut b) = (m.container[0].clone(), m.container[1].clone());
    if !(deplete(v, &o, &mut a, &mut b) || deplete(v, &o, &mut b, &mut a)) {
        return;
    }
    offers[k].uses = offers[k].uses.wrapping_add(1);
    let gone = |s: Option<Stack>| s.filter(|s| s.count > 0);
    m.container[0] = gone(a);
    m.container[1] = gone(b);
    update(v, m);
}

impl crate::sim::Sim<'_> {
    /// 1.16.5 `MerchantScreenHandler.switchTo(index)` after the screen picked an offer: the
    /// payments go back into the player part (last slot first; a payment that does not move
    /// at all stops here), then, with both payment slots empty, the offer's payments are
    /// gathered from the player part (stacks of the same item and NBT, up to a full stack).
    pub(crate) fn switch_trade(&mut self, index: usize) {
        let Some(o) = self
            .m
            .merchant
            .as_ref()
            .and_then(|m| m.offers.as_ref()?.get(index).cloned())
        else {
            return;
        };
        for k in 0..2 {
            if let Some(mut st) = self.get(k).cloned() {
                // the menu's insertItem never refuses here: player slots take anything
                if !self.insert(&mut st, 3, 39, true).unwrap_or(false) {
                    return;
                }
                let left = self.norm(st);
                self.set(k, left);
            }
        }
        if self.get(0).is_some() || self.get(1).is_some() {
            return;
        }
        self.autofill(0, o.first_price(self.v).as_ref());
        self.autofill(1, o.second.as_ref());
    }

    /// `MerchantScreenHandler.autofill(slot, stack)`.
    fn autofill(&mut self, slot: usize, want: Option<&Stack>) {
        let Some(want) = want else { return };
        let max = self.v.max_count(want);
        for i in 3..39 {
            let Some(t) = self.get(i).cloned() else {
                continue;
            };
            // `ItemStack.areTagsEqual`: the same NBT (or none on both)
            if t.id != want.id || !t.nbt_eq(want) {
                continue;
            }
            let j = self.get(slot).map_or(0, |s| s.count);
            let k = (max - j).min(t.count);
            let l = j + k;
            let rest = t.with_count(t.count - k);
            *self.slot_mut(i) = self.norm(rest);
            self.set(slot, Some(t.with_count(l)));
            if l >= max {
                break;
            }
        }
    }
}

/// Reads a 1.8 / 1.12 `MC|TrList` payload: the window id, then the offers
/// (`TraderOfferList.fromPacket`). `None` when it does not parse.
#[cfg(any(feature = "v1_8", feature = "v1_12"))]
pub(crate) fn parse_legacy(data: &bytes::Bytes) -> Option<(i32, Vec<TradeOffer>)> {
    use ghast_protocol::codec::{Decode, Reader};
    use ghast_protocol::types::slot::legacy;
    let mut r = Reader::new(data);
    let window = r.read_i32().ok()?;
    let n = r.read_u8().ok()?;
    let mut offers = Vec::with_capacity(usize::from(n));
    let slot = |r: &mut Reader<'_>| -> Option<Option<Stack>> {
        Some(crate::stack::from_legacy_slot(
            legacy::Slot::decode(r).ok()?,
        ))
    };
    for _ in 0..n {
        let first = slot(&mut r)?;
        let result = slot(&mut r)?;
        let second = if r.read_bool().ok()? {
            slot(&mut r)?
        } else {
            None
        };
        let disabled = r.read_bool().ok()?;
        let uses = r.read_i32().ok()?;
        let max_uses = r.read_i32().ok()?;
        offers.push(TradeOffer::new(
            first,
            second,
            result,
            if disabled { max_uses } else { uses },
            max_uses,
        ));
    }
    Some((window, offers))
}

/// The offers of a 1.16.5 `trade_list` (the client stores them through their NBT form, which
/// keeps the item counts as bytes).
#[cfg(feature = "v1_16_5")]
pub(crate) fn from_v1_16_5(
    p: &ghast_protocol::v1_16_5::play::clientbound::TradeList,
) -> Vec<TradeOffer> {
    let stack = |s: &ghast_protocol::types::slot::modern::Slot| {
        crate::stack::from_modern_slot(s.clone()).and_then(|s| {
            // ItemStack.fromNbt reads `Count` as a byte
            let c = i32::from(s.count as i8);
            (c > 0).then(|| s.with_count(c))
        })
    };
    p.trades
        .iter()
        .map(|t| TradeOffer {
            first: stack(&t.input_item1),
            second: t.input_item2.as_ref().and_then(stack),
            result: stack(&t.output_item),
            uses: if t.trade_disabled {
                t.maximum_nb_trade_uses
            } else {
                t.nb_trade_uses
            },
            max_uses: t.maximum_nb_trade_uses,
            xp: t.xp,
            special_price: t.special_price,
            price_multiplier: t.price_multiplier,
            demand: t.demand,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(all(feature = "v1_12", feature = "v1_16_5"))]
    fn nbt(t: Tag) -> RawNbt {
        RawNbt::from_named(&ghast_protocol::nbt::NamedTag {
            name: String::new(),
            tag: t,
        })
        .unwrap()
    }

    #[test]
    fn nbt_subset_matches() {
        let want = Tag::Compound(Compound(vec![("a".into(), Tag::Int(1))]));
        let got = Tag::Compound(Compound(vec![
            ("b".into(), Tag::Int(2)),
            ("a".into(), Tag::Int(1)),
        ]));
        assert!(nbt_contains(&want, &got));
        assert!(!nbt_contains(&got, &want));
        assert!(!nbt_contains(&Tag::Int(1), &Tag::Short(1)));
    }

    #[cfg(all(feature = "v1_12", feature = "v1_16_5"))]
    #[test]
    fn damage_written_on_1_16() {
        // a 1.16.5 diamond sword (damageable) without NBT reads as {Damage:0}
        let v = Version::V1_16_5;
        let sword = (1..2000).find(|&i| v.name(i) == "diamond_sword").unwrap();
        let sample = Stack::new(sword, 1, 0).with_nbt(Some(nbt(Tag::Compound(Compound(vec![(
            "Damage".into(),
            Tag::Int(0),
        )])))));
        assert!(accepts(v, Some(&Stack::new(sword, 1, 0)), Some(&sample)));
        // 1.12: no such rule
        assert!(!accepts(
            Version::V1_12,
            Some(&Stack::new(276, 1, 0)),
            Some(
                &Stack::new(276, 1, 0).with_nbt(Some(nbt(Tag::Compound(Compound(vec![(
                    "Damage".into(),
                    Tag::Int(0),
                )])))))
            )
        ));
    }

    #[cfg(feature = "v1_16_5")]
    #[test]
    fn adjusted_price() {
        let mut o = TradeOffer::new(Some(Stack::new(1, 10, 0)), None, None, 0, 1);
        o.demand = 3;
        o.price_multiplier = 0.05;
        o.special_price = -2;
        // 10 + floor(10 * 3 * 0.05) - 2 = 9
        assert_eq!(o.first_price(Version::V1_16_5).unwrap().count, 9);
        o.special_price = -100;
        assert_eq!(o.first_price(Version::V1_16_5).unwrap().count, 1);
    }
}
