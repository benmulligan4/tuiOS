/// tuiOS — main shell: event loop, custom navbar, and main container rendering.

use std::io;
use std::process::Command;
use std::time::{Duration, Instant};

use crossterm::{
    event::{self, Event, KeyEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Margin, Rect},
    style::{Color, Style},
    text::Text,
    widgets::{Block, Borders, BorderType, Clear, Paragraph},
    Frame, Terminal,
};
use serde_json::Value;

use super::boot_animation;
use super::input_handler::{action_to_key_name, map_key};
use super::models::{Action, AppStatus, FocusTarget, TuiosState};
use super::process_manager;
use super::registry;

use crate::applications::character_set::{AppAction, CharacterSetApp};
use crate::applications::file_explorer::{FileExplorerAction, FileExplorerApp};
use crate::applications::text_editor::{TextEditorAction, TextEditorApp};
use crate::dashboards::{dashboard_1, dashboard_2};
use crate::dashboards::installed_runner::InstalledDashboard;
use crate::settings;
use crate::settings::state::SettingsCategory;
use crate::touchscreen;
use crate::utilities::logging;
use crate::app_store;

// ---------------------------------------------------------------------------
// Internal app wrapper — supports multiple built-in app types
// ---------------------------------------------------------------------------

enum InternalApp {
    CharacterSet(CharacterSetApp),
    FileExplorer(FileExplorerApp),
    TextEditor(TextEditorApp),
}

impl InternalApp {
    fn render(&mut self, frame: &mut Frame, area: Rect, border_style: Style) {
        match self {
            InternalApp::CharacterSet(app) => app.render(frame, area, border_style),
            InternalApp::FileExplorer(app) => app.render(frame, area, border_style),
            InternalApp::TextEditor(app) => app.render(frame, area, border_style),
        }
    }

    fn handle_key(&mut self, code: &str) -> bool {
        // Returns true if the app wants to close
        match self {
            InternalApp::CharacterSet(app) => {
                matches!(app.handle_key(code), Some(AppAction::Back))
            }
            InternalApp::FileExplorer(app) => {
                matches!(app.handle_key(code), Some(FileExplorerAction::Back))
            }
            InternalApp::TextEditor(app) => {
                matches!(app.handle_key(code), Some(TextEditorAction::Back))
            }
        }
    }

    fn stop(&mut self) {
        match self {
            InternalApp::CharacterSet(app) => app.stop(),
            InternalApp::FileExplorer(app) => app.stop(),
            InternalApp::TextEditor(app) => app.stop(),
        }
    }

    /// Returns true if this app uses the OSK and has it active,
    /// meaning WASD should NOT be mapped to navigation.
    fn wants_raw_wasd(&self) -> bool {
        match self {
            InternalApp::TextEditor(app) => app.keyboard_active(),
            _ => false,
        }
    }
}

// ---------------------------------------------------------------------------
// Navbar definition
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub enum NavContent {
    /// Direct action string (e.g., "page:settings")
    Direct(String),
    /// Dropdown children: vec of (label, action_string)
    Children(Vec<(String, String)>),
}

#[derive(Debug, Clone)]
pub struct NavItem {
    pub label: String,
    pub content: NavContent,
}

fn build_nav_items(
    dashboards: &std::collections::HashMap<String, Value>,
    apps_registry: &std::collections::HashMap<String, Value>,
) -> Vec<NavItem> {
    let dash_children: Vec<(String, String)> = dashboards
        .iter()
        .map(|(name, meta)| {
            let label = meta
                .get("label")
                .and_then(|v| v.as_str())
                .unwrap_or(name)
                .to_string();
            (label, format!("dashboard:{}", name))
        })
        .collect();

    let app_children: Vec<(String, String)> = apps_registry
        .iter()
        .map(|(name, meta)| {
            let label = meta
                .get("label")
                .and_then(|v| v.as_str())
                .unwrap_or(name)
                .to_string();
            (label, format!("app:{}", name))
        })
        .collect();

    vec![
        NavItem {
            label: "Dashboard".to_string(),
            content: NavContent::Children(dash_children),
        },
        NavItem {
            label: "Apps".to_string(),
            content: NavContent::Children(app_children),
        },
        NavItem {
            label: "Touchscreen".to_string(),
            content: NavContent::Direct("page:touchscreen".to_string()),
        },
        NavItem {
            label: "Settings".to_string(),
            content: NavContent::Direct("page:settings".to_string()),
        },
        NavItem {
            label: "System".to_string(),
            content: NavContent::Children(vec![
                ("App Store".to_string(), "page:appstore".to_string()),
                ("Task Manager".to_string(), "page:taskmanager".to_string()),
                ("Logs".to_string(), "page:logs".to_string()),
                ("Open Git Repository".to_string(), "system:open_repo".to_string()),
                ("Restart".to_string(), "system:restart".to_string()),
                ("Shutdown".to_string(), "system:shutdown".to_string()),
            ]),
        },
    ]
}

// ---------------------------------------------------------------------------
// Custom navbar renderer
// ---------------------------------------------------------------------------

fn render_navbar(frame: &mut Frame, area: Rect, nav_items: &[NavItem], state: &TuiosState) {
    let focused = state.focus == FocusTarget::Navbar;
    let labels: Vec<&str> = nav_items.iter().map(|i| i.label.as_str()).collect();

    // Load accent colour from settings
    let settings = crate::settings::persistence::load();
    let accent_name = crate::settings::persistence::get_str(&settings, "appearance.accent_color", "Cyan");
    let accent_color = crate::settings::pages::appearance::color_from_name(&accent_name);

    let mut constraints: Vec<Constraint> = labels
        .iter()
        .map(|lbl| Constraint::Length((lbl.len() + 4) as u16))
        .collect();
    constraints.push(Constraint::Fill(1));

    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints(constraints)
        .split(area);

    for (i, label) in labels.iter().enumerate() {
        let style = if focused && i == state.nav_cursor {
            Style::default().fg(Color::Black).bg(accent_color)
        } else {
            Style::default().fg(Color::White)
        };
        frame.render_widget(
            Paragraph::new(Text::raw(format!("  {}  ", label))).style(style),
            cols[i],
        );
    }

    // Render clock in the rightmost column if enabled
    let clock_on = crate::settings::persistence::get_bool(&settings, "appearance.clock_enabled", false);
    if clock_on {
        let clock_24h = crate::settings::persistence::get_bool(&settings, "appearance.clock_format_24h", true);
        let clock_secs = crate::settings::persistence::get_bool(&settings, "appearance.clock_show_seconds", false);
        let now = chrono::Local::now();
        let time_str = if clock_24h {
            if clock_secs { now.format("%H:%M:%S").to_string() } else { now.format("%H:%M").to_string() }
        } else {
            if clock_secs { now.format("%I:%M:%S %p").to_string() } else { now.format("%I:%M %p").to_string() }
        };
        let clock_width = time_str.len() as u16 + 2;
        let last_col = cols[labels.len()]; // the Fill(1) column
        if last_col.width >= clock_width {
            let clock_rect = Rect {
                x: last_col.x + last_col.width - clock_width,
                y: last_col.y,
                width: clock_width,
                height: 1,
            };
            frame.render_widget(
                Paragraph::new(Text::raw(format!(" {} ", time_str)))
                    .style(Style::default().fg(accent_color)),
                clock_rect,
            );
        }
    }
}

fn render_dropdown(frame: &mut Frame, area: Rect, nav_items: &[NavItem], state: &TuiosState, open_upward: bool) {
    if !state.nav_expanded {
        return;
    }

    let item = &nav_items[state.nav_cursor];
    let children = match &item.content {
        NavContent::Children(c) => c,
        NavContent::Direct(_) => return,
    };

    if children.is_empty() {
        return;
    }

    // Calculate x offset below the nav item
    let labels: Vec<&str> = nav_items.iter().map(|i| i.label.as_str()).collect();
    let x_offset: u16 = labels[..state.nav_cursor]
        .iter()
        .map(|lbl| (lbl.len() + 4) as u16)
        .sum();

    let max_label_len = children.iter().map(|(l, _)| l.len()).max().unwrap_or(0);
    let mut dropdown_width = (max_label_len + 6) as u16;
    let dropdown_height = (children.len() + 2) as u16;

    let available_width = area.width.saturating_sub(x_offset);
    if dropdown_width > available_width {
        dropdown_width = available_width.max(10);
    }

    let dropdown_height = dropdown_height.min(area.height);
    let dropdown_rect = Rect {
        x: area.x + x_offset,
        y: if open_upward {
            area.y + area.height - dropdown_height
        } else {
            area.y
        },
        width: dropdown_width,
        height: dropdown_height,
    };

    // Clear the area and draw border
    frame.render_widget(Clear, dropdown_rect);
    frame.render_widget(
        Block::default()
            .borders(Borders::ALL)
            .style(Style::default().bg(Color::Black)),
        dropdown_rect,
    );

    let inner = dropdown_rect.inner(Margin { horizontal: 1, vertical: 1 });

    let row_constraints: Vec<Constraint> = children
        .iter()
        .map(|_| Constraint::Length(1))
        .collect();

    let inner_rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints(row_constraints)
        .split(inner);

    for (i, (label, _)) in children.iter().enumerate() {
        let (style, text) = if i == state.dropdown_cursor {
            (
                Style::default().fg(Color::Black).bg(Color::Cyan),
                format!(" » {} ", label),
            )
        } else {
            (
                Style::default().fg(Color::White).bg(Color::Black),
                format!("   {} ", label),
            )
        };
        if i < inner_rows.len() {
            frame.render_widget(
                Paragraph::new(Text::raw(text)).style(style),
                inner_rows[i],
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Status colors for system page
// ---------------------------------------------------------------------------

fn status_color(status: AppStatus) -> Color {
    match status {
        AppStatus::Running => Color::Green,
        AppStatus::Idle => Color::Yellow,
        AppStatus::Stopped => Color::DarkGray,
    }
}

// ---------------------------------------------------------------------------
// System page renderer
// ---------------------------------------------------------------------------

fn render_system_page(frame: &mut Frame, area: Rect, state: &TuiosState, border_style: Style, border_type: BorderType) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(border_type)
        .title(" Task Manager — Running Processes ")
        .title_alignment(ratatui::layout::Alignment::Right)
        .style(border_style);
    frame.render_widget(block, area);
    let inner = area.inner(Margin { horizontal: 1, vertical: 1 });

    let running = process_manager::list_running();
    if running.is_empty() {
        frame.render_widget(
            Paragraph::new(Text::raw(
                "\n  No applications are currently running.\n\n  Press Q to return.",
            ))
            .style(Style::default().fg(Color::DarkGray)),
            inner,
        );
        return;
    }

    let mut row_constraints = vec![Constraint::Length(1), Constraint::Length(1)];
    for _ in &running {
        row_constraints.push(Constraint::Length(1));
    }
    row_constraints.push(Constraint::Length(2));

    let header_rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints(row_constraints)
        .split(inner);

    frame.render_widget(
        Paragraph::new(Text::raw(format!(
            "  {:<20} {:<8} {:<10} {}",
            "NAME", "PID", "STATUS", "STARTED"
        )))
        .style(Style::default().fg(Color::DarkGray)),
        header_rows[0],
    );
    frame.render_widget(
        Paragraph::new(Text::raw(format!("  {}", "─".repeat(60))))
            .style(Style::default().fg(Color::DarkGray)),
        header_rows[1],
    );

    for (i, app) in running.iter().enumerate() {
        let started = app.start_time.format("%H:%M:%S").to_string();
        let name_display = if i == state.system_cursor {
            format!("» {}", app.name)
        } else {
            format!("  {}", app.name)
        };
        let row_text = format!(
            "  {:<20} {:<8} {:<10} {}",
            name_display,
            app.pid.map(|p| p.to_string()).unwrap_or_default(),
            format!("{:?}", app.status),
            started
        );
        let style = if i == state.system_cursor {
            Style::default().fg(Color::Black).bg(Color::Cyan)
        } else {
            Style::default().fg(status_color(app.status))
        };
        frame.render_widget(
            Paragraph::new(Text::raw(row_text)).style(style),
            header_rows[2 + i],
        );
    }

    frame.render_widget(
        Paragraph::new(Text::raw(
            "\n  Enter — stop selected app   Q — return",
        ))
        .style(Style::default().fg(Color::DarkGray)),
        header_rows[2 + running.len()],
    );
}

// ---------------------------------------------------------------------------
// App log tail renderer
// ---------------------------------------------------------------------------

fn render_app_log(frame: &mut Frame, area: Rect, app_name: &str, border_style: Style) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(format!(" {} — Log Output ", app_name))
        .title_alignment(ratatui::layout::Alignment::Right)
        .style(border_style);
    frame.render_widget(block, area);
    let inner = area.inner(Margin { horizontal: 1, vertical: 1 });

    let n = if inner.height > 0 { inner.height as usize } else { 20 };
    let lines = process_manager::log_tail(app_name, n);
    let content = if lines.is_empty() {
        "  (no output yet — waiting for subprocess...)".to_string()
    } else {
        lines.iter().map(|l| format!("  {}", l)).collect::<Vec<_>>().join("\n")
    };

    frame.render_widget(
        Paragraph::new(Text::raw(content)).style(Style::default().fg(Color::White)),
        inner,
    );
}

// ---------------------------------------------------------------------------
// Logs page renderer — shows tuios.log with colored severity levels
// ---------------------------------------------------------------------------

fn render_logs_page(frame: &mut Frame, area: Rect, state: &TuiosState, border_style: Style, border_type: BorderType) {
    use ratatui::text::{Line, Span};

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(border_type)
        .title(" Logs — tuios.log ")
        .title_alignment(ratatui::layout::Alignment::Right)
        .style(border_style);
    frame.render_widget(block, area);
    let inner = area.inner(Margin { horizontal: 1, vertical: 1 });

    let lines = logging::read_log_lines();

    if lines.is_empty() {
        frame.render_widget(
            Paragraph::new(Text::raw("\n  No log entries yet."))
                .style(Style::default().fg(Color::DarkGray)),
            inner,
        );
        return;
    }

    // log_scroll is a reverse offset from the bottom (0 = most recent entries).
    // Compute the start line so that the view ends at (total - log_scroll).
    let total = lines.len();
    let visible_height = inner.height as usize;
    let max_offset = total.saturating_sub(visible_height);
    let offset = state.log_scroll.min(max_offset);
    let start = max_offset.saturating_sub(offset);
    let end = (start + visible_height).min(total);
    let visible_lines = &lines[start..end];

    let styled_lines: Vec<Line> = visible_lines
        .iter()
        .map(|line| {
            // Find severity tag and color just the tag text
            if let Some(start) = line.find("[INFO]") {
                let before = &line[..start];
                let tag = "[INFO]";
                let after = &line[start + tag.len()..];
                Line::from(vec![
                    Span::styled(format!("  {}", before), Style::default().fg(Color::White)),
                    Span::styled(tag, Style::default().fg(Color::Cyan)),
                    Span::styled(after.to_string(), Style::default().fg(Color::White)),
                ])
            } else if let Some(start) = line.find("[WARN]") {
                let before = &line[..start];
                let tag = "[WARN]";
                let after = &line[start + tag.len()..];
                Line::from(vec![
                    Span::styled(format!("  {}", before), Style::default().fg(Color::White)),
                    Span::styled(tag, Style::default().fg(Color::Yellow)),
                    Span::styled(after.to_string(), Style::default().fg(Color::White)),
                ])
            } else if let Some(start) = line.find("[ERROR]") {
                let before = &line[..start];
                let tag = "[ERROR]";
                let after = &line[start + tag.len()..];
                Line::from(vec![
                    Span::styled(format!("  {}", before), Style::default().fg(Color::White)),
                    Span::styled(tag, Style::default().fg(Color::Red)),
                    Span::styled(after.to_string(), Style::default().fg(Color::White)),
                ])
            } else if let Some(start) = line.find("[SCRIPT]") {
                let before = &line[..start];
                let tag = "[SCRIPT]";
                let after = &line[start + tag.len()..];
                Line::from(vec![
                    Span::styled(format!("  {}", before), Style::default().fg(Color::White)),
                    Span::styled(tag, Style::default().fg(Color::Magenta)),
                    Span::styled(after.to_string(), Style::default().fg(Color::White)),
                ])
            } else if let Some(start) = line.find("[SETTINGS]") {
                let before = &line[..start];
                let tag = "[SETTINGS]";
                let after = &line[start + tag.len()..];
                Line::from(vec![
                    Span::styled(format!("  {}", before), Style::default().fg(Color::White)),
                    Span::styled(tag, Style::default().fg(Color::Green)),
                    Span::styled(after.to_string(), Style::default().fg(Color::White)),
                ])
            } else {
                Line::from(Span::styled(
                    format!("  {}", line),
                    Style::default().fg(Color::DarkGray),
                ))
            }
        })
        .collect();

    let scrollbar_info = format!(
        " Line {}-{} of {} ",
        start + 1,
        end,
        total
    );

    frame.render_widget(
        Paragraph::new(styled_lines),
        inner,
    );

    // Render scroll position indicator at bottom-right of border
    let info_width = scrollbar_info.len() as u16;
    if area.width > info_width + 2 {
        let info_rect = Rect {
            x: area.x + area.width - info_width - 1,
            y: area.y + area.height - 1,
            width: info_width,
            height: 1,
        };
        frame.render_widget(
            Paragraph::new(Text::raw(scrollbar_info))
                .style(Style::default().fg(Color::DarkGray)),
            info_rect,
        );
    }
}

// ---------------------------------------------------------------------------
// Main container renderer
// ---------------------------------------------------------------------------

fn render_main(
    frame: &mut Frame,
    area: Rect,
    state: &mut TuiosState,
    active_internal_app: &mut Option<InternalApp>,
    active_installed_dash: &mut Option<InstalledDashboard>,
    active_installed_app: &mut Option<InstalledDashboard>,
    registered_apps: &std::collections::HashMap<String, Value>,
) {
    let focused = state.focus == FocusTarget::Main;
    let settings = crate::settings::persistence::load();
    let accent_name = crate::settings::persistence::get_str(&settings, "appearance.accent_color", "Cyan");
    let accent_color = crate::settings::pages::appearance::color_from_name(&accent_name);
    let border_name = crate::settings::persistence::get_str(&settings, "appearance.border_style", "Rounded");
    let border_type = crate::settings::pages::appearance::border_type_from_name(&border_name);
    let border_style = if focused {
        Style::default().fg(accent_color)
    } else {
        Style::default()
    };

    if let Some(page) = &state.active_page.clone() {
        match page.as_str() {
            "settings" => settings::page::render(frame, area, border_style, &mut state.settings),
            "touchscreen" => touchscreen::page::render(frame, area, border_style),
            "system" | "taskmanager" => render_system_page(frame, area, state, border_style, border_type),
            "logs" => render_logs_page(frame, area, state, border_style, border_type),
            "appstore" => app_store::page::render(frame, area, focused, &state.app_store, registered_apps),
            _ => {}
        }
    } else if state.active_app.is_some() {
        if let Some(app) = active_internal_app {
            app.render(frame, area, border_style);
        } else if let Some(pty_app) = active_installed_app {
            pty_app.render(frame, area, border_style);
        } else if let Some(app_name) = &state.active_app {
            render_app_log(frame, area, app_name, border_style);
        }
    } else if let Some(dash) = active_installed_dash {
        if state.active_dashboard == dash.name {
            dash.render(frame, area, border_style);
        } else {
            // Dashboard changed away from installed one
            match state.active_dashboard.as_str() {
                "Dashboard-1" => dashboard_1::render(frame, area, border_style),
                "Dashboard-2" => dashboard_2::render(frame, area, border_style),
                _ => render_no_dashboard(frame, area, border_style),
            }
        }
    } else {
        match state.active_dashboard.as_str() {
            "Dashboard-1" => dashboard_1::render(frame, area, border_style),
            "Dashboard-2" => dashboard_2::render(frame, area, border_style),
            _ => render_no_dashboard(frame, area, border_style),
        }
    }
}

fn render_no_dashboard(frame: &mut Frame, area: Rect, border_style: Style) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" tuiOS ")
        .style(border_style);
    frame.render_widget(block, area);
    let inner = area.inner(Margin { horizontal: 1, vertical: 1 });
    frame.render_widget(
        Paragraph::new(Text::raw("\n  No dashboard loaded."))
            .style(Style::default().fg(Color::DarkGray)),
        inner,
    );
}

