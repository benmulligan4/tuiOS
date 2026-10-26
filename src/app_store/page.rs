/// App Store page — rendered in the tuiOS main container.
///
/// Left pane: search bar, sort/filter controls, scrollable app list.
/// Right pane: app preview with metadata and install/uninstall actions.
/// Bottom: terminal panel during operations.

use std::collections::HashMap;

use ratatui::{
    layout::{Constraint, Direction, Layout, Margin, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, Paragraph},
    Frame,
};
use serde_json::Value;

use super::queue::QueueStatus;
use super::state::{AppStoreFocus, AppStoreState, ConfirmAction, InstallStatus};

/// Width of the install queue column.
const QUEUE_WIDTH: u16 = 46;

/// Bordered wrapper that makes it obvious which pane the user is navigating in.
fn pane_block<'a>(title: &'a str, focused: bool, btype: BorderType, accent: Color) -> Block<'a> {
    let border_style = if focused {
        Style::default().fg(accent).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::DarkGray)
    };
    let title_style = if focused {
        Style::default().fg(accent).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::DarkGray)
    };
    Block::default()
        .borders(Borders::ALL)
        .border_type(btype)
        .border_style(border_style)
        .title(title)
        .title_style(title_style)
}

/// Add the page name to the right-hand pane, since there is no outer container
/// border to carry it.
fn page_label(block: Block<'_>) -> Block<'_> {
    block.title_top(
        Line::from(Span::styled(
            " App Store ",
            Style::default().fg(Color::DarkGray),
        ))
        .right_aligned(),
    )
}

pub fn render(
    frame: &mut Frame,
    area: Rect,
    page_focused: bool,
    state: &AppStoreState,
    registered: &HashMap<String, Value>,
) {
    let settings = crate::settings::persistence::load();
    let border_name =
        crate::settings::persistence::get_str(&settings, "appearance.border_style", "Rounded");
    let accent_name =
        crate::settings::persistence::get_str(&settings, "appearance.accent_color", "Cyan");
    let btype = crate::settings::pages::appearance::border_type_from_name(&border_name);
    let accent = crate::settings::pages::appearance::color_from_name(&accent_name);

    // No outer container border here — the pane borders are the only frame, so the
    // App Store does not end up with two nested boxes.
    let panes = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(34), Constraint::Fill(1)])
        .split(area);

    let left_area = panes[0];
    let full_right_area = panes[1];

    let left_focused = page_focused
        && !state.browser.active
        && matches!(
            state.focus,
            AppStoreFocus::LeftPane
                | AppStoreFocus::SearchBar
                | AppStoreFocus::FilterPanel
                | AppStoreFocus::BrowserButton
        );
    let right_focused =
        page_focused && !state.browser.active && state.focus == AppStoreFocus::RightPane;

    let queued = state.queue.active_count();
    let apps_title = if queued > 0 {
        format!(" Apps  ⏳{} ", queued)
    } else {
        " Apps ".to_string()
    };
    frame.render_widget(pane_block(&apps_title, left_focused, btype, accent), left_area);
    render_left_pane(
        frame,
        left_area.inner(Margin { horizontal: 1, vertical: 1 }),
        state,
        registered,
    );

    // If browser is active, render it in the right pane
    if state.browser.active {
        frame.render_widget(
            page_label(pane_block(" Awesome Ratatui ", page_focused, btype, accent)),
            full_right_area,
        );
        super::awesome_ratatui_page::render_browser(
            frame,
            full_right_area.inner(Margin { horizontal: 1, vertical: 1 }),
            &state.browser,
            registered,
            &state.install_statuses,
        );
    } else if state.queue_visible() {
        let right_sections = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Percentage(55), Constraint::Fill(1)])
            .split(full_right_area);
        frame.render_widget(
            page_label(pane_block(" Details ", right_focused, btype, accent)),
            right_sections[0],
        );
        render_right_pane(frame, right_sections[0], state, registered);

        // Terminal on the left of the bottom row, queue on the right
        let bottom_area = right_sections[1];
        if bottom_area.width >= 64 {
            let bottom = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Fill(1), Constraint::Length(QUEUE_WIDTH)])
                .split(bottom_area);
            render_terminal_panel(frame, bottom[0], state, btype, accent, page_focused);
            render_queue_panel(frame, bottom[1], state, btype, accent, page_focused);
        } else {
            // Too narrow to show both — whichever pane has focus wins
            if state.focus == AppStoreFocus::Queue {
                render_queue_panel(frame, bottom_area, state, btype, accent, page_focused);
            } else {
                render_terminal_panel(frame, bottom_area, state, btype, accent, page_focused);
            }
        }
    } else {
        frame.render_widget(
            page_label(pane_block(" Details ", right_focused, btype, accent)),
            full_right_area,
        );
        render_right_pane(frame, full_right_area, state, registered);
    }

    // Render overlay dialogs
    if state.confirm_dialog.is_some() {
        render_confirm_dialog(frame, area, state);
    }
    if state.install_location_dialog {
        render_install_location_dialog(frame, area, state);
    }
}

