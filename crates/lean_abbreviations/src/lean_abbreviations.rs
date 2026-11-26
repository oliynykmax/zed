//! Lean 4 Unicode input, ported from vscode-lean4's `@leanprover/unicode-input`: the
//! abbreviation table, and the rewriter that replaces abbreviations as they are typed.
//! All offsets are byte offsets.

mod rewriter;
mod table;

pub use rewriter::{Change, Config, Replacement, Rewriter, place_cursors};
pub use table::Abbreviations;
