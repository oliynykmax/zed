use settings::{
    LeanExpectedTypeVisibility, LeanInfoviewSettingsContent, LeanMessageOrder, RegisterSetting,
    Settings,
};

use crate::protocol::{ExpectedTypeVisibility, InfoviewConfig, MessageOrder};

#[derive(Clone, Debug, RegisterSetting)]
pub struct LeanInfoviewSettings {
    pub auto_open: bool,
    pub config: InfoviewConfig,
}

impl Settings for LeanInfoviewSettings {
    fn from_settings(content: &settings::SettingsContent) -> Self {
        let infoview = content.lean.clone().unwrap().infoview.unwrap();
        Self {
            auto_open: infoview.auto_open.unwrap(),
            config: InfoviewConfig {
                all_errors_on_line: infoview.all_errors_on_line.unwrap(),
                auto_open_shows_goal: infoview.auto_open_shows_goal.unwrap(),
                debounce_time: infoview.debounce_time.unwrap(),
                expected_type_visibility: infoview.expected_type_visibility.unwrap().into(),
                show_goal_names: infoview.show_goal_names.unwrap(),
                emphasize_first_goal: infoview.emphasize_first_goal.unwrap(),
                reverse_tactic_state: infoview.reverse_tactic_state.unwrap(),
                hide_type_assumptions: infoview.hide_type_assumptions.unwrap(),
                hide_instance_assumptions: infoview.hide_instance_assumptions.unwrap(),
                hide_inaccessible_assumptions: infoview.hide_inaccessible_assumptions.unwrap(),
                hide_let_values: infoview.hide_let_values.unwrap(),
                show_tooltip_on_hover: infoview.show_tooltip_on_hover.unwrap(),
                message_order: infoview.message_order.unwrap().into(),
            },
        }
    }
}

impl InfoviewConfig {
    /// Writes the config into settings, as vscode-lean4's `saveConfig` does.
    pub fn write_to(&self, settings: &mut LeanInfoviewSettingsContent) {
        settings.all_errors_on_line = Some(self.all_errors_on_line);
        settings.auto_open_shows_goal = Some(self.auto_open_shows_goal);
        settings.debounce_time = Some(self.debounce_time);
        settings.expected_type_visibility = Some(self.expected_type_visibility.into());
        settings.show_goal_names = Some(self.show_goal_names);
        settings.emphasize_first_goal = Some(self.emphasize_first_goal);
        settings.reverse_tactic_state = Some(self.reverse_tactic_state);
        settings.hide_type_assumptions = Some(self.hide_type_assumptions);
        settings.hide_instance_assumptions = Some(self.hide_instance_assumptions);
        settings.hide_inaccessible_assumptions = Some(self.hide_inaccessible_assumptions);
        settings.hide_let_values = Some(self.hide_let_values);
        settings.show_tooltip_on_hover = Some(self.show_tooltip_on_hover);
        settings.message_order = Some(self.message_order.into());
    }
}

impl From<LeanExpectedTypeVisibility> for ExpectedTypeVisibility {
    fn from(visibility: LeanExpectedTypeVisibility) -> Self {
        match visibility {
            LeanExpectedTypeVisibility::ExpandedByDefault => Self::ExpandedByDefault,
            LeanExpectedTypeVisibility::CollapsedByDefault => Self::CollapsedByDefault,
            LeanExpectedTypeVisibility::Hidden => Self::Hidden,
        }
    }
}

impl From<ExpectedTypeVisibility> for LeanExpectedTypeVisibility {
    fn from(visibility: ExpectedTypeVisibility) -> Self {
        match visibility {
            ExpectedTypeVisibility::ExpandedByDefault => Self::ExpandedByDefault,
            ExpectedTypeVisibility::CollapsedByDefault => Self::CollapsedByDefault,
            ExpectedTypeVisibility::Hidden => Self::Hidden,
        }
    }
}

impl From<LeanMessageOrder> for MessageOrder {
    fn from(order: LeanMessageOrder) -> Self {
        match order {
            LeanMessageOrder::SortByProximityToCursor => Self::SortByProximityToCursor,
            LeanMessageOrder::SortByMessageLocation => Self::SortByMessageLocation,
        }
    }
}

impl From<MessageOrder> for LeanMessageOrder {
    fn from(order: MessageOrder) -> Self {
        match order {
            MessageOrder::SortByProximityToCursor => Self::SortByProximityToCursor,
            MessageOrder::SortByMessageLocation => Self::SortByMessageLocation,
        }
    }
}

#[cfg(test)]
mod tests {
    use gpui::{App, BorrowAppContext as _};
    use settings::SettingsStore;

    use super::*;

    #[gpui::test]
    fn reads_the_defaults_and_writes_a_saved_config_back(cx: &mut App) {
        let store = SettingsStore::test(cx);
        cx.set_global(store);
        let settings = LeanInfoviewSettings::get_global(cx).clone();
        assert!(settings.auto_open);
        assert_eq!(settings.config.debounce_time, 50);
        assert_eq!(
            settings.config.expected_type_visibility,
            ExpectedTypeVisibility::ExpandedByDefault
        );

        let saved = InfoviewConfig {
            hide_let_values: true,
            message_order: MessageOrder::SortByMessageLocation,
            ..settings.config
        };
        cx.update_global::<SettingsStore, _>(|store, cx| {
            store.update_user_settings(cx, |content| {
                saved.write_to(
                    content
                        .lean
                        .get_or_insert_default()
                        .infoview
                        .get_or_insert_default(),
                )
            })
        });
        assert_eq!(LeanInfoviewSettings::get_global(cx).config, saved);
    }
}