fn render_left_pane(
    frame: &mut Frame,
    area: Rect,
    state: &AppStoreState,
    registered: &HashMap<String, Value>,
) {
    let in_left = matches!(
        state.focus,
        AppStoreFocus::LeftPane | AppStoreFocus::SearchBar | AppStoreFocus::FilterPanel | AppStoreFocus::BrowserButton
    );

    let mut lines: Vec<Line> = Vec::new();

    // Search bar
    let search_style = if state.focus == AppStoreFocus::SearchBar {
        Style::default().fg(Color::Black).bg(Color::Cyan)
    } else if in_left {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    };
    let search_text = if state.focus == AppStoreFocus::SearchBar {
        if state.search_query.is_empty() {
            " [/] Type to search...".to_string()
        } else {
            format!(" [/] {}|", state.search_query)
        }
    } else if state.search_query.is_empty() {
        " [/] Search".to_string()
    } else {
        format!(" [/] {}", state.search_query)
    };
    lines.push(Line::from(Span::styled(search_text, search_style)));

    // Awesome Ratatui App Browser button
    let browser_active = state.browser.active;
    let browser_btn_focused = state.focus == AppStoreFocus::BrowserButton;
    let browser_style = if browser_btn_focused {
        Style::default().fg(Color::Black).bg(Color::Cyan)
    } else if browser_active {
        Style::default().fg(Color::Black).bg(Color::Green)
    } else if in_left {
        Style::default().fg(Color::Green)
    } else {
        Style::default().fg(Color::DarkGray)
    };
    lines.push(Line::from(Span::styled(
        " ✚ Browse Awesome Ratatui",
        browser_style,
    )));

    // Filter panel toggle
    let panel_focused = state.focus == AppStoreFocus::FilterPanel;
    let arrow = if state.filter_panel_open { "▼" } else { "▶" };
    let panel_style = if panel_focused && state.filter_panel_cursor == 0 {
        Style::default().fg(Color::Black).bg(Color::Cyan)
    } else if in_left {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    };
    lines.push(Line::from(Span::styled(
        format!(" {} Filters & Sort", arrow),
        panel_style,
    )));

    if state.filter_panel_open {
        let categories = AppStoreState::all_categories(registered);

        let item_style = |idx: usize| -> Style {
            if panel_focused && state.filter_panel_cursor == idx {
                Style::default().fg(Color::Black).bg(Color::Cyan)
            } else {
                Style::default().fg(Color::White)
            }
        };
        let label_style = Style::default().fg(Color::DarkGray);

        // Sort mode (cursor 1)
        let sort_arrow = if state.sort_dropdown_open { "▲" } else { "▼" };
        lines.push(Line::from(Span::styled(
            format!("   Sort: {} {}", state.sort_mode.label(), sort_arrow),
            item_style(1),
        )));

        // Sort dropdown (inline, shown when open)
        if state.sort_dropdown_open {
            for (si, mode) in crate::app_store::state::SortMode::ALL.iter().enumerate() {
                let prefix = if si == state.sort_dropdown_cursor { "    » " } else { "      " };
                let s = if si == state.sort_dropdown_cursor {
                    Style::default().fg(Color::Black).bg(Color::Cyan)
                } else {
                    Style::default().fg(Color::DarkGray)
                };
                lines.push(Line::from(Span::styled(
                    format!("{}{}", prefix, mode.label()),
                    s,
                )));
            }
        }

        // Organise by category (cursor 2)
        let g_check = if state.group_by_category { "[*]" } else { "[ ]" };
        lines.push(Line::from(Span::styled(
            format!("   {}  Organise by Category", g_check),
            item_style(2),
        )));

        // Show Installed (cursor 3)
        let i_check = if state.filter_show_installed { "[*]" } else { "[ ]" };
        lines.push(Line::from(Span::styled(
            format!("   {}  Show Installed", i_check),
            item_style(3),
        )));

        // Show Uninstalled (cursor 4)
        let u_check = if state.filter_show_uninstalled { "[*]" } else { "[ ]" };
        lines.push(Line::from(Span::styled(
            format!("   {}  Show Uninstalled", u_check),
            item_style(4),
        )));

        // Category filters (cursor 5..5+N-1)
        if !categories.is_empty() {
            lines.push(Line::from(Span::styled("   Categories:", label_style)));
        }
        for (ci, cat) in categories.iter().enumerate() {
            let is_shown = !state.filter_categories.contains(cat);
            let check = if is_shown { "[*]" } else { "[ ]" };
            lines.push(Line::from(Span::styled(
                format!("    {}  {}", check, cat),
                item_style(AppStoreState::CATEGORY_FILTER_BASE + ci),
            )));
        }
    }

    // Refresh button (always visible, outside the panel)
    let refresh_idx = state.refresh_cursor_idx(registered);
    let refresh_style = if panel_focused && state.filter_panel_cursor == refresh_idx {
        Style::default().fg(Color::Black).bg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    };
    lines.push(Line::from(Span::styled(
        " ↻ Refresh installed apps",
        refresh_style,
    )));

    lines.push(Line::from(Span::styled(
        " ─────────────────────────────",
        Style::default().fg(Color::DarkGray),
    )));

    // App list
    let available_rows = area.height as usize;

    use super::state::LeftRowKind;
    for (i, row) in state.left_visible_rows.iter().enumerate() {
        if lines.len() >= available_rows {
            break;
        }

        let is_selected = i == state.left_cursor;
        let in_app_list = state.focus == AppStoreFocus::LeftPane;

        match row {
            LeftRowKind::Spacer => {
                lines.push(Line::from(""));
            }
            LeftRowKind::CategoryHeader(cat) => {
                let collapsed = state.collapsed_store_categories.contains(cat);
                let arrow = if collapsed { "▶" } else { "▼" };
                let cat_style = if is_selected && in_app_list {
                    Style::default().fg(Color::Black).bg(Color::Yellow)
                } else {
                    Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
                };
                lines.push(Line::from(Span::styled(
                    format!(" {} {}", arrow, cat),
                    cat_style,
                )));
            }
            LeftRowKind::App(key) => {
                let meta = match registered.get(key.as_str()) {
                    Some(m) => m,
                    None => continue,
                };
                let label = meta.get("label").and_then(|v| v.as_str()).unwrap_or(key);

                let queued = state.queue.has_active_for(key);
                let prefix = if is_selected && in_app_list { " » " } else { "   " };
                // ⏳ is double-width, so it replaces the glyph and its trailing space
                let text = if queued {
                    format!("{}⏳{}", prefix, label)
                } else {
                    let indicator = match state.install_statuses.get(key.as_str()) {
                        Some(InstallStatus::NotInstalled) | None => " ",
                        _ => "✓",
                    };
                    format!("{}{} {}", prefix, indicator, label)
                };

                let is_installed = matches!(
                    state.install_statuses.get(key.as_str()),
                    Some(s) if !matches!(s, InstallStatus::NotInstalled)
                );
                let is_failed = state.failed_installs.contains(key);

                let style = if is_selected && in_app_list {
                    Style::default().fg(Color::Black).bg(Color::Cyan)
                } else if queued {
                    Style::default().fg(Color::Yellow)
                } else if is_failed {
                    Style::default().fg(Color::Red)
                } else if is_installed {
                    Style::default().fg(Color::Green)
                } else if is_selected {
                    Style::default().fg(Color::Cyan)
                } else {
                    Style::default().fg(Color::White)
                };

                lines.push(Line::from(Span::styled(text, style)));
            }
        }
    }

    if state.left_visible_rows.is_empty() && lines.len() < available_rows {
        lines.push(Line::from(Span::styled(
            "   No apps match filters",
            Style::default().fg(Color::DarkGray),
        )));
    }

    frame.render_widget(Paragraph::new(lines), area);
}

