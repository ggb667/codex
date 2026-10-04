use crossterm::event::KeyCode;
use crossterm::event::KeyEventKind;
use ratatui::buffer::Buffer;
use ratatui::layout::Alignment;
use ratatui::layout::Rect;
use ratatui::style::Stylize;
use ratatui::text::Line;
use ratatui::widgets::Block;
use ratatui::widgets::Clear;
use ratatui::widgets::Paragraph;
use ratatui::widgets::Widget;

use crate::key_hint;
use crate::tui::TuiEvent;

pub(crate) const LAUNCH_GATE_HEIGHT: u16 = 6;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LaunchGateAction {
    Continue,
    Cancel,
    Redraw,
    Ignore,
}

pub(crate) fn action_for_event(event: &TuiEvent) -> LaunchGateAction {
    match event {
        TuiEvent::Key(key)
            if key.kind != KeyEventKind::Release
                && (key_hint::ctrl(KeyCode::Char('c')).is_press(*key)
                    || key_hint::ctrl(KeyCode::Char('d')).is_press(*key)) =>
        {
            LaunchGateAction::Cancel
        }
        TuiEvent::Key(key) if key.kind != KeyEventKind::Release => LaunchGateAction::Continue,
        TuiEvent::Draw | TuiEvent::Resize(_) | TuiEvent::Resume | TuiEvent::FocusGained => {
            LaunchGateAction::Redraw
        }
        TuiEvent::Key(_) | TuiEvent::Paste(_) | TuiEvent::FocusLost => LaunchGateAction::Ignore,
    }
}

pub(crate) fn render(area: Rect, buffer: &mut Buffer) {
    Clear.render(area, buffer);
    let block = Block::bordered().title(" Agent launch paused ".magenta().bold());
    let inner = block.inner(area);
    block.render(area, buffer);
    Paragraph::new(vec![
        Line::from("This worker is paused before agent activity."),
        Line::from("Press any key to activate · Ctrl+C to exit".dim()),
    ])
    .alignment(Alignment::Center)
    .render(inner, buffer);
}

#[cfg(test)]
#[path = "launch_gate_tests.rs"]
mod tests;
