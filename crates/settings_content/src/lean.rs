use collections::IndexMap;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use settings_macros::{MergeFrom, with_fallible_options};

/// Settings for Lean 4 files.
#[with_fallible_options]
#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema, MergeFrom, PartialEq)]
pub struct LeanSettingsContent {
    /// Unicode input, which turns abbreviations such as `\alpha` into symbols such as `α`.
    pub input: Option<LeanInputSettingsContent>,
}

#[with_fallible_options]
#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema, MergeFrom, PartialEq)]
pub struct LeanInputSettingsContent {
    /// Whether typing the leader in a Lean 4 file starts an abbreviation.
    ///
    /// Default: true
    pub enabled: Option<bool>,
    /// The text that starts an abbreviation.
    ///
    /// Default: "\\"
    pub leader: Option<String>,
    /// Whether to replace an abbreviation as soon as it is complete and no longer
    /// abbreviation starts with it. Otherwise only a character that continues no
    /// abbreviation, a cursor move, or `editor::ConvertLeanAbbreviation` replaces it.
    ///
    /// Default: true
    pub eager_replacement: Option<bool>,
    /// More abbreviations, written without the leader, and the symbols they produce.
    /// `$CURSOR` marks where the cursor goes.
    ///
    /// Default: {}
    pub custom_translations: Option<IndexMap<String, String>>,
}
