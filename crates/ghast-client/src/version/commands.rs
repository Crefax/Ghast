//! The server's command tree (`declare_commands`, 1.13+) and what a command line signs: the
//! arguments of type `minecraft:message` that a parse of the line reaches (vanilla
//! `SignableCommand.of` over `CommandDispatcher.parse`). 26.1 sends `chat_command_signed` for
//! such commands (with an empty signature list without a chat session) and `chat_command`
//! for the rest.
//!
//! The parse follows Brigadier's `parseNodes`: a literal that matches the next word is the only
//! candidate, otherwise every argument child; an argument must end at a space or the end of
//! the line; redirects (`execute … run`) continue at their target; among several candidates
//! the one that reads the whole line wins, then the one without errors. Argument types are
//! read with their token rules (unquoted / quoted strings, numbers, coordinates, entity
//! selectors with bracketed options, SNBT / JSON with balanced brackets, greedy messages); the
//! exact value parsers (is it a valid block id …) are not modelled — the reader only needs where
//! each argument ends.

use ghast_protocol::v26_1::play::clientbound as cb;
use ghast_protocol::v26_1::types::{
    CommandNodeExtraNodeData, CommandNodeExtraNodeDataCase2Parser as P,
};

/// How an argument's text is read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Reader {
    /// `readBoolean` / `readString`.
    StringToken,
    /// `readUnquotedString` (`brigadier:string` single word).
    Word,
    /// The rest of the line (`brigadier:string` greedy).
    Greedy,
    /// A number (`[0-9.-]+`).
    Number,
    /// A number with an optional unit (`minecraft:time`).
    Time,
    /// `minecraft:entity`: a selector (`single`: at most one result, `players`: players
    /// only), a player name or a UUID.
    Entity { single: bool, players: bool },
    /// `minecraft:game_profile`: a player name, a UUID or a players-only selector.
    GameProfile,
    /// `minecraft:score_holder`: a selector, `*` or any text up to a space.
    ScoreHolder,
    /// `minecraft:message`: the rest of the line, signed.
    Message,
    /// `n` coordinates separated by single spaces.
    Coords(u8),
    /// A token up to whitespace, with brackets, braces and quotes balanced.
    Balanced,
}

#[derive(Debug, Clone)]
struct Node {
    /// 0 root, 1 literal, 2 argument.
    kind: u8,
    name: Box<str>,
    reader: Reader,
    children: Box<[u32]>,
    redirect: Option<u32>,
}

/// A parsed command tree (only what the signing decision needs).
#[derive(Debug, Clone)]
pub(crate) struct CommandTree {
    nodes: Vec<Node>,
    root: u32,
}

impl CommandTree {
    /// Builds the tree from a `declare_commands` packet.
    pub(crate) fn from_packet(p: &cb::DeclareCommands) -> Option<Self> {
        let n = p.nodes.len();
        let index = |i: i32| u32::try_from(i).ok().filter(|&i| (i as usize) < n);
        let mut nodes = Vec::with_capacity(n);
        for node in &p.nodes {
            let (name, reader): (Box<str>, Reader) = match &node.extra_node_data {
                CommandNodeExtraNodeData::Case1(l) => ((&*l.name).into(), Reader::Balanced),
                CommandNodeExtraNodeData::Case2(a) => ((&*a.name).into(), reader_for(a)),
                _ => ("".into(), Reader::Balanced),
            };
            nodes.push(Node {
                kind: node.flags.command_node_type,
                name,
                reader,
                children: node.children.iter().filter_map(|&c| index(c)).collect(),
                redirect: node.redirect_node.and_then(index),
            });
        }
        Some(Self {
            nodes,
            root: index(p.root_index)?,
        })
    }

