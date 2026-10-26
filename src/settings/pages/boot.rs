/// Boot animation settings — opened from the Appearance category.

use ratatui::{
    layout::Rect,
    style::{Color, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use serde_json::Value;

use crate::settings::persistence;

use super::cycle_option;

const STYLES: &[&str] = &["Modern", "Retro"];
const COLOURS: &[&str] = &[
    "Default",
    "Rainbow Dynamic",
    "Rainbow Static",
    "Rainbow Vertical",
    "Nebula",
    "Aurora",
    "Warm",
    "Cool",
    "Neon",
    "Synthwave",
    "Sunset",
    "Ocean",
    "Matrix",
    "Ember",
];
const LOGOS: &[&str] = &["Static", "Faded"];
const DIRECTIONS: &[&str] = &["Center", "Up", "Down", "Left", "Right"];
const DURATIONS: &[&str] = &["Short", "Default", "Long"];

/// Height the intro needs before a preview is worth drawing.
const PREVIEW_MIN_ROWS: u16 = 8;

/// Rows in display order. Colour, logo and direction only apply to the modern
/// intro, so they are hidden while Retro is selected.
#[derive(Clone, Copy, PartialEq)]
enum Row {
    Enabled,
    Style,
    Duration,
    Colour,
    Logo,
    Effect,
    Direction,
    Preview,
}

impl Row {
    /// True for rows picked from a fixed list with ◄ ►.
    fn cycles(self) -> bool {
        !matches!(self, Row::Enabled | Row::Effect | Row::Preview)
    }
}

fn rows(settings: &Value) -> Vec<Row> {
    let mut rows = vec![Row::Enabled, Row::Style, Row::Duration];
    if style(settings) == "Modern" {
        rows.extend([Row::Colour, Row::Logo, Row::Effect]);
        // Direction only means something while the reveal effect is on
        if effect(settings) {
            rows.push(Row::Direction);
        }
    }
    rows.push(Row::Preview);
    rows
}

fn row_at(settings: &Value, cursor: usize) -> Option<Row> {
    rows(settings).get(cursor).copied()
}

/// Animation style. Rainbow values used to live on this key, so anything that
/// is not Retro reads as Modern.
pub fn style(settings: &Value) -> String {
    let stored = persistence::get_str(settings, "appearance.boot_animation_style", "Modern");
    if stored.eq_ignore_ascii_case("Retro") { "Retro".into() } else { "Modern".into() }
}

pub fn colour(settings: &Value) -> String {
    let stored = persistence::get_str(settings, "appearance.boot_animation_colour", "");
    if COLOURS.contains(&stored.as_str()) {
        return stored;
    }
    let legacy = persistence::get_str(settings, "appearance.boot_animation_style", "");
    if COLOURS.contains(&legacy.as_str()) { legacy } else { "Default".into() }
}

pub fn logo(settings: &Value) -> String {
    let stored = persistence::get_str(settings, "appearance.boot_logo", "Static");
    if LOGOS.contains(&stored.as_str()) { stored } else { "Static".into() }
}

pub fn direction(settings: &Value) -> String {
    let stored = persistence::get_str(settings, "appearance.boot_reveal", "Center");
    if DIRECTIONS.contains(&stored.as_str()) { stored } else { "Center".into() }
}

pub fn duration(settings: &Value) -> String {
    let stored = persistence::get_str(settings, "appearance.boot_duration", "Default");
    if DURATIONS.contains(&stored.as_str()) { stored } else { "Default".into() }
}

pub fn effect(settings: &Value) -> bool {
    persistence::get_bool(settings, "appearance.boot_splash_effect", true)
}

fn preview_enabled(settings: &Value) -> bool {
    persistence::get_bool(settings, "appearance.boot_preview", true)
}

fn label_value(row: Row, s: &Value) -> (&'static str, String) {
    match row {
        Row::Enabled => (
            "Intro Animation",
            if persistence::get_bool(s, "appearance.intro_animation_enabled", true) {
                "Enabled".into()
            } else {
                "Disabled".into()
            },
        ),
        Row::Style => ("Animation Style", style(s)),
        Row::Duration => ("Duration", duration(s)),
        Row::Colour => ("Colour", colour(s)),
        Row::Logo => ("Logo", logo(s)),
        Row::Effect => (
            "Splash Effect",
            if effect(s) { "Enabled".into() } else { "Disabled".into() },
        ),
        Row::Direction => ("Direction", direction(s)),
        Row::Preview => (
            "Preview",
            if preview_enabled(s) { "Enabled".into() } else { "Disabled".into() },
        ),
    }
}

pub fn render(frame: &mut Frame, area: Rect, cursor: usize, _scroll: usize, editing: bool) {
    let settings = persistence::load();
    let rows = rows(&settings);

    let value_style = Style::default().fg(Color::White);
    let mut lines: Vec<Line> = Vec::new();
    for (i, row) in rows.iter().enumerate() {
        let (label, value) = label_value(*row, &settings);
        let is_active = i == cursor;
        let prefix = if is_active { "  » " } else { "    " };
        let style = if is_active {
            Style::default().fg(Color::Black).bg(Color::Cyan)
        } else {
            value_style
        };
        if is_active && editing && row.cycles() {
            lines.push(Line::from(vec![
                Span::styled(format!("{}{:<22}", prefix, label), style),
                Span::styled("  ◄ ", Style::default().fg(Color::Yellow)),
                Span::styled(value, Style::default().fg(Color::White)),
                Span::styled(" ►", Style::default().fg(Color::Yellow)),
            ]));
        } else {
            lines.push(Line::from(vec![
                Span::styled(format!("{}{:<22}", prefix, label), style),
                Span::styled(format!("  {}", value), style),
            ]));
        }
    }

    let list_h = (lines.len() as u16).min(area.height);
    frame.render_widget(
        Paragraph::new(lines),
        Rect { height: list_h, ..area },
    );

    let mut preview = Rect {
        y: area.y + list_h,
        height: area.height.saturating_sub(list_h),
        ..area
    };
    if !preview_enabled(&settings) || preview.height < PREVIEW_MIN_ROWS {
        return;
    }

    // The heading is the first thing to go when the box is tight
    if preview.height >= PREVIEW_MIN_ROWS + 2 {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "    Preview",
                Style::default().fg(Color::DarkGray),
            ))),
            Rect { y: preview.y + 1, height: 1, ..preview },
        );
        preview = Rect {
            y: preview.y + 2,
            height: preview.height - 2,
            ..preview
        };
    }

    let accent = super::appearance::color_from_name(&persistence::get_str(
        &settings,
        "appearance.accent_color",
        "Cyan",
    ));
    crate::tuios::boot_animation::draw_preview(frame, preview, accent, options(&settings));
}

