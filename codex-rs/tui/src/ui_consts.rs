//! Shared UI constants for layout and alignment within the TUI.

use std::sync::OnceLock;
use std::sync::RwLock;
use unicode_width::UnicodeWidthStr;

/// Width (in terminal columns) reserved for the left gutter/prefix used by
/// live cells and aligned widgets.
///
/// Semantics:
/// - Chat composer reserves this many columns for the left border + padding.
/// - Status indicator lines begin with this many spaces for alignment.
/// - User history lines account for this many columns (e.g., "▌ ") when wrapping.
const DEFAULT_LIVE_PREFIX_COLS: usize = 2;
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

pub(crate) fn prompt_padding() -> String {
    " ".repeat(prompt_glyph_cols())
}

pub(crate) fn prompt_glyph_with_space() -> String {
    format!("{} ", prompt_glyph())
}

pub(crate) fn live_prefix_cols() -> u16 {
    prompt_prefix_cols().try_into().unwrap_or(u16::MAX)
}

pub(crate) fn footer_indent_cols() -> usize {
    prompt_prefix_cols()
}

pub(crate) fn live_prefix_spaces() -> String {
    " ".repeat(prompt_prefix_cols())
}

fn prompt_glyph_cols() -> usize {
    UnicodeWidthStr::width(prompt_glyph().as_str()).max(1)
}

fn prompt_prefix_cols() -> usize {
    prompt_prefix_cols_for(prompt_glyph().as_str())
}

fn prompt_prefix_cols_for(glyph: &str) -> usize {
    UnicodeWidthStr::width(format!("{glyph} ").as_str()).max(DEFAULT_LIVE_PREFIX_COLS)
}

#[cfg(test)]
mod tests {
    use super::DEFAULT_PROMPT_GLYPH;
    use super::prompt_prefix_cols_for;
    use pretty_assertions::assert_eq;

    #[test]
    fn prompt_prefix_cols_reserves_default_width_for_single_column_glyphs() {
        assert_eq!(prompt_prefix_cols_for(DEFAULT_PROMPT_GLYPH), 2);
        assert_eq!(prompt_prefix_cols_for(">"), 2);
    }

    #[test]
    fn prompt_prefix_cols_expands_for_wide_glyphs() {
        assert_eq!(prompt_prefix_cols_for("🎈"), 3);
    }
}
