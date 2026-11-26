//! Hosts the Lean 4 infoview, the same React app that VS Code shows, in a dock panel.
//! The panel needs native views, which GPUI composites only on macOS.

#[cfg(target_os = "macos")]
mod assets;
#[cfg(target_os = "macos")]
mod context_menu;
#[cfg(target_os = "macos")]
mod edits;
#[cfg(target_os = "macos")]
mod host;
#[cfg(target_os = "macos")]
mod panel;
#[cfg(target_os = "macos")]
mod protocol;
#[cfg(target_os = "macos")]
mod settings;
#[cfg(target_os = "macos")]
mod theme_css;
#[cfg(target_os = "macos")]
mod web_view;

#[cfg(target_os = "macos")]
pub use panel::LeanInfoviewPanel;

// Defined on every platform so that the default key bindings resolve.
gpui::actions!(
    lean,
    [
        /// Opens or closes the Lean InfoView panel.
        ToggleInfoview,
        /// Shows or hides all messages of the file in the Lean InfoView, opening it if needed.
        ToggleAllMessages,
        /// Stops or resumes the Lean InfoView following the cursor.
        TogglePaused,
        /// Pins the Lean InfoView's state at the cursor, or unpins it.
        TogglePin,
        /// Inserts the Lean InfoView's goal above the cursor, as a comment.
        CopyToComment,
        /// Shows or hides the expected type in the Lean InfoView.
        ToggleExpectedType,
    ]
);

pub fn init(cx: &mut gpui::App) {
    #[cfg(target_os = "macos")]
    panel::init(cx);
    #[cfg(not(target_os = "macos"))]
    let _ = cx;
}