fn render_right_pane(
    frame: &mut Frame,
    area: Rect,
    state: &AppStoreState,
    registered: &HashMap<String, Value>,
) {
    let right_inner = area.inner(Margin { horizontal: 2, vertical: 1 });

    let selected_key = match state.selected_app_key() {
        Some(k) => k.clone(),
        None => {
            frame.render_widget(
                Paragraph::new(Line::from(Span::styled(
                    "\n  Please select an app to preview details",
                    Style::default().fg(Color::DarkGray),
                ))),
                right_inner,
            );
            return;
        }
    };

    let meta = match registered.get(&selected_key) {
        Some(m) => m,
        None => return,
    };

    let label = meta.get("label").and_then(|v| v.as_str()).unwrap_or(&selected_key);
    let category = meta.get("category").and_then(|v| v.as_str()).unwrap_or("Other");
    let author = meta.get("author").and_then(|v| v.as_str()).unwrap_or("Unknown");
    let version = meta.get("version").and_then(|v| v.as_str()).unwrap_or("—");
    let license = meta.get("license").and_then(|v| v.as_str()).unwrap_or("—");
    let description = meta.get("description").and_then(|v| v.as_str()).unwrap_or("");
    let repository = meta.get("repository").and_then(|v| v.as_str()).unwrap_or("");

    let install_status = state
        .install_statuses
        .get(&selected_key)
        .cloned()
        .unwrap_or(InstallStatus::NotInstalled);

    let label_style = Style::default().fg(Color::DarkGray);
    let value_style = Style::default().fg(Color::White);
    let title_style = Style::default().fg(Color::White).add_modifier(Modifier::BOLD);
    let accent_style = Style::default().fg(Color::Cyan);

    let is_approved = meta.get("approved").and_then(|v| v.as_bool()).unwrap_or(true);

    let mut lines: Vec<Line> = Vec::new();

    // Title
    lines.push(Line::from(Span::styled(label, title_style)));

    // Approved badge or experimental warning
    if is_approved {
        lines.push(Line::from(Span::styled(
            "  ✓ tuiOS Approved",
            Style::default().fg(Color::Green),
        )));
    } else {
        lines.push(Line::from(Span::styled(
            "  ⚠ Experimental — added from Awesome Ratatui",
            Style::default().fg(Color::Yellow),
        )));
    }
    lines.push(Line::from(""));

    // Metadata grid
    lines.push(Line::from(vec![
        Span::styled("  Category:    ", label_style),
        Span::styled(category, value_style),
    ]));
    lines.push(Line::from(vec![
        Span::styled("  Author:      ", label_style),
        Span::styled(author, value_style),
    ]));
    lines.push(Line::from(vec![
        Span::styled("  Version:     ", label_style),
        Span::styled(version, value_style),
    ]));
    lines.push(Line::from(vec![
        Span::styled("  License:     ", label_style),
        Span::styled(license, value_style),
    ]));
    lines.push(Line::from(vec![
        Span::styled("  Repository:  ", label_style),
        Span::styled(repository, accent_style),
    ]));
    lines.push(Line::from(""));

    // Description
    lines.push(Line::from(Span::styled("  Description:", label_style)));
    // Word-wrap description
    let max_width = right_inner.width.saturating_sub(4) as usize;
    for desc_line in wrap_text(description, max_width) {
        lines.push(Line::from(Span::styled(
            format!("  {}", desc_line),
            value_style,
        )));
    }
    lines.push(Line::from(""));

    // Install status
    let via_git = super::actions::installed_via_git(&selected_key, category);
    let path_label = if via_git { "PATH via Git" } else { "PATH" };
    let (status_text, status_color) = match &install_status {
        InstallStatus::NotInstalled => ("Not installed".to_string(), Color::DarkGray),
        InstallStatus::Global(path) => (format!("Installed to {}: {}", path_label, path), Color::Green),
        InstallStatus::Local(path) => (format!("Installed to Downloads: {}", path), Color::Green),
        InstallStatus::Both(_, _) => ("Installed to Both".to_string(), Color::Green),
    };

    lines.push(Line::from(vec![
        Span::styled("  Status:      ", label_style),
        Span::styled(&status_text, Style::default().fg(status_color)),
    ]));

    if via_git {
        lines.push(Line::from(Span::styled(
            "               (cargo install --git)",
            Style::default().fg(Color::DarkGray),
        )));
    }

    if let InstallStatus::Both(g, l) = &install_status {
        lines.push(Line::from(Span::styled(
            format!("    {:<10} {}", format!("{}:", path_label), g),
            Style::default().fg(Color::Green),
        )));
        lines.push(Line::from(Span::styled(
            format!("    Downloads: {}", l),
            Style::default().fg(Color::Green),
        )));
    }

    lines.push(Line::from(""));

    // Action buttons
    lines.push(Line::from(Span::styled("  Actions:", label_style)));
    lines.push(Line::from(""));

    let in_actions = state.focus == AppStoreFocus::RightPane && state.in_right_actions;
    let mut action_idx: usize = 0;
    // Line index of every action button, so key handling can scroll it into view
    let mut action_lines: Vec<usize> = Vec::new();

    macro_rules! push_action {
        ($text:expr, $base:expr) => {{
            action_lines.push(lines.len());
            let style = if in_actions && state.right_action_cursor == action_idx {
                Style::default().fg(Color::Black).bg(Color::Cyan)
            } else {
                $base
            };
            lines.push(Line::from(Span::styled($text, style)));
            action_idx += 1;
        }};
    }

    let supports_local = super::actions::supports_downloads_install(meta);
    let supports_global = super::actions::supports_path_install(meta);

    // Run button (only for installed apps)
    let is_installed = !matches!(install_status, InstallStatus::NotInstalled);
    if is_installed {
        let locked = state.ui_locked();
        let run_text = if locked {
            "  [ Run ]  ⚠  UI locked while the queue is running"
        } else {
            "  [ Run ]"
        };
        let base = if locked {
            Style::default().fg(Color::DarkGray)
        } else {
            Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)
        };
        push_action!(run_text, base);
    }

    // Install / Uninstall buttons based on status
    match &install_status {
        InstallStatus::NotInstalled => {
            push_action!("  [ Install ]", Style::default().fg(Color::Green));
        }
        InstallStatus::Global(_) => {
            if supports_local {
                push_action!("  [ Install to Downloads ]", Style::default().fg(Color::Green));
            } else {
                lines.push(Line::from(Span::styled(
                    "    (local install not supported for this app)",
                    Style::default().fg(Color::DarkGray),
                )));
            }

            push_action!("  [ Uninstall from PATH ]", Style::default().fg(Color::Red));
        }
        InstallStatus::Local(_) => {
            if supports_global {
                push_action!("  [ Install to PATH ]", Style::default().fg(Color::Green));
            } else {
                lines.push(Line::from(Span::styled(
                    "    (PATH install not supported for this app)",
                    Style::default().fg(Color::DarkGray),
                )));
            }

            push_action!("  [ Uninstall from Downloads ]", Style::default().fg(Color::Red));
        }
        InstallStatus::Both(_, _) => {
            push_action!("  [ Uninstall ]", Style::default().fg(Color::Red));

            // Run source selector
            let current_source = super::actions::get_run_source(&selected_key);
            let source_label = if current_source == "local" { "Downloads" } else { "PATH" };
            let source_text = format!("  [ Default Source: {} — press to switch ]", source_label);
            push_action!(source_text, Style::default().fg(Color::Yellow));
            if current_source == "local" {
                lines.push(Line::from(Span::styled(
                    "    ⚠  Downloads source is experimental",
                    Style::default().fg(Color::Yellow),
                )));
            }
        }
    }

    lines.push(Line::from(""));

    push_action!("  [ Open Repository ]", accent_style);

    match &install_status {
        InstallStatus::NotInstalled => {}
        InstallStatus::Global(_) | InstallStatus::Local(_) => {
            push_action!("  [ Open Install Location ]", accent_style);
        }
        InstallStatus::Both(_, _) => {
            push_action!("  [ Open PATH Location ]", accent_style);
            push_action!("  [ Open Downloads Location ]", accent_style);
        }
    }

    // Window mode setting (for installed apps)
    if is_installed {
        let supports_embed = super::actions::supports_embedded(registered.get(&selected_key));
        lines.push(Line::from(""));

        if !supports_embed {
            lines.push(Line::from(Span::styled(
                "  ⚠  This app only supports running in a new window",
                Style::default().fg(Color::Yellow),
            )));
            lines.push(Line::from(Span::styled(
                "    (running inside tuiOS is not supported yet)",
                Style::default().fg(Color::DarkGray),
            )));
        } else {
            let current_mode = super::actions::get_window_mode(&selected_key, registered.get(&selected_key));
            let mode_label = super::actions::window_mode_label(&current_mode);
            let mode_text = format!("  [ Window Mode: {} — press to switch ]", mode_label);
            push_action!(mode_text, Style::default().fg(Color::Yellow));
        }
    }

    // Remove from App Store (only for non-approved, not-installed apps)
    if !is_approved && !is_installed {
        lines.push(Line::from(""));
        push_action!("  [ Remove from App Store ]", Style::default().fg(Color::Red));
    }
    let _ = action_idx;

    // Record geometry so key handling can scroll the focused action into view
    let visible_height = (right_inner.height as usize).max(1);
    let total_lines = lines.len();
    state.right_view_height.set(visible_height);
    state.right_total_lines.set(total_lines);
    *state.right_action_lines.borrow_mut() = action_lines;

    let max_scroll = total_lines.saturating_sub(visible_height);
    let scroll = state.right_scroll.min(max_scroll);
    let end = (scroll + visible_height).min(total_lines);
    let visible_lines: Vec<Line> = if total_lines > visible_height {
        lines[scroll..end].to_vec()
    } else {
        lines
    };

    frame.render_widget(Paragraph::new(visible_lines), right_inner);
}

