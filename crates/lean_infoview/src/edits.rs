use lsp::{DocumentChangeOperation, DocumentChanges, Edit, TextEdit, Uri, WorkspaceEdit};

/// Flattens a workspace edit into the text edits of each document, preferring
/// `documentChanges` over `changes` as the LSP specification asks. File creations,
/// renames, and deletions are dropped, and snippets are inserted as plain text.
pub fn text_edits(edit: WorkspaceEdit) -> Vec<(Uri, Vec<TextEdit>)> {
    let mut by_document: Vec<(Uri, Vec<TextEdit>)> = Vec::new();
    let mut add = |uri: Uri, edits: Vec<TextEdit>| match by_document
        .iter_mut()
        .find(|(existing, _)| *existing == uri)
    {
        Some((_, existing)) => existing.extend(edits),
        None => by_document.push((uri, edits)),
    };
    match (edit.document_changes, edit.changes) {
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
            for document_edit in document_edits {
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
                add(document_edit.text_document.uri, edits);
            }
        }
        (None, Some(changes)) => {
            for (uri, edits) in changes {
                add(uri, edits);
            }
        }
        (None, None) => {}
    }
    by_document
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

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
            json!([[
                "file:///p/A.lean",
                [
                    {"range": {"start": {"line": 1, "character": 2}, "end": {"line": 1, "character": 7}}, "newText": "exact h"},
                    {"range": {"start": {"line": 4, "character": 0}, "end": {"line": 4, "character": 0}}, "newText": "-- "},
                    {"range": {"start": {"line": 9, "character": 0}, "end": {"line": 9, "character": 1}}, "newText": ""}
                ]
            ]])
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
