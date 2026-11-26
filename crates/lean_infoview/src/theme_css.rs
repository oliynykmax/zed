//! Builds `theme.css`, which sets the `--vscode-*` variables that the infoview's
//! styles read, from the active Zed theme and fonts.

use std::fmt::Write as _;

use gpui::{Hsla, Rgba};
use theme::{StatusColors, ThemeColors};

/// The fonts the page uses, as Zed's settings name them.
pub struct PageFonts {
    pub ui_family: String,
    pub ui_size: f32,
    pub ui_weight: f32,
    pub buffer_family: String,
    pub buffer_size: f32,
    pub buffer_weight: f32,
    /// In pixels.
    pub buffer_line_height: f32,
}

type ColorOf = fn(&ThemeColors, &StatusColors) -> Hsla;

const COLOR_VARIABLES: &[(&str, ColorOf)] = &[
    ("--vscode-foreground", |c, _| c.text),
    ("--vscode-descriptionForeground", |c, _| c.text_muted),
    ("--vscode-disabledForeground", |c, _| c.text_disabled),
    ("--vscode-errorForeground", |_, s| s.error),
    ("--vscode-icon-foreground", |c, _| c.icon),
    ("--vscode-focusBorder", |c, _| c.border_focused),
    ("--vscode-widget-border", |c, _| c.border),
    ("--vscode-widget-shadow", |c, _| c.border_variant),
    ("--vscode-editor-background", |c, _| c.panel_background),
    ("--vscode-editor-foreground", |c, _| c.editor_foreground),
    ("--vscode-editor-selectionBackground", |c, _| {
        c.element_selection_background
    }),
    ("--vscode-editor-findMatchBackground", |c, _| {
        c.search_active_match_background
    }),
    (
        "--vscode-editorOverviewRuler-rangeHighlightForeground",
        |c, _| c.search_match_background,
    ),
    ("--vscode-editorError-foreground", |_, s| s.error),
    ("--vscode-editorWarning-foreground", |_, s| s.warning),
    ("--vscode-editorInfo-foreground", |_, s| s.info),
    ("--vscode-editorHint-foreground", |_, s| s.hint),
    ("--vscode-editorWidget-border", |c, _| c.border),
    ("--vscode-editorHoverWidget-background", |c, _| {
        c.elevated_surface_background
    }),
    ("--vscode-editorHoverWidget-foreground", |c, _| c.text),
    ("--vscode-editorHoverWidget-border", |c, _| c.border),
    ("--vscode-editorGroup-border", |c, _| c.border),
    ("--vscode-diffEditor-insertedTextBackground", |_, s| {
        s.created_background
    }),
    ("--vscode-diffEditor-insertedTextBorder", |_, s| {
        s.created_border
    }),
    ("--vscode-diffEditor-removedTextBackground", |_, s| {
        s.deleted_background
    }),
    ("--vscode-diffEditor-removedTextBorder", |_, s| {
        s.deleted_border
    }),
    ("--vscode-textLink-foreground", |c, _| c.text_accent),
    ("--vscode-textLink-activeForeground", |c, _| {
        c.link_text_hover
    }),
    ("--vscode-textPreformat-foreground", |c, _| c.text),
    ("--vscode-textPreformat-background", |c, _| {
        c.element_background
    }),
    ("--vscode-textCodeBlock-background", |c, _| {
        c.element_background
    }),
    ("--vscode-textBlockQuote-background", |c, _| {
        c.element_background
    }),
    ("--vscode-textBlockQuote-border", |c, _| c.border),
    ("--vscode-sideBar-background", |c, _| c.panel_background),
    ("--vscode-sideBarTitle-foreground", |c, _| c.text),
    ("--vscode-sideBarSectionHeader-background", |c, _| {
        c.panel_background
    }),
    ("--vscode-panel-background", |c, _| c.panel_background),
    ("--vscode-panelTitle-activeForeground", |c, _| c.text),
    ("--vscode-panelTitle-inactiveForeground", |c, _| {
        c.text_muted
    }),
    ("--vscode-panelTitle-activeBorder", |c, _| c.border_focused),
    ("--vscode-sash-hoverBorder", |c, _| c.border_focused),
    ("--vscode-progressBar-background", |c, _| c.text_accent),
    ("--vscode-badge-background", |c, _| c.element_selected),
    ("--vscode-badge-foreground", |c, _| c.text),
    ("--vscode-button-background", |c, _| c.element_selected),
    ("--vscode-button-foreground", |c, _| c.text),
    ("--vscode-button-hoverBackground", |c, _| c.element_hover),
    ("--vscode-button-border", |c, _| c.border_variant),
    ("--vscode-button-secondaryBackground", |c, _| {
        c.element_background
    }),
    ("--vscode-button-secondaryForeground", |c, _| c.text),
    ("--vscode-button-secondaryHoverBackground", |c, _| {
        c.element_hover
    }),
    ("--vscode-dropdown-background", |c, _| {
        c.elevated_surface_background
    }),
    ("--vscode-dropdown-foreground", |c, _| c.text),
    ("--vscode-dropdown-border", |c, _| c.border),
    ("--vscode-input-background", |c, _| c.editor_background),
    ("--vscode-input-foreground", |c, _| c.text),
    ("--vscode-input-border", |c, _| c.border),
    ("--vscode-input-placeholderForeground", |c, _| {
        c.text_placeholder
    }),
    ("--vscode-inputValidation-errorBackground", |_, s| {
        s.error_background
    }),
    ("--vscode-inputValidation-errorBorder", |_, s| {
        s.error_border
    }),
    ("--vscode-settings-textInputBackground", |c, _| {
        c.editor_background
    }),
    ("--vscode-settings-textInputForeground", |c, _| c.text),
    ("--vscode-settings-textInputBorder", |c, _| c.border),
    ("--vscode-settings-dropdownBackground", |c, _| {
        c.elevated_surface_background
    }),
    ("--vscode-settings-dropdownForeground", |c, _| c.text),
    ("--vscode-settings-dropdownBorder", |c, _| c.border),
    ("--vscode-settings-dropdownListBorder", |c, _| c.border),
    ("--vscode-settings-checkboxBackground", |c, _| {
        c.element_background
    }),
    ("--vscode-settings-checkboxForeground", |c, _| c.text),
    ("--vscode-settings-checkboxBorder", |c, _| c.border),
    ("--vscode-settings-headerBorder", |c, _| c.border),
    ("--vscode-checkbox-background", |c, _| c.element_background),
    ("--vscode-checkbox-foreground", |c, _| c.text),
    ("--vscode-checkbox-border", |c, _| c.border),
    ("--vscode-list-hoverBackground", |c, _| {
        c.ghost_element_hover
    }),
    ("--vscode-list-hoverForeground", |c, _| c.text),
    ("--vscode-list-activeSelectionBackground", |c, _| {
        c.element_selected
    }),
    ("--vscode-list-activeSelectionForeground", |c, _| c.text),
    ("--vscode-list-inactiveSelectionBackground", |c, _| {
        c.ghost_element_selected
    }),
    ("--vscode-list-inactiveSelectionForeground", |c, _| c.text),
    ("--vscode-list-inactiveSelectionIconForeground", |c, _| {
        c.icon
    }),
    ("--vscode-list-inactiveFocusBackground", |c, _| {
        c.ghost_element_selected
    }),
    ("--vscode-list-focusOutline", |c, _| c.border_focused),
    ("--vscode-list-inactiveFocusOutline", |c, _| {
        c.border_variant
    }),
    ("--vscode-list-highlightForeground", |c, _| c.text_accent),
    ("--vscode-tree-indentGuidesStroke", |c, _| {
        c.panel_indent_guide
    }),
    ("--vscode-tree-inactiveIndentGuidesStroke", |c, _| {
        c.panel_indent_guide
    }),
    ("--vscode-toolbar-hoverBackground", |c, _| {
        c.ghost_element_hover
    }),
    ("--vscode-toolbar-activeBackground", |c, _| {
        c.ghost_element_active
    }),
    ("--vscode-menu-background", |c, _| {
        c.elevated_surface_background
    }),
    ("--vscode-menu-foreground", |c, _| c.text),
    ("--vscode-menu-border", |c, _| c.border),
    ("--vscode-menu-selectionBackground", |c, _| {
        c.element_selected
    }),
    ("--vscode-menu-selectionForeground", |c, _| c.text),
    ("--vscode-menu-selectionBorder", |c, _| c.border_selected),
    ("--vscode-menu-separatorBackground", |c, _| c.border_variant),
    ("--vscode-keybindingTable-rowsBackground", |c, _| {
        c.element_background
    }),
    ("--vscode-scrollbar-shadow", |c, _| c.border_transparent),
    ("--vscode-scrollbarSlider-background", |c, _| {
        c.scrollbar_thumb_background
    }),
    ("--vscode-scrollbarSlider-hoverBackground", |c, _| {
        c.scrollbar_thumb_hover_background
    }),
    ("--vscode-scrollbarSlider-activeBackground", |c, _| {
        c.scrollbar_thumb_active_background
    }),
    ("--vscode-lean4-infoView\\.hypothesisName", |c, _| {
        c.terminal_ansi_yellow
    }),
    (
        "--vscode-lean4-infoView\\.inaccessibleHypothesisName",
        |c, _| c.text_muted,
    ),
    ("--vscode-lean4-infoView\\.goalCount", |c, _| {
        c.terminal_ansi_blue
    }),
    ("--vscode-lean4-infoView\\.turnstile", |c, _| {
        c.terminal_ansi_blue
    }),
    ("--vscode-lean4-infoView\\.caseLabel", |c, _| {
        c.terminal_ansi_green
    }),
];