fn render_terminal_panel(
    frame: &mut Frame,
    area: Rect,
    state: &AppStoreState,
    btype: BorderType,
    accent: Color,
    page_focused: bool,
) {
    let focused = state.focus == AppStoreFocus::Terminal;
    let pinned = state.viewing_item.is_some();

    // The job description lives in the border title rather than a separate banner
    let mut title = match state.queue.running() {
        Some(item) if !pinned => {
            let pos = state
                .queue
                .running_position()
                .map(|(i, n)| format!("  ({} of {})", i, n))
                .unwrap_or_default();
            format!(
                " ▸ {}ing {} {} {}{} ",
                item.op.verb(),
                item.label,
                item.op.arrow(),
                item.op.target_label(),
                pos
            )
        }
        _ => match state.viewed_item() {
            Some(item) => format!(
                " {} {} {} {} — {} ",
                item.op.verb(),
                item.label,
                item.op.arrow(),
                item.op.target_label(),
                item.status.label()
            ),
            None => " Terminal Output ".to_string(),
        },
    };
    if state.queue.paused && state.queue.pending_count() > 0 {
        title.push_str(&format!(
            "[PAUSED — {} waiting, P to resume] ",
            state.queue.pending_count()
        ));
    }
    if !focused {
        title.push_str("[Shift+Tab to enter] ");
    } else {
        title.push_str("[W/S scroll] ");
    }

    frame.render_widget(
        pane_block(&title, page_focused && focused, btype, accent),
        area,
    );

    let inner = area.inner(Margin { horizontal: 1, vertical: 1 });
    let visible_height = inner.height as usize;
    state.terminal_view_height.set(visible_height.max(1));

    let log = state.visible_log();
    let mut lines: Vec<Line> = Vec::new();

    if log.is_empty() {
        lines.push(Line::from(Span::styled(
            "  (waiting for output…)",
            Style::default().fg(Color::DarkGray),
        )));
    } else {
        let scroll = state.effective_terminal_scroll();
        let end = (scroll + visible_height).min(log.len());
        for line in &log[scroll..end] {
            lines.push(Line::from(Span::styled(
                format!("  {}", line),
                Style::default().fg(log_color(line)),
            )));
        }

        if focused && log.len() > visible_height {
            // Counted against the last scrollable position, so the bottom reads N/N
            let info = format!(" {}/{} ", scroll, log.len() - visible_height);
            let info_width = info.len() as u16;
            if area.width > info_width + 2 {
                let indicator_rect = Rect {
                    x: area.x + area.width - info_width - 1,
                    y: area.y + area.height - 1,
                    width: info_width,
                    height: 1,
                };
                frame.render_widget(
                    Paragraph::new(info).style(Style::default().fg(Color::DarkGray)),
                    indicator_rect,
                );
            }
        }
    }

    frame.render_widget(Paragraph::new(lines), inner);
}

