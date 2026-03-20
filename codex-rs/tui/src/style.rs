use crate::color::blend;
use crate::color::is_light;
use crate::terminal_palette::best_color;
use crate::terminal_palette::default_bg;
use ratatui::style::Color;
use ratatui::style::Style;
use std::sync::OnceLock;
use std::sync::RwLock;

static USER_MESSAGE_BG_OVERRIDE: OnceLock<RwLock<Option<Color>>> = OnceLock::new();

fn user_message_bg_override_lock() -> &'static RwLock<Option<Color>> {
    USER_MESSAGE_BG_OVERRIDE.get_or_init(|| RwLock::new(None))
}

pub fn user_message_style() -> Style {
    user_message_style_for(default_bg())
}

pub fn proposed_plan_style() -> Style {
    proposed_plan_style_for(default_bg())
}

/// Returns the style for a user-authored message using the provided terminal background.
pub fn user_message_style_for(terminal_bg: Option<(u8, u8, u8)>) -> Style {
    if let Some(color) = *user_message_bg_override_lock()
        .read()
        .expect("user message background lock poisoned")
    {
        return Style::default().bg(color);
    }

    match terminal_bg {
        Some(bg) => Style::default().bg(user_message_bg(bg)),
        None => Style::default(),
    }
}

pub fn proposed_plan_style_for(terminal_bg: Option<(u8, u8, u8)>) -> Style {
    match terminal_bg {
        Some(bg) => Style::default().bg(proposed_plan_bg(bg)),
        None => Style::default(),
    }
}

#[allow(clippy::disallowed_methods)]
pub fn user_message_bg(terminal_bg: (u8, u8, u8)) -> Color {
    let (top, alpha) = if is_light(terminal_bg) {
        ((0, 0, 0), 0.04)
    } else {
        ((255, 255, 255), 0.12)
    };
    best_color(blend(top, terminal_bg, alpha))
}

#[allow(clippy::disallowed_methods)]
pub fn proposed_plan_bg(terminal_bg: (u8, u8, u8)) -> Color {
    user_message_bg(terminal_bg)
}

pub(crate) fn set_user_message_bg_override(value: Option<String>) -> Option<String> {
    let override_color = match value {
        Some(value) => match parse_hex_color(&value) {
            Ok(color) => Some(color),
            Err(err) => {
                *user_message_bg_override_lock()
                    .write()
                    .expect("user message background lock poisoned") = None;
                return Some(format!("Ignoring tui.prompt_background={value:?}: {err}"));
            }
        },
        None => None,
    };

    *user_message_bg_override_lock()
        .write()
        .expect("user message background lock poisoned") = override_color;
    None
}

fn parse_hex_color(value: &str) -> Result<Color, &'static str> {
    let hex = value
        .strip_prefix('#')
        .ok_or("expected a #RRGGBB hex color")?;
    if hex.len() != 6 {
        return Err("expected exactly 6 hex digits");
    }

    let rgb = u32::from_str_radix(hex, 16).map_err(|_| "expected only hex digits")?;
    #[allow(clippy::disallowed_methods)]
    Ok(Color::Rgb(
        ((rgb >> 16) & 0xff) as u8,
        ((rgb >> 8) & 0xff) as u8,
        (rgb & 0xff) as u8,
    ))
}

#[cfg(test)]
mod tests {
    use super::parse_hex_color;
    use pretty_assertions::assert_eq;
    use ratatui::style::Color;

    #[test]
    fn parse_hex_color_accepts_hash_prefixed_rgb() {
        assert_eq!(parse_hex_color("#112233"), Ok(Color::Rgb(0x11, 0x22, 0x33)));
    }

    #[test]
    fn parse_hex_color_rejects_invalid_values() {
        assert_eq!(
            parse_hex_color("112233"),
            Err("expected a #RRGGBB hex color")
        );
        assert_eq!(
            parse_hex_color("#12345"),
            Err("expected exactly 6 hex digits")
        );
    }
}
