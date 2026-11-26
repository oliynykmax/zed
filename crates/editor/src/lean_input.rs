//! Lean 4 Unicode input: typing `\alpha` gives `α`, as in vscode-lean4.

use std::{any::Any, ops::Range};

use gpui::{App, Context, Entity, HighlightStyle, KeyContext, UnderlineStyle, Window, px};
use language::Buffer;
use lean_abbreviations::{Abbreviations, Change, Config, Replacement, Rewriter, place_cursors};
use multi_buffer::{Anchor, MultiBufferOffset, MultiBufferSnapshot, ToOffset as _};
use settings::{RegisterSetting, Settings};

use crate::{
    Addon, ConvertLeanAbbreviation, Editor, EditorEvent, HighlightKey, SelectionEffects,
    lean_ext::is_lean_language,
};

#[derive(Clone, Debug, RegisterSetting)]
pub struct LeanInputSettings {
    pub enabled: bool,
    pub config: Config,
}

impl Settings for LeanInputSettings {
    fn from_settings(content: &settings::SettingsContent) -> Self {
        let input = content.lean.clone().unwrap().input.unwrap();
        let custom = input.custom_translations.unwrap();
        let abbreviations = if custom.is_empty() {
            Abbreviations::bundled()
        } else {
            Abbreviations::with_custom(custom.iter().map(|(a, s)| (a.as_str(), s.as_str()))).into()
        };
        Self {
            enabled: input.enabled.unwrap(),
            config: Config {
                abbreviations,
                leader: input.leader.unwrap(),
                eager_replacement: input.eager_replacement.unwrap(),
            },
        }
    }
}

/// The abbreviations being typed in an editor of a Lean 4 buffer.
struct LeanInput {
    buffer: Entity<Buffer>,
    /// The version whose edits the rewriter has seen.
    version: clock::Global,
    rewriter: Rewriter,
}

impl Addon for LeanInput {
    fn extend_key_context(&self, context: &mut KeyContext, _: &App) {
        if !self.rewriter.is_empty() {
            context.add("lean_abbreviation");
        }
    }

    fn to_any(&self) -> &dyn Any {
        self
    }

    fn to_any_mut(&mut self) -> Option<&mut dyn Any> {
        Some(self)
    }
}

pub(crate) fn init(cx: &mut App) {
    cx.observe_new(|editor: &mut Editor, window, cx| {
        let Some(window) = window else {
            return;
        };
        if !editor.mode().is_full() {
            return;
        }
        cx.subscribe_in(&cx.entity(), window, |editor, _, event, window, cx| {
            if matches!(
                event,
                EditorEvent::BufferEdited | EditorEvent::SelectionsChanged { .. }
            ) {
                update(editor, Due::Settled, window, cx);
            }
        })
        .detach();
    })
    .detach();
}

pub(crate) fn convert(
    editor: &mut Editor,
    _: &ConvertLeanAbbreviation,
    window: &mut Window,
    cx: &mut Context<Editor>,
) {
    update(editor, Due::All, window, cx);
}

/// Which tracked abbreviations to replace.
enum Due {
    Settled,
    All,
}

/// Brings the rewriter up to date with the buffer and selections, and makes the
/// replacements that are due.
fn update(editor: &mut Editor, due: Due, window: &mut Window, cx: &mut Context<Editor>) {
    let settings = LeanInputSettings::get_global(cx);
    let buffer = editor.buffer().read(cx).as_singleton().filter(|buffer| {
        buffer
            .read(cx)
            .language()
            .is_some_and(|language| is_lean_language(language))
    });
    let Some(buffer) = buffer.filter(|_| settings.enabled) else {
        if editor.addon::<LeanInput>().is_some() {
            editor.unregister_addon::<LeanInput>();
            editor.clear_highlights(HighlightKey::LeanAbbreviation, cx);
        }
        return;
    };
    let config = settings.config.clone();
    if editor
        .addon::<LeanInput>()
        .is_none_or(|input| input.buffer != buffer)
    {
        editor.register_addon(LeanInput {
            version: buffer.read(cx).version(),
            buffer: buffer.clone(),
            rewriter: Rewriter::default(),
        });
    }

    let display_snapshot = editor.display_snapshot(cx);
    let selections: Vec<Range<usize>> = editor
        .selections
        .all::<MultiBufferOffset>(&display_snapshot)
        .into_iter()
        .map(|selection| selection.start.0..selection.end.0)
        .collect();
    let Some(input) = editor.addon_mut::<LeanInput>() else {
        return;
    };
    let changes = input.edits(cx);
    input.rewriter.edited(&config, changes, true);
    let replacements = match due {
        Due::Settled => input.rewriter.take_due(&config, &selections),
        Due::All => input.rewriter.take_all(&config),
    };
    if !replacements.is_empty() {
        buffer.update(cx, |buffer, cx| {
            buffer.edit(
                replacements
                    .iter()
                    .map(|replacement| (replacement.range.clone(), replacement.new_text.clone())),
                None,
                cx,
            )
        });
        let changes = input.edits(cx);
        input.rewriter.edited(&config, changes, false);
    }
    let pending: Vec<_> = input.rewriter.pending().collect();
    if replacements
        .iter()
        .any(|replacement| replacement.cursor.is_some())
    {
        move_cursors_to_markers(editor, &replacements, window, cx);
    }
    underline(editor, pending, cx);
}