fn log_color(line: &str) -> Color {
    let t = line.trim_start();
    if t.starts_with('✗') || t.starts_with("error") || t.starts_with("error:") {
        Color::Red
    } else if t.starts_with('✓') {
        Color::Green
    } else if t.starts_with('⚠') || t.starts_with("warning") {
        Color::Yellow
    } else {
        Color::White
    }
}

fn render_queue_panel(
    frame: &mut Frame,
    area: Rect,
    state: &AppStoreState,
    btype: BorderType,
    accent: Color,
    page_focused: bool,
) {
    let focused = state.focus == AppStoreFocus::Queue;
    let active = state.queue.active_count();
    let title = if active > 0 {
        format!(" Install Queue ({}) ", active)
    } else {
        " Install Queue ".to_string()
    };
    frame.render_widget(
        pane_block(&title, page_focused && focused, btype, accent),
        area,
    );

    let inner = area.inner(Margin { horizontal: 1, vertical: 1 });
    let width = inner.width as usize;
    let hint_style = Style::default().fg(Color::DarkGray);

    // Queue-wide hotkeys are pinned to the bottom; per-job ones live under the
    // selected row so the two never get confused.
    let mut footer: Vec<Line> = Vec::new();
    footer.push(Line::from(Span::styled("─".repeat(width), hint_style)));
    if focused {
        let paused = if state.queue.paused { "P resume" } else { "P pause" };
        footer.push(Line::from(Span::styled(
            format!(" {}   X clear queue", paused),
            hint_style,
        )));
    } else {
        footer.push(Line::from(Span::styled(
            " Shift+Tab to enter",
            hint_style,
        )));
    }

    let mut lines: Vec<Line> = Vec::new();
    let mut cursor_line = 0usize;

    if state.queue.items.is_empty() {
        lines.push(Line::from(Span::styled(
            " Queue is empty",
            Style::default().fg(Color::DarkGray),
        )));
    }

    for (i, item) in state.queue.items.iter().enumerate() {
        let selected = focused && i == state.queue.cursor;
        let expanded = selected && state.queue.item_focused;
        if selected {
            cursor_line = lines.len();
        }
        let base = match item.status {
            QueueStatus::Pending => Color::Gray,
            QueueStatus::Running => Color::Yellow,
            QueueStatus::Done => Color::Green,
            QueueStatus::Failed => Color::Red,
        };
        let style = if expanded {
            Style::default().fg(Color::Black).bg(base).add_modifier(Modifier::BOLD)
        } else if selected {
            Style::default().fg(Color::Black).bg(base)
        } else if item.status == QueueStatus::Running {
            Style::default().fg(base).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(base)
        };

        let viewing = state.viewing_item == Some(item.id);
        let marker = if viewing { "│" } else { " " };
        let left = format!(
            "{}{} {} {}",
            marker,
            item.status.glyph(),
            item.op.verb(),
            item.label
        );
        let right = format!("{} {} ", item.op.arrow(), item.op.target_label());
        let (lw, rw) = (left.chars().count(), right.chars().count());
        let row = if lw + rw + 1 <= width {
            format!("{}{}{}", left, " ".repeat(width - lw - rw), right)
        } else {
            pad_clip(&format!("{} {}", left, right), width)
        };
        lines.push(Line::from(Span::styled(row, style)));

        if expanded {
            lines.push(Line::from(Span::styled(
                format!("     {} — {}", item.op.destination_phrase(), item.status.label()),
                Style::default().fg(base),
            )));
            if item.status == QueueStatus::Failed {
                if let Some(note) = &item.note {
                    for l in wrap_text(note, width.saturating_sub(6)) {
                        lines.push(Line::from(Span::styled(
                            format!("     {}", l),
                            Style::default().fg(Color::Red),
                        )));
                    }
                }
            }
            for (oi, opt) in state.queue.item_actions(i).iter().enumerate() {
                let sel = oi == state.queue.option_cursor;
                let opt_style = if sel {
                    Style::default().fg(Color::Black).bg(Color::Cyan)
                } else {
                    Style::default().fg(Color::White)
                };
                let prefix = if sel { "   » " } else { "     " };
                lines.push(Line::from(Span::styled(
                    format!("{}[ {} ]", prefix, opt.label()),
                    opt_style,
                )));
            }
            lines.push(Line::from(Span::styled(
                "     Q / ← back",
                Style::default().fg(Color::DarkGray),
            )));
        }
    }

    // Keep the selected row on screen above the pinned footer
    let body_height = (inner.height as usize).saturating_sub(footer.len());
    let mut out: Vec<Line> = if lines.len() > body_height {
        let start = cursor_line
            .saturating_sub(body_height / 3)
            .min(lines.len() - body_height);
        lines[start..start + body_height].to_vec()
    } else {
        lines
    };
    while out.len() < body_height {
        out.push(Line::from(""));
    }
    out.extend(footer);

    frame.render_widget(Paragraph::new(out), inner);
}

