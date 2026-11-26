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
    /// The Lean InfoView panel, which shows goals and messages at the cursor.
    pub infoview: Option<LeanInfoviewSettingsContent>,
}

/// The InfoView's display settings, like vscode-lean4's `lean4.infoview.*`. The
/// InfoView's "Save Current Settings to Default Settings" writes them here.
#[with_fallible_options]
#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema, MergeFrom, PartialEq)]
pub struct LeanInfoviewSettingsContent {
    /// Whether the InfoView opens by itself for the first Lean 4 file.
    ///
    /// Default: true
    pub auto_open: Option<bool>,
    /// Whether the InfoView, when it opens by itself, shows the goal and messages at the
    /// cursor instead of all messages of the file.
    ///
    /// Default: true
    pub auto_open_shows_goal: Option<bool>,
    /// Whether to show all errors on the cursor's line, not only those to its right.
    ///
    /// Default: true
    pub all_errors_on_line: Option<bool>,
    /// How long, in milliseconds, the InfoView waits after the cursor stops before updating.
    ///
    /// Default: 50
    pub debounce_time: Option<u64>,
    /// How the expected type shows at first.
    ///
    /// Default: expanded_by_default
    pub expected_type_visibility: Option<LeanExpectedTypeVisibility>,
    /// Whether to show goal names, such as `case inl`.
    ///
    /// Default: true
    pub show_goal_names: Option<bool>,
    /// Whether to show goals other than the main goal in a smaller font.
    ///
    /// Default: false
    pub emphasize_first_goal: Option<bool>,
    /// Whether to show the goal before the local context.
    ///
    /// Default: false
    pub reverse_tactic_state: Option<bool>,
    /// Whether to hide type assumptions from the local context.
    ///
    /// Default: false
    pub hide_type_assumptions: Option<bool>,
    /// Whether to hide instance assumptions from the local context.
    ///
    /// Default: false
    pub hide_instance_assumptions: Option<bool>,
    /// Whether to hide inaccessible assumptions from the local context.
    ///
    /// Default: false
    pub hide_inaccessible_assumptions: Option<bool>,
    /// Whether to hide the values of let-bound assumptions.
    ///
    /// Default: false
    pub hide_let_values: Option<bool>,
    /// Whether hovering an expression shows its tooltip. Otherwise a click does.
    ///
    /// Default: true
    pub show_tooltip_on_hover: Option<bool>,
    /// How to sort "All Messages".
    ///
    /// Default: sort_by_proximity_to_cursor
    pub message_order: Option<LeanMessageOrder>,
}

#[derive(
    Copy,
    Clone,
    Debug,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    JsonSchema,
    MergeFrom,
    strum::VariantArray,
    strum::VariantNames,
)]
#[serde(rename_all = "snake_case")]
pub enum LeanExpectedTypeVisibility {
    ExpandedByDefault,
    CollapsedByDefault,
    Hidden,
}

#[derive(
    Copy,
    Clone,
    Debug,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    JsonSchema,
    MergeFrom,
    strum::VariantArray,
    strum::VariantNames,
)]
#[serde(rename_all = "snake_case")]
pub enum LeanMessageOrder {
    SortByProximityToCursor,
    SortByMessageLocation,
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
