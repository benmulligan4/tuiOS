/// About tuiOS — system information and stats.

use ratatui::{
    layout::Rect,
    style::{Color, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};
use sysinfo::System;

use crate::settings::persistence;

pub fn render(frame: &mut Frame, area: Rect, _cursor: usize, _scroll: usize) {
    let mut sys = System::new_all();
    sys.refresh_all();

    let total_mem = sys.total_memory() / 1024 / 1024;
    let used_mem = sys.used_memory() / 1024 / 1024;
    let cpu_usage: f32 = sys.cpus().iter().map(|c| c.cpu_usage()).sum::<f32>()
        / sys.cpus().len().max(1) as f32;

    let os_name = System::name().unwrap_or_else(|| "Unknown".into());
    let os_version = System::os_version().unwrap_or_else(|| "".into());
    let hostname = System::host_name().unwrap_or_else(|| "Unknown".into());
    let cpu_name = sys.cpus().first().map(|c| c.brand().to_string()).unwrap_or_else(|| "Unknown".into());
    let uptime_secs = System::uptime();
    let uptime_h = uptime_secs / 3600;
    let uptime_m = (uptime_secs % 3600) / 60;

    let (term_w, term_h) = crossterm::terminal::size().unwrap_or((0, 0));

    let branch = std::process::Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "unknown".into());

    let settings = persistence::load();
    let default_dash = persistence::get_str(
        &settings,
        "default_dashboard",
        crate::dashboards::dashboard_1::KEY,
    );
    let accent = persistence::get_str(&settings, "appearance.accent_color", "Cyan");
    let dash_count = persistence::load_dashboard_names().len();
    let app_count = persistence::load_app_names().len();

    // Get local IP for VNC viewer
    let vnc_ip = get_local_ip();

    let info_style = Style::default().fg(Color::White);
    let label_style = Style::default().fg(Color::DarkGray);

    let lines: Vec<Line> = vec![
        // Line::from(vec![
        //     Span::styled("  tuiOS Version:    ", label_style),
        //     Span::styled(env!("CARGO_PKG_VERSION"), info_style),
        // ]),
        Line::from(vec![
            Span::styled("  Git Branch:      ", label_style),
            Span::styled(&branch, info_style),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled("  OS:              ", label_style),
            Span::styled(format!("{} {}", os_name, os_version), info_style),
        ]),
        Line::from(vec![
            Span::styled("  Hostname:        ", label_style),
            Span::styled(&hostname, info_style),
        ]),
        Line::from(vec![
            Span::styled("  CPU:             ", label_style),
            Span::styled(&cpu_name, info_style),
        ]),
        Line::from(vec![
            Span::styled("  CPU Usage:       ", label_style),
            Span::styled(format!("{:.1}%", cpu_usage), info_style),
        ]),
        Line::from(vec![
            Span::styled("  RAM:             ", label_style),
            Span::styled(format!("{} / {} MB", used_mem, total_mem), info_style),
        ]),
        Line::from(vec![
            Span::styled("  Uptime:          ", label_style),
            Span::styled(format!("{}h {}m", uptime_h, uptime_m), info_style),
        ]),
        Line::from(vec![
            Span::styled("  Terminal Size:   ", label_style),
            Span::styled(format!("{}x{}", term_w, term_h), info_style),
        ]),
        Line::from(vec![
            Span::styled("  VNC Viewer IP:   ", label_style),
            Span::styled(&vnc_ip, info_style),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled("  Default Dash:    ", label_style),
            Span::styled(&default_dash, info_style),
        ]),
        Line::from(vec![
            Span::styled("  tuiOS Colour:     ", label_style),
            Span::styled(&accent, info_style),
        ]),
        Line::from(vec![
            Span::styled("  Dashboards:      ", label_style),
            Span::styled(format!("{}", dash_count), info_style),
        ]),
        Line::from(vec![
            Span::styled("  Applications:    ", label_style),
            Span::styled(format!("{}", app_count), info_style),
        ]),
    ];

    frame.render_widget(Paragraph::new(lines), area);
}

pub fn item_count() -> usize { 0 }

/// Get the local IP address (for VNC viewer connection info).
fn get_local_ip() -> String {
    // Try hostname -I on Linux, ipconfig on Windows
    if cfg!(target_os = "windows") {
        std::process::Command::new("powershell")
            .args(["-Command", "(Get-NetIPAddress -AddressFamily IPv4 | Where-Object { $_.PrefixOrigin -ne 'WellKnown' } | Select-Object -First 1).IPAddress"])
            .output()
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .map(|s| s.trim().to_string())
            .unwrap_or_else(|| "Unknown".into())
    } else {
        std::process::Command::new("hostname")
            .arg("-I")
            .output()
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .map(|s| s.split_whitespace().next().unwrap_or("Unknown").to_string())
            .unwrap_or_else(|| "Unknown".into())
    }
}
