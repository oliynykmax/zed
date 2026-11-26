//! The infoview's context menu entries, which vscode-lean4 adds to VS Code's webview
//! context menu. The page marks elements with `data-vscode-context`, and an entry shows
//! when the context of the clicked element has the entry's key.

use serde::Serialize;
use serde_json::{Map, Value};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ContextMenuEntry {
    GoToDefinition,
    GoToPinnedLocation,
    GoToMessageLocation,
    HideTraceSearch,
    ShowTraceSearch,
    Select,
    Unselect,
    UnselectAll,
    Pin,
    Unpin,
    Pause,
    Unpause,
    Refresh,
    PauseAllMessages,
    UnpauseAllMessages,
    CopyState,
    CopyMessage,
    DisplayTargetBeforeAssumptions,
    DisplayAssumptionsBeforeTarget,
    HideTypeAssumptions,
    ShowTypeAssumptions,
    HideInstanceAssumptions,
    ShowInstanceAssumptions,
    HideInaccessibleAssumptions,
    ShowInaccessibleAssumptions,
    HideLetValues,
    ShowLetValues,
    HideGoalNames,
    ShowGoalNames,
    EmphasizeFirstGoal,
    DeemphasizeFirstGoal,
    SaveSettings,
}

/// Where an entry goes. Separators divide the groups, and the settings groups form the
/// "Settings…" submenu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuGroup {
    Term,
    Info,
    Settings,
    SaveSettings,
}

struct MenuItem {
    entry: ContextMenuEntry,
    /// The `data-vscode-context` key whose value identifies what the entry acts on.
    key: &'static str,
    label: &'static str,
    group: MenuGroup,
}

const fn item(
    entry: ContextMenuEntry,
    key: &'static str,
    label: &'static str,
    group: MenuGroup,
) -> MenuItem {
    MenuItem {
        entry,
        key,
        label,
        group,
    }
}

/// In vscode-lean4's menu order.
const ITEMS: [MenuItem; 32] = {
    use ContextMenuEntry::*;
    use MenuGroup::{Info, SaveSettings as Save, Settings, Term};
    [
        item(
            GoToDefinition,
            "interactiveCodeTagId",
            "Go to Definition",
            Term,
        ),
        item(
            GoToPinnedLocation,
            "goToPinnedLocationId",
            "Go to Pinned Location",
            Term,
        ),
        item(
            GoToMessageLocation,
            "goToMessageLocationId",
            "Go to Source Location of Message",
            Term,
        ),
        item(HideTraceSearch, "hideTraceSearchId", "Hide Search", Term),
        item(ShowTraceSearch, "showTraceSearchId", "Show Search", Term),
        item(Select, "selectableLocationId", "Select", Term),
        item(Unselect, "unselectableLocationId", "Unselect", Term),
        item(UnselectAll, "selectedLocationsId", "Unselect All", Term),
        item(Pin, "pinId", "Pin State to Top", Info),
        item(Unpin, "unpinId", "Unpin State", Info),
        item(Pause, "pauseId", "Pause State", Info),
        item(Unpause, "unpauseId", "Unpause State", Info),
        item(Refresh, "refreshId", "Refresh Paused State", Info),
        item(
            PauseAllMessages,
            "pauseAllMessagesId",
            "Pause 'All Messages'",
            Info,
        ),
        item(
            UnpauseAllMessages,
            "unpauseAllMessagesId",
            "Unpause 'All Messages'",
            Info,
        ),
        item(CopyState, "copyStateId", "Copy State", Info),
        item(CopyMessage, "copyMessageId", "Copy Message", Info),
        item(
            DisplayTargetBeforeAssumptions,
            "displayTargetBeforeAssumptionsId",
            "Display Target Before Assumptions",
            Settings,
        ),
        item(
            DisplayAssumptionsBeforeTarget,
            "displayAssumptionsBeforeTargetId",
            "Display Assumptions Before Target",
            Settings,
        ),
        item(
            HideTypeAssumptions,
            "hideTypeAssumptionsId",
            "Hide Type Assumptions",
            Settings,
        ),
        item(
            ShowTypeAssumptions,
            "showTypeAssumptionsId",
            "Show Type Assumptions",
            Settings,
        ),
        item(
            HideInstanceAssumptions,
            "hideInstanceAssumptionsId",
            "Hide Instance Assumptions",
            Settings,
        ),
        item(
            ShowInstanceAssumptions,
            "showInstanceAssumptionsId",
            "Show Instance Assumptions",
            Settings,
        ),
        item(
            HideInaccessibleAssumptions,
            "hideInaccessibleAssumptionsId",
            "Hide Inaccessible Assumptions",
            Settings,
        ),
        item(
            ShowInaccessibleAssumptions,
            "showInaccessibleAssumptionsId",
            "Show Inaccessible Assumptions",
            Settings,
        ),
        item(
            HideLetValues,
            "hideLetValuesId",
            "Hide Let-Values",
            Settings,
        ),
        item(
            ShowLetValues,
            "showLetValuesId",
            "Show Let-Values",
            Settings,
        ),
        item(
            HideGoalNames,
            "hideGoalNamesId",
            "Hide Goal Names",
            Settings,
        ),
        item(
            ShowGoalNames,
            "showGoalNamesId",
            "Show Goal Names",
            Settings,
        ),
        item(
            EmphasizeFirstGoal,
            "emphasizeFirstGoalId",
            "Emphasize First Goal",
            Settings,
        ),
        item(
            DeemphasizeFirstGoal,
            "deemphasizeFirstGoalId",
            "De-Emphasize First Goal",
            Settings,
        ),
        item(
            SaveSettings,
            "saveSettingsId",
            "Save Current Settings to Default Settings",
            Save,
        ),
    ]
};

