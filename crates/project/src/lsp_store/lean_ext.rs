use ::serde::{Deserialize, Serialize};
use lsp::LanguageServerName;

pub const LEAN_SERVER_NAME: LanguageServerName = LanguageServerName::new_static("lean4-lsp");

/// Returns the tactic state at a position, rendered as Markdown.
///
/// [Lean 4 server source](https://github.com/leanprover/lean4/blob/master/src/Lean/Server/FileWorker/RequestHandling.lean)
pub enum PlainGoal {}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlainGoalResult {
    pub rendered: String,
    pub goals: Vec<String>,
}

impl lsp::request::Request for PlainGoal {
    type Params = lsp::TextDocumentPositionParams;
    type Result = Option<PlainGoalResult>;
    const METHOD: &'static str = "$/lean/plainGoal";
}
