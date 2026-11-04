//! Chat components.
//!
//! On the wire, chat is a JSON string; packets keep it as the raw string
//! ([`ByteString`](crate::types::ByteString)). Parsing is opt-in: with the `chat-json`
//! feature, [`parse`] and [`to_plain_text`] turn it into a JSON value or plain text.

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