    /// The signable arguments (`minecraft:message`) a parse of `command` (without the `/`)
    /// reaches, as (argument name, its text), in visiting order.
    /// `allow_selectors`: the player may use entity selectors (permission level 2+).
    pub(crate) fn signable_arguments<'a>(
        &self,
        command: &'a str,
        allow_selectors: bool,
    ) -> Vec<(Box<str>, &'a str)> {
        let cx = Cx {
            input: command,
            allow_selectors,
        };
        let best = self.parse_nodes(&cx, self.root, 0, Vec::new(), true);
        best.args
            .into_iter()
            .filter(|a| a.signed)
            .map(|a| {
                (
                    self.nodes[a.node as usize].name.clone(),
                    &command[a.from..a.to],
                )
            })
            .collect()
    }

    /// `CommandDispatcher.parseNodes` from `node` at `cursor`; `args` are the arguments
    /// parsed so far on the way here.
    ///
    /// `visited`: arguments parsed here count for signing (`ArgumentVisitor` stops at the
    /// first context a redirect to the root started: `execute … run say` signs nothing).
    fn parse_nodes(
        &self,
        cx: &Cx<'_>,
        node: u32,
        cursor: usize,
        args: Vec<Arg>,
        visited: bool,
    ) -> Parse {
        let input = cx.input;
        let here = &self.nodes[node as usize];
        let mut potentials: Vec<Parse> = Vec::new();
        // getRelevantNodes: the literal named like the next word, else every argument child.
        let word_end = input[cursor..]
            .find(' ')
            .map_or(input.len(), |i| cursor + i);
        let word = &input[cursor..word_end];
        let literal =
            here.children.iter().copied().find(|&c| {
                self.nodes[c as usize].kind == 1 && &*self.nodes[c as usize].name == word
            });
        let candidates: Vec<u32> = match literal {
            Some(l) => vec![l],
            None => here
                .children
                .iter()
                .copied()
                .filter(|&c| self.nodes[c as usize].kind == 2)
                .collect(),
        };
        for child in candidates {
            let c = &self.nodes[child as usize];
            let end = if c.kind == 1 {
                Some(word_end)
            } else {
                read(c.reader, cx, cursor)
            };
            // The argument must be followed by a separator (or the end).
            let Some(end) = end.filter(|&e| e == input.len() || input.as_bytes()[e] == b' ') else {
                potentials.push(Parse {
                    args: args.clone(),
                    consumed: cursor,
                    errors: 1,
                });
                continue;
            };
            let mut next = args.clone();
            if c.kind == 2 {
                next.push(Arg {
                    node: child,
                    from: cursor,
                    to: end,
                    signed: visited && c.reader == Reader::Message,
                });
            }
            // reader.canRead(redirect == null ? 2 : 1)
            let need = if c.redirect.is_none() { 2 } else { 1 };
            let parse = if input.len() >= end + need {
                if let Some(target) = c.redirect {
                    // A redirect continues in a child context: return at once.
                    let visited = visited && target != self.root;
                    return self.parse_nodes(cx, target, end + 1, next, visited);
                }
                self.parse_nodes(cx, child, end + 1, next, visited)
            } else {
                Parse {
                    args: next,
                    consumed: end,
                    errors: 0,
                }
            };
            potentials.push(parse);
        }
        // Prefer a parse that read everything, then one without errors.
        potentials.sort_by_key(|p| (p.consumed < input.len(), p.errors > 0));
        potentials.into_iter().next().unwrap_or(Parse {
            args,
            consumed: cursor,
            errors: 1,
        })
    }
}

#[derive(Debug, Clone)]
struct Arg {
    node: u32,
    from: usize,
    to: usize,
    signed: bool,
}

#[derive(Debug)]
struct Parse {
    args: Vec<Arg>,
    consumed: usize,
    errors: u32,
}

fn reader_for(a: &ghast_protocol::v26_1::types::CommandNodeExtraNodeDataCase2) -> Reader {
    use ghast_protocol::v26_1::types::CommandNodeExtraNodeDataCase2Properties as Props;
    match a.parser {
        P::BRIGADIER_BOOL => Reader::StringToken,
        P::BRIGADIER_FLOAT | P::BRIGADIER_DOUBLE | P::BRIGADIER_INTEGER | P::BRIGADIER_LONG => {
            Reader::Number
        }
        P::BRIGADIER_STRING => match &a.properties {
            Props::BrigadierString(s) if s.0 == 0 => Reader::Word,
            Props::BrigadierString(s) if s.0 == 2 => Reader::Greedy,
            _ => Reader::StringToken,
        },
        P::MINECRAFT_ENTITY => match &a.properties {
            Props::Entity(f) => Reader::Entity {
                single: f.only_allow_entities == 1,
                players: f.only_allow_players == 1,
            },
            _ => Reader::Entity {
                single: false,
                players: false,
            },
        },
        P::MINECRAFT_GAME_PROFILE => Reader::GameProfile,
        P::MINECRAFT_SCORE_HOLDER => Reader::ScoreHolder,
        P::MINECRAFT_MESSAGE => Reader::Message,
        P::MINECRAFT_VEC3 | P::MINECRAFT_BLOCK_POS => Reader::Coords(3),
        P::MINECRAFT_VEC2 | P::MINECRAFT_COLUMN_POS | P::MINECRAFT_ROTATION => Reader::Coords(2),
        P::MINECRAFT_ANGLE => Reader::Coords(1),
        P::MINECRAFT_TIME => Reader::Time,
        _ => Reader::Balanced,
    }
}

