/// Settings page — rendered in the tuiOS main container.
///
/// Left pane: settings categories with emoji icons.
/// Right pane: selected category's settings content.

use ratatui::{
    layout::{Constraint, Direction, Layout, Margin, Rect},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use super::pages::{
    about, appearance, audio, boot, button_mapping, display, git, hotkeys, power, utilities,
    wifi_bluetooth,
};
use super::state::{SettingsCategory, SettingsState, SubPage};

pub fn render(frame: &mut Frame, area: Rect, border_style: Style, ss: &mut SettingsState) {
    let border_name = {
        let s = super::persistence::load();
        super::persistence::get_str(&s, "appearance.border_style", "Rounded")
    };
    let btype = appearance::border_type_from_name(&border_name);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(btype)
        .title(" Settings ")
        .title_alignment(ratatui::layout::Alignment::Right)
        .style(border_style);
    frame.render_widget(block, area);

    let inner = area.inner(Margin {
        horizontal: 1,
        vertical: 1,
    });

    // Split into left (categories) and right (content) panes
    let panes = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(26), Constraint::Fill(1)])
        .split(inner);

    let left_area = panes[0];
    let right_area = panes[1];

    // --- Left pane: category list ---
    let categories = SettingsCategory::ALL;
    let mut left_lines: Vec<Line> = Vec::new();

    for (i, cat) in categories.iter().enumerate() {
        let is_selected = i == ss.category_cursor;
        let available = cat.is_available();
        let in_left = !ss.in_right_pane;

        let prefix = if is_selected && in_left { " » " } else { "   " };

        let style = if is_selected && in_left {
            Style::default().fg(Color::Black).bg(Color::Cyan)
        } else if is_selected && ss.in_right_pane {
            // Selected but focus is in right pane — highlight but no bg
            Style::default().fg(Color::Cyan)
        } else if !available {
            Style::default().fg(Color::DarkGray)
        } else {
            Style::default().fg(Color::White)
        };

        left_lines.push(Line::from(Span::styled(
            format!("{}{}", prefix, cat.label()),
            style,
        )));
    }

    frame.render_widget(Paragraph::new(left_lines), left_area);

    // --- Vertical separator ---
    // Draw a thin border between panes using the right pane's block
    let right_block = Block::default()
        .borders(Borders::LEFT)
        .style(Style::default().fg(Color::DarkGray));
    frame.render_widget(right_block, right_area);

    let right_inner = right_area.inner(Margin {
        horizontal: 1,
        vertical: 0,
    });

    // --- Right pane: category content ---
    let cat = ss.selected_category();
    // Only show cursor highlight when the right pane is focused
    let cursor = if ss.in_right_pane { ss.right_cursor } else { usize::MAX };
    let scroll = ss.right_scroll;

    // Category title
    let title_area = Rect {
        x: right_inner.x,
        y: right_inner.y,
        width: right_inner.width,
        height: 2.min(right_inner.height),
    };
    let content_area = Rect {
        x: right_inner.x,
        y: right_inner.y + 2,
        width: right_inner.width,
        height: right_inner.height.saturating_sub(2),
    };

    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            match ss.subpage {
                Some(sub) => format!("  {} › {}", cat.label(), sub.label()),
                None => format!("  {}", cat.label()),
            },
            Style::default().fg(Color::White),
        ))),
        title_area,
    );

    // Render the selected category's content
    let editing = ss.editing_setting;
    if let Some(SubPage::BootAnimation) = ss.subpage {
        boot::render(frame, content_area, cursor, scroll, editing);
        return;
    }
    match cat {
        SettingsCategory::About => about::render(frame, content_area, cursor, scroll),
        SettingsCategory::WifiBluetooth => {
            wifi_bluetooth::render(frame, content_area, cursor, scroll)
        }
        SettingsCategory::Audio => audio::render(frame, content_area, cursor, scroll),
        SettingsCategory::Display => display::render(frame, content_area, cursor, scroll, editing),
        SettingsCategory::Appearance => appearance::render(frame, content_area, cursor, scroll, editing),
        SettingsCategory::ButtonMapping => {
            button_mapping::render(frame, content_area, cursor, scroll, ss.awaiting_key)
        }
        SettingsCategory::Hotkeys => hotkeys::render(frame, content_area, cursor, scroll, editing),
        SettingsCategory::Git => git::render(frame, content_area, cursor, scroll, &ss.terminal_output, ss.terminal_focused, ss.terminal_scroll),
        SettingsCategory::Utilities => utilities::render(frame, content_area, cursor, scroll),
        SettingsCategory::Power => power::render(frame, content_area, cursor, scroll),
    }
}

