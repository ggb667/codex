use crate::render::renderable::Renderable;
use crate::render::renderable::RowRenderable;
use crate::ui_consts::prompt_glyph;
use crate::ui_consts::prompt_padding;
use ratatui::style::Style;
use ratatui::style::Styled as _;
use ratatui::style::Stylize as _;
use ratatui::widgets::Paragraph;
use ratatui::widgets::Wrap;
use unicode_width::UnicodeWidthStr;

pub(crate) fn selection_option_row(
    index: usize,
    label: String,
    is_selected: bool,
) -> Box<dyn Renderable> {
    selection_option_row_with_dim(index, label, is_selected, /*dim*/ false)
}

pub(crate) fn selection_option_row_with_dim(
    index: usize,
    label: String,
    is_selected: bool,
    dim: bool,
) -> Box<dyn Renderable> {
    let prefix = if is_selected {
        format!("{} {}. ", prompt_glyph(), index + 1)
    } else {
        format!("{} {}. ", prompt_padding(), index + 1)
    };
    let style = if is_selected {
        Style::default().cyan()
    } else if dim {
        Style::default().dim()
    } else {
        Style::default()
    };
    let prefix_width = UnicodeWidthStr::width(prefix.as_str()) as u16;
    let mut row = RowRenderable::new();
    row.push(prefix_width, prefix.set_style(style));
    row.push(
        u16::MAX,
        Paragraph::new(label)
            .style(style)
            .wrap(Wrap { trim: false }),
    );
    row.into()
}