impl LeanInput {
    /// The edits since the last call.
    fn edits(&mut self, cx: &App) -> Vec<Change> {
        let snapshot = self.buffer.read(cx).text_snapshot();
        let changes = snapshot
            .edits_since::<usize>(&self.version)
            .map(|edit| Change {
                range: edit.old,
                new_text: snapshot.text_for_range(edit.new).collect(),
            })
            .collect();
        self.version = snapshot.version().clone();
        changes
    }
}

fn move_cursors_to_markers(
    editor: &mut Editor,
    replacements: &[Replacement],
    window: &mut Window,
    cx: &mut Context<Editor>,
) {
    let selections = editor
        .selections
        .all::<MultiBufferOffset>(&editor.display_snapshot(cx));
    let starts: Vec<_> = selections
        .iter()
        .map(|selection| selection.start.0)
        .collect();
    let placed = place_cursors(replacements, &starts);
    if placed == starts {
        return;
    }
    editor.change_selections(SelectionEffects::no_scroll(), window, cx, |s| {
        s.select_ranges(selections.iter().zip(placed).map(|(selection, start)| {
            MultiBufferOffset(start)..MultiBufferOffset(start + selection.end.0 - selection.start.0)
        }))
    });
}

/// How to type the Lean 4 symbols that start at `anchor`, as Markdown, and the text they cover.
pub(crate) fn abbreviation_hover(
    buffer: &MultiBufferSnapshot,
    anchor: Anchor,
    cx: &App,
) -> Option<(String, Range<Anchor>)> {
    let offset = anchor.to_offset(buffer);
    if !buffer
        .language_at(offset)
        .is_some_and(|language| is_lean_language(language))
    {
        return None;
    }
    let config = &LeanInputSettings::get_global(cx).config;
    let line: String = buffer.chars_at(offset).take_while(|c| *c != '\n').collect();
    let symbols = config.abbreviations.symbols_at(&line);
    let len = symbols.iter().map(|input| input.symbol.len()).max()?;
    let markdown = symbols
        .iter()
        .map(|input| input.hover_markdown(&config.leader))
        .collect::<Vec<_>>()
        .join("\n\n");
    Some((markdown, anchor..buffer.anchor_after(offset + len)))
}