/// An entry that the clicked element offers.
#[derive(Debug, Clone, PartialEq)]
pub struct OfferedEntry {
    pub entry: ContextMenuEntry,
    pub label: &'static str,
    pub group: MenuGroup,
    /// What the entry acts on, passed back to the infoview as is.
    pub id: Value,
}

/// The entries offered by a clicked element's merged `data-vscode-context`, in menu order.
pub fn offered_entries(context: &Map<String, Value>) -> Vec<OfferedEntry> {
    ITEMS
        .iter()
        .filter_map(|item| {
            let id = context.get(item.key).filter(|id| is_truthy(id))?;
            Some(OfferedEntry {
                entry: item.entry,
                label: item.label,
                group: item.group,
                id: id.clone(),
            })
        })
        .collect()
}

/// Whether a VS Code `when` clause counts the value as set.
fn is_truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(value) => *value,
        Value::Number(number) => number.as_f64() != Some(0.),
        Value::String(text) => !text.is_empty(),
        Value::Array(_) | Value::Object(_) => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn entries(context: Value) -> Vec<(ContextMenuEntry, MenuGroup, Value)> {
        offered_entries(context.as_object().unwrap())
            .into_iter()
            .map(|offered| (offered.entry, offered.group, offered.id))
            .collect()
    }

    #[test]
    fn offers_the_entries_whose_keys_are_set() {
        assert_eq!(
            entries(json!({
                "copyStateId": "goals-1",
                "pinId": "4",
                "interactiveCodeTagId": "12",
                "hideTypeAssumptionsId": "7",
                "saveSettingsId": "7",
                "unrelated": "x",
            })),
            [
                (
                    ContextMenuEntry::GoToDefinition,
                    MenuGroup::Term,
                    json!("12")
                ),
                (ContextMenuEntry::Pin, MenuGroup::Info, json!("4")),
                (
                    ContextMenuEntry::CopyState,
                    MenuGroup::Info,
                    json!("goals-1")
                ),
                (
                    ContextMenuEntry::HideTypeAssumptions,
                    MenuGroup::Settings,
                    json!("7")
                ),
                (
                    ContextMenuEntry::SaveSettings,
                    MenuGroup::SaveSettings,
                    json!("7")
                ),
            ]
        );
    }

    #[test]
    fn skips_keys_that_a_when_clause_counts_as_unset() {
        assert_eq!(
            entries(json!({
                "pinId": "",
                "unpinId": null,
                "pauseId": false,
                "unpauseId": 0,
                "refreshId": 3,
            })),
            [(ContextMenuEntry::Refresh, MenuGroup::Info, json!(3))]
        );
        assert_eq!(entries(json!({})), []);
    }

    #[test]
    fn names_entries_as_the_infoview_does() {
        assert_eq!(
            serde_json::to_value(ContextMenuEntry::DeemphasizeFirstGoal).unwrap(),
            json!("deemphasizeFirstGoal")
        );
        assert_eq!(
            serde_json::to_value(ContextMenuEntry::GoToDefinition).unwrap(),
            json!("goToDefinition")
        );
    }
}
