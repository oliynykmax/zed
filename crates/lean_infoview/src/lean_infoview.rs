//! Hosts the Lean 4 infoview, the same React app that VS Code shows, in a dock panel.
//! The panel needs native views, which GPUI composites only on macOS.

#[cfg(target_os = "macos")]
mod assets;
#[cfg(target_os = "macos")]
mod edits;
#[cfg(target_os = "macos")]
mod host;
#[cfg(target_os = "macos")]
mod panel;
#[cfg(target_os = "macos")]
mod protocol;
#[cfg(target_os = "macos")]
mod theme_css;
#[cfg(target_os = "macos")]
mod web_view;

#[cfg(target_os = "macos")]
pub use panel::{LeanInfoviewPanel, ToggleInfoview};

pub fn init(cx: &mut gpui::App) {
    #[cfg(target_os = "macos")]
    panel::init(cx);
    #[cfg(not(target_os = "macos"))]
    let _ = cx;
}
