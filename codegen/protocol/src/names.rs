//! Identifier conversions.

/// `map_chunk` / `mapChunk` / `minecraft:crafting` -> `MapChunk` / `MinecraftCrafting`.
pub fn pascal(s: &str) -> String {
    let mut out = String::new();
    for word in split_words(s) {
        let mut cs = word.chars();
        if let Some(c) = cs.next() {
            out.extend(c.to_uppercase());
            out.extend(cs.flat_map(char::to_lowercase));
        }
    }
    if out.is_empty() || out.starts_with(|c: char| c.is_ascii_digit()) {
        out.insert(0, 'T');
    }
    out
}

/// `chunkData` / `playerUUID` / `old_radius` -> `chunk_data` / `player_uuid` / `old_radius`.
pub fn snake(s: &str) -> String {
    let lead = s.chars().take_while(|&c| c == '_').count();
    let mut out: String = "_".repeat(lead);
    out.push_str(
        &split_words(s)
            .iter()
            .map(|w| w.to_lowercase())
            .collect::<Vec<_>>()
            .join("_"),
    );
    escape(out)
}

/// Splits on `_`, `:`, `-`, `.`, spaces and camel-case boundaries (keeping acronyms together).
fn split_words(s: &str) -> Vec<String> {
    let chars: Vec<char> = s.chars().collect();
    let mut words = Vec::new();
    let mut cur = String::new();
    for i in 0..chars.len() {
        let c = chars[i];
        if !c.is_ascii_alphanumeric() {
            if !cur.is_empty() {
                words.push(std::mem::take(&mut cur));
            }
            continue;
        }
        if c.is_ascii_uppercase() && !cur.is_empty() {
            let prev = chars[i - 1];
            let next_lower = chars.get(i + 1).is_some_and(|n| n.is_ascii_lowercase());
            if prev.is_ascii_lowercase()
                || prev.is_ascii_digit()
                || (prev.is_ascii_uppercase() && next_lower)
            {
                words.push(std::mem::take(&mut cur));
            }
        }
        cur.push(c);
    }
    if !cur.is_empty() {
        words.push(cur);
    }
    words
}

/// Escapes Rust keywords.
pub fn escape(s: String) -> String {
    const RAW: &[&str] = &[
        "as", "break", "const", "continue", "else", "enum", "extern", "false", "fn", "for", "if",
        "impl", "in", "let", "loop", "match", "mod", "move", "mut", "pub", "ref", "return",
        "static", "struct", "trait", "true", "type", "unsafe", "use", "where", "while", "async",
        "await", "dyn", "abstract", "become", "box", "do", "final", "macro", "override", "priv",
        "typeof", "unsized", "virtual", "yield", "try", "gen",
    ];
    if matches!(s.as_str(), "self" | "Self" | "super" | "crate") {
        format!("{s}_")
    } else if RAW.contains(&s.as_str()) {
        format!("r#{s}")
    } else {
        s
    }
}

/// `SCREAMING_SNAKE` constant name.
pub fn screaming(s: &str) -> String {
    let s = snake(s);
    let s = s.trim_start_matches("r#");
    let mut out = s.to_uppercase();
    if out.starts_with(|c: char| c.is_ascii_digit()) {
        out.insert(0, '_');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conversions() {
        assert_eq!(pascal("map_chunk"), "MapChunk");
        assert_eq!(
            pascal("minecraft:crafting_shaped"),
            "MinecraftCraftingShaped"
        );
        assert_eq!(pascal("entityMetadata"), "EntityMetadata");
        assert_eq!(pascal("UUID"), "Uuid");
        assert_eq!(snake("chunkData"), "chunk_data");
        assert_eq!(snake("playerUUID"), "player_uuid");
        assert_eq!(snake("itemID"), "item_id");
        assert_eq!(snake("type"), "r#type");
        assert_eq!(snake("_unused"), "_unused");
        assert_eq!(snake("x"), "x");
        assert_eq!(screaming("keep_alive"), "KEEP_ALIVE");
        assert_eq!(screaming("type"), "TYPE");
    }
}