/// Returns true if the current right-pane item supports left/right cycling mode.
pub fn is_edit_mode_item(ss: &SettingsState) -> bool {
    let cursor = ss.right_cursor;
    if let Some(SubPage::BootAnimation) = ss.subpage {
        return boot::is_edit_mode_item(cursor);
    }
    match ss.selected_category() {
        SettingsCategory::Appearance => appearance::is_edit_mode_item(cursor),
        SettingsCategory::Display => display::is_edit_mode_item(cursor),
        SettingsCategory::Hotkeys => hotkeys::is_edit_mode_item(cursor),
        _ => false,
    }
}

/// Cycle the current multi-option setting forward or backward.
pub fn handle_setting_cycle(ss: &mut SettingsState, forward: bool) {
    let cursor = ss.right_cursor;
    if let Some(SubPage::BootAnimation) = ss.subpage {
        boot::handle_cycle(cursor, forward);
        return;
    }
    match ss.selected_category() {
        SettingsCategory::Appearance => appearance::handle_cycle(cursor, forward),
        SettingsCategory::Display => display::handle_cycle(cursor, forward),
        SettingsCategory::Hotkeys => hotkeys::handle_cycle(cursor, forward),
        _ => {}
    }
}

/// Get the item count for the currently selected category.
pub fn current_item_count(ss: &SettingsState) -> usize {
    if let Some(SubPage::BootAnimation) = ss.subpage {
        return boot::item_count();
    }
    match ss.selected_category() {
        SettingsCategory::About => about::item_count(),
        SettingsCategory::WifiBluetooth => wifi_bluetooth::item_count(),
        SettingsCategory::Audio => audio::item_count(),
        SettingsCategory::Display => display::item_count(),
        SettingsCategory::Appearance => appearance::item_count(),
        SettingsCategory::ButtonMapping => button_mapping::item_count(),
        SettingsCategory::Hotkeys => hotkeys::item_count(),
        SettingsCategory::Git => git::item_count(),
        SettingsCategory::Utilities => utilities::item_count(),
        SettingsCategory::Power => power::item_count(),
    }
}

/// Result from a settings Enter action.
pub enum SettingsAction {
    None,
    Quit,
    Restart,
    ShowPopup(String),
}

/// Handle Enter press in the right pane (binary toggles and actions only).
pub fn handle_right_pane_enter(ss: &mut SettingsState) -> SettingsAction {
    let cursor = ss.right_cursor;
    if let Some(SubPage::BootAnimation) = ss.subpage {
        boot::handle_enter(cursor);
        return SettingsAction::None;
    }
    match ss.selected_category() {
        SettingsCategory::Display => { display::handle_enter(cursor); SettingsAction::None }
        SettingsCategory::Appearance => {
            if appearance::is_boot_settings(cursor) {
                ss.subpage = Some(SubPage::BootAnimation);
                ss.right_cursor = 0;
                ss.editing_setting = false;
            } else {
                appearance::handle_enter(cursor);
            }
            SettingsAction::None
        }
        SettingsCategory::ButtonMapping => { button_mapping::handle_enter(ss); SettingsAction::None }
        SettingsCategory::Hotkeys => { hotkeys::handle_enter(cursor); SettingsAction::None }
        SettingsCategory::Git => { git::handle_enter(cursor, ss); SettingsAction::None }
        SettingsCategory::Utilities => {
            if utilities::handle_enter(cursor) {
                SettingsAction::ShowPopup("Logs cleared.".to_string())
            } else {
                SettingsAction::None
            }
        }
        SettingsCategory::Power => power::handle_enter(cursor),
        _ => SettingsAction::None,
    }
}