fn underline(editor: &mut Editor, pending: Vec<Range<usize>>, cx: &mut Context<Editor>) {
    if pending.is_empty() {
        editor.clear_highlights(HighlightKey::LeanAbbreviation, cx);
        return;
    }
    let snapshot = editor.buffer().read(cx).snapshot(cx);
    let ranges = pending
        .into_iter()
        .map(|range| {
            snapshot.anchor_after(MultiBufferOffset(range.start))
                ..snapshot.anchor_before(MultiBufferOffset(range.end))
        })
        .collect();
    editor.highlight_text(
        HighlightKey::LeanAbbreviation,
        ranges,
        HighlightStyle {
            underline: Some(UnderlineStyle {
                thickness: px(1.),
                color: None,
                wavy: false,
            }),
            ..Default::default()
        },
        cx,
    );
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use gpui::{BorrowAppContext as _, KeyBinding, TestAppContext};
    use language::{Language, LanguageConfig};
    use settings::SettingsStore;

    use super::*;
    use crate::{
        editor_tests::init_test,
        test::{
            editor_lsp_test_context::EditorLspTestContext, editor_test_context::EditorTestContext,
        },
    };

    async fn lean_editor(cx: &mut TestAppContext) -> EditorTestContext {
        init_test(cx, |_| {});
        let mut cx = EditorTestContext::new(cx).await;
        let lean = Arc::new(Language::new(
            LanguageConfig {
                name: "Lean 4".into(),
                ..LanguageConfig::default()
            },
            None,
        ));
        cx.update_buffer(|buffer, cx| buffer.set_language(Some(lean), cx));
        cx.update(|_, cx| {
            cx.bind_keys([KeyBinding::new(
                "tab",
                ConvertLeanAbbreviation,
                Some("Editor && lean_abbreviation"),
            )])
        });
        cx
    }

    #[gpui::test]
    async fn replaces_abbreviations_as_they_are_typed(cx: &mut TestAppContext) {
        let mut cx = lean_editor(cx).await;
        cx.set_state("ˇ");
        cx.simulate_input("\\alpha ");
        cx.assert_editor_state("α ˇ");
        cx.simulate_input("\\to x \\\\ \\<>");
        cx.assert_editor_state("α → x \\ ⟨ˇ⟩");
    }

    #[gpui::test]
    async fn underlines_a_pending_abbreviation_until_tab(cx: &mut TestAppContext) {
        let mut cx = lean_editor(cx).await;
        cx.set_state("a ˇb");
        cx.simulate_input("\\to");
        cx.assert_editor_state("a \\toˇb");
        cx.assert_editor_text_highlights(HighlightKey::LeanAbbreviation, "a «\\to»b");
        cx.simulate_keystrokes("tab");
        cx.assert_editor_state("a →ˇb");
        cx.assert_editor_text_highlights(HighlightKey::LeanAbbreviation, "a →b");
        cx.simulate_keystrokes("tab");
        cx.assert_editor_state("a →\tˇb");
    }

    #[gpui::test]
    async fn replaces_when_the_cursor_moves_away(cx: &mut TestAppContext) {
        let mut cx = lean_editor(cx).await;
        cx.set_state("ˇx");
        cx.simulate_input("\\to");
        cx.update_editor(|editor, window, cx| editor.move_right(&crate::MoveRight, window, cx));
        cx.assert_editor_state("→xˇ");
    }

    #[gpui::test]
    async fn types_at_every_cursor(cx: &mut TestAppContext) {
        let mut cx = lean_editor(cx).await;
        cx.set_state("aˇ\nbˇ");
        cx.simulate_input("\\and\\to");
        cx.assert_editor_state("a∧\\toˇ\nb∧\\toˇ");
        cx.simulate_input(" ");
        cx.assert_editor_state("a∧→ ˇ\nb∧→ ˇ");
    }

    #[gpui::test]
    async fn leaves_other_languages_alone(cx: &mut TestAppContext) {
        let mut cx = lean_editor(cx).await;
        cx.update_buffer(|buffer, cx| buffer.set_language(None, cx));
        cx.set_state("ˇ");
        cx.simulate_input("\\alpha ");
        cx.assert_editor_state("\\alpha ˇ");
    }

    #[gpui::test]
    async fn can_be_turned_off(cx: &mut TestAppContext) {
        let mut cx = lean_editor(cx).await;
        cx.update(|_, cx| {
            cx.update_global::<SettingsStore, _>(|store, cx| {
                store.update_user_settings(cx, |settings| {
                    settings.lean.get_or_insert_default().input =
                        Some(settings::LeanInputSettingsContent {
                            enabled: Some(false),
                            ..Default::default()
                        });
                });
            });
        });
        cx.set_state("ˇ");
        cx.simulate_input("\\alpha ");
        cx.assert_editor_state("\\alpha ˇ");
    }

    #[gpui::test]
    async fn hovers_how_to_type_a_symbol_next_to_the_server_hover(cx: &mut TestAppContext) {
        let mut cx = crate::lean_ext::lean_lsp_test_context(
            lsp::ServerCapabilities {
                hover_provider: Some(lsp::HoverProviderCapability::Simple(true)),
                ..Default::default()
            },
            cx,
        )
        .await;
        cx.set_request_handler::<lsp::request::HoverRequest, _, _>(|_, _, _| async {
            Ok(Some(lsp::Hover {
                contents: lsp::HoverContents::Markup(lsp::MarkupContent {
                    kind: lsp::MarkupKind::Markdown,
                    value: "anonymous constructor".into(),
                }),
                range: None,
            }))
        });
        cx.set_state("ˇ⟨a, b⟩");

        let hovers = |cx: &mut EditorLspTestContext, offset: usize| {
            cx.update_editor(|editor, window, cx| {
                let anchor = editor
                    .buffer()
                    .read(cx)
                    .snapshot(cx)
                    .anchor_before(MultiBufferOffset(offset));
                editor.change_selections(SelectionEffects::no_scroll(), window, cx, |s| {
                    s.select_anchor_ranges([anchor..anchor])
                });
                crate::hover_popover::hover(editor, &crate::Hover, window, cx);
            });
            cx.run_until_parked();
            cx.editor(|editor, _, cx| {
                editor
                    .hover_state
                    .info_popovers
                    .iter()
                    .filter_map(|popover| popover.parsed_content.as_ref())
                    .map(|markdown| markdown.read(cx).source().to_string())
                    .collect::<Vec<_>>()
            })
        };
        assert_eq!(
            hovers(&mut cx, 0),
            [
                "anonymous constructor",
                "Type ⟨ using `\\<` or `\\langle`. ⟨ can be auto-closed with ⟩ using `\\<>`."
            ]
        );
        assert_eq!(hovers(&mut cx, "⟨".len()), ["anonymous constructor"]);
    }

    #[gpui::test]
    async fn uses_the_configured_leader(cx: &mut TestAppContext) {
        let mut cx = lean_editor(cx).await;
        cx.update(|_, cx| {
            cx.update_global::<SettingsStore, _>(|store, cx| {
                store.update_user_settings(cx, |settings| {
                    settings.lean.get_or_insert_default().input =
                        Some(settings::LeanInputSettingsContent {
                            leader: Some(",".into()),
                            custom_translations: Some(
                                [("wknight".to_string(), "♘".to_string())]
                                    .into_iter()
                                    .collect(),
                            ),
                            ..Default::default()
                        });
                });
            });
        });
        cx.set_state("ˇ");
        cx.simulate_input("\\alpha ,alpha ,wknight");
        cx.assert_editor_state("\\alpha α ♘ˇ");
    }
}
