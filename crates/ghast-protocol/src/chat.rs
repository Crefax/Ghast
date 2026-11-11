//! Chat components.
//!
//! Up to 1.20.2, chat is a JSON string on the wire; packets keep it as the raw string
//! ([`ByteString`](crate::types::ByteString)). Parsing is opt-in: with the `chat-json`
//! feature, [`parse`] and [`to_plain_text`] turn it into a JSON value or plain text.
//!
//! Since 1.20.3 chat is unnamed network NBT ([`AnonNbt`]) with the
//! same structure (a bare string tag, a compound, or a list); [`nbt_to_plain_text`]
//! extracts the text without any extra dependency.

use crate::nbt::{AnonNbt, Tag};

/// Converts an NBT chat component (1.20.3+) to plain text: `text`, `extra`, and
/// `translate` keys with their `with` arguments appended, like `to_plain_text`.
/// Undecodable NBT gives an empty string.
pub fn nbt_to_plain_text(nbt: &AnonNbt) -> String {
    fn walk(t: &Tag, out: &mut String) {
        match t {
            Tag::String(s) => out.push_str(s),
            Tag::List(l) => l.items.iter().for_each(|x| walk(x, out)),
            Tag::Compound(c) => {
                // A compound with an empty key is a wrapped primitive (vanilla writes
                // heterogeneous lists that way).
                if let Some(t) = c.get("text").or_else(|| c.get("")) {
                    walk(t, out);
                }
                if let Some(Tag::String(t)) = c.get("translate") {
                    out.push_str(t);
                    if let Some(Tag::List(with)) = c.get("with") {
                        for w in &with.items {
                            out.push(' ');
                            walk(w, out);
                        }
                    }
                }
                if let Some(e) = c.get("extra") {
                    walk(e, out);
                }
            }
            Tag::Byte(v) => out.push_str(&v.to_string()),
            Tag::Short(v) => out.push_str(&v.to_string()),
            Tag::Int(v) => out.push_str(&v.to_string()),
            Tag::Long(v) => out.push_str(&v.to_string()),
            Tag::Float(v) => out.push_str(&v.to_string()),
            Tag::Double(v) => out.push_str(&v.to_string()),
            _ => {}
        }
    }
    let mut out = String::new();
    if let Ok(Some(t)) = nbt.parse() {
        walk(&t, &mut out);
    }
    out
}

/// Removes legacy `§x` formatting codes from a string.
pub fn strip_formatting(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars();
    while let Some(c) = it.next() {
        if c == '\u{a7}' {
            it.next();
        } else {
            out.push(c);
        }
    }
    out
}

/// Parses a chat component JSON string.
#[cfg(feature = "chat-json")]
pub fn parse(json: &str) -> Result<serde_json::Value, serde_json::Error> {
    serde_json::from_str(json)
}

/// Converts a chat component JSON string to plain text (`text`, `extra`, and
/// `translate` keys with their `with` arguments appended). Invalid JSON is returned
/// as-is, since some servers send bare strings.
#[cfg(feature = "chat-json")]
pub fn to_plain_text(json: &str) -> String {
    fn walk(v: &serde_json::Value, out: &mut String) {
        use serde_json::Value;
        match v {
            Value::String(s) => out.push_str(s),
            Value::Array(a) => a.iter().for_each(|x| walk(x, out)),
            Value::Object(o) => {
                if let Some(t) = o.get("text") {
                    walk(t, out);
                }
                if let Some(Value::String(t)) = o.get("translate") {
                    out.push_str(t);
                    if let Some(Value::Array(with)) = o.get("with") {
                        for w in with {
                            out.push(' ');
                            walk(w, out);
                        }
                    }
                }
                if let Some(e) = o.get("extra") {
                    walk(e, out);
                }
            }
            Value::Number(n) => out.push_str(&n.to_string()),
            Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
            Value::Null => {}
        }
    }
    match parse(json) {
        Ok(v) => {
            let mut out = String::new();
            walk(&v, &mut out);
            out
        }
        Err(_) => json.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip() {
        assert_eq!(
            strip_formatting("\u{a7}aHello \u{a7}lworld\u{a7}"),
            "Hello world"
        );
    }

    #[test]
    fn nbt_plain_text() {
        use crate::nbt::{Compound, List, tag};
        let bare = AnonNbt::from_tag(&Tag::String("bare".into())).unwrap();
        assert_eq!(nbt_to_plain_text(&bare), "bare");
        let c = Tag::Compound(Compound(vec![
            ("text".into(), Tag::String("Hi ".into())),
            (
                "extra".into(),
                Tag::List(List {
                    elem_type: tag::STRING,
                    items: vec![Tag::String("there".into()), Tag::String("!".into())],
                }),
            ),
        ]));
        assert_eq!(
            nbt_to_plain_text(&AnonNbt::from_tag(&c).unwrap()),
            "Hi there!"
        );
        let tr = Tag::Compound(Compound(vec![
            ("translate".into(), Tag::String("chat.type.text".into())),
            (
                "with".into(),
                Tag::List(List {
                    elem_type: tag::STRING,
                    items: vec![Tag::String("Bot".into()), Tag::String("hello".into())],
                }),
            ),
        ]));
        assert_eq!(
            nbt_to_plain_text(&AnonNbt::from_tag(&tr).unwrap()),
            "chat.type.text Bot hello"
        );
        assert_eq!(nbt_to_plain_text(&AnonNbt::END), "");
    }

    #[cfg(feature = "chat-json")]
    #[test]
    fn plain_text() {
        assert_eq!(
            to_plain_text(r#"{"text":"Hi ","extra":[{"text":"there","bold":true},"!"]}"#),
            "Hi there!"
        );
        assert_eq!(to_plain_text(r#""bare""#), "bare");
        assert_eq!(to_plain_text("not json"), "not json");
    }
}
