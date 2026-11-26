use std::sync::{Arc, LazyLock};

/// Abbreviations, as typed after the leader, and the symbols they produce. A symbol
/// may contain `$CURSOR`, which marks where the cursor goes after the replacement.
#[derive(Debug, Clone, PartialEq)]
pub struct Abbreviations {
    /// In JavaScript property order, which vscode-lean4 uses to break ties.
    entries: Vec<(String, String)>,
}

static BUNDLED: LazyLock<Arc<Abbreviations>> = LazyLock::new(|| {
    let table: serde_json::Map<String, serde_json::Value> =
        serde_json::from_str(include_str!("../vendor/abbreviations.json"))
            .expect("the vendored abbreviation table is a JSON object");
    let entries = table
        .into_iter()
        .map(|(abbreviation, symbol)| match symbol {
            serde_json::Value::String(symbol) => (abbreviation, symbol),
            other => panic!("abbreviation {abbreviation:?} maps to {other}, not a string"),
        })
        .collect();
    Arc::new(Abbreviations::from_entries(entries))
});

impl Abbreviations {
    pub fn bundled() -> Arc<Self> {
        BUNDLED.clone()
    }

    /// The bundled table with `custom` merged over it, like `lean4.input.customTranslations`.
    pub fn with_custom<'a>(custom: impl IntoIterator<Item = (&'a str, &'a str)>) -> Self {
        let mut entries = BUNDLED.entries.clone();
        for (abbreviation, symbol) in custom {
            match entries
                .iter_mut()
                .find(|(existing, _)| existing == abbreviation)
            {
                Some(entry) => entry.1 = symbol.to_owned(),
                None => entries.push((abbreviation.to_owned(), symbol.to_owned())),
            }
        }
        Self::from_entries(entries)
    }

    fn from_entries(mut entries: Vec<(String, String)>) -> Self {
        // JavaScript objects list array-index keys first, in numeric order.
        entries.sort_by_key(|(abbreviation, _)| match array_index(abbreviation) {
            Some(index) => (0, index),
            None => (1, 0),
        });
        Self { entries }
    }

    /// The symbol of exactly this abbreviation.
    pub fn symbol(&self, abbreviation: &str) -> Option<&str> {
        self.entries
            .iter()
            .find(|(existing, _)| existing == abbreviation)
            .map(|(_, symbol)| symbol.as_str())
    }

    /// Whether some abbreviation starts with `prefix`.
    pub fn continues(&self, prefix: &str) -> bool {
        self.entries
            .iter()
            .any(|(abbreviation, _)| abbreviation.starts_with(prefix))
    }

    /// The symbols of the abbreviations that start with `prefix`, shortest abbreviation first.
    pub fn symbols_with_prefix(&self, prefix: &str) -> Vec<&str> {
        let mut matches: Vec<_> = self
            .entries
            .iter()
            .filter(|(abbreviation, _)| abbreviation.starts_with(prefix))
            .collect();
        matches.sort_by_key(|(abbreviation, _)| abbreviation.encode_utf16().count());
        matches
            .into_iter()
            .map(|(_, symbol)| symbol.as_str())
            .collect()
    }

    /// Whether `abbreviation` has a symbol and no longer abbreviation starts with it.
    pub fn is_unique_and_complete(&self, abbreviation: &str) -> bool {
        self.symbol(abbreviation).is_some()
            && self
                .entries
                .iter()
                .filter(|(existing, _)| existing.starts_with(abbreviation))
                .count()
                == 1
    }

    /// What a typed abbreviation becomes: the symbol of the shortest abbreviation that
    /// starts with it, or else the replacement of its longest such prefix followed by
    /// the rest, so that `alp7` becomes `α7`.
    pub fn replacement(&self, abbreviation: &str) -> Option<String> {
        if abbreviation.is_empty() {
            return None;
        }
        if let Some(symbol) = self.symbols_with_prefix(abbreviation).first() {
            return Some((*symbol).to_owned());
        }
        let (last, _) = abbreviation.char_indices().next_back()?;
        let mut replacement = self.replacement(&abbreviation[..last])?;
        replacement.push_str(&abbreviation[last..]);
        Some(replacement)
    }
}

/// How to type a symbol: the abbreviations that produce exactly it, and those that
/// produce it with a closing symbol after the cursor.
#[derive(Debug, Clone, PartialEq)]
pub struct SymbolInput<'a> {
    pub symbol: &'a str,
    pub abbreviations: Vec<&'a str>,
    /// Each abbreviation with the closing symbol it puts after the cursor.
    pub closings: Vec<(&'a str, &'a str)>,
}

impl Abbreviations {
    /// How to type each symbol that `text` starts with, in table order.
    pub fn symbols_at(&self, text: &str) -> Vec<SymbolInput<'_>> {
        let mut symbols: Vec<&str> = Vec::new();
        for (_, symbol) in &self.entries {
            // An empty symbol would show at every character.
            if !symbol.is_empty()
                && text.starts_with(symbol.as_str())
                && !symbols.contains(&&**symbol)
            {
                symbols.push(symbol);
            }
        }
        symbols
            .into_iter()
            .map(|symbol| self.symbol_input(symbol))
            .collect()
    }

    fn symbol_input<'a>(&'a self, symbol: &'a str) -> SymbolInput<'a> {
        let opening = format!("{symbol}{CURSOR}");
        SymbolInput {
            symbol,
            abbreviations: self
                .entries
                .iter()
                .filter(|(_, existing)| existing == symbol)
                .map(|(abbreviation, _)| abbreviation.as_str())
                .collect(),
            closings: self
                .entries
                .iter()
                .filter_map(|(abbreviation, existing)| {
                    Some((abbreviation.as_str(), existing.strip_prefix(&opening)?))
                })
                .collect(),
        }
    }
}