/// Brigadier's `isAllowedInUnquotedString`.
fn unquoted(c: u8) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-' | b'.' | b'+')
}

/// Brigadier's `isAllowedNumber`.
fn number_char(c: u8) -> bool {
    c.is_ascii_digit() || matches!(c, b'.' | b'-')
}

/// `readQuotedString` from an opening quote at `at`: the index after the closing quote.
fn quoted(b: &[u8], at: usize) -> Option<usize> {
    let q = b[at];
    let mut i = at + 1;
    while i < b.len() {
        match b[i] {
            b'\\' => i += 2,
            c if c == q => return Some(i + 1),
            _ => i += 1,
        }
    }
    None
}

/// `readString`: quoted, or unquoted characters (possibly none).
fn string_token(b: &[u8], at: usize) -> Option<usize> {
    match b.get(at) {
        Some(b'"' | b'\'') => quoted(b, at),
        _ => Some(at + b[at..].iter().take_while(|&&c| unquoted(c)).count()),
    }
}

/// A token up to whitespace (or the end), with `[]`, `{}`, `()` and quotes balanced.
fn balanced(b: &[u8], at: usize) -> Option<usize> {
    let mut depth = 0i32;
    let mut i = at;
    while i < b.len() {
        match b[i] {
            b'"' | b'\'' => {
                i = quoted(b, i)?;
                continue;
            }
            b'[' | b'{' | b'(' => depth += 1,
            b']' | b'}' | b')' => {
                depth -= 1;
                if depth < 0 {
                    return None;
                }
            }
            b' ' if depth == 0 => break,
            _ => {}
        }
        i += 1;
    }
    (depth == 0 && i > at).then_some(i)
}

/// What a parse reads.
struct Cx<'a> {
    input: &'a str,
    allow_selectors: bool,
}

/// `EntitySelectorParser.parse` + the argument's checks; the end of the selector or name.
/// `players`/`single`: the argument's limits (`None` = no check).
fn selector(cx: &Cx<'_>, at: usize, single: bool, players: bool) -> Option<usize> {
    let b = cx.input.as_bytes();
    if b.get(at) != Some(&b'@') {
        // parseNameOrUUID: readString, 1–16 characters or a UUID (a UUID may be any entity).
        let end = string_token(b, at)?;
        let name = &cx.input[at..end];
        let name = name.trim_matches(|c| c == '"' || c == '\'');
        let uuid = uuid::Uuid::parse_str(name).is_ok() && name.len() == 36;
        if !uuid && (name.is_empty() || name.chars().count() > 16) {
            return None;
        }
        return Some(end);
    }
    if !cx.allow_selectors {
        return None;
    }
    let kind = *b.get(at + 1)?;
    let (mut max_one, mut entities) = match kind {
        b'p' | b'r' => (true, false),
        b'a' => (false, false),
        b'e' => (false, true),
        b'n' | b's' => (true, true),
        _ => return None,
    };
    let mut end = at + 2;
    if b.get(end) == Some(&b'[') {
        let close = balanced_brackets(b, end)?;
        for opt in split_options(&cx.input[end + 1..close - 1]) {
            let Some((k, v)) = opt.split_once('=') else {
                continue;
            };
            match k.trim() {
                "limit" => {
                    if let Ok(n) = v.trim().parse::<i32>() {
                        max_one = n <= 1;
                    }
                }
                "type" => {
                    let v = v.trim();
                    if matches!(v, "player" | "minecraft:player") {
                        entities = false;
                    }
                }
                _ => {}
            }
        }
        end = close;
    }
    // EntityArgument.parse: single needs one result; players-only refuses entity selectors
    // (the self selector is checked later, at run time).
    if single && !max_one {
        return None;
    }
    if players && entities && kind != b's' {
        return None;
    }
    Some(end)
}