/// Pad to the pane width so the selection highlight fills the whole row.
fn pad_clip(text: &str, width: usize) -> String {
    let mut s: String = text.chars().take(width).collect();
    let len = s.chars().count();
    if len < width {
        s.push_str(&" ".repeat(width - len));
    }
    s
}

fn render_confirm_dialog(frame: &mut Frame, area: Rect, state: &AppStoreState) {
    let confirm = match &state.confirm_dialog {
        Some(c) => c,
        None => return,
    };

    let is_choose = matches!(confirm, ConfirmAction::UninstallChoose(_));
    let (title, msg, accent) = match confirm {
        ConfirmAction::ClearQueue => (
            " Clear Install Queue ",
            "Clear finished jobs and cancel everything still pending?".to_string(),
            Color::Yellow,
        ),
        ConfirmAction::UninstallChoose(n) => {
            (" Uninstall ", format!("Uninstall {} from:", n), Color::Red)
        }
        ConfirmAction::UninstallGlobal(n) | ConfirmAction::UninstallLocal(n) => (
            " Confirm Uninstall ",
            format!("Are you sure you want to uninstall {}?", n),
            Color::Red,
        ),
    };

    if is_choose {
        let width = (msg.len() + 10).max(40).min(area.width as usize) as u16;
        let height = 8;
        let popup_rect = Rect {
            x: area.x + (area.width.saturating_sub(width)) / 2,
            y: area.y + (area.height.saturating_sub(height)) / 2,
            width,
            height,
        };
        frame.render_widget(Clear, popup_rect);
        frame.render_widget(
            Block::default()
                .borders(Borders::ALL)
                .title(title)
                .style(Style::default().fg(accent).bg(Color::Black)),
            popup_rect,
        );
        let inner = popup_rect.inner(Margin { horizontal: 1, vertical: 1 });
        let options = ["PATH only", "Downloads only", "Both", "Cancel"];
        let mut lines = vec![
            Line::from(Span::styled(format!("  {}", msg), Style::default().fg(Color::White))),
            Line::from(""),
        ];
        for (i, opt) in options.iter().enumerate() {
            let style = if i == state.confirm_cursor {
                Style::default().fg(Color::Black).bg(Color::Cyan)
            } else {
                Style::default().fg(Color::White)
            };
            let prefix = if i == state.confirm_cursor { "  » " } else { "    " };
            lines.push(Line::from(Span::styled(format!("{}{}", prefix, opt), style)));
        }
        frame.render_widget(Paragraph::new(lines), inner);
    } else {
        let width = (msg.len() + 6).min(area.width as usize) as u16;
        let height = 5;
        let popup_rect = Rect {
            x: area.x + (area.width.saturating_sub(width)) / 2,
            y: area.y + (area.height.saturating_sub(height)) / 2,
            width,
            height,
        };
        frame.render_widget(Clear, popup_rect);
        frame.render_widget(
            Block::default()
                .borders(Borders::ALL)
                .title(title)
                .style(Style::default().fg(accent).bg(Color::Black)),
            popup_rect,
        );
        let inner = popup_rect.inner(Margin { horizontal: 1, vertical: 1 });
        let yes_style = if state.confirm_cursor == 0 {
            Style::default().fg(Color::Black).bg(accent)
        } else {
            Style::default().fg(accent)
        };
        let no_style = if state.confirm_cursor == 1 {
            Style::default().fg(Color::Black).bg(Color::Green)
        } else {
            Style::default().fg(Color::Green)
        };
        let lines = vec![
            Line::from(Span::styled(format!("  {}", msg), Style::default().fg(Color::White))),
            Line::from(""),
            Line::from(vec![
                Span::styled("     ", Style::default()),
                Span::styled(" Yes ", yes_style),
                Span::styled("    ", Style::default()),
                Span::styled(" No ", no_style),
            ]),
        ];
        frame.render_widget(Paragraph::new(lines), inner);
    }
}