/// Returns the page's `theme.css`.
pub fn stylesheet(colors: &ThemeColors, status: &StatusColors, fonts: &PageFonts) -> String {
    let mut css = String::from(":root {\n");
    for (name, color_of) in COLOR_VARIABLES {
        writeln!(css, "  {name}: {};", css_color(color_of(colors, status))).ok();
    }
    let ui_family = css_font_family(&fonts.ui_family, "-apple-system, system-ui, sans-serif");
    let buffer_family = css_font_family(&fonts.buffer_family, "ui-monospace, Menlo, monospace");
    for (name, value) in [
        ("--vscode-font-family", ui_family),
        ("--vscode-font-size", format!("{}px", fonts.ui_size)),
        ("--vscode-font-weight", fonts.ui_weight.to_string()),
        ("--vscode-editor-font-family", buffer_family.clone()),
        ("--monaco-monospace-font", buffer_family),
        (
            "--vscode-editor-font-size",
            format!("{}px", fonts.buffer_size),
        ),
        (
            "--vscode-editor-font-weight",
            fonts.buffer_weight.to_string(),
        ),
        (
            "--vscode-editor-line-height",
            format!("{}px", fonts.buffer_line_height),
        ),
        ("--vscode-editor-font-feature-settings", "normal".into()),
    ] {
        writeln!(css, "  {name}: {value};").ok();
    }
    css.push_str("}\n");
    css
}