fn render_status_bar(frame: &mut Frame, area: Rect) {
    use sysinfo::System;
    let mut sys = System::new_all();
    sys.refresh_cpu_usage();
    sys.refresh_memory();

    let cpu: f32 = sys.cpus().iter().map(|c| c.cpu_usage()).sum::<f32>()
        / sys.cpus().len().max(1) as f32;
    let used_mb = sys.used_memory() / 1024 / 1024;
    let total_mb = sys.total_memory() / 1024 / 1024;
    let now = chrono::Local::now().format("%H:%M:%S").to_string();

    let s = crate::settings::persistence::load();
    let accent_name = crate::settings::persistence::get_str(&s, "appearance.accent_color", "Cyan");
    let accent = crate::settings::pages::appearance::color_from_name(&accent_name);

    let text = format!("  {}   CPU: {:.0}%   RAM: {}/{} MB  ", now, cpu, used_mb, total_mb);
    frame.render_widget(
        Paragraph::new(text).style(Style::default().fg(accent)),
        area,
    );
}

// ---------------------------------------------------------------------------
// Navbar key handling
// ---------------------------------------------------------------------------

enum NavResult {
    None,
    Quit,
    Restart,
    ActivateInternalApp,
}

fn handle_navbar_key(
    action: Action,
    state: &mut TuiosState,
    nav_items: &[NavItem],
    apps_registry: &std::collections::HashMap<String, Value>,
    dashboards_registry: &std::collections::HashMap<String, Value>,
    registered_apps: &std::collections::HashMap<String, Value>,
    active_internal_app: &mut Option<InternalApp>,
    active_installed_dash: &mut Option<InstalledDashboard>,
    active_installed_app: &mut Option<InstalledDashboard>,
) -> NavResult {
    // Tab focus toggle is now handled at the raw event level

    if action == Action::Quit {
        return NavResult::Quit;
    }

    if action == Action::Back {
        if state.nav_expanded {
            state.nav_expanded = false;
            state.dropdown_cursor = 0;
        }
        return NavResult::None;
    }

    // --- Dropdown is open ---
    if state.nav_expanded {
        let item = &nav_items[state.nav_cursor];
        if let NavContent::Children(children) = &item.content {
            match action {
                Action::Up => {
                    if state.dropdown_cursor > 0 {
                        state.dropdown_cursor -= 1;
                    }
                }
                Action::Down => {
                    if state.dropdown_cursor < children.len().saturating_sub(1) {
                        state.dropdown_cursor += 1;
                    }
                }
                Action::Enter => {
                    if !children.is_empty() {
                        let data = children[state.dropdown_cursor].1.clone();
                        return execute_action(
                            &data,
                            state,
                            apps_registry,
                            dashboards_registry,
                            registered_apps,
                            active_internal_app,
                            active_installed_dash,
                            active_installed_app,
                        );
                    }
                }
                _ => {}
            }
        }
        return NavResult::None;
    }

    // --- Dropdown is closed (top-level navigation) ---
    match action {
        Action::Left => {
            if state.nav_cursor == 0 {
                state.nav_cursor = nav_items.len() - 1;
            } else {
                state.nav_cursor -= 1;
            }
        }
        Action::Right => {
            state.nav_cursor = (state.nav_cursor + 1) % nav_items.len();
        }
        Action::Enter | Action::Down => {
            let item = &nav_items[state.nav_cursor];
            match &item.content {
                NavContent::Children(_) => {
                    state.nav_expanded = true;
                    state.dropdown_cursor = 0;
                }
                NavContent::Direct(data) => {
                    let data = data.clone();
                    return execute_action(
                        &data,
                        state,
                        apps_registry,
                        dashboards_registry,
                        registered_apps,
                        active_internal_app,
                        active_installed_dash,
                        active_installed_app,
                    );
                }
            }
        }
        _ => {}
    }

    NavResult::None
}

fn execute_action(
    data: &str,
    state: &mut TuiosState,
    apps_registry: &std::collections::HashMap<String, Value>,
    dashboards_registry: &std::collections::HashMap<String, Value>,
    registered_apps: &std::collections::HashMap<String, Value>,
    active_internal_app: &mut Option<InternalApp>,
    active_installed_dash: &mut Option<InstalledDashboard>,
    active_installed_app: &mut Option<InstalledDashboard>,
) -> NavResult {
    state.nav_expanded = false;
    state.dropdown_cursor = 0;

    if let Some(name) = data.strip_prefix("dashboard:") {
        logging::info(&format!("Navigated to dashboard: {}", name));

        // Check window mode preference for this dashboard
        let window_mode = app_store::actions::get_window_mode(name, registered_apps.get(name));

        // Check if this is an installed (third-party) dashboard
        if let Some(meta) = dashboards_registry.get(name) {
            let dash_type = meta.get("type").and_then(|t| t.as_str()).unwrap_or("");
            if dash_type == "installed" {
                let cmd = app_store::actions::resolve_launch_cmd(
                    name,
                    "Dashboard",
                    registered_apps.get(name),
                    Some(meta),
                );
                if cmd.is_empty() {
                    logging::error(&format!("No runnable binary found for dashboard {}", name));
                }

                if window_mode == "fullscreen" && !cmd.is_empty() {
                    logging::info(&format!("Launching dashboard {} in a new window", name));
                    *active_installed_dash = None;
                    state.pending_foreground = Some(cmd);
                    return NavResult::None;
                }

                logging::info(&format!("Launching dashboard {} in embedded tuiOS mode", name));
                state.active_dashboard = name.to_string();
                state.active_page = None;
                state.active_app = None;
                state.focus = FocusTarget::Main;

                let label = meta.get("label").and_then(|v| v.as_str()).unwrap_or(name).to_string();
                *active_installed_dash = None;
                if !cmd.is_empty() {
                    *active_installed_dash = InstalledDashboard::start(name, &label, &cmd, 80, 24);
                }
            } else {
                // Switching to a built-in dashboard; drop any installed one
                state.active_dashboard = name.to_string();
                state.active_page = None;
                state.active_app = None;
                state.focus = FocusTarget::Main;
                *active_installed_dash = None;
            }
        } else {
            state.active_dashboard = name.to_string();
            state.active_page = None;
            state.active_app = None;
            state.focus = FocusTarget::Main;
            *active_installed_dash = None;
        }
    } else if let Some(name) = data.strip_prefix("app:") {
        logging::info(&format!("Opened app: {}", name));
        let meta = apps_registry.get(name);
        let app_type = meta
            .and_then(|m| m.get("type"))
            .and_then(|t| t.as_str())
            .unwrap_or("external");

        if app_type == "internal" {
            let internal = match name {
                "TextEditor" => {
                    let mut app = TextEditorApp::new();
                    app.start();
                    InternalApp::TextEditor(app)
                }
                "FileExplorer" => {
                    let mut app = FileExplorerApp::new();
                    app.start();
                    InternalApp::FileExplorer(app)
                }
                _ => {
                    let mut app = CharacterSetApp::new();
                    app.start();
                    InternalApp::CharacterSet(app)
                }
            };
            *active_internal_app = Some(internal);
            state.active_app = Some(name.to_string());
            state.active_page = None;
            state.focus = FocusTarget::Main;
            return NavResult::ActivateInternalApp;
        } else {
            let category = registered_apps
                .get(name)
                .and_then(|m| m.get("category"))
                .and_then(|v| v.as_str())
                .unwrap_or("Other");
            let launch_cmd = app_store::actions::resolve_launch_cmd(
                name,
                category,
                registered_apps.get(name),
                meta,
            );

            if !launch_cmd.is_empty() {
                let window_mode = app_store::actions::get_window_mode(name, registered_apps.get(name));
                if window_mode == "fullscreen" {
                    logging::info(&format!("Launching {} in a new window", name));
                    state.pending_foreground = Some(launch_cmd);
                    return NavResult::None;
                }

                logging::info(&format!("Launching {} in embedded tuiOS mode", name));
                *active_installed_app = InstalledDashboard::start(name, name, &launch_cmd, 80, 24);
                state.active_app = Some(name.to_string());
                state.active_page = None;
                state.focus = FocusTarget::Main;
                state.nav_expanded = false;
            } else {
                logging::error(&format!("No runnable binary found for app {}", name));
            }
        }
    } else if let Some(page_name) = data.strip_prefix("page:") {
        if page_name == "logs" {
            state.log_scroll = 0;
        } else {
            logging::info(&format!("Opened page: {}", page_name));
        }
        if page_name == "appstore" {
            state.needs_sync = true;
        }
        state.active_page = Some(page_name.to_string());
        state.active_app = None;
        state.focus = FocusTarget::Main;
    } else if data == "system:shutdown" {
        logging::info("System shutdown requested");
        return NavResult::Quit;
    } else if data == "system:restart" {
        logging::info("System restart requested");
        return NavResult::Restart;
    } else if data == "system:open_repo" {
        logging::info("Opening tuiOS Git repository");
        let url = "https://github.com/benmulligan4/tuiOS";
        // Use spawn() (non-blocking) so tuiOS doesn't freeze waiting for the browser
        let spawned = if cfg!(target_os = "windows") {
            Command::new("cmd").args(["/C", "start", "", url]).spawn().is_ok()
        } else if cfg!(target_os = "macos") {
            Command::new("open").arg(url).spawn().is_ok()
        } else {
            Command::new("xdg-open").arg(url).spawn().is_ok()
        };
        if spawned {
            state.popup = Some(("Opening Git Repository...".to_string(), Instant::now()));
        } else {
            state.popup = Some((format!("Visit: {}", url), Instant::now()));
            logging::warn(&format!("Could not open browser. Visit: {}", url));
        }
    }

    NavResult::None
}

