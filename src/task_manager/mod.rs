/// Task Manager — background sessions, live system graphs and network details.

pub mod bar;
pub mod bar_config;
pub mod config;
pub mod gpu;
#[cfg(windows)]
mod gpu_windows;
pub mod graphs;
pub mod keys;
pub mod metrics;
pub mod net;
pub mod netinfo;
pub mod page;
pub mod perf;
pub mod state;
pub mod switcher;

/// Make text from outside tuiOS (SSIDs, process names, window titles) safe to draw:
/// control characters could otherwise smuggle terminal escape sequences onto the screen.
pub fn clean_text(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_control() { '?' } else { c })
        .take(64)
        .collect()
}
