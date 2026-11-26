use std::{ops::Range, sync::Arc};

use crate::Abbreviations;

const CURSOR: &str = "$CURSOR";

#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    pub abbreviations: Arc<Abbreviations>,
    /// The text that starts an abbreviation.
    pub leader: String,
    /// Replace an abbreviation as soon as it is complete and no longer one starts with it.
    pub eager_replacement: bool,
}

/// An edit to the text, with `range` in offsets from before the edit.
#[derive(Debug, Clone, PartialEq)]
pub struct Change {
    pub range: Range<usize>,
    pub new_text: String,
}

/// Text that replaces an abbreviation and its leader.
#[derive(Debug, Clone, PartialEq)]
pub struct Replacement {
    pub range: Range<usize>,
    pub new_text: String,
    /// Where `$CURSOR` was in `new_text`.
    pub cursor: Option<usize>,
}

/// An abbreviation being typed.
#[derive(Debug, Clone, PartialEq)]
struct Tracked {
    leader_start: usize,
    leader_len: usize,
    /// The text typed after the leader.
    text: String,
    /// The user typed a character that no abbreviation continues with.
    finished: bool,
}

enum ChangeEffect {
    Unaffected,
    Absorbed,
    Untrackable,
}

impl Tracked {
    fn text_start(&self) -> usize {
        self.leader_start + self.leader_len
    }

    fn range(&self) -> Range<usize> {
        self.leader_start..self.text_start() + self.text.len()
    }

    fn apply(&mut self, change: &Change, abbreviations: &Abbreviations) -> ChangeEffect {
        let text_start = self.text_start();
        let end = self.range().end;
        let Range {
            start,
            end: old_end,
        } = change.range;
        if text_start <= start && old_end <= end {
            self.finished = false;
            if start == end {
                let continued = format!("{}{}", self.text, change.new_text);
                if !abbreviations.continues(&continued) {
                    self.finished = true;
                    return ChangeEffect::Unaffected;
                }
            }
            let edited = start - text_start..old_end - text_start;
            if !self.text.is_char_boundary(edited.start) || !self.text.is_char_boundary(edited.end)
            {
                return ChangeEffect::Untrackable;
            }
            self.text.replace_range(edited, &change.new_text);
            ChangeEffect::Absorbed
        } else if old_end <= self.leader_start {
            self.leader_start = self.leader_start - change.range.len() + change.new_text.len();
            ChangeEffect::Unaffected
        } else if start >= end {
            ChangeEffect::Unaffected
        } else {
            ChangeEffect::Untrackable
        }
    }

    fn replacement(&self, abbreviations: &Abbreviations) -> Option<Replacement> {
        let symbol = abbreviations.replacement(&self.text)?;
        Some(Replacement {
            range: self.range(),
            new_text: symbol.replacen(CURSOR, "", 1),
            cursor: symbol.find(CURSOR),
        })
    }
}

/// The abbreviations being typed in one buffer, which never overlap. A port of
/// vscode-lean4's `AbbreviationRewriter`.
#[derive(Debug, Default)]
pub struct Rewriter {
    tracked: Vec<Tracked>,
}

impl Rewriter {
    pub fn is_empty(&self) -> bool {
        self.tracked.is_empty()
    }

    /// The tracked abbreviations, leaders included, in no particular order.
    pub fn pending(&self) -> impl Iterator<Item = Range<usize>> + '_ {
        self.tracked.iter().map(Tracked::range)
    }

    /// Follows edits that were made at once. Edits that replace abbreviations pass
    /// `track_new: false`, so that a replacement such as `\` starts no abbreviation.
    pub fn edited(&mut self, config: &Config, mut changes: Vec<Change>, track_new: bool) {
        // Bottom first, so that each change's offsets are still valid when it applies.
        changes.sort_by_key(|change| std::cmp::Reverse(change.range.start));
        for change in changes {
            let mut absorbed = false;
            self.tracked.retain_mut(|tracked| {
                match tracked.apply(&change, &config.abbreviations) {
                    ChangeEffect::Unaffected => true,
                    ChangeEffect::Absorbed => {
                        absorbed = true;
                        true
                    }
                    ChangeEffect::Untrackable => false,
                }
            });
            if track_new
                && !absorbed
                && !config.leader.is_empty()
                && change.new_text == config.leader
            {
                self.tracked.push(Tracked {
                    leader_start: change.range.start,
                    leader_len: config.leader.len(),
                    text: String::new(),
                    finished: false,
                });
            }
        }
    }

    /// Stops tracking, and returns the replacements of, the abbreviations that are due:
    /// those that were finished, those that eager replacement completes, and those that
    /// no selection touches any more.
    pub fn take_due(&mut self, config: &Config, selections: &[Range<usize>]) -> Vec<Replacement> {
        self.take(config, |tracked| {
            let range = tracked.range();
            tracked.finished
                || (config.eager_replacement
                    && config.abbreviations.is_unique_and_complete(&tracked.text))
                || !selections
                    .iter()
                    .any(|selection| range.start <= selection.start && selection.start <= range.end)
        })
    }

    /// Stops tracking all abbreviations, and returns their replacements.
    pub fn take_all(&mut self, config: &Config) -> Vec<Replacement> {
        self.take(config, |_| true)
    }

    fn take(&mut self, config: &Config, due: impl Fn(&Tracked) -> bool) -> Vec<Replacement> {
        let (due, kept): (Vec<_>, Vec<_>) =
            std::mem::take(&mut self.tracked).into_iter().partition(due);
        self.tracked = kept;
        let mut replacements: Vec<_> = due
            .iter()
            .filter_map(|tracked| tracked.replacement(&config.abbreviations))
            .collect();
        replacements.sort_by_key(|replacement| replacement.range.start);
        replacements
    }
}

