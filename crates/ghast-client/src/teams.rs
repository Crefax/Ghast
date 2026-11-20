//! The client scoreboard's teams on every version (`Interests::TEAMS`): who is on which team,
//! each team's prefix, suffix, colour and options, as the vanilla client keeps them from the
//! scoreboard team packets (1.8 `scoreboard_team`, 1.9 – 1.16.5 `teams`, 26.x
//! `set_player_team`).
//!
//! ```
//! let teams = ghast_client::teams::Teams::default();
//! assert!(teams.team_of("Steve").is_none());
//! ```

use std::collections::HashMap;

use bytes::Bytes;
use ghast_protocol::codec::Reader;
use ghast_protocol::nbt::AnonNbt;

/// The field order of a version's team packet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)] // one variant per version row; single-version builds use one
pub(crate) enum Layout {
    /// 1.8: display name, prefix, suffix (strings), options byte, name tag visibility
    /// (string), colour byte.
    V1_8,
    /// 1.9 – 1.12.2: as 1.8 with the collision rule (string) after the name tag visibility.
    V1_12,
    /// 1.13 – 1.16.5: display name (chat JSON), options byte, name tag visibility and collision
    /// rule (strings), colour (VarInt), prefix and suffix (chat JSON).
    V1_16,
    /// 26.1: display name (NBT component), options byte, name tag visibility and collision rule
    /// (VarInt enums), colour (VarInt), prefix, suffix (NBT components).
    V26_1,
    /// 26.2: display name, prefix, suffix (NBT components), name tag visibility and collision
    /// rule (VarInt enums), optional colour (VarInt), options byte.
    V26_2,
}

/// The body of one team packet ([`crate::common::Inbound::Teams`]): parsed by the engine only
/// when the behaviour asks for [`crate::Interests::TEAMS`] (busy servers send many).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TeamsPacket {
    body: Bytes,
    layout: Layout,
}

impl TeamsPacket {
    pub(crate) fn new(body: Bytes, layout: Layout) -> Self {
        TeamsPacket { body, layout }
    }

    /// The raw packet body.
    #[must_use]
    pub fn body(&self) -> &Bytes {
        &self.body
    }
}

/// One scoreboard team (`#[non_exhaustive]`: read-only, fields may be added).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Team {
    /// The team's name (its key).
    pub name: Box<str>,
    /// Display name as plain text (1.8 / 1.12 the string as sent, with any `§` codes).
    pub display_name: String,
    /// Prefix shown before member names (plain text; legacy: as sent, with `§` codes).
    pub prefix: String,
    /// Suffix shown after member names.
    pub suffix: String,
    /// Colour as a chat formatting colour index 0 – 15 (0 black … 15 white); `None` for none
    /// / reset.
    pub color: Option<u8>,
    /// Members may hurt each other (options bit 0, `allowFriendlyFire`).
    pub friendly_fire: bool,
    /// Members see invisible team mates (options bit 1).
    pub see_friendly_invisibles: bool,
    /// Name tag visibility: `always`, `never`, `hideForOtherTeams` or `hideForOwnTeam`.
    pub name_tag_visibility: &'static str,
    /// Collision rule (1.9+; `always` on 1.8): `always`, `never`, `pushOtherTeams` or
    /// `pushOwnTeam`.
    pub collision_rule: &'static str,
    members: Vec<Box<str>>,
}

impl Team {
    fn new(name: &str) -> Self {
        Team {
            name: name.into(),
            display_name: name.to_owned(),
            prefix: String::new(),
            suffix: String::new(),
            color: None,
            friendly_fire: true,
            see_friendly_invisibles: true,
            name_tag_visibility: VISIBILITY[0],
            collision_rule: COLLISION[0],
            members: Vec::new(),
        }
    }

    /// The members' scoreboard names (player names; other entities by UUID string), in the
    /// order they joined.
    #[must_use]
    pub fn members(&self) -> &[Box<str>] {
        &self.members
    }
}

/// `Team.Visibility` names by network id.
const VISIBILITY: [&str; 4] = ["always", "never", "hideForOtherTeams", "hideForOwnTeam"];
/// `Team.CollisionRule` names by network id.
const COLLISION: [&str; 4] = ["always", "never", "pushOtherTeams", "pushOwnTeam"];

/// A visibility / collision name as sent (1.8 – 1.16.5); an unknown one keeps the old value.
fn known(names: &[&'static str; 4], s: &str, old: &'static str) -> &'static str {
    names.iter().copied().find(|n| *n == s).unwrap_or(old)
}

