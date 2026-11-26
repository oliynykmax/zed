use gpui::Context;
use language::{Buffer, Point, PointUtf16, Unclipped};
use lsp::{DocumentChangeOperation, DocumentChanges, Edit, TextEdit, Uri, WorkspaceEdit};

/// Flattens a workspace edit into batches of text edits, preferring
/// `documentChanges` over `changes` as the LSP specification asks. Each batch is one
/// document edit, sorted by position, and applies after the batches before it. File
/// creations, renames, and deletions are dropped, and snippets are inserted as plain text.
pub fn text_edits(edit: WorkspaceEdit) -> Vec<(Uri, Vec<TextEdit>)> {
    let mut batches: Vec<(Uri, Vec<TextEdit>)> = match (edit.document_changes, edit.changes) {
        (Some(document_changes), _) => {
            let document_edits = match document_changes {
                DocumentChanges::Edits(edits) => edits,
                DocumentChanges::Operations(operations) => operations
                    .into_iter()
                    .filter_map(|operation| match operation {
                        DocumentChangeOperation::Edit(edit) => Some(edit),
                        DocumentChangeOperation::Op(_) => None,
                    })
                    .collect(),
            };
            document_edits
                .into_iter()
                .map(|document_edit| {
                    let edits = document_edit
                        .edits
                        .into_iter()
                        .map(|edit| match edit {
                            Edit::Plain(edit) => edit,
                            Edit::Annotated(annotated) => annotated.text_edit,
                            Edit::Snippet(snippet) => TextEdit {
                                range: snippet.range,
                                new_text: snippet.snippet.value,
                            },
                        })
                        .collect();
                    (document_edit.text_document.uri, edits)
                })
                .collect()
        }
        (None, Some(changes)) => changes.into_iter().collect(),
        (None, None) => Vec::new(),
    };
    for (_, edits) in &mut batches {
        edits.sort_by_key(|edit| edit.range.start);
    }
    batches
}

/// Applies edits whose ranges all refer to the buffer as it is now.
pub fn apply(buffer: &mut Buffer, edits: Vec<TextEdit>, cx: &mut Context<Buffer>) {
    let snapshot = buffer.snapshot();
    let edits: Vec<_> = edits
        .into_iter()
        .map(|edit| {
            let start = utf16_point(edit.range.start, &snapshot);
            let end = utf16_point(edit.range.end, &snapshot);
            (start..end, edit.new_text)
        })
        .collect();
    buffer.edit(edits, None, cx);
}

pub fn utf16_point(position: lsp::Position, snapshot: &text::BufferSnapshot) -> Point {
    snapshot.point_utf16_to_point(snapshot.clip_point_utf16(
        Unclipped(PointUtf16::new(position.line, position.character)),
        text::Bias::Left,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{AppContext as _, TestAppContext};
    use serde_json::json;

    fn apply_workspace_edit(
        text: &str,
        edit: serde_json::Value,
        cx: &mut TestAppContext,
    ) -> String {
        let buffer = cx.new(|cx| Buffer::local(text, cx));
        let edit: WorkspaceEdit = serde_json::from_value(edit).unwrap();
        for (_, edits) in text_edits(edit) {
            buffer.update(cx, |buffer, cx| apply(buffer, edits, cx));
        }
        buffer.read_with(cx, |buffer, _| buffer.text())
    }

    #[gpui::test]
    fn applies_edits_given_bottom_up(cx: &mut TestAppContext) {
        let text = apply_workspace_edit(
            "a\nb\nc\n",
            json!({"changes": {"file:///p/A.lean": [
                {"range": {"start": {"line": 2, "character": 0}, "end": {"line": 2, "character": 1}}, "newText": "C"},
                {"range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 1}}, "newText": "A"}
            ]}}),
            cx,
        );
        assert_eq!(text, "A\nb\nC\n");
    }

    #[gpui::test]
    fn applies_each_document_edit_after_the_one_before(cx: &mut TestAppContext) {
        let text = apply_workspace_edit(
            "theorem t : True := by\n  sorry\n",
            json!({"documentChanges": [
                {
                    "textDocument": {"uri": "file:///p/A.lean", "version": 3},
                    "edits": [{"range": {"start": {"line": 1, "character": 0}, "end": {"line": 1, "character": 0}}, "newText": "  trivial\n"}]
                },
                {
                    "textDocument": {"uri": "file:///p/A.lean", "version": 4},
                    "edits": [{"range": {"start": {"line": 2, "character": 0}, "end": {"line": 3, "character": 0}}, "newText": ""}]
                }
            ]}),
            cx,
        );
        assert_eq!(text, "theorem t : True := by\n  trivial\n");
    }

    #[test]
    fn flattens_document_changes() {
        let edit: WorkspaceEdit = serde_json::from_value(json!({
            "changes": {"file:///p/Ignored.lean": []},
            "documentChanges": [
                {
                    "textDocument": {"uri": "file:///p/A.lean", "version": 3},
                    "edits": [
                        {"range": {"start": {"line": 1, "character": 2}, "end": {"line": 1, "character": 7}}, "newText": "exact h"},
                        {"range": {"start": {"line": 4, "character": 0}, "end": {"line": 4, "character": 0}}, "newText": "-- ", "annotationId": "a"}
                    ]
                },
                {"kind": "create", "uri": "file:///p/New.lean"},
                {
                    "textDocument": {"uri": "file:///p/A.lean", "version": 3},
                    "edits": [
                        {"range": {"start": {"line": 9, "character": 0}, "end": {"line": 9, "character": 1}}, "newText": ""}
                    ]
                }
            ]
        }))
        .unwrap();

        let edits = text_edits(edit);
        assert_eq!(
            serde_json::to_value(&edits).unwrap(),
            json!([
                [
                    "file:///p/A.lean",
                    [
                        {"range": {"start": {"line": 1, "character": 2}, "end": {"line": 1, "character": 7}}, "newText": "exact h"},
                        {"range": {"start": {"line": 4, "character": 0}, "end": {"line": 4, "character": 0}}, "newText": "-- "}
                    ]
                ],
                [
                    "file:///p/A.lean",
                    [{"range": {"start": {"line": 9, "character": 0}, "end": {"line": 9, "character": 1}}, "newText": ""}]
                ]
            ])
        );
    }

    #[test]
    fn falls_back_to_changes() {
        let edit: WorkspaceEdit = serde_json::from_value(json!({
            "changes": {"file:///p/B.lean": [
                {"range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 0}}, "newText": "import Mathlib\n"}
            ]}
        }))
        .unwrap();
        assert_eq!(
            serde_json::to_value(text_edits(edit)).unwrap(),
            json!([[
                "file:///p/B.lean",
                [{"range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 0}}, "newText": "import Mathlib\n"}]
            ]])
        );
    }
}