/// Moves each cursor that is right after a replacement with a `$CURSOR` to where the
/// marker was. `replacements` are sorted, and `cursors` are offsets after they were made.
pub fn place_cursors(replacements: &[Replacement], cursors: &[usize]) -> Vec<usize> {
    let mut placed = cursors.to_vec();
    let mut start = 0;
    let mut end_before = 0;
    for replacement in replacements {
        start += replacement.range.start - end_before;
        end_before = replacement.range.end;
        let end = start + replacement.new_text.len();
        if let Some(cursor) = replacement.cursor {
            for (placed, original) in placed.iter_mut().zip(cursors) {
                if *original == end {
                    *placed = start + cursor;
                }
            }
        }
        start = end;
    }
    placed
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> Config {
        Config {
            abbreviations: Abbreviations::bundled(),
            leader: "\\".into(),
            eager_replacement: true,
        }
    }

    /// Plays typed text into a buffer with one cursor, as the editor does.
    struct Typist {
        config: Config,
        text: String,
        cursor: usize,
        rewriter: Rewriter,
    }

    impl Typist {
        fn new(config: Config, text: &str) -> Self {
            Self {
                config,
                text: text.into(),
                cursor: text.len(),
                rewriter: Rewriter::default(),
            }
        }

        fn edit(&mut self, range: Range<usize>, new_text: &str) {
            self.text.replace_range(range.clone(), new_text);
            self.cursor = range.start + new_text.len();
            self.rewriter.edited(
                &self.config,
                vec![Change {
                    range,
                    new_text: new_text.into(),
                }],
                true,
            );
            self.settle();
        }

        fn type_text(&mut self, text: &str) {
            for character in text.chars() {
                self.edit(self.cursor..self.cursor, &character.to_string());
            }
        }

        fn backspace(&mut self) {
            let previous = self.text[..self.cursor].chars().next_back().unwrap();
            self.edit(self.cursor - previous.len_utf8()..self.cursor, "");
        }

        fn move_to(&mut self, cursor: usize) {
            self.cursor = cursor;
            self.settle();
        }

        fn convert(&mut self) {
            let replacements = self.rewriter.take_all(&self.config);
            self.replace(replacements);
        }

        fn settle(&mut self) {
            let replacements = self
                .rewriter
                .take_due(&self.config, &[self.cursor..self.cursor]);
            self.replace(replacements);
        }

        fn replace(&mut self, replacements: Vec<Replacement>) {
            if replacements.is_empty() {
                return;
            }
            let mut changes = Vec::new();
            for replacement in replacements.iter().rev() {
                self.text
                    .replace_range(replacement.range.clone(), &replacement.new_text);
                if replacement.range.end <= self.cursor {
                    self.cursor =
                        self.cursor - replacement.range.len() + replacement.new_text.len();
                }
                changes.push(Change {
                    range: replacement.range.clone(),
                    new_text: replacement.new_text.clone(),
                });
            }
            self.rewriter.edited(&self.config, changes, false);
            self.cursor = place_cursors(&replacements, &[self.cursor])[0];
        }

        fn state(&self) -> String {
            let mut state = self.text.clone();
            state.insert(self.cursor, '|');
            state
        }

        fn pending(&self) -> Vec<&str> {
            self.rewriter
                .pending()
                .map(|range| &self.text[range])
                .collect()
        }
    }

    fn typed(text: &str) -> String {
        let mut typist = Typist::new(config(), "");
        typist.type_text(text);
        typist.state()
    }

    #[test]
    fn an_empty_leader_starts_no_abbreviation() {
        let mut typist = Typist::new(
            Config {
                leader: String::new(),
                ..config()
            },
            "abc",
        );
        typist.backspace();
        typist.type_text("alpha ");
        assert_eq!(typist.state(), "abalpha |");
        assert!(typist.pending().is_empty());
    }

    #[test]
    fn replaces_abbreviations_as_they_are_typed() {
        assert_eq!(typed("\\alpha "), "α |");
        assert_eq!(typed("\\to x"), "→ x|");
        assert_eq!(typed("\\to"), "\\to|");
        assert_eq!(typed("a \\and b"), "a ∧ b|");
        assert_eq!(typed("\\alp7"), "α7|");
        assert_eq!(typed("\\Nat"), "ℕ|");
    }

    #[test]
    fn types_a_literal_leader_with_two_leaders() {
        assert_eq!(typed("\\\\"), "\\|");
        assert_eq!(typed("\\\\alpha"), "\\alpha|");
    }

    #[test]
    fn leaves_text_that_starts_no_abbreviation_alone() {
        assert_eq!(typed("\\ "), "\\ |");
        assert_eq!(typed("\\W"), "\\W|");
        assert_eq!(typed("\\qqq"), "∎qq|");
    }

    #[test]
    fn places_the_cursor_where_the_symbol_says() {
        assert_eq!(typed("\\<>"), "⟨|⟩");
        assert_eq!(typed("\\norm"), "‖|‖");
        assert_eq!(typed("\\[] "), "[] |");
    }

    #[test]
    fn keeps_an_ambiguous_abbreviation_pending() {
        let mut typist = Typist::new(config(), "x ");
        typist.type_text("\\to");
        assert_eq!(typist.pending(), ["\\to"]);
        typist.backspace();
        assert_eq!(typist.state(), "x \\t|");
        assert_eq!(typist.pending(), ["\\t"]);
        typist.convert();
        assert_eq!(typist.state(), "x ▸|");
        assert!(typist.rewriter.is_empty());
    }

    #[test]
    fn replaces_when_the_cursor_leaves() {
        let mut typist = Typist::new(config(), "ab");
        typist.move_to(1);
        typist.type_text("\\to");
        assert_eq!(typist.state(), "a\\to|b");
        typist.move_to(0);
        assert_eq!(typist.state(), "|a→b");
    }

    #[test]
    fn follows_edits_before_and_stops_at_overlapping_ones() {
        let mut typist = Typist::new(config(), "ab ");
        typist.type_text("\\to");
        typist.rewriter.edited(
            &typist.config,
            vec![Change {
                range: 0..1,
                new_text: "xyz".into(),
            }],
            true,
        );
        typist.text.replace_range(0..1, "xyz");
        typist.cursor += 2;
        assert_eq!(typist.pending(), ["\\to"]);

        typist.rewriter.edited(
            &typist.config,
            vec![Change {
                range: 3..6,
                new_text: String::new(),
            }],
            true,
        );
        assert!(typist.rewriter.is_empty());
    }

    #[test]
    fn tracks_one_abbreviation_per_cursor() {
        let mut rewriter = Rewriter::default();
        let config = config();
        let insert = |offset: usize, text: &str| Change {
            range: offset..offset,
            new_text: text.into(),
        };
        // "a\nb" with cursors at 1 and 3.
        rewriter.edited(&config, vec![insert(1, "\\"), insert(3, "\\")], true);
        rewriter.edited(&config, vec![insert(2, "t"), insert(5, "t")], true);
        rewriter.edited(&config, vec![insert(3, "o"), insert(7, "o")], true);
        let mut pending: Vec<_> = rewriter.pending().collect();
        pending.sort_by_key(|range| range.start);
        assert_eq!(pending, [1..4, 6..9]);

        assert_eq!(rewriter.take_due(&config, &[4..4, 9..9]), []);
        assert_eq!(
            rewriter.take_due(&config, &[4..4]),
            [Replacement {
                range: 6..9,
                new_text: "→".into(),
                cursor: None
            }]
        );
    }

    #[test]
    fn uses_the_configured_leader_and_custom_translations() {
        let mut typist = Typist::new(
            Config {
                abbreviations: Arc::new(Abbreviations::with_custom([("wknight", "♘")])),
                leader: ",".into(),
                eager_replacement: false,
            },
            "",
        );
        typist.type_text("\\alpha ,alpha ,wknight");
        assert_eq!(typist.state(), "\\alpha α ,wknight|");
        typist.type_text(" ");
        assert_eq!(typist.state(), "\\alpha α ♘ |");
    }

    #[test]
    fn places_cursors_after_earlier_replacements() {
        let replacements = [
            Replacement {
                range: 0..6,
                new_text: "α".into(),
                cursor: None,
            },
            Replacement {
                range: 7..10,
                new_text: "⟨⟩".into(),
                cursor: Some(3),
            },
        ];
        // "\alpha \<>|" became "α ⟨⟩|".
        assert_eq!(place_cursors(&replacements, &[9, 0]), [6, 0]);
    }
}