/// The client scoreboard's teams.
#[derive(Debug, Clone, Default)]
pub struct Teams {
    teams: HashMap<Box<str>, Team>,
    /// Scoreboard name → team name.
    members: HashMap<Box<str>, Box<str>>,
}

impl Teams {
    /// The team of a scoreboard name (a player's name), if any.
    #[must_use]
    pub fn team_of(&self, name: &str) -> Option<&Team> {
        self.teams.get(self.members.get(name)?)
    }

    /// The team with this name.
    #[must_use]
    pub fn team(&self, name: &str) -> Option<&Team> {
        self.teams.get(name)
    }

    /// `true` when both names are on the same team.
    #[must_use]
    pub fn same_team(&self, a: &str, b: &str) -> bool {
        matches!((self.members.get(a), self.members.get(b)), (Some(x), Some(y)) if x == y)
    }

    /// Every team, in no particular order.
    pub fn iter(&self) -> impl Iterator<Item = &Team> {
        self.teams.values()
    }

    /// Number of teams.
    #[must_use]
    pub fn len(&self) -> usize {
        self.teams.len()
    }

    /// `true` without teams.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.teams.is_empty()
    }

    /// Applies one team packet (`handleTeam` / `handleSetPlayerTeamPacket`); `false` for a body
    /// that does not parse (nothing is changed then) or a team that does not exist.
    pub(crate) fn apply(&mut self, p: &TeamsPacket) -> bool {
        self.apply_inner(&p.body, p.layout).is_some()
    }

    fn apply_inner(&mut self, body: &Bytes, layout: Layout) -> Option<()> {
        let mut r = Reader::new(body);
        let name = r.read_string().ok()?;
        let mode = r.read_i8().ok()?;
        // Parse everything first: a malformed body changes nothing.
        let info = if mode == 0 || mode == 2 {
            Some(read_info(&mut r, layout)?)
        } else {
            None
        };
        let players = if mode == 0 || mode == 3 || mode == 4 {
            let n = r.read_varint().ok()?;
            let mut v = Vec::with_capacity(n.clamp(0, 64) as usize);
            for _ in 0..n.max(0) {
                v.push(Box::<str>::from(&*r.read_string().ok()?));
            }
            v
        } else {
            Vec::new()
        };
        let key: Box<str> = (&*name).into();
        match mode {
            // ADD: `Scoreboard.addTeam` (an existing team is kept, its values are replaced).
            0 => {
                self.teams
                    .entry(key.clone())
                    .or_insert_with(|| Team::new(&name));
            }
            // The others need the team (vanilla ignores the packet otherwise).
            _ if !self.teams.contains_key(&key) => return None,
            // REMOVE: its members have no team any more.
            1 => {
                self.teams.remove(&key);
                self.members.retain(|_, t| *t != key);
                return Some(());
            }
            _ => {}
        }
        if let Some(i) = info {
            let t = self.teams.get_mut(&key)?;
            i.apply(t);
        }
        for p in players {
            if mode == 4 {
                // removePlayerFromTeam: only a member of this team.
                if self.members.get(&p) == Some(&key) {
                    self.members.remove(&p);
                    if let Some(t) = self.teams.get_mut(&key) {
                        t.members.retain(|m| *m != p);
                    }
                }
            } else {
                // addPlayerToTeam: leaves its old team first.
                if let Some(old) = self.members.insert(p.clone(), key.clone())
                    && let Some(t) = self.teams.get_mut(&old)
                {
                    t.members.retain(|m| *m != p);
                }
                if let Some(t) = self.teams.get_mut(&key) {
                    t.members.push(p);
                }
            }
        }
        Some(())
    }
}

/// A team's values from an ADD / UPDATE packet.
struct Info {
    display_name: String,
    prefix: String,
    suffix: String,
    options: u8,
    visibility: Name,
    collision: Option<Name>,
    color: Option<u8>,
}

/// A visibility or collision value: a name (1.8 – 1.16.5) or a network id (26.x).
enum Name {
    Text(String),
    Id(i32),
}

impl Name {
    fn resolve(&self, names: &[&'static str; 4], old: &'static str) -> &'static str {
        match self {
            Name::Text(s) => known(names, s, old),
            // ByIdMap with OutOfBoundsStrategy.ZERO.
            Name::Id(i) => usize::try_from(*i)
                .ok()
                .and_then(|i| names.get(i).copied())
                .unwrap_or(names[0]),
        }
    }
}