fn render_install_location_dialog(frame: &mut Frame, area: Rect, state: &AppStoreState) {
    let width = 66u16.min(area.width);
    let inner_width = width.saturating_sub(4) as usize;

    let note_lines: Vec<String> = state
        .install_dialog_note
        .as_deref()
        .map(|n| wrap_text(n, inner_width))
        .unwrap_or_default();

    let height =
        (state.available_install_methods.len() as u16 + note_lines.len() as u16 + 5).min(area.height);

    let popup_rect = Rect {
        x: area.x + (area.width.saturating_sub(width)) / 2,
        y: area.y + (area.height.saturating_sub(height)) / 2,
        width,
        height,
    };

    frame.render_widget(Clear, popup_rect);

    let retrying = state.retry_item_id.is_some();
    let block = Block::default()
        .borders(Borders::ALL)
        .title(if retrying { " Retry Install " } else { " Choose Install Location " })
        .style(Style::default().fg(Color::Cyan).bg(Color::Black));
    frame.render_widget(block, popup_rect);

    let inner = popup_rect.inner(Margin { horizontal: 1, vertical: 1 });

    let mut lines: Vec<Line> = Vec::new();
    for note in &note_lines {
        lines.push(Line::from(Span::styled(
            format!("  {}", note),
            if retrying { Style::default().fg(Color::Yellow) } else { Style::default().fg(Color::Green) },
        )));
    }
    if !note_lines.is_empty() {
        lines.push(Line::from(""));
    }
    lines.push(Line::from(Span::styled(
        if retrying {
            "  Which source would you like to retry with?"
        } else {
            "  Where would you like to install?"
        },
        Style::default().fg(Color::White),
    )));
    lines.push(Line::from(""));

    for (i, method) in state.available_install_methods.iter().enumerate() {
        let selected = i == state.install_location_cursor;
        let failed = state.install_dialog_failed.contains(method);
        let style = match (selected, failed) {
            (true, true) => Style::default().fg(Color::Black).bg(Color::Red),
            (true, false) => Style::default().fg(Color::Black).bg(Color::Cyan),
            (false, true) => Style::default().fg(Color::Red),
            (false, false) => Style::default().fg(Color::White),
        };
        let prefix = if selected { "  » " } else { "    " };
        let suffix = if failed { "  ✗ failed previously" } else { "" };
        lines.push(Line::from(Span::styled(
            format!("{}{}{}", prefix, method.label(), suffix),
            style,
        )));
    }

    frame.render_widget(Paragraph::new(lines), inner);
}