/// The animation options as currently stored, for the preview.
fn options(settings: &Value) -> crate::tuios::boot_animation::Options<'static> {
    // Match the stored value back to its entry in the option list so the
    // preview can borrow a 'static name
    fn fixed(value: String, options: &'static [&'static str], fallback: &'static str) -> &'static str {
        options
            .iter()
            .find(|o| **o == value)
            .copied()
            .unwrap_or(fallback)
    }
    crate::tuios::boot_animation::Options {
        style: fixed(style(settings), STYLES, "Modern"),
        colour: fixed(colour(settings), COLOURS, "Default"),
        logo: fixed(logo(settings), LOGOS, "Static"),
        reveal: fixed(direction(settings), DIRECTIONS, "Center"),
        duration: fixed(duration(settings), DURATIONS, "Default"),
        effect: effect(settings),
    }
}

/// Returns true for items that use left/right cycling.
pub fn is_edit_mode_item(cursor: usize) -> bool {
    row_at(&persistence::load(), cursor).map_or(false, |row| row.cycles())
}

pub fn item_count() -> usize {
    rows(&persistence::load()).len()
}

/// Cycle a multi-option item forward (+1) or backward (-1).
pub fn handle_cycle(cursor: usize, forward: bool) {
    let mut settings = persistence::load();
    let row = match row_at(&settings, cursor) {
        Some(row) => row,
        None => return,
    };
    match row {
        Row::Style => {
            let current = style(&settings);
            let next = cycle_option(&mut settings, "appearance.boot_animation_style", STYLES, &current, forward);
            crate::utilities::logging::settings(&format!("Boot animation set to {}", next));
        }
        Row::Colour => {
            let current = colour(&settings);
            let next = cycle_option(&mut settings, "appearance.boot_animation_colour", COLOURS, &current, forward);
            crate::utilities::logging::settings(&format!("Boot colour set to {}", next));
        }
        Row::Logo => {
            let current = logo(&settings);
            let next = cycle_option(&mut settings, "appearance.boot_logo", LOGOS, &current, forward);
            crate::utilities::logging::settings(&format!("Boot logo set to {}", next));
        }
        Row::Direction => {
            let current = direction(&settings);
            let next = cycle_option(&mut settings, "appearance.boot_reveal", DIRECTIONS, &current, forward);
            crate::utilities::logging::settings(&format!("Boot direction set to {}", next));
        }
        Row::Duration => {
            let current = duration(&settings);
            let next = cycle_option(&mut settings, "appearance.boot_duration", DURATIONS, &current, forward);
            crate::utilities::logging::settings(&format!("Boot duration set to {}", next));
        }
        Row::Enabled | Row::Effect | Row::Preview => {}
    }
    persistence::save(&settings);
}

pub fn handle_enter(cursor: usize) {
    let mut settings = persistence::load();
    match row_at(&settings, cursor) {
        Some(Row::Enabled) => {
            let current = persistence::get_bool(&settings, "appearance.intro_animation_enabled", true);
            persistence::set(
                &mut settings,
                "appearance.intro_animation_enabled",
                serde_json::Value::Bool(!current),
            );
            crate::utilities::logging::settings(&format!(
                "Intro animation {}",
                if !current { "enabled" } else { "disabled" }
            ));
        }
        Some(Row::Effect) => {
            let current = effect(&settings);
            persistence::set(
                &mut settings,
                "appearance.boot_splash_effect",
                serde_json::Value::Bool(!current),
            );
            crate::utilities::logging::settings(&format!(
                "Boot splash effect {}",
                if !current { "enabled" } else { "disabled" }
            ));
        }
        Some(Row::Preview) => {
            let current = preview_enabled(&settings);
            persistence::set(
                &mut settings,
                "appearance.boot_preview",
                serde_json::Value::Bool(!current),
            );
            crate::utilities::logging::settings(&format!(
                "Boot preview {}",
                if !current { "enabled" } else { "disabled" }
            ));
        }
        _ => return,
    }
    persistence::save(&settings);
}
