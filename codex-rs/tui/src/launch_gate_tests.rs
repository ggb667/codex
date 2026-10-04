use super::*;
use crossterm::event::KeyEvent;
use crossterm::event::KeyModifiers;
use ratatui::buffer::Buffer;

#[test]
fn launch_gate_event_policy_requires_an_intentional_key_press() {
    let key = KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE);
    let release = KeyEvent::new_with_kind(
        KeyCode::Char('x'),
        KeyModifiers::NONE,
        KeyEventKind::Release,
    );
    let cancel = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);

    assert_eq!(
        action_for_event(&TuiEvent::Key(key)),
        LaunchGateAction::Continue
    );
    assert_eq!(
        action_for_event(&TuiEvent::Key(release)),
        LaunchGateAction::Ignore
    );
    assert_eq!(
        action_for_event(&TuiEvent::Key(cancel)),
        LaunchGateAction::Cancel
    );
    assert_eq!(action_for_event(&TuiEvent::Draw), LaunchGateAction::Redraw);
    assert_eq!(
        action_for_event(&TuiEvent::Paste("not a key".to_string())),
        LaunchGateAction::Ignore
    );
}

#[test]
fn paused_worker_screen_snapshot() {
    let area = Rect::new(
        /*x*/ 0,
        /*y*/ 0,
        /*width*/ 56,
        LAUNCH_GATE_HEIGHT,
    );
    let mut buffer = Buffer::empty(area);

    render(area, &mut buffer);

    let rendered = (0..area.height)
        .map(|row| {
            (0..area.width)
                .map(|column| buffer[(column, row)].symbol())
                .collect::<String>()
                .trim_end()
                .to_string()
        })
        .collect::<Vec<_>>()
        .join("\n");
    insta::assert_snapshot!(rendered, @r"
    ┌ Agent launch paused ─────────────────────────────────┐
    │     This worker is paused before agent activity.     │
    │      Press any key to activate · Ctrl+C to exit      │
    │                                                      │
    │                                                      │
    └──────────────────────────────────────────────────────┘
    ");
}