// ---------------------------------------------------------------------------
// Main container key handling
// ---------------------------------------------------------------------------

enum MainResult {
    None,
    Quit,
    Restart,
}

fn handle_main_key(
    action: Action,
    state: &mut TuiosState,
    active_internal_app: &mut Option<InternalApp>,
    active_installed_dash: &mut Option<InstalledDashboard>,
    active_installed_app: &mut Option<InstalledDashboard>,
    registered_apps: &std::collections::HashMap<String, Value>,
) -> MainResult {
    // App store page handles its own Quit/Back (pane/dialog switching)
    if matches!(state.active_page.as_deref(), Some("appstore")) {
        if action == Action::Quit {
            // Esc in appstore: close dialogs or navigate back, don't quit tuiOS
            let ss = &mut state.app_store;
            if ss.confirm_dialog.is_some() {
                ss.confirm_dialog = None;
                ss.confirm_cursor = 0;
            } else if ss.install_location_dialog {
                ss.install_location_dialog = false;
                ss.install_dialog_note = None;
                ss.pending_install_key = None;
            } else if ss.browser.active {
                // Close browser sub-views first, then browser itself
                use crate::app_store::awesome_ratatui_manager::BrowserFocus;
                match ss.browser.focus {
                    BrowserFocus::Actions => {
                        ss.browser.focus = BrowserFocus::List;
                        ss.browser.action_cursor = 0;
                    }
                    BrowserFocus::SearchBar => {
                        ss.browser.search_query.clear();
                        ss.browser.focus = BrowserFocus::List;
                        ss.browser.recompute_visible();
                    }
                    BrowserFocus::DescriptionToggle => {
                        ss.browser.focus = BrowserFocus::List;
                    }
                    BrowserFocus::RepoButton => {
                        ss.browser.focus = BrowserFocus::List;
                    }
                    BrowserFocus::List => {
                        ss.browser.active = false;
                        ss.browser.search_query.clear();
                    }
                }
            } else if ss.filter_panel_open {
                ss.filter_panel_open = false;
                ss.focus = crate::app_store::state::AppStoreFocus::LeftPane;
            } else if ss.focus == crate::app_store::state::AppStoreFocus::BrowserButton {
                ss.focus = crate::app_store::state::AppStoreFocus::LeftPane;
            } else if ss.focus == crate::app_store::state::AppStoreFocus::FilterPanel {
                ss.focus = crate::app_store::state::AppStoreFocus::LeftPane;
            } else if ss.focus == crate::app_store::state::AppStoreFocus::RightPane {
                ss.focus = crate::app_store::state::AppStoreFocus::LeftPane;
                ss.in_right_actions = false;
            } else if ss.focus == crate::app_store::state::AppStoreFocus::Terminal
                || ss.focus == crate::app_store::state::AppStoreFocus::Queue
            {
                ss.focus = crate::app_store::state::AppStoreFocus::RightPane;
                ss.in_right_actions = true;
            } else if ss.focus == crate::app_store::state::AppStoreFocus::SearchBar {
                ss.search_query.clear();
                ss.focus = crate::app_store::state::AppStoreFocus::LeftPane;
                ss.recompute_app_list(registered_apps);
            } else if ss.ui_locked() {
                ui_lock_popup(state);
            } else {
                state.active_page = None;
            }
            return MainResult::None;
        }
        handle_appstore_key(action, state, registered_apps, active_installed_app);
        return MainResult::None;
    }

    if action == Action::Quit {
        return MainResult::Quit;
    }

    // Tab focus toggle is now handled at the raw event level
    // using the configured toggle key. Don't handle Action::Tab here.

    // If an internal app has its OSK active, forward ALL mapped keys to it
    // (including Back/Q which normally closes the app — the OSK handles
    // Q as "close keyboard", not "close app").
    if let Some(app) = active_internal_app {
        if app.wants_raw_wasd() {
            if let Some(key_name) = action_to_key_name(action) {
                if app.handle_key(key_name) {
                    app.stop();
                    state.active_app = None;
                    *active_internal_app = None;
                }
            }
            return MainResult::None;
        }
    }

    if action == Action::Back {
        // Settings and App Store pages handle their own Back (pane switching)
        if matches!(state.active_page.as_deref(), Some("settings")) {
            // Handled in the settings navigation section below — don't return here
        } else if matches!(state.active_page.as_deref(), Some("appstore")) {
            // Already handled above — should not reach here
        } else if state.active_page.is_some() {
            state.active_page = None;
            state.active_app = None;
            return MainResult::None;
        } else if state.active_app.is_some() {
            if let Some(app) = active_internal_app {
                app.stop();
            }
            state.active_app = None;
            *active_internal_app = None;
            *active_installed_app = None;
            return MainResult::None;
        } else if active_installed_dash.is_some() {
            // Close the installed dashboard and go back to default
            *active_installed_dash = None;
            state.active_dashboard = "Dashboard-1".to_string();
            return MainResult::None;
        } else {
            return MainResult::None;
        }
    }

    // Forward keys to installed dashboard if active
    if let Some(dash) = active_installed_dash {
        if state.active_dashboard == dash.name && state.active_page.is_none() && state.active_app.is_none() {
            dash.send_key(action);
            return MainResult::None;
        }
    }

    // System page navigation
    if matches!(state.active_page.as_deref(), Some("system") | Some("taskmanager")) {
        let running = process_manager::list_running();
        match action {
            Action::Up => {
                if state.system_cursor > 0 {
                    state.system_cursor -= 1;
                }
            }
            Action::Down => {
                let max_idx = running.len().saturating_sub(1);
                if state.system_cursor < max_idx {
                    state.system_cursor += 1;
                }
            }
            Action::Enter => {
                if !running.is_empty() {
                    let idx = state.system_cursor.min(running.len() - 1);
                    process_manager::stop(&running[idx].name);
                    state.system_cursor = 0;
                }
            }
            _ => {}
        }
        return MainResult::None;
    }

    // Logs page navigation
    if matches!(state.active_page.as_deref(), Some("logs")) {
        let total_lines = logging::read_log_lines().len();
        match action {
            // Up = scroll toward older entries (increase reverse offset from bottom)
            Action::Up => {
                let max_offset = total_lines.saturating_sub(1);
                state.log_scroll = (state.log_scroll + 1).min(max_offset);
            }
            // Down = scroll toward newer entries (decrease reverse offset)
            Action::Down => {
                state.log_scroll = state.log_scroll.saturating_sub(1);
            }
            _ => {}
        }
        return MainResult::None;
    }

    // Settings page navigation
    if matches!(state.active_page.as_deref(), Some("settings")) {
        let ss = &mut state.settings;

        // Key capture mode is now handled at the raw event level

        if ss.in_right_pane {
            // ---- Edit mode: Left/Right cycles the current multi-option setting ----
            if ss.editing_setting {
                match action {
                    Action::Left => {
                        settings::page::handle_setting_cycle(ss, false);
                    }
                    Action::Right => {
                        settings::page::handle_setting_cycle(ss, true);
                    }
                    Action::Enter | Action::Back => {
                        // Exit edit mode; Back exits without further action
                        ss.editing_setting = false;
                    }
                    _ => {}
                }
                return MainResult::None;
            }

            // ---- Normal right pane navigation ----
            let count = settings::page::current_item_count(ss);
            match action {
                Action::Up => {
                    if ss.right_cursor > 0 {
                        ss.right_cursor -= 1;
                    }
                }
                Action::Down => {
                    if count > 0 && ss.right_cursor < count.saturating_sub(1) {
                        ss.right_cursor += 1;
                    }
                }
                // Left/Right (including numpad 4/6) directly cycle multi-option settings
                Action::Left => {
                    if settings::page::is_edit_mode_item(ss) {
                        settings::page::handle_setting_cycle(ss, false);
                    }
                }
                Action::Right => {
                    if settings::page::is_edit_mode_item(ss) {
                        settings::page::handle_setting_cycle(ss, true);
                    }
                }
                Action::Enter => {
                    if settings::page::is_edit_mode_item(ss) {
                        // Enter edit mode to show ◄ ► arrows
                        ss.editing_setting = true;
                    } else {
                        match settings::page::handle_right_pane_enter(ss) {
                            settings::page::SettingsAction::Quit => return MainResult::Quit,
                            settings::page::SettingsAction::Restart => return MainResult::Restart,
                            settings::page::SettingsAction::ShowPopup(msg) => {
                                state.popup = Some((msg, Instant::now()));
                            }
                            settings::page::SettingsAction::None => {}
                        }
                    }
                }
                Action::Back => {
                    // Back leaves a sub-page before leaving the right pane
                    if ss.subpage.take().is_some() {
                        ss.right_cursor = 0;
                    } else {
                        ss.in_right_pane = false;
                    }
                }
                _ => {}
            }
        } else {
            // In left pane — navigate categories
            let cat_count = SettingsCategory::ALL.len();
            match action {
                Action::Up => {
                    if ss.category_cursor > 0 {
                        ss.category_cursor -= 1;
                    }
                }
                Action::Down => {
                    if ss.category_cursor < cat_count.saturating_sub(1) {
                        ss.category_cursor += 1;
                    }
                }
                Action::Enter | Action::Right => {
                    let cat = ss.selected_category();
                    if cat.is_available() || cat.is_pi_only() {
                        ss.in_right_pane = true;
                        ss.reset_right_pane();
                    }
                }
                Action::Back => {
                    // Back from left pane closes the settings page
                    state.active_page = None;
                    return MainResult::None;
                }
                _ => {}
            }
        }
        return MainResult::None;
    }

    // App Store page navigation
    if matches!(state.active_page.as_deref(), Some("appstore")) {
        handle_appstore_key(action, state, registered_apps, active_installed_app);
        return MainResult::None;
    }

    // Internal app key forwarding
    if let Some(app) = active_internal_app {
        if let Some(key_name) = action_to_key_name(action) {
            if app.handle_key(key_name) {
                app.stop();
                state.active_app = None;
                *active_internal_app = None;
            }
        }
    }

    MainResult::None
}

// ---------------------------------------------------------------------------
// App Store key handling
// ---------------------------------------------------------------------------

fn app_label(key: &str, registered: &std::collections::HashMap<String, Value>) -> String {
    registered
        .get(key)
        .and_then(|m| m.get("label"))
        .and_then(|v| v.as_str())
        .unwrap_or(key)
        .to_string()
}

/// Add a job to the install queue, or explain why it clashes with one already there.
fn enqueue_job(
    state: &mut TuiosState,
    registered_apps: &std::collections::HashMap<String, Value>,
    key: &str,
    op: crate::app_store::queue::QueueOp,
) {
    let label = app_label(key, registered_apps);
    match state.app_store.queue.enqueue(key, &label, op) {
        Ok(_) => {
            logging::info(&format!(
                "App Store: queued {} of {} ({})",
                op.verb().to_lowercase(),
                key,
                op.target_label()
            ));
            state.app_store.queue.clamp_cursor();
            state.popup = Some((
                format!(
                    "Queued: {} {} {} {}",
                    op.verb(),
                    label,
                    op.arrow(),
                    op.target_label()
                ),
                Instant::now(),
            ));
        }
        Err(msg) => {
            state.popup = Some((msg, Instant::now()));
        }
    }
}

/// Shown whenever the queue blocks something instead of silently swallowing the key.
fn ui_lock_popup(state: &mut TuiosState) {
    state.popup = Some((
        "UI lock enabled — the install queue is still running. You can keep browsing \
         the App Store, but you cannot leave it or launch apps until the queue finishes."
            .to_string(),
        Instant::now(),
    ));
}

