//! Shared UI constants for layout and alignment within the TUI.

use std::sync::OnceLock;
use std::sync::RwLock;

/// Width (in terminal columns) reserved for the left gutter/prefix used by
/// live cells and aligned widgets.
///
/// Semantics:
/// - Chat composer reserves this many columns for the left border + padding.
/// - Status indicator lines begin with this many spaces for alignment.
/// - User history lines account for this many columns (e.g., "▌ ") when wrapping.
pub(crate) const LIVE_PREFIX_COLS: u16 = 2;
pub(crate) const FOOTER_INDENT_COLS: usize = LIVE_PREFIX_COLS as usize;
const DEFAULT_PROMPT_GLYPH: &str = "›";

static PROMPT_GLYPH: OnceLock<RwLock<String>> = OnceLock::new();

fn prompt_glyph_lock() -> &'static RwLock<String> {
    PROMPT_GLYPH.get_or_init(|| RwLock::new(DEFAULT_PROMPT_GLYPH.to_string()))
}

pub(crate) fn set_prompt_glyph(glyph: Option<String>) {
    let glyph = glyph
        .filter(|glyph| !glyph.is_empty())
        .unwrap_or_else(|| DEFAULT_PROMPT_GLYPH.to_string());
    *prompt_glyph_lock()
        .write()
        .expect("prompt glyph lock poisoned") = glyph;
}

pub(crate) fn prompt_glyph() -> String {
    prompt_glyph_lock()
        .read()
        .expect("prompt glyph lock poisoned")
        .clone()
}

pub(crate) fn prompt_glyph_with_space() -> String {
    format!("{} ", prompt_glyph())
}
