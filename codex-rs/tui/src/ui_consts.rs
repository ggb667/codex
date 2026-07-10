//! Shared UI constants for layout and alignment within the TUI.

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

pub(crate) const LIVE_PREFIX_COLS: u16 = DEFAULT_LIVE_PREFIX_COLS as u16;
pub(crate) const FOOTER_INDENT_COLS: usize = DEFAULT_LIVE_PREFIX_COLS;
pub(crate) const TRANSCRIPT_HINT: &str = "ctrl + t to view transcript";

pub(crate) fn prompt_glyph() -> String {
    DEFAULT_PROMPT_GLYPH.to_string()
}

pub(crate) fn prompt_padding() -> String {
    " ".repeat(prompt_glyph_cols())
}

pub(crate) fn footer_indent_cols() -> usize {
    prompt_prefix_cols_for(DEFAULT_PROMPT_GLYPH)
}

fn prompt_glyph_cols() -> usize {
    UnicodeWidthStr::width(DEFAULT_PROMPT_GLYPH).max(1)
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