/// Top-level `,`-separated options of a selector (quotes and brackets respected).
fn split_options(s: &str) -> Vec<&str> {
    let b = s.as_bytes();
    let (mut out, mut start, mut depth, mut i) = (Vec::new(), 0, 0i32, 0);
    while i < b.len() {
        match b[i] {
            b'"' | b'\'' => {
                i = quoted(b, i).unwrap_or(b.len());
                continue;
            }
            b'[' | b'{' => depth += 1,
            b']' | b'}' => depth -= 1,
            b',' if depth == 0 => {
                out.push(&s[start..i]);
                start = i + 1;
            }
            _ => {}
        }
        i += 1;
    }
    out.push(&s[start..]);
    out
}

/// Reads one argument from `at`; the index where it ends, or `None` on a syntax error.
fn read(r: Reader, cx: &Cx<'_>, at: usize) -> Option<usize> {
    let b = cx.input.as_bytes();
    if at > b.len() {
        return None;
    }
    let end = match r {
        Reader::Greedy | Reader::Message => b.len(),
        Reader::StringToken => string_token(b, at)?,
        Reader::Word => at + b[at..].iter().take_while(|&&c| unquoted(c)).count(),
        Reader::Number => at + b[at..].iter().take_while(|&&c| number_char(c)).count(),
        Reader::Time => {
            let n = at + b[at..].iter().take_while(|&&c| number_char(c)).count();
            if matches!(b.get(n), Some(b'd' | b's' | b't')) {
                n + 1
            } else {
                n
            }
        }
        Reader::Entity { single, players } => selector(cx, at, single, players)?,
        Reader::GameProfile => selector(cx, at, false, true)?,
        Reader::ScoreHolder => match b.get(at) {
            Some(b'@') => selector(cx, at, false, false)?,
            _ => at + b[at..].iter().take_while(|&&c| c != b' ').count(),
        },
        Reader::Coords(n) => {
            let mut i = at;
            for k in 0..n {
                if k > 0 {
                    if b.get(i) != Some(&b' ') {
                        return None;
                    }
                    i += 1;
                }
                let start = i;
                if matches!(b.get(i), Some(b'~' | b'^')) {
                    i += 1;
                }
                i += b[i..].iter().take_while(|&&c| number_char(c)).count();
                if i == start {
                    return None;
                }
            }
            i
        }
        Reader::Balanced => balanced(b, at)?,
    };
    (end > at || matches!(r, Reader::Greedy | Reader::Message | Reader::StringToken)).then_some(end)
}

/// From an opening `[` at `at`: the index after its matching `]` (quotes respected).
fn balanced_brackets(b: &[u8], at: usize) -> Option<usize> {
    let mut depth = 0i32;
    let mut i = at;
    while i < b.len() {
        match b[i] {
            b'"' | b'\'' => {
                i = quoted(b, i)?;
                continue;
            }
            b'[' | b'{' => depth += 1,
            b']' | b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i + 1);
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(reader: Reader, s: &str) -> Option<usize> {
        read(
            reader,
            &Cx {
                input: s,
                allow_selectors: true,
            },
            0,
        )
    }

    #[test]
    fn readers() {
        let any = Reader::Entity {
            single: false,
            players: false,
        };
        let players = Reader::Entity {
            single: false,
            players: true,
        };
        assert_eq!(r(Reader::Word, "abc def"), Some(3));
        assert_eq!(r(Reader::StringToken, "\"a b\" c"), Some(5));
        assert_eq!(r(Reader::Coords(3), "~ ~1 ^-2 x"), Some(8));
        assert_eq!(r(Reader::Coords(3), "1 2"), None);
        assert_eq!(r(any, "@a[name=\"x y\",distance=..5] hi"), Some(27));
        assert_eq!(r(any, "Steve hi"), Some(5));
        assert_eq!(r(players, "@e[type=minecraft:pig] x"), None);
        assert_eq!(r(players, "@e[type=player] x"), Some(15));
        assert_eq!(r(Reader::Balanced, "minecraft:stone{a:[1, 2]} 3"), Some(25));
        assert_eq!(r(Reader::Message, "hello world"), Some(11));
    }
}
