//! Modern team collision rules for entity pushes: the part of the client scoreboard that
//! `EntitySelector.pushableBy` reads (each team's `collisionRule` and who is on which team),
//! kept from `set_player_team` packets only while entities are tracked.

use std::collections::HashMap;

use bytes::Bytes;
use ghast_protocol::codec::Reader;
use ghast_protocol::nbt::AnonNbt;

/// `Team.CollisionRule` network ids.
pub(crate) const ALWAYS: u8 = 0;
pub(crate) const NEVER: u8 = 1;
pub(crate) const PUSH_OTHER_TEAMS: u8 = 2;
pub(crate) const PUSH_OWN_TEAM: u8 = 3;

/// The order of a team's parameters in `set_player_team` (`Parameters.STREAM_CODEC`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)] // one variant per modern row; single-version builds use one
pub(crate) enum TeamLayout {
    /// 26.1: display name, options byte, name tag visibility, collision rule, colour
    /// (`ChatFormatting`), prefix, suffix.
    RuleBeforeAffixes,
    /// 26.2: display name, prefix, suffix, name tag visibility, collision rule, optional
    /// colour (`TeamColor`), options byte.
    AffixesFirst,
}

/// The client scoreboard's teams as far as collision rules go.
#[derive(Debug, Default)]
pub(crate) struct Teams {
    /// Team name → slot in `rules`.
    by_name: HashMap<Box<str>, u32>,
    /// Slot → collision rule (`None`: a free slot).
    rules: Vec<Option<u8>>,
    /// Scoreboard name (player name, or an entity's UUID string) → team slot.
    members: HashMap<Box<str>, u32>,
    /// Teams whose rule is not `always` (none: every push is allowed, nothing is looked up).
    strict: u32,
}

impl Teams {
    /// Forgets everything (a new play phase has a new scoreboard).
    pub(crate) fn clear(&mut self) {
        *self = Teams::default();
    }

    /// `true` when some team restricts pushing.
    #[inline]
    pub(crate) fn any_rule(&self) -> bool {
        self.strict > 0
    }

    /// The team slot and collision rule of a scoreboard name (`always` without a team).
    pub(crate) fn of(&self, name: &str) -> (Option<u32>, u8) {
        match self.members.get(name) {
            Some(&s) => (Some(s), self.rules[s as usize].unwrap_or(ALWAYS)),
            None => (None, ALWAYS),
        }
    }

    fn set_rule(&mut self, slot: u32, rule: u8) {
        let r = &mut self.rules[slot as usize];
        if r.is_some_and(|o| o != ALWAYS) {
            self.strict -= 1;
        }
        if rule != ALWAYS {
            self.strict += 1;
        }
        *r = Some(rule);
    }

    /// `ClientPacketListener.handleSetPlayerTeamPacket` for a `set_player_team` body. Only the
    /// name, the action, the collision rule and the member list are read (the components are
    /// skipped). Returns `false` for a body that does not parse (nothing more is applied).
    pub(crate) fn apply(&mut self, body: &Bytes, layout: TeamLayout) -> bool {
        self.apply_inner(body, layout).is_some()
    }

    fn apply_inner(&mut self, body: &Bytes, layout: TeamLayout) -> Option<()> {
        let mut r = Reader::new(body);
        let name = r.read_string().ok()?;
        let mode = r.read_i8().ok()?;
        // Action ADD (0): Scoreboard.addPlayerTeam (an existing team is kept); else the team
        // must exist (vanilla logs and ignores the packet).
        let slot = if mode == 0 {
            match self.by_name.get(&*name) {
                Some(&s) => s,
                None => {
                    let s = match self.rules.iter().position(Option::is_none) {
                        Some(i) => i as u32,
                        None => {
                            self.rules.push(None);
                            (self.rules.len() - 1) as u32
                        }
                    };
                    self.rules[s as usize] = Some(ALWAYS);
                    self.by_name.insert((&*name).into(), s);
                    s
                }
            }
        } else {
            *self.by_name.get(&*name)?
        };
        if mode == 0 || mode == 2 {
            AnonNbt::read(&mut r).ok()?; // display name
            let rule = match layout {
                TeamLayout::RuleBeforeAffixes => {
                    r.read_u8().ok()?; // options
                    r.read_varint().ok()?; // name tag visibility
                    let rule = r.read_varint().ok()?;
                    r.read_varint().ok()?; // color
                    AnonNbt::read(&mut r).ok()?; // prefix
                    AnonNbt::read(&mut r).ok()?; // suffix
                    rule
                }
                TeamLayout::AffixesFirst => {
                    AnonNbt::read(&mut r).ok()?; // prefix
                    AnonNbt::read(&mut r).ok()?; // suffix
                    r.read_varint().ok()?; // name tag visibility
                    let rule = r.read_varint().ok()?;
                    if r.read_bool().ok()? {
                        r.read_varint().ok()?; // color
                    }
                    r.read_u8().ok()?; // options
                    rule
                }
            };
            // ByIdMap.continuous with OutOfBoundsStrategy.ZERO: unknown ids are `always`.
            let rule = u8::try_from(rule)
                .ok()
                .filter(|&x| x <= PUSH_OWN_TEAM)
                .unwrap_or(ALWAYS);
            self.set_rule(slot, rule);
        }
        if mode == 0 || mode == 3 || mode == 4 {
            let n = r.read_varint().ok()?;
            for _ in 0..n.max(0) {
                let player = r.read_string().ok()?;
                if mode == 4 {
                    // removePlayerFromTeam(player, team): only a member of this team (vanilla
                    // throws otherwise; the membership stays).
                    if self.members.get(&*player) == Some(&slot) {
                        self.members.remove(&*player);
                    }
                } else {
                    // addPlayerToTeam: leaves its old team first.
                    self.members.insert((&*player).into(), slot);
                }
            }
        }
        if mode == 1 {
            // removePlayerTeam: its members have no team any more.
            self.members.retain(|_, s| *s != slot);
            if self.rules[slot as usize].is_some_and(|o| o != ALWAYS) {
                self.strict -= 1;
            }
            self.rules[slot as usize] = None;
            self.by_name.remove(&*name);
        }
        Some(())
    }