fn css_color(color: Hsla) -> String {
    let Rgba { r, g, b, a } = color.to_rgb();
    let byte = |channel: f32| (channel.clamp(0., 1.) * 255.).round() as u8;
    format!(
        "#{:02x}{:02x}{:02x}{:02x}",
        byte(r),
        byte(g),
        byte(b),
        byte(a)
    )
}

/// Resolves Zed's font aliases to the families that `base.css` declares, then adds fallbacks.
fn css_font_family(family: &str, fallbacks: &str) -> String {
    match gpui::font_name_with_fallbacks(family, "") {
        "" => fallbacks.to_string(),
        family => format!("\"{}\", {fallbacks}", family.replace('"', "")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::hsla;

    fn fonts() -> PageFonts {
        PageFonts {
            ui_family: ".ZedSans".into(),
            ui_size: 16.,
            ui_weight: 400.,
            buffer_family: "Fira Code".into(),
            buffer_size: 15.,
            buffer_weight: 400.,
            buffer_line_height: 23.25,
        }
    }

    fn declaration<'a>(css: &'a str, name: &str) -> &'a str {
        css.lines()
            .find_map(|line| line.trim().strip_prefix(&format!("{name}: ")))
            .unwrap_or_else(|| panic!("{name} is not declared"))
    }

    #[test]
    fn maps_theme_colors_to_vscode_variables() {
        let mut colors = ThemeColors::dark();
        colors.text = hsla(0., 0., 1., 1.);
        colors.panel_background = hsla(0., 0., 0., 1.);
        colors.terminal_ansi_yellow = hsla(60. / 360., 1., 0.5, 1.);
        let mut status = StatusColors::dark();
        status.error = hsla(0., 1., 0.5, 0.5);

        let css = stylesheet(&colors, &status, &fonts());
        assert_eq!(declaration(&css, "--vscode-foreground"), "#ffffffff;");
        assert_eq!(
            declaration(&css, "--vscode-editor-background"),
            "#000000ff;"
        );
        assert_eq!(declaration(&css, "--vscode-errorForeground"), "#ff000080;");
        assert_eq!(
            declaration(&css, "--vscode-lean4-infoView\\.hypothesisName"),
            "#ffff00ff;"
        );
    }

    #[test]
    fn maps_fonts_to_vscode_variables() {
        let css = stylesheet(&ThemeColors::light(), &StatusColors::light(), &fonts());
        assert_eq!(
            declaration(&css, "--vscode-font-family"),
            "\"IBM Plex Sans\", -apple-system, system-ui, sans-serif;"
        );
        assert_eq!(
            declaration(&css, "--vscode-editor-font-family"),
            "\"Fira Code\", ui-monospace, Menlo, monospace;"
        );
        assert_eq!(declaration(&css, "--vscode-editor-font-size"), "15px;");
        assert_eq!(declaration(&css, "--vscode-editor-line-height"), "23.25px;");

        let system = PageFonts {
            ui_family: ".SystemUIFont".into(),
            ..fonts()
        };
        let css = stylesheet(&ThemeColors::light(), &StatusColors::light(), &system);
        assert_eq!(
            declaration(&css, "--vscode-font-family"),
            "-apple-system, system-ui, sans-serif;"
        );
    }
}