fn handle_appstore_key(
    action: Action,
    state: &mut TuiosState,
    registered_apps: &std::collections::HashMap<String, Value>,
    active_installed_app: &mut Option<InstalledDashboard>,
) {
    use crate::app_store::queue::QueueOp;
    use crate::app_store::state::{AppStoreFocus, ConfirmAction, InstallStatus};

    let ss = &mut state.app_store;

    // Awesome Ratatui browser handles its own navigation when active
    if ss.browser.active {
        handle_browser_key(action, state, registered_apps);
        return;
    }

    // Confirm dialog
    if ss.confirm_dialog.is_some() {
        let is_choose = matches!(ss.confirm_dialog, Some(ConfirmAction::UninstallChoose(_)));
        let max_cursor = if is_choose { 3 } else { 1 }; // choose: PATH/Downloads/Both/Cancel, normal: Yes/No
        match action {
            Action::Left => { if ss.confirm_cursor > 0 { ss.confirm_cursor -= 1; } }
            Action::Right => { if ss.confirm_cursor < max_cursor { ss.confirm_cursor += 1; } }
            Action::Up => { if is_choose && ss.confirm_cursor > 0 { ss.confirm_cursor -= 1; } }
            Action::Down => { if is_choose && ss.confirm_cursor < max_cursor { ss.confirm_cursor += 1; } }
            Action::Enter => {
                let confirm = ss.confirm_dialog.take().unwrap();

                if matches!(confirm, ConfirmAction::ClearQueue) {
                    if ss.confirm_cursor == 0 {
                        let cleared = ss.queue.clear_finished();
                        let pending = ss.queue.cancel_all_pending();
                        ss.viewing_item = None;
                        ss.queue.item_focused = false;
                        if ss.queue.is_empty() {
                            ss.focus = AppStoreFocus::RightPane;
                            ss.in_right_actions = true;
                        }
                        state.popup = Some((
                            format!(
                                "Cleared {} finished job(s), cancelled {} pending",
                                cleared, pending
                            ),
                            Instant::now(),
                        ));
                    }
                    state.app_store.confirm_cursor = 0;
                    return;
                }

                // 0=PATH, 1=Downloads, 2=Both for the choose variant; 0=Yes otherwise
                let (key, remove_global, remove_local) = match &confirm {
                    ConfirmAction::UninstallGlobal(key) if ss.confirm_cursor == 0 => {
                        (Some(key.clone()), true, false)
                    }
                    ConfirmAction::UninstallLocal(key) if ss.confirm_cursor == 0 => {
                        (Some(key.clone()), false, true)
                    }
                    ConfirmAction::UninstallChoose(key) if ss.confirm_cursor <= 2 => (
                        Some(key.clone()),
                        ss.confirm_cursor == 0 || ss.confirm_cursor == 2,
                        ss.confirm_cursor == 1 || ss.confirm_cursor == 2,
                    ),
                    _ => (None, false, false),
                };

                if let Some(key) = key {
                    if remove_global || remove_local {
                        enqueue_job(
                            state,
                            registered_apps,
                            &key,
                            QueueOp::Uninstall { path: remove_global, downloads: remove_local },
                        );
                    }
                }
                state.app_store.confirm_cursor = 0;
            }
            Action::Back => {
                ss.confirm_dialog = None;
                ss.confirm_cursor = 0;
            }
            _ => {}
        }
        return;
    }

    // Install location dialog
    if ss.install_location_dialog {
        match action {
            Action::Up => {
                if ss.install_location_cursor > 0 {
                    ss.install_location_cursor -= 1;
                }
            }
            Action::Down => {
                if ss.install_location_cursor < ss.available_install_methods.len().saturating_sub(1) {
                    ss.install_location_cursor += 1;
                }
            }
            Action::Enter => {
                let location = ss.available_install_methods[ss.install_location_cursor];
                ss.install_location_dialog = false;
                ss.install_dialog_note = None;

                // A browser "Add & Install" pins its own key; otherwise use the selection.
                let key = ss
                    .pending_install_key
                    .take()
                    .or_else(|| ss.selected_app_key().cloned());

                if let Some(key) = key {
                    // The app may have just been added, so `registered_apps` can be stale.
                    let known = registered_apps.contains_key(&key)
                        || registry::load_registered_apps().contains_key(&key);
                    if known {
                        enqueue_job(state, registered_apps, &key, QueueOp::Install(location));
                    } else {
                        logging::error(&format!("App Store: install requested for unknown app '{}'", key));
                    }
                }
            }
            Action::Back => {
                ss.install_location_dialog = false;
                ss.install_dialog_note = None;
                ss.pending_install_key = None;
            }
            _ => {}
        }
        return;
    }

    // Main pane navigation
    match ss.focus {
        AppStoreFocus::FilterPanel => {
            let categories = crate::app_store::state::AppStoreState::all_categories(registered_apps);
            let refresh_idx = ss.refresh_cursor_idx(registered_apps);
            let max_cursor = ss.filter_max_cursor(registered_apps);

            // Sort dropdown is open — handle it first
            if ss.sort_dropdown_open {
                match action {
                    Action::Up => {
                        if ss.sort_dropdown_cursor > 0 {
                            ss.sort_dropdown_cursor -= 1;
                        }
                    }
                    Action::Down => {
                        if ss.sort_dropdown_cursor < crate::app_store::state::SortMode::ALL.len().saturating_sub(1) {
                            ss.sort_dropdown_cursor += 1;
                        }
                    }
                    Action::Enter => {
                        ss.sort_mode = crate::app_store::state::SortMode::ALL[ss.sort_dropdown_cursor];
                        ss.sort_dropdown_open = false;
                        ss.recompute_app_list(registered_apps);
                    }
                    Action::Back => {
                        ss.sort_dropdown_open = false;
                    }
                    _ => {}
                }
                return;
            }

            match action {
                Action::Up => {
                    if ss.filter_panel_cursor > 0 {
                        ss.filter_panel_cursor -= 1;
                    } else {
                        ss.focus = AppStoreFocus::BrowserButton;
                    }
                }
                Action::Down => {
                    if ss.filter_panel_cursor < max_cursor {
                        ss.filter_panel_cursor += 1;
                    } else {
                        ss.focus = AppStoreFocus::LeftPane;
                    }
                }
                Action::Enter => {
                    if ss.filter_panel_cursor == 0 {
                        // Toggle panel open/close
                        ss.filter_panel_open = !ss.filter_panel_open;
                        if !ss.filter_panel_open {
                            ss.sort_dropdown_open = false;
                        }
                    } else if ss.filter_panel_cursor == refresh_idx {
                        // Refresh
                        logging::info("App Store: manual refresh triggered");
                        state.needs_sync = true;
                    } else if ss.filter_panel_open {
                        match ss.filter_panel_cursor {
                            1 => {
                                // Open sort dropdown
                                ss.sort_dropdown_open = !ss.sort_dropdown_open;
                                ss.sort_dropdown_cursor = crate::app_store::state::SortMode::ALL
                                    .iter().position(|m| *m == ss.sort_mode).unwrap_or(0);
                            }
                            2 => {
                                ss.filter_show_installed = !ss.filter_show_installed;
                                ss.recompute_app_list(registered_apps);
                            }
                            3 => {
                                ss.filter_show_uninstalled = !ss.filter_show_uninstalled;
                                ss.recompute_app_list(registered_apps);
                            }
                            n if n >= 4 && n < 4 + categories.len() => {
                                let cat_idx = n - 4;
                                if let Some(cat) = categories.get(cat_idx) {
                                    if ss.filter_categories.contains(cat) {
                                        ss.filter_categories.remove(cat);
                                    } else {
                                        ss.filter_categories.insert(cat.clone());
                                    }
                                    ss.recompute_app_list(registered_apps);
                                }
                            }
                            _ => {}
                        }
                    }
                }
                Action::Back => {
                    if ss.filter_panel_open {
                        ss.filter_panel_open = false;
                        ss.sort_dropdown_open = false;
                    } else {
                        ss.focus = AppStoreFocus::LeftPane;
                    }
                }
                _ => {}
            }
            return;
        }
        AppStoreFocus::SearchBar => {
            match action {
                Action::Back => {
                    if ss.search_query.is_empty() {
                        ss.focus = AppStoreFocus::LeftPane;
                    } else {
                        ss.search_query.pop();
                        ss.recompute_app_list(registered_apps);
                    }
                }
                Action::Enter | Action::Down => {
                    ss.focus = AppStoreFocus::BrowserButton;
                }
                _ => {}
            }
        }
        AppStoreFocus::BrowserButton => {
            match action {
                Action::Up => {
                    ss.focus = AppStoreFocus::SearchBar;
                }
                Action::Down => {
                    ss.focus = AppStoreFocus::FilterPanel;
                    ss.filter_panel_cursor = 0;
                }
                Action::Enter => {
                    open_awesome_ratatui_browser(ss);
                }
                Action::Back => {
                    ss.focus = AppStoreFocus::LeftPane;
                }
                _ => {}
            }
        }
        AppStoreFocus::LeftPane => {
            use crate::app_store::state::LeftRowKind;
            match action {
                Action::Up => {
                    if ss.left_cursor > 0 {
                        ss.left_cursor -= 1;
                        // Skip spacers
                        while ss.left_cursor > 0
                            && matches!(ss.left_visible_rows.get(ss.left_cursor), Some(LeftRowKind::Spacer))
                        {
                            ss.left_cursor -= 1;
                        }
                        ss.reset_right_pane();
                    } else {
                        ss.focus = AppStoreFocus::FilterPanel;
                        ss.filter_panel_cursor = ss.refresh_cursor_idx(registered_apps);
                    }
                }
                Action::Down => {
                    if ss.left_cursor < ss.left_visible_rows.len().saturating_sub(1) {
                        ss.left_cursor += 1;
                        // Skip spacers
                        while ss.left_cursor < ss.left_visible_rows.len().saturating_sub(1)
                            && matches!(ss.left_visible_rows.get(ss.left_cursor), Some(LeftRowKind::Spacer))
                        {
                            ss.left_cursor += 1;
                        }
                        ss.reset_right_pane();
                    }
                }
                Action::Enter | Action::Right => {
                    match ss.left_visible_rows.get(ss.left_cursor) {
                        Some(LeftRowKind::CategoryHeader(cat)) => {
                            let cat = cat.clone();
                            if ss.collapsed_store_categories.contains(&cat) {
                                ss.collapsed_store_categories.remove(&cat);
                            } else {
                                ss.collapsed_store_categories.insert(cat);
                            }
                            ss.recompute_app_list(registered_apps);
                        }
                        Some(LeftRowKind::App(_)) => {
                            ss.focus = AppStoreFocus::RightPane;
                            ss.in_right_actions = true;
                            ss.right_action_cursor = 0;
                            ss.right_scroll = 0;
                        }
                        _ => {}
                    }
                }
                Action::Back => {
                    if ss.ui_locked() {
                        ui_lock_popup(state);
                        return;
                    }
                    state.active_page = None;
                    logging::info("App Store: closed");
                    return;
                }
                _ => {}
            }
        }
        AppStoreFocus::RightPane => {
            if ss.in_right_actions {
                let status = ss.selected_app_key()
                    .and_then(|k| ss.install_statuses.get(k))
                    .cloned()
                    .unwrap_or(InstallStatus::NotInstalled);
                let app_meta = ss.selected_app_key().and_then(|k| registered_apps.get(k));
                let supports_embed = app_meta
                    .map(|m| app_store::actions::supports_embedded(Some(m)))
                    .unwrap_or(true);
                let supports_local = app_meta
                    .map(app_store::actions::supports_downloads_install)
                    .unwrap_or(false);
                let supports_global = app_meta
                    .map(app_store::actions::supports_path_install)
                    .unwrap_or(true);
                let is_approved = app_meta
                    .and_then(|m| m.get("approved"))
                    .and_then(|v| v.as_bool())
                    .unwrap_or(true);
                let count = app_store::page::action_count(
                    &status, supports_embed,
                    supports_local, supports_global,
                    is_approved,
                );
                match action {
                    Action::Up => {
                        // The top action is the ceiling; scroll the details instead
                        // so the metadata above it can still be read.
                        if ss.right_action_cursor > 0 {
                            ss.right_action_cursor -= 1;
                            ss.ensure_action_visible();
                        } else if ss.right_scroll > 0 {
                            ss.right_scroll -= 1;
                        }
                    }
                    Action::Down => {
                        if ss.right_action_cursor < count.saturating_sub(1) {
                            ss.right_action_cursor += 1;
                            ss.ensure_action_visible();
                        } else if ss.terminal_visible() {
                            // Past the last action, drop into the terminal below
                            ss.focus = AppStoreFocus::Terminal;
                        }
                    }
                    Action::Right => {
                        if ss.queue_visible() {
                            ss.focus = AppStoreFocus::Queue;
                            ss.queue.clamp_cursor();
                        }
                    }
                    Action::Enter => {
                        handle_appstore_action(state, registered_apps, active_installed_app);
                    }
                    Action::Back | Action::Left => {
                        ss.focus = AppStoreFocus::LeftPane;
                        ss.in_right_actions = false;
                    }
                    _ => {}
                }
            } else {
                match action {
                    Action::Down => {
                        ss.in_right_actions = true;
                        ss.right_action_cursor = 0;
                    }
                    Action::Up => {
                        if ss.right_scroll > 0 {
                            ss.right_scroll -= 1;
                        }
                    }
                    Action::Enter => {
                        ss.in_right_actions = true;
                        ss.right_action_cursor = 0;
                    }
                    Action::Back | Action::Left => {
                        ss.focus = AppStoreFocus::LeftPane;
                    }
                    _ => {}
                }
            }
        }
        AppStoreFocus::Terminal => {
            match action {
                Action::Up => {
                    ss.terminal_follow = false;
                    ss.terminal_scroll = ss.effective_terminal_scroll().saturating_sub(1);
                }
                Action::Down => {
                    let max = ss.terminal_max_scroll();
                    ss.terminal_scroll = (ss.effective_terminal_scroll() + 1).min(max);
                    ss.terminal_follow = ss.terminal_scroll >= max;
                }
                Action::Left => {
                    ss.focus = AppStoreFocus::LeftPane;
                    ss.in_right_actions = false;
                }
                Action::Right => {
                    if ss.queue_visible() {
                        ss.focus = AppStoreFocus::Queue;
                        ss.queue.clamp_cursor();
                    }
                }
                Action::Back => {
                    ss.focus = AppStoreFocus::RightPane;
                    ss.in_right_actions = true;
                }
                _ => {}
            }
        }
        AppStoreFocus::Queue => {
            let item_focused = ss.queue.item_focused;
            match action {
                Action::Up => {
                    if item_focused {
                        ss.queue.move_item(ss.queue.cursor, true);
                    } else if ss.queue.cursor > 0 {
                        ss.queue.cursor -= 1;
                    } else {
                        ss.focus = AppStoreFocus::RightPane;
                        ss.in_right_actions = true;
                        ss.ensure_action_visible();
                    }
                }
                Action::Down => {
                    if item_focused {
                        ss.queue.move_item(ss.queue.cursor, false);
                    } else if ss.queue.cursor + 1 < ss.queue.items.len() {
                        ss.queue.cursor += 1;
                    }
                }
                Action::Left | Action::Back => {
                    if item_focused {
                        ss.queue.item_focused = false;
                    } else if action == Action::Left {
                        ss.focus = AppStoreFocus::Terminal;
                    } else {
                        ss.focus = AppStoreFocus::RightPane;
                        ss.in_right_actions = true;
                    }
                }
                Action::Enter => {
                    if !item_focused {
                        if !ss.queue.items.is_empty() {
                            ss.queue.item_focused = true;
                        }
                    } else if let Some(item) = ss.queue.selected() {
                        // Pin the terminal to this job's log, or unpin when it is
                        // the one already being followed.
                        let id = item.id;
                        let running = ss.queue.running().map(|r| r.id) == Some(id);
                        ss.viewing_item = if running || ss.viewing_item == Some(id) {
                            None
                        } else {
                            Some(id)
                        };
                        ss.terminal_follow = ss.viewing_item.is_none();
                        ss.terminal_scroll = 0;
                        ss.focus = AppStoreFocus::Terminal;
                    }
                }
                _ => {}
            }
        }
        _ => {}
    }
}

