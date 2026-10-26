/// tuiOS — shared enums, structs, and state containers.

use chrono::{DateTime, Utc};
use std::time::Instant;

use crate::app_store::state::AppStoreState;
use crate::settings::state::SettingsState;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Up,
    Down,
    Left,
    Right,
    Enter,
    Back,  // Q / Backspace
    Tab,
    Quit,  // Esc
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FocusTarget {
    Navbar,
    Main,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum AppStatus {
    Running,
    Idle,
    Stopped,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct RunningApp {
    pub name: String,
    pub pid: Option<u32>,
    pub status: AppStatus,
    pub start_time: DateTime<Utc>,
    pub log_path: String,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct TuiosState {
    pub focus: FocusTarget,
    pub active_dashboard: String,
    /// "settings" | "touchscreen" | "system" | None
    pub active_page: Option<String>,
    /// name of an active internal app, or None
    pub active_app: Option<String>,
    pub running_apps: Vec<RunningApp>,
    /// index highlighted in the System page process list
    pub system_cursor: usize,
    /// Custom navbar state
    pub nav_cursor: usize,
    pub nav_expanded: bool,
    pub dropdown_cursor: usize,
    /// Scroll offset for the log viewer — reverse offset from the bottom.
    /// 0 = show the most recent (bottom) entries. N = scroll N lines toward the top.
    pub log_scroll: usize,
    /// Settings page UI state.
    pub settings: SettingsState,
    /// App Store page UI state.
    pub app_store: AppStoreState,
    /// Temporary popup message (text, when it was set).
    pub popup: Option<(String, Instant)>,
    /// Run-source chooser dialog: (app_name, global_cmd, local_cmd, cursor)
    pub run_source_dialog: Option<(String, Vec<String>, Vec<String>, usize)>,
    /// Flag: main loop should re-sync installed apps and refresh nav
    pub needs_sync: bool,
    /// Command the main loop should launch outside the tuiOS container
    /// (new OS window, or by handing over the current terminal).
    pub pending_foreground: Option<Vec<String>>,
    /// Navbar action ("app:X" / "dashboard:X") the main loop should run next,
    /// used by the App Store to launch built-ins through the normal path.
    pub pending_nav_action: Option<String>,
    /// Flag: rebuild the navbar dropdowns without a full install sync
    pub needs_nav_rebuild: bool,
    /// Category headings collapsed in the Apps dropdown.
    pub nav_collapsed: std::collections::HashSet<String>,
}

impl TuiosState {
    pub fn new(default_dashboard: String) -> Self {
        Self {
            focus: FocusTarget::Navbar,
            active_dashboard: default_dashboard,
            active_page: None,
            active_app: None,
            running_apps: Vec::new(),
            system_cursor: 0,
            nav_cursor: 0,
            nav_expanded: false,
            dropdown_cursor: 0,
            log_scroll: 0,
            settings: SettingsState::new(),
            app_store: AppStoreState::new(),
            popup: None,
            run_source_dialog: None,
            needs_sync: false,
            pending_foreground: None,
            pending_nav_action: None,
            needs_nav_rebuild: false,
            nav_collapsed: std::collections::HashSet::new(),
        }
    }
}
