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

//! The tree is version-free; each modern protocol's wire layer builds it from its own
//! `declare_commands` types (`command_tree` in `version/modern.rs`).

/// How an argument's text is read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Reader {
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

/// A node of the tree (indices into the node list).
#[derive(Debug, Clone)]
pub(crate) struct Node {
    /// 0 root, 1 literal, 2 argument.
    pub(crate) kind: u8,
    pub(crate) name: Box<str>,
    pub(crate) reader: Reader,
    pub(crate) children: Box<[u32]>,
    pub(crate) redirect: Option<u32>,
    /// The node runs a command (`FLAG_EXECUTABLE`).
    pub(crate) executable: bool,
    /// Usable only with the permission the server grants (`FLAG_RESTRICTED`).
    pub(crate) restricted: bool,
}

/// A parsed command tree (only what the signing decision needs).
#[derive(Debug, Clone)]
pub(crate) struct CommandTree {
    nodes: Vec<Node>,
    root: u32,
}

impl CommandTree {
    /// A tree of `nodes` (children and redirects already checked to be in range).
    pub(crate) fn new(nodes: Vec<Node>, root: u32) -> Self {
        Self { nodes, root }
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
            allow_restricted: true,
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

    /// `ClientPacketListener.verifyCommand` (dialog commands): the line must parse with the
    /// player's permissions plus the restricted nodes (`isValidCommand`: all read, no errors, an
    /// executable last node), have no signable argument, and parse without any permission
    /// (no restricted nodes, no entity selectors). `op_level`: the player's permission level.
    pub(crate) fn check(&self, command: &str, op_level: u8) -> crate::dialog::CommandCheck {
        use crate::dialog::CommandCheck;
        let valid = |allow_selectors: bool, allow_restricted: bool| {
            let cx = Cx {
                input: command,
                allow_selectors,
                allow_restricted,
            };
            let p = self.parse_nodes(&cx, self.root, 0, Vec::new(), true);
            p.consumed == command.len()
                && p.errors == 0
                && self
                    .nodes
                    .get(p.last as usize)
                    .is_some_and(|n| n.executable)
        };
        if !valid(op_level >= 2, true) {
            CommandCheck::ParseErrors
        } else if !self.signable_arguments(command, op_level >= 2).is_empty() {
            CommandCheck::SignatureRequired
        } else if !valid(false, false) {
            CommandCheck::PermissionsRequired
        } else {
            CommandCheck::NoIssues
        }
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
        // `child.canUse(source)`: restricted nodes need the permission.
        let candidates = candidates
            .into_iter()
            .filter(|&c| cx.allow_restricted || !self.nodes[c as usize].restricted);
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
                    last: node,
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
                    last: child,
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
            last: node,
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
    /// The node the parse ended at (`getLastChild().getNode()`).
    last: u32,
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
    /// Restricted nodes can be used (the player's suggestion provider; not the dialog check's
    /// permission-free one).
    allow_restricted: bool,
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
                allow_restricted: true,
            },
            0,
        )
    }

    /// `verifyCommand`'s four outcomes on a small tree.
    #[test]
    fn dialog_command_check() {
        use crate::dialog::CommandCheck as C;
        let node = |kind, name: &str, reader, children: &[u32], executable, restricted| Node {
            kind,
            name: name.into(),
            reader,
            children: children.into(),
            redirect: None,
            executable,
            restricted,
        };
        let entity = Reader::Entity {
            single: false,
            players: false,
        };
        let t = CommandTree::new(
            vec![
                node(0, "", Reader::Balanced, &[1, 3, 5, 7], false, false),
                node(1, "login", Reader::Balanced, &[2], false, false),
                node(2, "password", Reader::Greedy, &[], true, false),
                node(1, "say", Reader::Balanced, &[4], false, false),
                node(2, "message", Reader::Message, &[], true, false),
                node(1, "op", Reader::Balanced, &[6], false, true),
                node(2, "target", Reader::Word, &[], true, false),
                node(1, "tp", Reader::Balanced, &[8], false, false),
                node(2, "targets", entity, &[], true, false),
            ],
            0,
        );
        assert_eq!(t.check("login abc def", 0), C::NoIssues);
        assert_eq!(t.check("login", 0), C::ParseErrors, "not executable");
        assert_eq!(t.check("logout", 0), C::ParseErrors);
        assert_eq!(
            t.check("login abc ", 0),
            C::NoIssues,
            "greedy reads the space"
        );
        assert_eq!(t.check("say hi there", 0), C::SignatureRequired);
        assert_eq!(t.check("op bob", 0), C::PermissionsRequired);
        assert_eq!(t.check("tp Steve", 0), C::NoIssues);
        assert_eq!(
            t.check("tp @p", 2),
            C::PermissionsRequired,
            "selectors need level 2"
        );
        assert_eq!(t.check("tp @p", 0), C::ParseErrors);
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