    /// `EntitySelector.pushableBy(pusher)` for the local player as the pushed entity (on the
    /// client only the local player passes the team test): the pusher's and the player's
    /// teams by scoreboard name.
    pub(crate) fn may_push(&self, pusher: &str, player: &str) -> bool {
        let (own, own_rule) = self.of(pusher);
        if own_rule == NEVER {
            return false;
        }
        let (theirs, their_rule) = self.of(player);
        if their_rule == NEVER {
            return false;
        }
        let same = own.is_some() && own == theirs;
        if (own_rule == PUSH_OWN_TEAM || their_rule == PUSH_OWN_TEAM) && same {
            return false;
        }
        (own_rule != PUSH_OTHER_TEAMS && their_rule != PUSH_OTHER_TEAMS) || same
    }
}

#[cfg(test)]
impl Teams {
    fn apply26(&mut self, body: &Bytes) -> bool {
        self.apply(body, TeamLayout::RuleBeforeAffixes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::BufMut;
    use ghast_protocol::McWrite;

    fn packet(team: &str, mode: i8, rule: Option<i32>, players: &[&str]) -> Bytes {
        let mut b = bytes::BytesMut::new();
        b.put_string(team).unwrap();
        b.put_i8(mode);
        if mode == 0 || mode == 2 {
            // Display name: an empty string tag; options, name tag visibility.
            b.put_u8(8);
            b.put_u16(0);
            b.put_u8(0);
            b.put_varint(0);
            b.put_varint(rule.unwrap_or(0));
            b.put_varint(15);
            b.put_u8(8);
            b.put_u16(0);
            b.put_u8(8);
            b.put_u16(0);
        }
        if mode == 0 || mode == 3 || mode == 4 {
            b.put_varint(players.len() as i32);
            for p in players {
                b.put_string(p).unwrap();
            }
        }
        b.freeze()
    }

    /// The vanilla predicate written out from `EntitySelector.pushableBy`, for the table test.
    fn vanilla(own: Option<(u32, u8)>, theirs: Option<(u32, u8)>) -> bool {
        let own_rule = own.map_or(ALWAYS, |t| t.1);
        if own_rule == NEVER {
            return false;
        }
        let their_rule = theirs.map_or(ALWAYS, |t| t.1);
        if their_rule == NEVER {
            return false;
        }
        let same = own.is_some() && own.map(|t| t.0) == theirs.map(|t| t.0);
        if (own_rule == PUSH_OWN_TEAM || their_rule == PUSH_OWN_TEAM) && same {
            false
        } else {
            own_rule != PUSH_OTHER_TEAMS && their_rule != PUSH_OTHER_TEAMS || same
        }
    }

    #[test]
    fn rules_follow_pushable_by() {
        // Every combination: pusher in team A / B / none, player in team A / none, any rules.
        for ra in 0..4u8 {
            for rb in 0..4u8 {
                let mut t = Teams::default();
                assert!(t.apply26(&packet("a", 0, Some(i32::from(ra)), &["me", "zombie-uuid"])));
                assert!(t.apply26(&packet("b", 0, Some(i32::from(rb)), &["cow-uuid"])));
                assert_eq!(t.any_rule(), ra != 0 || rb != 0);
                let a = Some((0, ra));
                let b = Some((1, rb));
                assert_eq!(t.may_push("zombie-uuid", "me"), vanilla(a, a), "{ra} {rb}");
                assert_eq!(t.may_push("cow-uuid", "me"), vanilla(b, a), "{ra} {rb}");
                assert_eq!(t.may_push("pig-uuid", "me"), vanilla(None, a), "{ra} {rb}");
                // The player leaves its team.
                assert!(t.apply26(&packet("a", 4, None, &["me"])));
                assert_eq!(t.may_push("zombie-uuid", "me"), vanilla(a, None));
                assert_eq!(t.may_push("cow-uuid", "me"), vanilla(b, None));
            }
        }
    }

    /// The 26.2 parameter order: display name, prefix, suffix, visibility, collision rule,
    /// optional colour, options (with and without a colour).
    #[test]
    fn layout_26_2() {
        for color in [None, Some(12)] {
            let mut b = bytes::BytesMut::new();
            b.put_string("a").unwrap();
            b.put_i8(0);
            for _ in 0..3 {
                b.put_u8(8);
                b.put_u16(0);
            }
            b.put_varint(1); // name tag visibility
            b.put_varint(i32::from(NEVER));
            match color {
                Some(c) => {
                    b.put_u8(1);
                    b.put_varint(c);
                }
                None => b.put_u8(0),
            }
            b.put_u8(3); // options
            b.put_varint(1);
            b.put_string("x").unwrap();
            let mut t = Teams::default();
            assert!(t.apply(&b.freeze(), TeamLayout::AffixesFirst), "{color:?}");
            assert_eq!(t.of("x"), (Some(0), NEVER));
        }
    }

    #[test]
    fn scoreboard_semantics() {
        let mut t = Teams::default();
        t.apply26(&packet("a", 0, Some(i32::from(NEVER)), &["x"]));
        assert!(!t.may_push("x", "me"));
        // Moving to another team leaves the first one.
        t.apply26(&packet("b", 0, Some(0), &[]));
        t.apply26(&packet("b", 3, None, &["x"]));
        assert!(t.may_push("x", "me"));
        // A remove for a team the member is not on keeps the membership.
        t.apply26(&packet("a", 4, None, &["x"]));
        assert_eq!(t.of("x"), (Some(1), ALWAYS));
        // Update info changes the rule; ADD on an existing team keeps it (and its members).
        t.apply26(&packet("b", 2, Some(i32::from(NEVER)), &[]));
        assert!(!t.may_push("x", "me"));
        t.apply26(&packet("b", 0, Some(i32::from(NEVER)), &["y"]));
        assert_eq!(t.of("x"), (Some(1), NEVER));
        // Removing a team frees its members and its slot.
        t.apply26(&packet("b", 1, None, &[]));
        assert_eq!(t.of("x"), (None, ALWAYS));
        assert!(t.may_push("y", "me"));
        assert_eq!(t.strict, 1);
        t.apply26(&packet("a", 1, None, &[]));
        assert!(!t.any_rule());
        // Unknown team, unknown rule id, a truncated body.
        assert!(!t.apply26(&packet("zzz", 3, None, &["x"])));
        t.apply26(&packet("c", 0, Some(9), &["x"]));
        assert_eq!(t.of("x"), (Some(0), ALWAYS));
        let p = packet("c", 3, None, &["y"]);
        assert!(!t.apply26(&p.slice(..p.len() - 1)));
    }
}

/// Cost probe: `GHAST_TEAMS_N` team packets (create with one member, add a member, update the
/// rule, remove) through [`Teams::apply`], counted with perfcount against `GHAST_TEAMS_N=0`.
#[cfg(test)]
#[test]
#[ignore = "cost probe"]
fn teams_cost() {
    let n: usize = std::env::var("GHAST_TEAMS_N")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let pk = |team: &str, mode: i8, players: &[&str]| tests_packet(team, mode, Some(1), players);
    let names: Vec<String> = (0..64).map(|i| format!("team{i}")).collect();
    let packets: Vec<Bytes> = names
        .iter()
        .flat_map(|t| {
            [
                pk(t, 0, &["Steve_123"]),
                pk(t, 3, &["Alex_4567"]),
                pk(t, 2, &[]),
                pk(t, 1, &[]),
            ]
        })
        .collect();
    let mut t = Teams::default();
    let mut ok = 0;
    for i in 0..n {
        ok += usize::from(t.apply(&packets[i % packets.len()], TeamLayout::RuleBeforeAffixes));
    }
    println!("{ok}");
}

#[cfg(test)]
fn tests_packet(team: &str, mode: i8, rule: Option<i32>, players: &[&str]) -> Bytes {
    use bytes::BufMut;
    use ghast_protocol::McWrite;
    let mut b = bytes::BytesMut::new();
    b.put_string(team).unwrap();
    b.put_i8(mode);
    if mode == 0 || mode == 2 {
        // A display name component (a short text compound), then the options.
        b.put_u8(10);
        b.put_u8(8);
        b.put_u16(4);
        b.put_slice(b"text");
        b.put_u16(6);
        b.put_slice(b"Team A");
        b.put_u8(0);
        b.put_u8(0);
        b.put_varint(0);
        b.put_varint(rule.unwrap_or(0));
        b.put_varint(15);
        b.put_u8(8);
        b.put_u16(0);
        b.put_u8(8);
        b.put_u16(0);
    }
    if mode == 0 || mode == 3 || mode == 4 {
        b.put_varint(players.len() as i32);
        for p in players {
            b.put_string(p).unwrap();
        }
    }
    b.freeze()
}