fn handle_appstore_action(
    state: &mut TuiosState,
    registered_apps: &std::collections::HashMap<String, Value>,
    active_installed_app: &mut Option<InstalledDashboard>,
) {
    use crate::app_store::queue::QueueOp;
    use crate::app_store::state::{ConfirmAction, InstallLocation, InstallStatus};

    let ss = &mut state.app_store;
    let key = match ss.selected_app_key().cloned() {
        Some(k) => k,
        None => return,
    };
    let meta = match registered_apps.get(&key) {
        Some(m) => m,
        None => return,
    };
    let status = ss.install_statuses.get(&key).cloned().unwrap_or(InstallStatus::NotInstalled);
    let is_installed = !matches!(status, InstallStatus::NotInstalled);

    // For installed apps, index 0 = Run, then the rest shift by 1
    if is_installed && ss.right_action_cursor == 0 {
        if ss.ui_locked() {
            ui_lock_popup(state);
            return;
        }
        let category = meta.get("category").and_then(|v| v.as_str()).unwrap_or("Other");
        let launch_cmd = app_store::actions::resolve_launch_cmd(&key, category, Some(meta), None);

        if launch_cmd.is_empty() {
            logging::error(&format!("App Store: no runnable command found for {}", key));
            state.popup = Some((format!("No runnable binary found for {}", key), Instant::now()));
            return;
        }

        if app_store::actions::get_window_mode(&key, Some(meta)) == "fullscreen" {
            logging::info(&format!("App Store: launching {} in a new window", key));
            state.pending_foreground = Some(launch_cmd);
            return;
        }

        logging::info(&format!("App Store: launching {} in embedded tuiOS mode", key));
        *active_installed_app = InstalledDashboard::start(&key, &key, &launch_cmd, 80, 24);
        state.active_app = Some(key.clone());
        state.active_page = None;
        return;
    }

    // Offset cursor for installed apps (Run button takes index 0)
    let cursor = if is_installed { ss.right_action_cursor - 1 } else { ss.right_action_cursor };

    // Window mode toggle is the last button for installed apps that support embedded
    let supports_embed = app_store::actions::supports_embedded(Some(meta));
    if is_installed && supports_embed {
        let has_cross_install = match &status {
            InstallStatus::Global(_) => app_store::actions::supports_downloads_install(meta),
            InstallStatus::Local(_) => app_store::actions::supports_path_install(meta),
            _ => true,
        };
        let mode_cursor = match &status {
            InstallStatus::NotInstalled => usize::MAX,
            InstallStatus::Global(_) | InstallStatus::Local(_) => {
                if has_cross_install { 4 } else { 3 }
            }
            InstallStatus::Both(_, _) => 5,
        };
        if cursor == mode_cursor {
            let current = app_store::actions::get_window_mode(&key, Some(meta));
            let new_mode = if current == "fullscreen" { "embedded" } else { "fullscreen" };
            app_store::actions::set_window_mode(&key, new_mode);
            let label = app_store::actions::window_mode_label(new_mode);
            state.popup = Some((format!("Window mode set to {}", label), Instant::now()));
            return;
        }
    }

    match &status {
        InstallStatus::NotInstalled => {
            match cursor {
                0 => {
                    let repo = meta.get("repository").and_then(|v| v.as_str()).unwrap_or("");
                    if !repo.is_empty() { app_store::actions::open_url(repo); }
                }
                1 => {
                    let methods = app_store::actions::install_methods_for(meta);

                    if methods.len() == 1 {
                        let loc = methods[0];
                        enqueue_job(state, registered_apps, &key, QueueOp::Install(loc));
                        return;
                    } else {
                        ss.available_install_methods = methods;
                        ss.install_location_cursor = 0;
                        ss.install_dialog_note = None;
                        ss.install_location_dialog = true;
                    }
                }
                _ => {}
            }
        }
        InstallStatus::Global(path) => {
            let has_local_method = app_store::actions::supports_downloads_install(meta);

            // Buttons: Open Repo(0), Uninstall(1), [Install to Downloads(2) if supported], Open Location
            match cursor {
                0 => {
                    let repo = meta.get("repository").and_then(|v| v.as_str()).unwrap_or("");
                    if !repo.is_empty() { app_store::actions::open_url(repo); }
                }
                1 => {
                    ss.confirm_dialog = Some(ConfirmAction::UninstallGlobal(key.clone()));
                    ss.confirm_cursor = 1;
                }
                2 if has_local_method => {
                    enqueue_job(
                        state,
                        registered_apps,
                        &key,
                        QueueOp::Install(InstallLocation::Local),
                    );
                    return;
                }
                n => {
                    // Open Location is after the optional Install button
                    let open_idx = if has_local_method { 3 } else { 2 };
                    if n == open_idx {
                        let dir = app_store::actions::install_dir_from_path(path);
                        app_store::actions::open_in_os_explorer(&dir);
                    }
                }
            }
        }
        InstallStatus::Local(path) => {
            let has_global_method = app_store::actions::supports_path_install(meta);

            match cursor {
                0 => {
                    let repo = meta.get("repository").and_then(|v| v.as_str()).unwrap_or("");
                    if !repo.is_empty() { app_store::actions::open_url(repo); }
                }
                1 => {
                    ss.confirm_dialog = Some(ConfirmAction::UninstallLocal(key.clone()));
                    ss.confirm_cursor = 1;
                }
                2 if has_global_method => {
                    let methods: Vec<InstallLocation> = app_store::actions::install_methods_for(meta)
                        .into_iter()
                        .filter(|m| !matches!(m, InstallLocation::Local))
                        .collect();
                    if methods.len() > 1 {
                        ss.available_install_methods = methods;
                        ss.install_location_cursor = 0;
                        ss.install_dialog_note = None;
                        ss.install_location_dialog = true;
                    } else {
                        let loc = methods.first().copied().unwrap_or(InstallLocation::Global);
                        enqueue_job(state, registered_apps, &key, QueueOp::Install(loc));
                        return;
                    }
                }
                n => {
                    let open_idx = if has_global_method { 3 } else { 2 };
                    if n == open_idx {
                        let dir = app_store::actions::install_dir_from_path(path);
                        app_store::actions::open_in_os_explorer(&dir);
                    }
                }
            }
        }
        InstallStatus::Both(g_path, l_path) => {
            // Buttons: Open Repo(0), Uninstall(1), Set Source(2), Open PATH(3), Open Downloads(4)
            match cursor {
                0 => {
                    let repo = meta.get("repository").and_then(|v| v.as_str()).unwrap_or("");
                    if !repo.is_empty() { app_store::actions::open_url(repo); }
                }
                1 => {
                    ss.confirm_dialog = Some(ConfirmAction::UninstallChoose(key.clone()));
                    ss.confirm_cursor = 0;
                }
                2 => {
                    // Toggle run source between global and local
                    let current = app_store::actions::get_run_source(&key);
                    let new_source = if current == "global" { "local" } else { "global" };
                    app_store::actions::set_run_source(&key, new_source);
                    let label = if new_source == "local" { "Downloads (experimental)" } else { "PATH" };
                    state.popup = Some((format!("Default source set to {}", label), Instant::now()));
                }
                3 => {
                    let dir = app_store::actions::install_dir_from_path(g_path);
                    app_store::actions::open_in_os_explorer(&dir);
                }
                4 => {
                    let dir = app_store::actions::install_dir_from_path(l_path);
                    app_store::actions::open_in_os_explorer(&dir);
                }
                _ => {}
            }
        }
    }

    // Remove from App Store (for non-approved, not-installed apps — last button)
    let is_approved = meta.get("approved").and_then(|v| v.as_bool()).unwrap_or(true);
    if !is_approved && matches!(status, InstallStatus::NotInstalled) {
        let supports_embed = app_store::actions::supports_embedded(Some(meta));
        let remove_cursor = app_store::page::action_count(
            &status, supports_embed,
            app_store::actions::supports_downloads_install(meta),
            app_store::actions::supports_path_install(meta),
            is_approved,
        ) - 1;
        if ss.right_action_cursor == remove_cursor {
            let mut reg = registered_apps.clone();
            if crate::app_store::awesome_ratatui_manager::remove_from_registered(&key, &mut reg) {
                logging::info(&format!("App Store: removed '{}' (non-approved)", key));
                state.popup = Some((
                    format!("{} removed from App Store", key),
                    Instant::now(),
                ));
                state.needs_sync = true;
                ss.focus = crate::app_store::state::AppStoreFocus::LeftPane;
                ss.in_right_actions = false;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Install queue pump — finish the running job, then start the next one
// ---------------------------------------------------------------------------

/// Returns true when config registries changed and the nav dropdowns were rebuilt.
fn pump_install_queue(
    state: &mut TuiosState,
    registered_apps: &mut std::collections::HashMap<String, Value>,
    dashboards: &mut std::collections::HashMap<String, Value>,
    apps_registry: &mut std::collections::HashMap<String, Value>,
    nav_items: &mut Vec<NavItem>,
) -> bool {
    let mut registries_changed = false;

    if state.app_store.poll_operation() {
        registries_changed = finish_queue_item(state, registered_apps);
    }

    if state.app_store.queue.running_index().is_none() {
        if let Some(id) = state.app_store.start_next_job() {
            start_queue_item(state, registered_apps, id);
        }
    }

    // One summary when the whole queue drains, instead of a toast per job
    if !state.app_store.queue.is_active() {
        if let Some(msg) = state.app_store.queue.take_summary() {
            state.popup = Some((msg, Instant::now()));
        }
    }

    // The terminal and queue panes are gone once the queue is empty
    if state.app_store.queue.is_empty()
        && matches!(
            state.app_store.focus,
            crate::app_store::state::AppStoreFocus::Terminal
                | crate::app_store::state::AppStoreFocus::Queue
        )
    {
        state.app_store.focus = crate::app_store::state::AppStoreFocus::RightPane;
        state.app_store.in_right_actions = true;
    }

    if registries_changed {
        *dashboards = registry::load_dashboards();
        *apps_registry = registry::load_apps();
        *nav_items = build_nav_items(dashboards, apps_registry);
    }
    registries_changed
}

fn start_queue_item(
    state: &mut TuiosState,
    registered_apps: &mut std::collections::HashMap<String, Value>,
    id: u64,
) {
    use crate::app_store::queue::{QueueOp, QueueStatus};

    let (key, op, label) = match state.app_store.queue.item_by_id(id) {
        Some(i) => (i.key.clone(), i.op, i.label.clone()),
        None => return,
    };

    // The app may have been registered from the browser after the job was queued
    if !registered_apps.contains_key(&key) {
        *registered_apps = registry::load_registered_apps();
    }
    let meta = match registered_apps.get(&key) {
        Some(m) => m.clone(),
        None => {
            // Removed from the App Store while it sat in the queue
            if let Some(item) = state.app_store.queue.item_by_id_mut(id) {
                item.status = QueueStatus::Failed;
                item.note = Some("App is no longer in the App Store".to_string());
                item.push_log(format!("✗ {} is no longer registered — skipped", label));
            }
            logging::error(&format!("App Store: queued job for unknown app '{}'", key));
            return;
        }
    };

    let output = state.app_store.thread_output.clone();
    let done = state.app_store.thread_done.clone();
    let ok_flag = state.app_store.thread_success.clone();
    let warning = state.app_store.thread_warning.clone();

    if let Some(item) = state.app_store.queue.item_by_id_mut(id) {
        item.push_log(format!(
            "▸ {}ing {} {} {}",
            op.verb(),
            label,
            op.arrow(),
            op.target_label()
        ));
    }

    match op {
        QueueOp::Install(location) => {
            logging::info(&format!(
                "App Store: starting install of {} ({})",
                key,
                op.target_label()
            ));
            app_store::actions::spawn_install(location, &key, &meta, output, done, ok_flag);
        }
        QueueOp::Uninstall { path, downloads } => {
            let package = app_store::actions::resolve_package_name(&key, &meta);
            let candidates = app_store::actions::binary_candidates(&key, &meta);
            let category = meta
                .get("category")
                .and_then(|v| v.as_str())
                .unwrap_or("Other")
                .to_string();
            logging::info(&format!(
                "App Store: starting uninstall of {} ({})",
                key,
                op.target_label()
            ));
            app_store::actions::spawn_uninstall(
                &key, &package, &category, candidates, path, downloads, output, done, ok_flag,
                warning,
            );
        }
    }
}

/// Apply the result of the job that just finished. Returns true if apps.json /
/// dashboards.json changed and the nav dropdowns need rebuilding.
fn finish_queue_item(
    state: &mut TuiosState,
    registered_apps: &mut std::collections::HashMap<String, Value>,
) -> bool {
    use crate::app_store::queue::{QueueOp, QueueStatus};
    use crate::app_store::state::{InstallLocation, InstallStatus};

    let idx = match state.app_store.queue.running_index() {
        Some(i) => i,
        None => return false,
    };
    let id = state.app_store.queue.items[idx].id;
    let key = state.app_store.queue.items[idx].key.clone();
    let label = state.app_store.queue.items[idx].label.clone();
    let op = state.app_store.queue.items[idx].op;

    let succeeded = state
        .app_store
        .thread_success
        .load(std::sync::atomic::Ordering::Relaxed);
    let warning = state
        .app_store
        .thread_warning
        .lock()
        .ok()
        .and_then(|w| w.clone());

    let mut log_extra: Vec<String> = Vec::new();
    let mut note: Option<String> = None;

    match op {
        QueueOp::Install(location) => {
            if succeeded {
                if !registered_apps.contains_key(&key) {
                    *registered_apps = registry::load_registered_apps();
                }

                // The produced binary is often not named after the crate
                // (connect-four -> play), so resolve it from what actually appeared.
                let job_log: Vec<String> = state
                    .app_store
                    .queue
                    .item_by_id(id)
                    .map(|i| i.log.clone())
                    .unwrap_or_default();
                let discovered = match registered_apps.get(&key) {
                    Some(meta) => match location {
                        InstallLocation::Local => {
                            app_store::actions::resolve_built_binary(&key, meta)
                        }
                        _ => app_store::actions::resolve_installed_binary(
                            &key,
                            meta,
                            &state.app_store.pre_install_bins,
                            &job_log,
                        ),
                    },
                    None => None,
                };
                if let Some(bin) = &discovered {
                    app_store::actions::record_binary_name(&key, bin);
                    *registered_apps = registry::load_registered_apps();
                    log_extra.push(format!("Run command for {}: {}", key, bin));
                }

                if let Some(meta) = registered_apps.get(&key) {
                    app_store::actions::add_to_config(&key, meta, &location);
                    let from = app_store::actions::install_source_label(&location, &key, meta);
                    let to = app_store::actions::install_destination(&key, meta, &location);
                    logging::info(&format!(
                        "App Store: installed {} from {} to {}",
                        key, from, to
                    ));
                    log_extra.push(format!("Installed from: {}", from));
                    log_extra.push(format!("Installed to:   {}", to));
                }

                // If it landed on PATH, make that the default run source
                if matches!(location, InstallLocation::Global | InstallLocation::Git) {
                    if app_store::actions::get_run_source(&key) == "local" {
                        app_store::actions::set_run_source(&key, "global");
                    }
                }
                state.app_store.failed_installs.remove(&key);
            } else {
                logging::error(&format!("App Store: install failed for {}", key));
                state.app_store.failed_installs.insert(key.clone());
                note = Some(format!("Install to {} failed", op.target_label()));
            }
        }
        QueueOp::Uninstall { .. } => {
            if let Some(msg) = &warning {
                note = Some(msg.clone());
            } else if !succeeded {
                note = Some("Uninstall reported errors — see the log".to_string());
            }
            logging::info(&format!("App Store: uninstalled {} ({})", key, op.target_label()));

            // If the removed copy was the active run source, fall back to the other
            let current_source = app_store::actions::get_run_source(&key);
            let new_status_check = app_store::actions::get_install_status(
                &key,
                registered_apps.get(&key).unwrap_or(&serde_json::Value::Null),
            );
            match new_status_check {
                InstallStatus::Global(_) if current_source == "local" => {
                    app_store::actions::set_run_source(&key, "global");
                }
                InstallStatus::Local(_) if current_source == "global" => {
                    app_store::actions::set_run_source(&key, "local");
                }
                _ => {}
            }
        }
    }

    if let Some(item) = state.app_store.queue.item_by_id_mut(id) {
        item.extend_log(log_extra);
        item.status = if succeeded { QueueStatus::Done } else { QueueStatus::Failed };
        // A locked-file warning is a partial success worth surfacing on the row
        if succeeded && warning.is_some() {
            item.note = warning.clone();
        } else {
            item.note = note;
        }
        let verdict = if succeeded {
            format!("✓ {} {} complete", op.verb(), label)
        } else {
            format!("✗ {} {} failed", op.verb(), label)
        };
        item.push_log(verdict);
    }

    state.app_store.install_statuses = app_store::actions::refresh_all_statuses(registered_apps);
    state.app_store.recompute_app_list(registered_apps);
    state.app_store.browser.probe_cache.clear();

    // Clamp the action cursor — the button list changes with the install status
    let new_status = state
        .app_store
        .selected_app_key()
        .and_then(|k| state.app_store.install_statuses.get(k))
        .cloned()
        .unwrap_or(InstallStatus::NotInstalled);
    let new_meta = state
        .app_store
        .selected_app_key()
        .and_then(|k| registered_apps.get(k));
    let new_count = app_store::page::action_count(
        &new_status,
        new_meta
            .map(|m| app_store::actions::supports_embedded(Some(m)))
            .unwrap_or(true),
        new_meta
            .map(app_store::actions::supports_downloads_install)
            .unwrap_or(false),
        new_meta
            .map(app_store::actions::supports_path_install)
            .unwrap_or(true),
        new_meta
            .and_then(|m| m.get("approved"))
            .and_then(|v| v.as_bool())
            .unwrap_or(true),
    );
    if state.app_store.right_action_cursor >= new_count {
        state.app_store.right_action_cursor = new_count.saturating_sub(1);
    }

    true
}

// ---------------------------------------------------------------------------
// Awesome Ratatui browser — open, navigate, add/remove apps
// ---------------------------------------------------------------------------

fn open_awesome_ratatui_browser(ss: &mut crate::app_store::state::AppStoreState) {
    use crate::app_store::awesome_ratatui_manager;
    use crate::utilities::logging;

    ss.browser.active = true;
    ss.browser.loading = true;
    ss.browser.error = None;
    ss.browser.focus = awesome_ratatui_manager::BrowserFocus::List;
    ss.browser.cursor = 0;
    ss.browser.scroll = 0;
    ss.browser.action_cursor = 0;

    match awesome_ratatui_manager::load_or_fetch() {
        Ok((apps, ts)) => {
            let cats = awesome_ratatui_manager::build_categories(&apps);
            ss.browser.apps = apps;
            ss.browser.categories = cats;
            ss.browser.fetched_at = Some(ts);
            ss.browser.loading = false;
            ss.browser.recompute_visible();
            ss.browser.probe_cache.clear();
            ss.browser.probe_selected();
            logging::info(&format!(
                "Awesome Ratatui browser: opened with {} apps",
                ss.browser.apps.len()
            ));
        }
        Err(e) => {
            ss.browser.loading = false;
            ss.browser.error = Some(e.clone());
            logging::error(&format!("Awesome Ratatui browser: {}", e));
        }
    }
}

fn handle_browser_key(
    action: Action,
    state: &mut TuiosState,
    registered_apps: &std::collections::HashMap<String, Value>,
) {
    use crate::app_store::awesome_ratatui_manager::{self, BrowserFocus, RowKind};

    let ss = &mut state.app_store;

    match ss.browser.focus {
        BrowserFocus::SearchBar => {
            match action {
                Action::Back => {
                    if ss.browser.search_query.is_empty() {
                        ss.browser.focus = BrowserFocus::List;
                    } else {
                        ss.browser.search_query.pop();
                        ss.browser.recompute_visible();
                    }
                }
                Action::Enter | Action::Down => {
                    ss.browser.focus = BrowserFocus::RepoButton;
                }
                _ => {}
            }
        }
        BrowserFocus::RepoButton => {
            match action {
                Action::Up => {
                    ss.browser.focus = BrowserFocus::SearchBar;
                }
                Action::Down => {
                    ss.browser.focus = BrowserFocus::DescriptionToggle;
                }
                Action::Enter => {
                    app_store::actions::open_url(awesome_ratatui_manager::REPO_URL);
                }
                Action::Back => {
                    ss.browser.focus = BrowserFocus::List;
                }
                _ => {}
            }
        }
        BrowserFocus::DescriptionToggle => {
            match action {
                Action::Up => {
                    ss.browser.focus = BrowserFocus::RepoButton;
                }
                Action::Down => {
                    ss.browser.focus = BrowserFocus::List;
                    ss.browser.cursor = 0;
                    // Skip spacers
                    while matches!(ss.browser.visible_rows.get(ss.browser.cursor), Some(RowKind::Spacer)) {
                        ss.browser.cursor += 1;
                    }
                }
                Action::Enter => {
                    ss.browser.show_descriptions = !ss.browser.show_descriptions;
                }
                Action::Back => {
                    ss.browser.focus = BrowserFocus::List;
                }
                _ => {}
            }
        }
        BrowserFocus::List => {
            match action {
                Action::Up => {
                    if ss.browser.cursor > 0 {
                        ss.browser.cursor -= 1;
                        // Skip spacers
                        while ss.browser.cursor > 0
                            && matches!(ss.browser.visible_rows.get(ss.browser.cursor), Some(RowKind::Spacer))
                        {
                            ss.browser.cursor -= 1;
                        }
                        if ss.browser.cursor < ss.browser.scroll {
                            ss.browser.scroll = ss.browser.cursor;
                        }
                        ss.browser.probe_selected();
                    } else {
                        ss.browser.focus = BrowserFocus::DescriptionToggle;
                    }
                }
                Action::Down => {
                    if ss.browser.cursor < ss.browser.visible_rows.len().saturating_sub(1) {
                        ss.browser.cursor += 1;
                        // Skip spacers
                        while ss.browser.cursor < ss.browser.visible_rows.len().saturating_sub(1)
                            && matches!(ss.browser.visible_rows.get(ss.browser.cursor), Some(RowKind::Spacer))
                        {
                            ss.browser.cursor += 1;
                        }
                        let visible_h = 20usize;
                        if ss.browser.cursor >= ss.browser.scroll + visible_h {
                            ss.browser.scroll = ss.browser.cursor.saturating_sub(visible_h - 1);
                        }
                        ss.browser.probe_selected();
                    }
                }
                Action::Enter | Action::Right => {
                    if let Some(row) = ss.browser.visible_rows.get(ss.browser.cursor).cloned() {
                        match row {
                            RowKind::CategoryHeader(ref cat) => {
                                if ss.browser.collapsed.contains(cat) {
                                    ss.browser.collapsed.remove(cat);
                                } else {
                                    ss.browser.collapsed.insert(cat.clone());
                                }
                                ss.browser.recompute_visible();
                            }
                            RowKind::App(_) => {
                                ss.browser.probe_selected();
                                ss.browser.focus = BrowserFocus::Actions;
                                ss.browser.action_cursor = 0;
                            }
                            RowKind::Spacer => {}
                        }
                    }
                }
                Action::Back | Action::Left => {
                    ss.browser.active = false;
                    ss.browser.search_query.clear();
                }
                _ => {}
            }
        }
        BrowserFocus::Actions => {
            let app = ss.browser.selected_app().cloned();
            let app = match app {
                Some(a) => a,
                None => {
                    ss.browser.focus = BrowserFocus::List;
                    return;
                }
            };
            let key = awesome_ratatui_manager::repo_to_key(&app.repo_url);
            let is_reg = registered_apps.contains_key(&key);
            let is_inst = matches!(
                ss.browser.status_for(&key, &ss.install_statuses),
                Some(s) if !matches!(s, crate::app_store::state::InstallStatus::NotInstalled)
            );
            let max_actions = app_store::awesome_ratatui_page::build_browser_actions(is_reg, is_inst).len();

            match action {
                Action::Up => {
                    if ss.browser.action_cursor > 0 {
                        ss.browser.action_cursor -= 1;
                    }
                }
                Action::Down => {
                    if ss.browser.action_cursor < max_actions.saturating_sub(1) {
                        ss.browser.action_cursor += 1;
                    }
                }
                Action::Enter => {
                    handle_browser_action(state, registered_apps);
                }
                Action::Back | Action::Left => {
                    ss.browser.focus = BrowserFocus::List;
                    ss.browser.action_cursor = 0;
                }
                _ => {}
            }
        }
    }
}

fn handle_browser_action(
    state: &mut TuiosState,
    registered_apps: &std::collections::HashMap<String, Value>,
) {
    use crate::app_store::awesome_ratatui_manager;
    use crate::utilities::logging;

    let ss = &mut state.app_store;
    let app = match ss.browser.selected_app().cloned() {
        Some(a) => a,
        None => return,
    };
    let key = awesome_ratatui_manager::repo_to_key(&app.repo_url);
    let is_reg = registered_apps.contains_key(&key);
    let is_inst = matches!(
        ss.browser.status_for(&key, &ss.install_statuses),
        Some(s) if !matches!(s, crate::app_store::state::InstallStatus::NotInstalled)
    );

    let actions = app_store::awesome_ratatui_page::build_browser_actions(is_reg, is_inst);
    let action_label = match actions.get(ss.browser.action_cursor) {
        Some(l) => l.clone(),
        None => return,
    };

    match action_label.as_str() {
        "Open Repository" => {
            if !app.repo_url.is_empty() {
                app_store::actions::open_url(&app.repo_url);
            }
        }
        "Add to App Store" => {
            awesome_ratatui_manager::add_to_registered(&app);
            logging::info(&format!("Awesome Ratatui: added '{}' to app store", app.name));
            let msg = if is_inst {
                format!("{} added to App Store (already installed)", app.name)
            } else {
                format!("{} added to App Store", app.name)
            };
            state.popup = Some((msg, std::time::Instant::now()));
            state.needs_sync = true;
            ss.browser.focus = awesome_ratatui_manager::BrowserFocus::List;
            ss.browser.action_cursor = 0;
        }
        "Add to App Store & Install" => {
            let added_key = awesome_ratatui_manager::add_to_registered(&app);
            logging::info(&format!("Awesome Ratatui: added '{}' to app store, prompting install", app.name));
            state.needs_sync = true;
            // Open install location dialog
            ss.browser.active = false;
            ss.browser.search_query.clear();
            ss.install_location_dialog = true;
            ss.install_location_cursor = 0;
            // Shown inside the dialog rather than as a toast, so the two do not overlap
            ss.install_dialog_note = Some(format!("✓ {} added to the App Store", app.name));
            ss.available_install_methods =
                app_store::actions::install_methods_for(&awesome_ratatui_manager::to_registered_value(&app));
            // The left-pane cursor still points at the previously selected app, and
            // `registered_apps` here predates the insert — pin the target explicitly.
            ss.pending_install_key = Some(added_key);
        }
        "Remove from App Store" => {
            let mut reg = registered_apps.clone();
            if awesome_ratatui_manager::remove_from_registered(&key, &mut reg) {
                logging::info(&format!("Awesome Ratatui: removed '{}' from app store", app.name));
                state.popup = Some((
                    format!("{} removed from App Store", app.name),
                    std::time::Instant::now(),
                ));
                state.needs_sync = true;
            }
            ss.browser.focus = awesome_ratatui_manager::BrowserFocus::List;
            ss.browser.action_cursor = 0;
        }
        "Open in App Store" => {
            // Close browser and navigate to the app in the main app store
            let target_key = awesome_ratatui_manager::repo_to_key(&app.repo_url);
            ss.browser.active = false;
            ss.browser.search_query.clear();
            ss.recompute_app_list(registered_apps);
            if let Some(pos) = ss.computed_app_list.iter().position(|k| k == &target_key) {
                ss.left_cursor = pos;
            }
            ss.focus = crate::app_store::state::AppStoreFocus::RightPane;
            ss.in_right_actions = true;
            ss.right_action_cursor = 0;
        }
        _ => {}
    }
}

// ---------------------------------------------------------------------------
// Restart is handled by returning from the main loop and re-entering it.
// No exec or spawn needed — clean terminal restore between cycles.
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Foreground subprocess execution (takes over terminal)
// ---------------------------------------------------------------------------

fn run_foreground(cmd: &[String]) {
    if cmd.is_empty() {
        return;
    }
    let _ = Command::new(&cmd[0]).args(&cmd[1..]).status();
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

pub fn main() {
    loop {
        let should_restart = run_app();
        if !should_restart {
            break;
        }
    }
}

/// Run the tuiOS app. Returns `true` if the user requested a restart.
fn run_app() -> bool {
    std::fs::create_dir_all("logs").ok();

    logging::init();
    logging::info("tuiOS started");

    let settings_map = registry::load_settings();
    let mut registered_apps = registry::load_registered_apps();

    // Sync config files with actual installs on disk/PATH before loading
    app_store::actions::sync_installed_apps(&registered_apps);

    let mut dashboards = registry::load_dashboards();
    let mut apps_registry = registry::load_apps();

    let default_dashboard = settings_map
        .get("default_dashboard")
        .and_then(|v| v.as_str())
        .unwrap_or("Dashboard-1")
        .to_string();

    let mut state = TuiosState::new(default_dashboard.clone());

    // Initialize app store: compute install statuses and app list
    state.app_store.install_statuses = app_store::actions::refresh_all_statuses(&registered_apps);
    state.app_store.recompute_app_list(&registered_apps);

    let mut nav_items = build_nav_items(&dashboards, &apps_registry);
    let mut active_internal_app: Option<InternalApp> = None;
    let mut active_installed_dash: Option<InstalledDashboard> = None;
    let mut active_installed_app: Option<InstalledDashboard> = None;

    // Auto-launch default dashboard if it's an installed (third-party) one
    if let Some(meta) = dashboards.get(&default_dashboard) {
        let dash_type = meta.get("type").and_then(|t| t.as_str()).unwrap_or("");
        if dash_type == "installed" {
            let source = meta.get("source").and_then(|s| s.as_str()).unwrap_or("global");
            let cmd: Vec<String> = if source == "local" {
                let local_bin = format!("downloads/dashboards/{}/target/release/{}", default_dashboard, default_dashboard);
                if std::path::Path::new(&local_bin).exists() {
                    vec![local_bin]
                } else {
                    meta.get("cmd").and_then(|c| c.as_array())
                        .map(|arr| arr.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect())
                        .unwrap_or_default()
                }
            } else {
                meta.get("cmd").and_then(|c| c.as_array())
                    .map(|arr| arr.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect())
                    .unwrap_or_default()
            };
            let label = meta.get("label").and_then(|v| v.as_str()).unwrap_or(&default_dashboard).to_string();
            if !cmd.is_empty() {
                active_installed_dash = InstalledDashboard::start(&default_dashboard, &label, &cmd, 80, 24);
            }
        }
    }

    let mut should_restart = false;

    // Setup terminal
    enable_raw_mode().expect("Failed to enable raw mode");
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen).expect("Failed to enter alternate screen");
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend).expect("Failed to create terminal");

    {
        let s = settings::persistence::load();
        if settings::persistence::get_bool(&s, "appearance.intro_animation_enabled", true) {
            let accent = settings::pages::appearance::color_from_name(
                &settings::persistence::get_str(&s, "appearance.accent_color", "Cyan"),
            );
            let style = settings::pages::boot::style(&s);
            let colour = settings::pages::boot::colour(&s);
            let logo = settings::pages::boot::logo(&s);
            let reveal = settings::pages::boot::direction(&s);
            let duration = settings::pages::boot::duration(&s);
            let _ = boot_animation::play(
                &mut terminal,
                accent,
                boot_animation::Options {
                    style: &style,
                    colour: &colour,
                    logo: &logo,
                    reveal: &reveal,
                    duration: &duration,
                    effect: settings::pages::boot::effect(&s),
                },
            );
        }
    }

    loop {
        // Drive the install queue: finish the running job, then start the next one
        let _ = pump_install_queue(
            &mut state,
            &mut registered_apps,
            &mut dashboards,
            &mut apps_registry,
            &mut nav_items,
        );

        // Handle deferred sync request (triggered when opening app store or refresh button)
        if state.needs_sync {
            let show_popup = matches!(state.active_page.as_deref(), Some("appstore"))
                && state.app_store.focus == crate::app_store::state::AppStoreFocus::FilterPanel;
            state.needs_sync = false;
            registered_apps = registry::load_registered_apps();
            let changed = app_store::actions::sync_installed_apps(&registered_apps);
            registered_apps = registry::load_registered_apps();
            state.app_store.install_statuses = app_store::actions::refresh_all_statuses(&registered_apps);
            state.app_store.recompute_app_list(&registered_apps);
            state.app_store.browser.probe_cache.clear();
            if changed {
                dashboards = registry::load_dashboards();
                apps_registry = registry::load_apps();
                nav_items = build_nav_items(&dashboards, &apps_registry);
            }
            if show_popup {
                if changed {
                    state.popup = Some(("App list synced with installed apps".to_string(), Instant::now()));
                } else {
                    state.popup = Some(("App list is up to date".to_string(), Instant::now()));
                }
            } else if changed && !state.app_store.install_location_dialog {
                // Don't toast over the install dialog
                state.popup = Some(("App list synced with installed apps".to_string(), Instant::now()));
            }
        }

        // Draw UI
        terminal
            .draw(|frame| {
                let (status_bar_on, navbar_bottom) = {
                    let s = crate::settings::persistence::load();
                    (
                        crate::settings::persistence::get_bool(&s, "appearance.status_bar_enabled", false),
                        crate::settings::persistence::get_str(&s, "appearance.navbar_position", "Top") == "Bottom",
                    )
                };
                // Row order: navbar and main swap depending on navbar_position; status bar is always last
                let mut constraints: Vec<Constraint> = if navbar_bottom {
                    vec![Constraint::Fill(1), Constraint::Length(1)]
                } else {
                    vec![Constraint::Length(1), Constraint::Fill(1)]
                };
                if status_bar_on {
                    constraints.push(Constraint::Length(1));
                }
                let rows = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints(constraints)
                    .split(frame.area());
                let (nav_row, main_row) = if navbar_bottom {
                    (rows[1], rows[0])
                } else {
                    (rows[0], rows[1])
                };

                // 1. Draw main container
                render_main(frame, main_row, &mut state, &mut active_internal_app, &mut active_installed_dash, &mut active_installed_app, &registered_apps);

                // 2. Draw navbar label row
                render_navbar(frame, nav_row, &nav_items, &state);

                // 3. Draw status bar if enabled
                if status_bar_on {
                    render_status_bar(frame, rows[2]);
                }

                // 4. Draw dropdown LAST (overlays main container)
                if state.nav_expanded {
                    render_dropdown(frame, main_row, &nav_items, &state, navbar_bottom);
                }

                // 4. Draw popup notification if active
                if let Some((ref msg, ref created)) = state.popup {
                    // Longer messages need longer on screen to be readable
                    let secs = 3 + (msg.chars().count() as u64 / 40).min(7);
                    if created.elapsed() < Duration::from_secs(secs) {
                        let area = frame.area();
                        let max_width = area.width.saturating_sub(8).max(24);
                        let lines = app_store::page::wrap_text(msg, max_width.saturating_sub(4) as usize);
                        let text_width = lines
                            .iter()
                            .map(|l| l.chars().count())
                            .max()
                            .unwrap_or(0) as u16;
                        let popup_width = (text_width + 4).min(max_width);
                        let popup_height = (lines.len() as u16 + 2).min(area.height);
                        let popup_rect = Rect {
                            x: area.width.saturating_sub(popup_width) / 2,
                            y: area.height.saturating_sub(popup_height) / 2,
                            width: popup_width,
                            height: popup_height,
                        };
                        let body = lines
                            .iter()
                            .map(|l| format!("  {}", l))
                            .collect::<Vec<_>>()
                            .join("\n");
                        frame.render_widget(Clear, popup_rect);
                        frame.render_widget(
                            Paragraph::new(Text::raw(body))
                                .style(Style::default().fg(Color::White).bg(Color::DarkGray))
                                .block(Block::default().borders(Borders::ALL).style(Style::default().bg(Color::DarkGray))),
                            popup_rect,
                        );
                    }
                }

                // 5. Draw run source chooser dialog if active
                if let Some((ref app_name, _, _, cursor)) = state.run_source_dialog {
                    let title = format!("Run {} from:", app_name);
                    let width = (title.len() + 10).max(36) as u16;
                    let height = 6u16;
                    let area = frame.area();
                    let popup_rect = Rect {
                        x: area.width.saturating_sub(width) / 2,
                        y: area.height.saturating_sub(height) / 2,
                        width: width.min(area.width),
                        height,
                    };
                    frame.render_widget(Clear, popup_rect);
                    frame.render_widget(
                        Block::default()
                            .borders(Borders::ALL)
                            .title(" Choose Run Source ")
                            .style(Style::default().fg(Color::Cyan).bg(Color::Black)),
                        popup_rect,
                    );
                    let inner = popup_rect.inner(Margin { horizontal: 1, vertical: 1 });
                    let path_style = if cursor == 0 {
                        Style::default().fg(Color::Black).bg(Color::Cyan)
                    } else {
                        Style::default().fg(Color::White)
                    };
                    let local_style = if cursor == 1 {
                        Style::default().fg(Color::Black).bg(Color::Cyan)
                    } else {
                        Style::default().fg(Color::White)
                    };
                    let lines = vec![
                        ratatui::text::Line::from(ratatui::text::Span::styled(&title, Style::default().fg(Color::White))),
                        ratatui::text::Line::from(""),
                        ratatui::text::Line::from(ratatui::text::Span::styled(
                            if cursor == 0 { "  » PATH (global install)" } else { "    PATH (global install)" },
                            path_style,
                        )),
                        ratatui::text::Line::from(ratatui::text::Span::styled(
                            if cursor == 1 { "  » Downloads (local build)" } else { "    Downloads (local build)" },
                            local_style,
                        )),
                    ];
                    frame.render_widget(Paragraph::new(lines), inner);
                }
            })
            .expect("Failed to draw frame");

        // Clear expired popup
        if let Some((_, created)) = &state.popup {
            if created.elapsed() >= Duration::from_secs(3) {
                state.popup = None;
            }
        }

        // Poll for events with timeout
        if !event::poll(Duration::from_millis(100)).unwrap_or(false) {
            continue;
        }

        let ev = match event::read() {
            Ok(ev) => ev,
            Err(_) => continue,
        };

        // Only handle key press events (not release/repeat)
        let key_event = match ev {
            Event::Key(key) if key.kind == KeyEventKind::Press => key,
            _ => continue,
        };

        // ---- Intercept: Run source chooser dialog ----
        if state.run_source_dialog.is_some() {
            match key_event.code {
                crossterm::event::KeyCode::Up | crossterm::event::KeyCode::Char('w') => {
                    if let Some((_, _, _, ref mut c)) = state.run_source_dialog { *c = 0; }
                }
                crossterm::event::KeyCode::Down | crossterm::event::KeyCode::Char('s') => {
                    if let Some((_, _, _, ref mut c)) = state.run_source_dialog { *c = 1; }
                }
                crossterm::event::KeyCode::Enter | crossterm::event::KeyCode::Char('e') => {
                    if let Some((name, global_cmd, local_cmd, cursor)) = state.run_source_dialog.take() {
                        let cmd = if cursor == 0 { global_cmd } else { local_cmd };
                        if !cmd.is_empty() {
                            process_manager::launch(&name, &cmd);
                        }
                        state.active_app = Some(name);
                        state.active_page = None;
                        state.focus = FocusTarget::Main;
                    }
                }
                crossterm::event::KeyCode::Esc | crossterm::event::KeyCode::Char('q') | crossterm::event::KeyCode::Backspace => {
                    state.run_source_dialog = None;
                }
                _ => {}
            }
            continue;
        }

        // ---- Intercept: Settings awaiting_key mode (captures ANY key) ----
        if state.focus == FocusTarget::Main
            && matches!(state.active_page.as_deref(), Some("settings"))
            && state.settings.awaiting_key
        {
            let key_name = match key_event.code {
                crossterm::event::KeyCode::Tab => "Tab",
                crossterm::event::KeyCode::Enter => "Enter",
                crossterm::event::KeyCode::Esc => "Esc",
                crossterm::event::KeyCode::Backspace => "Backspace",
                crossterm::event::KeyCode::Up => "Up",
                crossterm::event::KeyCode::Down => "Down",
                crossterm::event::KeyCode::Left => "Left",
                crossterm::event::KeyCode::Right => "Right",
                crossterm::event::KeyCode::Char(ch) => {
                    match ch {
                        'a' => "A", 'b' => "B", 'c' => "C", 'd' => "D",
                        'e' => "E", 'f' => "F", 'g' => "G", 'h' => "H",
                        'i' => "I", 'j' => "J", 'k' => "K", 'l' => "L",
                        'm' => "M", 'n' => "N", 'o' => "O", 'p' => "P",
                        'q' => "Q", 'r' => "R", 's' => "S", 't' => "T",
                        'u' => "U", 'v' => "V", 'w' => "W", 'x' => "X",
                        'y' => "Y", 'z' => "Z", '0' => "0", '1' => "1",
                        '2' => "2", '3' => "3", '4' => "4", '5' => "5",
                        '6' => "6", '7' => "7", '8' => "8", '9' => "9",
                        _ => "Tab",
                    }
                }
                _ => "Tab",
            };
            crate::settings::pages::button_mapping::capture_key(&mut state.settings, key_name);
            continue;
        }

        // ---- Intercept: Shift+Tab for terminal output focus (Scripts & Git) ----
        if matches!(key_event.code, crossterm::event::KeyCode::BackTab)
            && state.focus == FocusTarget::Main
            && matches!(state.active_page.as_deref(), Some("settings"))
            && matches!(state.settings.selected_category(), SettingsCategory::Git)
        {
            state.settings.terminal_focused = !state.settings.terminal_focused;
            continue;
        }

        // ---- Intercept: Shift+Tab cycles Details -> Terminal -> Queue ----
        if matches!(key_event.code, crossterm::event::KeyCode::BackTab)
            && state.focus == FocusTarget::Main
            && matches!(state.active_page.as_deref(), Some("appstore"))
            && !state.app_store.browser.active
            && state.app_store.queue_visible()
        {
            use crate::app_store::state::AppStoreFocus;
            let ss = &mut state.app_store;
            ss.focus = match ss.focus {
                AppStoreFocus::Terminal => AppStoreFocus::Queue,
                AppStoreFocus::Queue => {
                    ss.in_right_actions = true;
                    AppStoreFocus::RightPane
                }
                _ => AppStoreFocus::Terminal,
            };
            ss.queue.clamp_cursor();
            continue;
        }

        // ---- Intercept: per-job install queue commands (selected row only) ----
        if state.focus == FocusTarget::Main
            && matches!(state.active_page.as_deref(), Some("appstore"))
            && state.app_store.focus == crate::app_store::state::AppStoreFocus::Queue
            && state.app_store.queue.item_focused
            && state.app_store.confirm_dialog.is_none()
            && !state.app_store.install_location_dialog
        {
            use crossterm::event::KeyCode;
            let cursor = state.app_store.queue.cursor;
            match key_event.code {
                KeyCode::Char('c') | KeyCode::Char('C') => {
                    let msg = match state.app_store.queue.cancel(cursor) {
                        Ok(m) | Err(m) => m,
                    };
                    state.app_store.queue.clamp_cursor();
                    state.popup = Some((msg, Instant::now()));
                    continue;
                }
                KeyCode::Char('r') | KeyCode::Char('R') => {
                    let msg = match state.app_store.queue.retry(cursor) {
                        Ok(m) | Err(m) => m,
                    };
                    state.app_store.queue.item_focused = false;
                    state.popup = Some((msg, Instant::now()));
                    continue;
                }
                _ => {}
            }
        }

        // ---- Intercept: X asks before clearing the install queue ----
        if state.focus == FocusTarget::Main
            && matches!(state.active_page.as_deref(), Some("appstore"))
            && state.app_store.focus == crate::app_store::state::AppStoreFocus::Queue
            && state.app_store.confirm_dialog.is_none()
            && !state.app_store.install_location_dialog
            && matches!(
                key_event.code,
                crossterm::event::KeyCode::Char('x') | crossterm::event::KeyCode::Char('X')
            )
        {
            if state.app_store.queue.is_empty() {
                state.popup = Some(("The queue is already empty".to_string(), Instant::now()));
            } else {
                state.app_store.confirm_dialog =
                    Some(crate::app_store::state::ConfirmAction::ClearQueue);
                state.app_store.confirm_cursor = 1;
            }
            continue;
        }

        // ---- Intercept: P pauses/resumes the queue from anywhere in the App Store ----
        if state.focus == FocusTarget::Main
            && matches!(state.active_page.as_deref(), Some("appstore"))
            && state.app_store.queue_visible()
            && state.app_store.confirm_dialog.is_none()
            && !state.app_store.install_location_dialog
            && state.app_store.focus != crate::app_store::state::AppStoreFocus::SearchBar
            && !(state.app_store.browser.active
                && state.app_store.browser.focus
                    == crate::app_store::awesome_ratatui_manager::BrowserFocus::SearchBar)
            && matches!(
                key_event.code,
                crossterm::event::KeyCode::Char('p') | crossterm::event::KeyCode::Char('P')
            )
        {
            state.app_store.queue.paused = !state.app_store.queue.paused;
            let msg = if state.app_store.queue.paused {
                "Queue paused — the running job will still finish"
            } else {
                "Queue resumed"
            };
            state.popup = Some((msg.to_string(), Instant::now()));
            continue;
        }

        // ---- Intercept: Shift+C clears finished App Store queue jobs ----
        if state.focus == FocusTarget::Main
            && matches!(state.active_page.as_deref(), Some("appstore"))
            && state.app_store.queue_visible()
            && state.app_store.confirm_dialog.is_none()
            && matches!(key_event.code, crossterm::event::KeyCode::Char('C'))
            && key_event.modifiers.contains(crossterm::event::KeyModifiers::SHIFT)
        {
            state.app_store.confirm_dialog =
                Some(crate::app_store::state::ConfirmAction::ClearQueue);
            state.app_store.confirm_cursor = 1;
            continue;
        }

        // When terminal output is focused, Up/Down/W/S scroll it
        if state.settings.terminal_focused
            && state.focus == FocusTarget::Main
            && matches!(state.active_page.as_deref(), Some("settings"))
        {
            let total = state.settings.terminal_output.len();
            match key_event.code {
                crossterm::event::KeyCode::Up | crossterm::event::KeyCode::Char('w') => {
                    if state.settings.terminal_scroll > 0 {
                        state.settings.terminal_scroll -= 1;
                    }
                }
                crossterm::event::KeyCode::Down | crossterm::event::KeyCode::Char('s') => {
                    if state.settings.terminal_scroll < total.saturating_sub(1) {
                        state.settings.terminal_scroll += 1;
                    }
                }
                _ => {}
            }
            continue;
        }

        // ---- Helper: check if key matches configured focus toggle key ----
        let toggle_key_name = {
            let s = crate::settings::persistence::load();
            crate::settings::persistence::get_str(&s, "button_mapping.focus_toggle_key", "Tab")
        };
        let is_toggle_key = match key_event.code {
            crossterm::event::KeyCode::Tab => toggle_key_name == "Tab",
            crossterm::event::KeyCode::Enter => toggle_key_name == "Enter",
            crossterm::event::KeyCode::Esc => toggle_key_name == "Esc",
            crossterm::event::KeyCode::Backspace => toggle_key_name == "Backspace",
            crossterm::event::KeyCode::Char(ch) => {
                let upper: String = ch.to_uppercase().collect();
                upper == toggle_key_name
            }
            _ => false,
        };

        // If the configured toggle key is pressed, switch focus
        if is_toggle_key {
            // The UI lock keeps the user inside the App Store while jobs are running
            if matches!(state.active_page.as_deref(), Some("appstore"))
                && state.app_store.ui_locked()
            {
                ui_lock_popup(&mut state);
                continue;
            }
            state.focus = match state.focus {
                FocusTarget::Navbar => FocusTarget::Main,
                FocusTarget::Main => FocusTarget::Navbar,
            };
            state.nav_expanded = false;
            continue;
        }

        // ---- Intercept: Numpad navigation (always active) ----
        // 8=Up, 2=Down, 4=Left, 6=Right, 7=Back/Q, 9=Enter
        if let crossterm::event::KeyCode::Char(ch) = key_event.code {
            let nav_action = match ch {
                '8' => Some(Action::Up),
                '2' => Some(Action::Down),
                '4' => Some(Action::Left),
                '6' => Some(Action::Right),
                '7' => Some(Action::Back),
                '9' => Some(Action::Enter),
                _ => None,
            };
            if let Some(a) = nav_action {
                if state.focus == FocusTarget::Navbar {
                    match handle_navbar_key(
                        a, &mut state, &nav_items, &apps_registry, &dashboards,
                        &registered_apps, &mut active_internal_app, &mut active_installed_dash,
                        &mut active_installed_app,
                    ) {
                        NavResult::Quit => break,
                        NavResult::Restart => { should_restart = true; break; }
                        _ => {}
                    }
                } else {
                    match handle_main_key(a, &mut state, &mut active_internal_app, &mut active_installed_dash, &mut active_installed_app, &registered_apps) {
                        MainResult::Quit => break,
                        MainResult::Restart => { should_restart = true; break; }
                        _ => {}
                    }
                }
                continue;
            }
        }

        // ---- Intercept: Hotkey number keys (1-9), any focus, no page/app active ----
        if state.active_page.is_none()
            && state.active_app.is_none()
            && active_installed_dash.is_none()
        {
            if let crossterm::event::KeyCode::Char(ch) = key_event.code {
                if ('1'..='9').contains(&ch) {
                    let key_num = ch.to_digit(10).unwrap() as usize;
                    let hotkey_settings = crate::settings::persistence::load();
                    // Skip if numpad navigation is on (those digits are nav keys)
                    // Numpad nav digits are always reserved — skip hotkey for them
                    if !matches!(ch, '8' | '2' | '4' | '6' | '7' | '9') {
                        let path = format!("hotkeys.key_{}", key_num);
                        if let Some(val) = crate::settings::persistence::get(&hotkey_settings, &path) {
                            if let Some(action_str) = val.as_str() {
                                if !action_str.is_empty() && action_str != "None" {
                                    let result = execute_action(
                                        action_str,
                                        &mut state,
                                        &apps_registry,
                                        &dashboards,
                                        &registered_apps,
                                        &mut active_internal_app,
                                        &mut active_installed_dash,
                                        &mut active_installed_app,
                                    );
                                    match result {
                                        NavResult::Quit => break,
                                        NavResult::Restart => {
                                            should_restart = true;
                                            break;
                                        }
                                        _ => {}
                                    }
                                    continue;
                                }
                            }
                        }
                    } // end if !numpad_on
                }
            }
        }

        // When an installed dashboard is active and main is focused,
        // forward raw key events directly to the PTY subprocess.
        // Only Esc (quit) and Tab (switch to navbar) are reserved by tuiOS.
        if state.focus == FocusTarget::Main {
            if let Some(dash) = &mut active_installed_dash {
                if state.active_dashboard == dash.name
                    && state.active_page.is_none()
                    && state.active_app.is_none()
                {
                    match key_event.code {
                        crossterm::event::KeyCode::Esc => break, // Quit tuiOS
                        crossterm::event::KeyCode::Tab => {
                            state.focus = FocusTarget::Navbar;
                            continue;
                        }
                        _ => {
                            dash.send_key_event(key_event);
                            continue;
                        }
                    }
                }
            }
        }

        // Forward raw key events to installed (PTY) app in the main container
        if state.focus == FocusTarget::Main
            && active_installed_app.is_some()
            && state.active_app.is_some()
            && state.active_page.is_none()
        {
            match key_event.code {
                crossterm::event::KeyCode::Backspace => {
                    active_installed_app.take();
                    state.active_app = None;
                    continue;
                }
                crossterm::event::KeyCode::Tab => {
                    state.focus = FocusTarget::Navbar;
                    continue;
                }
                _ => {
                    if let Some(pty_app) = &mut active_installed_app {
                        pty_app.send_key_event(key_event);
                    }
                    continue;
                }
            }
        }

        // When text editor is in typing_mode, forward raw character keys directly
        if state.focus == FocusTarget::Main {
            if let Some(InternalApp::TextEditor(ref mut editor)) = active_internal_app {
                if editor.typing_mode {
                    match key_event.code {
                        crossterm::event::KeyCode::Char(ch) => {
                            editor.insert_char(ch);
                            continue;
                        }
                        crossterm::event::KeyCode::Backspace => {
                            editor.backspace();
                            continue;
                        }
                        crossterm::event::KeyCode::Esc => {
                            editor.typing_mode = false;
                            continue;
                        }
                        crossterm::event::KeyCode::Enter => {
                            editor.insert_newline();
                            continue;
                        }
                        crossterm::event::KeyCode::Tab => {
                            state.focus = FocusTarget::Navbar;
                            continue;
                        }
                        _ => {
                            // Let arrow keys etc. go through map_key below
                        }
                    }
                }
            }
        }

        // When Awesome Ratatui browser search bar is focused, capture character input
        if state.focus == FocusTarget::Main
            && matches!(state.active_page.as_deref(), Some("appstore"))
            && state.app_store.browser.active
            && state.app_store.browser.focus == crate::app_store::awesome_ratatui_manager::BrowserFocus::SearchBar
        {
            match key_event.code {
                crossterm::event::KeyCode::Char(ch) => {
                    state.app_store.browser.search_query.push(ch);
                    state.app_store.browser.recompute_visible();
                    continue;
                }
                crossterm::event::KeyCode::Backspace => {
                    state.app_store.browser.search_query.pop();
                    state.app_store.browser.recompute_visible();
                    continue;
                }
                crossterm::event::KeyCode::Esc => {
                    state.app_store.browser.search_query.clear();
                    state.app_store.browser.focus = crate::app_store::awesome_ratatui_manager::BrowserFocus::List;
                    state.app_store.browser.recompute_visible();
                    continue;
                }
                crossterm::event::KeyCode::Enter | crossterm::event::KeyCode::Down => {
                    state.app_store.browser.focus = crate::app_store::awesome_ratatui_manager::BrowserFocus::List;
                    continue;
                }
                _ => {}
            }
        }

        // Browser `/` shortcut to activate search when in list view
        if state.focus == FocusTarget::Main
            && matches!(state.active_page.as_deref(), Some("appstore"))
            && state.app_store.browser.active
            && state.app_store.browser.focus == crate::app_store::awesome_ratatui_manager::BrowserFocus::List
        {
            if let crossterm::event::KeyCode::Char('/') = key_event.code {
                state.app_store.browser.focus = crate::app_store::awesome_ratatui_manager::BrowserFocus::SearchBar;
                continue;
            }
        }

        // When App Store search bar is focused, capture character input
        if state.focus == FocusTarget::Main
            && matches!(state.active_page.as_deref(), Some("appstore"))
            && state.app_store.focus == crate::app_store::state::AppStoreFocus::SearchBar
        {
            match key_event.code {
                crossterm::event::KeyCode::Char(ch) => {
                    state.app_store.search_query.push(ch);
                    state.app_store.recompute_app_list(&registered_apps);
                    continue;
                }
                crossterm::event::KeyCode::Backspace => {
                    state.app_store.search_query.pop();
                    state.app_store.recompute_app_list(&registered_apps);
                    continue;
                }
                crossterm::event::KeyCode::Esc => {
                    state.app_store.search_query.clear();
                    state.app_store.focus = crate::app_store::state::AppStoreFocus::LeftPane;
                    state.app_store.recompute_app_list(&registered_apps);
                    continue;
                }
                crossterm::event::KeyCode::Enter | crossterm::event::KeyCode::Down => {
                    state.app_store.focus = crate::app_store::state::AppStoreFocus::LeftPane;
                    continue;
                }
                _ => {}
            }
        }

        // App Store shortcut keys (when in left pane)
        if state.focus == FocusTarget::Main
            && matches!(state.active_page.as_deref(), Some("appstore"))
            && state.app_store.focus == crate::app_store::state::AppStoreFocus::LeftPane
        {
            if let crossterm::event::KeyCode::Char(ch) = key_event.code {
                match ch {
                    '/' => {
                        state.app_store.focus = crate::app_store::state::AppStoreFocus::SearchBar;
                        continue;
                    }
                    _ => {}
                }
            }
        }

        let action = match map_key(key_event) {
            Some(a) => a,
            None => continue,
        };

        if action == Action::Quit {
            // Don't quit tuiOS when in the app store — close the page instead
            if matches!(state.active_page.as_deref(), Some("appstore")) {
                if state.app_store.ui_locked() {
                    ui_lock_popup(&mut state);
                    continue;
                }
                if state.focus == FocusTarget::Navbar {
                    // Esc from navbar while appstore is open: close appstore
                    state.active_page = None;
                    continue;
                }
                // Fall through to handle_main_key below
            } else {
                break;
            }
        }

        if state.focus == FocusTarget::Navbar {
            match handle_navbar_key(
                action,
                &mut state,
                &nav_items,
                &apps_registry,
                &dashboards,
                &registered_apps,
                &mut active_internal_app,
                &mut active_installed_dash,
                &mut active_installed_app,
            ) {
                NavResult::Quit => break,
                NavResult::Restart => {
                    should_restart = true;
                    break;
                }
                NavResult::ActivateInternalApp | NavResult::None => {}
            }
        } else {
            match handle_main_key(action, &mut state, &mut active_internal_app, &mut active_installed_dash, &mut active_installed_app, &registered_apps) {
                MainResult::Quit => break,
                MainResult::Restart => {
                    should_restart = true;
                    break;
                }
                MainResult::None => {}
            }
            if state.active_app.is_none() && active_internal_app.is_some() {
                active_internal_app = None;
            }
        }

        // An app was asked to run outside the tuiOS container
        if let Some(cmd) = state.pending_foreground.take() {
            if app_store::actions::launch_detached(&cmd) {
                state.popup = Some(("Launched in a new window".to_string(), Instant::now()));
            } else {
                // Hand our own terminal over to the app until it exits
                disable_raw_mode().ok();
                execute!(terminal.backend_mut(), LeaveAlternateScreen).ok();
                run_foreground(&cmd);
                enable_raw_mode().ok();
                execute!(terminal.backend_mut(), EnterAlternateScreen).ok();
                terminal.clear().ok();
            }
        }
    }

    // Drop PTY runners before restoring terminal
    drop(active_installed_dash);
    drop(active_installed_app);
    drop(active_internal_app);

    // Restore terminal
    disable_raw_mode().ok();
    execute!(terminal.backend_mut(), LeaveAlternateScreen).ok();
    drop(terminal);

    should_restart
}