/// Simple word-wrap helper.
pub fn wrap_text(text: &str, max_width: usize) -> Vec<String> {
    if max_width == 0 {
        return vec![text.to_string()];
    }
    let mut result = Vec::new();
    let mut current_line = String::new();
    for word in text.split_whitespace() {
        if current_line.len() + word.len() + 1 > max_width && !current_line.is_empty() {
            result.push(current_line);
            current_line = String::new();
        }
        if !current_line.is_empty() {
            current_line.push(' ');
        }
        current_line.push_str(word);
    }
    if !current_line.is_empty() {
        result.push(current_line);
    }
    if result.is_empty() {
        result.push(String::new());
    }
    result
}

/// Get the number of action buttons for the current app's install status.
pub fn action_count(status: &InstallStatus, supports_embed: bool, supports_local: bool, supports_global: bool, is_approved: bool) -> usize {
    let run_btn = if matches!(status, InstallStatus::NotInstalled) { 0 } else { 1 };
    let mode_btn = if !matches!(status, InstallStatus::NotInstalled) && supports_embed { 1 } else { 0 };
    let remove_btn = if !is_approved && matches!(status, InstallStatus::NotInstalled) { 1 } else { 0 };
    let base = match status {
        InstallStatus::NotInstalled => 2,  // Open Repo, Install
        InstallStatus::Global(_) => {
            if supports_local { 4 } else { 3 }
        }
        InstallStatus::Local(_) => {
            if supports_global { 4 } else { 3 }
        }
        InstallStatus::Both(_, _) => 5,
    };
    run_btn + base + mode_btn + remove_btn
}