impl Info {
    fn apply(self, t: &mut Team) {
        t.display_name = self.display_name;
        t.prefix = self.prefix;
        t.suffix = self.suffix;
        t.friendly_fire = self.options & 1 != 0;
        t.see_friendly_invisibles = self.options & 2 != 0;
        t.name_tag_visibility = self.visibility.resolve(&VISIBILITY, t.name_tag_visibility);
        if let Some(c) = self.collision {
            t.collision_rule = c.resolve(&COLLISION, t.collision_rule);
        }
        t.color = self.color;
    }
}

/// A colour index: 0 – 15 are the colours, anything else (reset −1 / 21, formats) none.
fn color(v: i32) -> Option<u8> {
    u8::try_from(v).ok().filter(|&c| c < 16)
}

fn text_nbt(r: &mut Reader<'_>) -> Option<String> {
    Some(ghast_protocol::chat::nbt_to_plain_text(
        &AnonNbt::read(r).ok()?,
    ))
}

fn text_json(r: &mut Reader<'_>) -> Option<String> {
    Some(crate::common::chat_to_plain(&r.read_string().ok()?))
}

fn read_info(r: &mut Reader<'_>, layout: Layout) -> Option<Info> {
    let s = |r: &mut Reader<'_>| r.read_string().ok().map(|s| s.to_string());
    Some(match layout {
        Layout::V1_8 | Layout::V1_12 => {
            let display_name = s(r)?;
            let prefix = s(r)?;
            let suffix = s(r)?;
            let options = r.read_u8().ok()?;
            let visibility = Name::Text(s(r)?);
            let collision = if layout == Layout::V1_12 {
                Some(Name::Text(s(r)?))
            } else {
                None
            };
            let color = color(i32::from(r.read_i8().ok()?));
            Info {
                display_name,
                prefix,
                suffix,
                options,
                visibility,
                collision,
                color,
            }
        }
        Layout::V1_16 => {
            let display_name = text_json(r)?;
            let options = r.read_u8().ok()?;
            let visibility = Name::Text(s(r)?);
            let collision = Some(Name::Text(s(r)?));
            let color = color(r.read_varint().ok()?);
            Info {
                display_name,
                prefix: text_json(r)?,
                suffix: text_json(r)?,
                options,
                visibility,
                collision,
                color,
            }
        }
        Layout::V26_1 => {
            let display_name = text_nbt(r)?;
            let options = r.read_u8().ok()?;
            let visibility = Name::Id(r.read_varint().ok()?);
            let collision = Some(Name::Id(r.read_varint().ok()?));
            let color = color(r.read_varint().ok()?);
            Info {
                display_name,
                prefix: text_nbt(r)?,
                suffix: text_nbt(r)?,
                options,
                visibility,
                collision,
                color,
            }
        }
        Layout::V26_2 => {
            let display_name = text_nbt(r)?;
            let prefix = text_nbt(r)?;
            let suffix = text_nbt(r)?;
            let visibility = Name::Id(r.read_varint().ok()?);
            let collision = Some(Name::Id(r.read_varint().ok()?));
            let color = if r.read_bool().ok()? {
                color(r.read_varint().ok()?)
            } else {
                None
            };
            Info {
                display_name,
                prefix,
                suffix,
                options: r.read_u8().ok()?,
                visibility,
                collision,
                color,
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::BufMut;
    use ghast_protocol::McWrite;

    fn legacy(team: &str, mode: i8, layout: Layout, players: &[&str]) -> TeamsPacket {
        let mut b = bytes::BytesMut::new();
        b.put_string(team).unwrap();
        b.put_i8(mode);
        if mode == 0 || mode == 2 {
            b.put_string("Red Team").unwrap();
            b.put_string("§c[R] ").unwrap();
            b.put_string("").unwrap();
            b.put_u8(0x02); // no friendly fire, sees invisible mates
            b.put_string("hideForOtherTeams").unwrap();
            if layout == Layout::V1_12 {
                b.put_string("pushOwnTeam").unwrap();
            }
            b.put_i8(12); // red
        }
        if mode == 0 || mode == 3 || mode == 4 {
            b.put_varint(players.len() as i32);
            for p in players {
                b.put_string(p).unwrap();
            }
        }
        TeamsPacket::new(b.freeze(), layout)
    }

    /// Add, update, join, leave, move between teams and remove, as `Scoreboard` does.
    #[test]
    fn legacy_team_lifecycle() {
        for layout in [Layout::V1_8, Layout::V1_12] {
            let mut t = Teams::default();
            assert!(t.apply(&legacy("red", 0, layout, &["Alice", "Bob"])));
            let red = t.team_of("Alice").unwrap();
            assert_eq!(red.name.as_ref(), "red");
            assert_eq!(red.display_name, "Red Team");
            assert_eq!(red.prefix, "§c[R] ");
            assert_eq!(red.color, Some(12));
            assert!(!red.friendly_fire && red.see_friendly_invisibles);
            assert_eq!(red.name_tag_visibility, "hideForOtherTeams");
            let rule = if layout == Layout::V1_12 {
                "pushOwnTeam"
            } else {
                "always"
            };
            assert_eq!(red.collision_rule, rule);
            assert!(t.same_team("Alice", "Bob"));
            assert!(t.apply(&legacy("blue", 0, layout, &["Bob"])));
            assert!(!t.same_team("Alice", "Bob"));
            assert_eq!(t.team("red").unwrap().members(), &["Alice".into()]);
            assert!(t.apply(&legacy("red", 4, layout, &["Alice"])));
            assert!(t.team_of("Alice").is_none());
            // A packet for an unknown team is ignored.
            assert!(!t.apply(&legacy("green", 3, layout, &["Carol"])));
            assert!(t.team_of("Carol").is_none());
            assert!(t.apply(&legacy("blue", 1, layout, &[])));
            assert!(t.team_of("Bob").is_none());
            assert_eq!(t.len(), 1);
            // Truncated: nothing changes.
            let p = legacy("red", 0, layout, &["Dave"]);
            let cut = TeamsPacket::new(p.body.slice(..p.body.len() - 2), layout);
            assert!(!t.apply(&cut));
            assert!(t.team_of("Dave").is_none());
        }
    }

    fn nbt_string(b: &mut bytes::BytesMut, s: &str) {
        b.put_u8(8);
        b.put_u16(s.len() as u16);
        b.put_slice(s.as_bytes());
    }

    /// The 1.16 and 26.x layouts: text components, enum ids, colours.
    #[test]
    fn json_and_nbt_layouts() {
        let mut b = bytes::BytesMut::new();
        b.put_string("t").unwrap();
        b.put_i8(0);
        b.put_string(r#"{"text":"Team"}"#).unwrap();
        b.put_u8(0x01);
        b.put_string("never").unwrap();
        b.put_string("pushOtherTeams").unwrap();
        b.put_varint(21); // reset: no colour
        b.put_string(r#"{"text":"[T] "}"#).unwrap();
        b.put_string(r#"{"text":""}"#).unwrap();
        b.put_varint(1);
        b.put_string("Ann").unwrap();
        let mut t = Teams::default();
        assert!(t.apply(&TeamsPacket::new(b.freeze(), Layout::V1_16)));
        let x = t.team_of("Ann").unwrap();
        assert_eq!(
            (x.display_name.as_str(), x.prefix.as_str()),
            ("Team", "[T] ")
        );
        assert_eq!((x.color, x.friendly_fire), (None, true));
        assert_eq!(
            (x.name_tag_visibility, x.collision_rule),
            ("never", "pushOtherTeams")
        );

        for layout in [Layout::V26_1, Layout::V26_2] {
            let mut b = bytes::BytesMut::new();
            b.put_string("t").unwrap();
            b.put_i8(0);
            nbt_string(&mut b, "Team");
            if layout == Layout::V26_1 {
                b.put_u8(0x03);
                b.put_varint(3);
                b.put_varint(1);
                b.put_varint(9); // blue
                nbt_string(&mut b, "[B] ");
                nbt_string(&mut b, "!");
            } else {
                nbt_string(&mut b, "[B] ");
                nbt_string(&mut b, "!");
                b.put_varint(3);
                b.put_varint(1);
                b.put_u8(1);
                b.put_varint(9);
                b.put_u8(0x03);
            }
            b.put_varint(1);
            b.put_string("Ann").unwrap();
            let mut t = Teams::default();
            assert!(t.apply(&TeamsPacket::new(b.freeze(), layout)), "{layout:?}");
            let x = t.team_of("Ann").unwrap();
            assert_eq!(
                (x.prefix.as_str(), x.suffix.as_str(), x.color),
                ("[B] ", "!", Some(9)),
                "{layout:?}"
            );
            assert!(x.friendly_fire && x.see_friendly_invisibles);
            assert_eq!(
                (x.name_tag_visibility, x.collision_rule),
                ("hideForOwnTeam", "never")
            );
        }
    }
}
