/// Scripts & Git settings — run scripts, pull, rebuild, restore defaults.

use ratatui::{
    layout::{Constraint, Direction, Layout, Margin, Rect},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use crate::settings::state::SettingsState;

pub fn render(
    frame: &mut Frame,
    area: Rect,
    cursor: usize,
    _scroll: usize,
    terminal_output: &[String],
    terminal_focused: bool,
    terminal_scroll: usize,
) {
    let branch = std::process::Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "unknown".into());

    let script_ext = if cfg!(target_os = "windows") { ".bat" } else { ".sh" };

    let items: Vec<String> = vec![
        "Git Pull".to_string(),
        format!("Run Build Script (build{})", script_ext),
        format!("Run Install Script (install{})", script_ext),
        format!("Run Clone Script (clone{})", script_ext),
        format!("Run Setup Script (setup{})", script_ext),
        "Check VNC Viewer Status".to_string(),
        "Enable VNC Viewer".to_string(),
        "Disable VNC Viewer".to_string(),
        "Restore tuiOS Settings to Default".to_string(),
    ];

    let value_style = Style::default().fg(Color::White);
    let label_style = Style::default().fg(Color::DarkGray);

    // Split area: top for items, bottom for terminal output
    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length((items.len() as u16) + 4), Constraint::Fill(1)])
        .split(area);

    let mut lines: Vec<Line> = Vec::new();

    lines.push(Line::from(vec![
        Span::styled("    Current Branch:  ", label_style),
        Span::styled(&branch, value_style),
    ]));
    lines.push(Line::from(vec![
        Span::styled("    Script Type:     ", label_style),
        Span::styled(
            if cfg!(target_os = "windows") { "Windows (.bat)" } else { "Linux (.sh)" },
            value_style,
        ),
    ]));
    lines.push(Line::from(""));

    for (i, label) in items.iter().enumerate() {
        let prefix = if i == cursor { "  » " } else { "    " };
        let style = if i == cursor {
            Style::default().fg(Color::Black).bg(Color::Cyan)
        } else {
            value_style
        };
        lines.push(Line::from(Span::styled(format!("{}{}", prefix, label), style)));
    }

    frame.render_widget(Paragraph::new(lines), sections[0]);

    // Terminal output area with border — highlighted when focused
    let terminal_border_style = if terminal_focused {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let terminal_title = if terminal_focused {
        " Terminal Output  [Shift+Tab to exit] "
    } else {
        " Terminal Output  [Shift+Tab to enter] "
    };

    let terminal_block = Block::default()
        .borders(Borders::ALL)
        .title(terminal_title)
        .style(terminal_border_style);
    frame.render_widget(terminal_block, sections[1]);

    let term_inner = sections[1].inner(Margin { horizontal: 1, vertical: 1 });
    let visible_height = term_inner.height as usize;

    let mut term_lines: Vec<Line> = Vec::new();

    if terminal_output.is_empty() {
        term_lines.push(Line::from(Span::styled(
            "  (Run a script to see output here)",
            Style::default().fg(Color::DarkGray),
        )));
    } else {
        // Apply scroll offset
        let max_scroll = terminal_output.len().saturating_sub(visible_height);
        let scroll = terminal_scroll.min(max_scroll);
        let end = (scroll + visible_height).min(terminal_output.len());
        for line in &terminal_output[scroll..end] {
            term_lines.push(Line::from(Span::styled(
                format!("  {}", line),
                Style::default().fg(Color::White),
            )));
        }

        // Scroll indicator
        if terminal_focused && terminal_output.len() > visible_height {
            let info = format!(" {}/{} ", scroll + 1, terminal_output.len());
            let info_width = info.len() as u16;
            if sections[1].width > info_width + 2 {
                let indicator_rect = Rect {
                    x: sections[1].x + sections[1].width - info_width - 1,
                    y: sections[1].y + sections[1].height - 1,
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

    frame.render_widget(Paragraph::new(term_lines), term_inner);
}

pub fn item_count() -> usize { 9 }

pub fn handle_enter(cursor: usize, ss: &mut SettingsState) {
    let script_dir = std::env::current_dir().unwrap_or_default().join("x");
    let ext = if cfg!(target_os = "windows") { "bat" } else { "sh" };

    ss.terminal_output.clear();

    match cursor {
        0 => {
            // Git Pull
            ss.terminal_output.push("Running: git pull".into());
            let output = run_command("git", &["pull"]);
            ss.terminal_output.extend(output);
            crate::utilities::logging::settings("Ran Git Pull");
        }
        1 => {
            // Run Build Script
            let script = script_dir.join(format!("build.{}", ext));
            ss.terminal_output.push(format!("Running: {}", script.display()));
            let output = run_script(&script);
            ss.terminal_output.extend(output);
            crate::utilities::logging::settings("Ran build script");
        }
        2 => {
            // Run Install Script
            let script = script_dir.join(format!("install.{}", ext));
            ss.terminal_output.push(format!("Running: {}", script.display()));
            let output = run_script(&script);
            ss.terminal_output.extend(output);
            crate::utilities::logging::settings("Ran install script");
        }
        3 => {
            // Run Clone Script
            let script = script_dir.join(format!("clone.{}", ext));
            ss.terminal_output.push(format!("Running: {}", script.display()));
            let output = run_script(&script);
            ss.terminal_output.extend(output);
            crate::utilities::logging::settings("Ran clone script");
        }
        4 => {
            // Run Setup Script
            let script = script_dir.join(format!("setup.{}", ext));
            ss.terminal_output.push(format!("Running: {}", script.display()));
            let output = run_script(&script);
            ss.terminal_output.extend(output);
            crate::utilities::logging::settings("Ran setup script");
        }
        5 => {
            // Check VNC Viewer Status
            ss.terminal_output.push("Checking VNC status...".into());
            if cfg!(target_os = "linux") {
                let output = run_command("systemctl", &["status", "vncserver-x11-serviced"]);
                ss.terminal_output.extend(output);
            } else {
                ss.terminal_output.push("VNC check is only available on Raspberry Pi.".into());
            }
        }
        6 => {
            // Enable VNC Viewer
            ss.terminal_output.push("Enabling VNC Viewer...".into());
            if cfg!(target_os = "linux") {
                let output = run_command("sudo", &["systemctl", "enable", "vncserver-x11-serviced"]);
                ss.terminal_output.extend(output);
                let output = run_command("sudo", &["systemctl", "start", "vncserver-x11-serviced"]);
                ss.terminal_output.extend(output);
                crate::utilities::logging::settings("VNC Viewer enabled");
            } else {
                ss.terminal_output.push("VNC is only available on Raspberry Pi.".into());
            }
        }
        7 => {
            // Disable VNC Viewer
            ss.terminal_output.push("Disabling VNC Viewer...".into());
            if cfg!(target_os = "linux") {
                let output = run_command("sudo", &["systemctl", "stop", "vncserver-x11-serviced"]);
                ss.terminal_output.extend(output);
                let output = run_command("sudo", &["systemctl", "disable", "vncserver-x11-serviced"]);
                ss.terminal_output.extend(output);
                crate::utilities::logging::settings("VNC Viewer disabled");
            } else {
                ss.terminal_output.push("VNC is only available on Raspberry Pi.".into());
            }
        }
        8 => {
            // Restore defaults
            let defaults = serde_json::json!({
                "default_dashboard": "Dashboard-1",
                "theme": "default",
                "appearance": {
                    "accent_color": "Cyan",
                    "clock_enabled": false,
                    "clock_format_24h": true,
                    "clock_show_seconds": false,
                    "border_style": "Rounded",
                    "status_bar_enabled": false,
                    "new_window_keeps_tuios_open": false,
                    "navbar_position": "Top",
                    "intro_animation_enabled": true,
                    "boot_animation_style": "Modern",
                    "boot_animation_colour": "Default",
                    "boot_logo": "Static",
                    "boot_reveal": "Center",
                    "boot_duration": "Default",
                    "boot_splash_effect": true,
                    "boot_preview": true
                },
                "display": {
                    "fullscreen": false
                },
                "hotkeys": {},
                "button_mapping": {
                    "focus_toggle_key": "Tab"
                },
                "utilities": {
                    "onscreen_keyboard_enabled": true
                }
            });
            crate::settings::persistence::save(&defaults);
            ss.terminal_output.push("Settings restored to defaults.".into());
            crate::utilities::logging::settings("Settings restored to defaults");
        }
        _ => {}
    }
}

fn run_command(program: &str, args: &[&str]) -> Vec<String> {
    match std::process::Command::new(program).args(args).output() {
        Ok(output) => {
            let mut lines = Vec::new();
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            for line in stdout.lines() {
                lines.push(line.to_string());
            }
            for line in stderr.lines() {
                lines.push(format!("[err] {}", line));
            }
            if lines.is_empty() {
                lines.push("(no output)".into());
            }
            lines
        }
        Err(e) => vec![format!("Error: {}", e)],
    }
}

fn run_script(path: &std::path::Path) -> Vec<String> {
    if !path.exists() {
        return vec![format!("Script not found: {}", path.display())];
    }
    if cfg!(target_os = "windows") {
        run_command("cmd", &["/C", &path.to_string_lossy()])
    } else {
        run_command("bash", &[&path.to_string_lossy()])
    }
}