const CURSOR: &str = "$CURSOR";

impl SymbolInput<'_> {
    /// The hover text vscode-lean4 shows, such as "Type ⟨ using `\<` or `\langle`".
    pub fn hover_markdown(&self, leader: &str) -> String {
        let code = |abbreviation: &str| markdown_code(&format!("{leader}{abbreviation}"));
        let abbreviations: Vec<_> = self.abbreviations.iter().map(|a| code(a)).collect();
        let mut markdown = format!("Type {} using {}", self.symbol, abbreviations.join(" or "));
        if !self.closings.is_empty() {
            let closings: Vec<_> = self
                .closings
                .iter()
                .map(|(abbreviation, closing)| format!("{closing} using {}", code(abbreviation)))
                .collect();
            markdown.push_str(&format!(
                ". {} can be auto-closed with {}.",
                self.symbol,
                closings.join(" or ")
            ));
        }
        markdown
    }
}

fn markdown_code(text: &str) -> String {
    if !text.contains('`') {
        return format!("`{text}`");
    }
    let prefix = if text.starts_with('`') { " " } else { "" };
    let suffix = if text.ends_with('`') { " " } else { "" };
    format!("``{prefix}{text}{suffix}``")
}

fn array_index(key: &str) -> Option<u32> {
    let canonical =
        key == "0" || (!key.starts_with('0') && key.bytes().all(|b| b.is_ascii_digit()));
    key.parse::<u32>()
        .ok()
        .filter(|index| canonical && *index != u32::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_bundled_abbreviations() {
        let table = Abbreviations::bundled();
        assert_eq!(table.symbol("alpha"), Some("α"));
        assert_eq!(table.symbol("to"), Some("→"));
        assert_eq!(table.symbol("\\"), Some("\\"));
        assert_eq!(table.symbol("<>"), Some("⟨$CURSOR⟩"));
        assert_eq!(table.symbol("alp"), None);
    }

    #[test]
    fn replaces_the_shortest_match_then_falls_back_to_prefixes() {
        let table = Abbreviations::bundled();
        assert_eq!(table.replacement(""), None);
        assert_eq!(table.replacement("alpha").as_deref(), Some("α"));
        assert_eq!(table.replacement("alp").as_deref(), Some("α"));
        assert_eq!(table.replacement("alp7").as_deref(), Some("α7"));
        assert_eq!(table.replacement("to").as_deref(), Some("→"));
        assert_eq!(table.replacement("[").as_deref(), Some("[$CURSOR]"));
    }

    #[test]
    fn knows_when_an_abbreviation_is_unique_and_complete() {
        let table = Abbreviations::bundled();
        assert!(table.is_unique_and_complete("alpha"));
        assert!(table.is_unique_and_complete("\\"));
        assert!(!table.is_unique_and_complete("to"));
        assert!(!table.is_unique_and_complete("alp"));
        assert!(table.continues("alp"));
        assert!(!table.continues("alpha "));
    }

    #[test]
    fn breaks_ties_in_javascript_property_order() {
        let table = Abbreviations::bundled();
        assert_eq!(table.entries[0], ("0".to_owned(), "₀".to_owned()));
        assert_eq!(table.entries[9], ("9".to_owned(), "₉".to_owned()));
        assert_eq!(table.entries[10], ("{}".to_owned(), "{$CURSOR}".to_owned()));
        assert_eq!(
            &table.symbols_with_prefix("[]")[..2],
            ["[$CURSOR]", "[$CURSOR]_"]
        );
    }

    #[test]
    fn explains_how_to_type_the_symbols_at_a_position() {
        let table = Abbreviations::bundled();
        let markdown: Vec<_> = table
            .symbols_at("⟨a, b⟩")
            .iter()
            .map(|input| input.hover_markdown("\\"))
            .collect();
        assert_eq!(
            markdown,
            ["Type ⟨ using `\\<` or `\\langle`. ⟨ can be auto-closed with ⟩ using `\\<>`."]
        );
        assert_eq!(
            table.symbols_at("α + 1"),
            [SymbolInput {
                symbol: "α",
                abbreviations: vec!["a", "alpha", "Ga"],
                closings: vec![],
            }]
        );
        assert_eq!(table.symbols_at("x"), []);
        assert_eq!(table.symbols_at(""), []);
    }

    #[test]
    fn lists_every_symbol_that_starts_the_text() {
        let table = Abbreviations::with_custom([("ninv", "ℕ⁻¹"), ("tick", "`")]);
        let symbols: Vec<_> = table
            .symbols_at("ℕ⁻¹")
            .into_iter()
            .map(|input| input.symbol)
            .collect();
        assert_eq!(symbols, ["ℕ", "ℕ⁻¹"]);
        assert_eq!(
            table.symbols_at("`")[0].hover_markdown(","),
            "Type ` using `,tick`"
        );
        assert_eq!(
            Abbreviations::with_custom([("t", "`x")]).symbols_at("`x")[0].hover_markdown("`"),
            "Type `x using `` `t``"
        );
    }

    #[test]
    fn merges_custom_translations() {
        let table = Abbreviations::with_custom([("wknight", "♘"), ("alpha", "A"), ("12", "⑫")]);
        assert_eq!(table.symbol("wknight"), Some("♘"));
        assert_eq!(table.symbol("alpha"), Some("A"));
        assert_eq!(table.replacement("wkn").as_deref(), Some("♘"));
        assert_eq!(table.entries[10], ("12".to_owned(), "⑫".to_owned()));
        assert_eq!(
            table.entries.last(),
            Some(&("wknight".to_owned(), "♘".to_owned()))
        );
    }
}
