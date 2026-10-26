/// App Store state — enums and structs for the App Store UI.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, atomic::{AtomicBool, Ordering}};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortMode {
    AtoZ,
    Author,
    RecentlyInstalled,
}

impl SortMode {
    #[allow(dead_code)]
    pub const ALL: &'static [SortMode] = &[
        SortMode::AtoZ,
        SortMode::Author,
        SortMode::RecentlyInstalled,
    ];

    pub fn label(&self) -> &'static str {
        match self {
            SortMode::AtoZ => "A to Z",
            SortMode::Author => "Author",
            SortMode::RecentlyInstalled => "Recently Installed",
        }
    }

    #[allow(dead_code)]
    pub fn next(&self) -> SortMode {
        match self {
            SortMode::AtoZ => SortMode::Author,
            SortMode::Author => SortMode::RecentlyInstalled,
            SortMode::RecentlyInstalled => SortMode::AtoZ,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallLocation {
    Global,
    Local,
    Git,
}

impl InstallLocation {
    pub fn label(&self) -> &'static str {
        match self {
            InstallLocation::Global => "PATH (cargo install)",
            InstallLocation::Git => "PATH from Git (cargo install --git)",
            InstallLocation::Local => "Downloads (git clone + build)",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstallStatus {
    NotInstalled,
    Global(String),
    Local(String),
    Both(String, String),
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub enum ConfirmAction {
    UninstallGlobal(String),
    UninstallLocal(String),
    UninstallChoose(String),
    ClearQueue,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum AppStoreFocus {
    LeftPane,
    RightPane,
    SearchBar,
    FilterPanel,
    ConfirmDialog,
    InstallLocationDialog,
    BrowserButton,
    Terminal,
    Queue,
}

#[derive(Debug, Clone)]
pub enum LeftRowKind {
    CategoryHeader(String),
    App(String),
    Spacer,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct AppStoreState {
    pub focus: AppStoreFocus,
    pub left_cursor: usize,
    pub left_scroll: usize,
    pub right_scroll: usize,
    pub sort_mode: SortMode,
    /// Group the app list under collapsible category headers, with the active
    /// sort applied inside each group.
    pub group_by_category: bool,
    pub filter_panel_open: bool,
    pub filter_panel_cursor: usize,
    pub sort_dropdown_open: bool,
    pub sort_dropdown_cursor: usize,
    pub filter_show_installed: bool,
    pub filter_show_uninstalled: bool,
    pub filter_categories: HashSet<String>,
    pub search_query: String,
    pub search_active: bool,
    pub terminal_scroll: usize,
    /// Follow the tail of the log instead of holding the manual scroll position
    pub terminal_follow: bool,
    /// Viewport height recorded by the renderer, so key handling can clamp scrolling
    pub terminal_view_height: std::cell::Cell<usize>,
    /// Queue item whose log the terminal is showing; None follows the running job
    pub viewing_item: Option<u64>,
    pub queue: super::queue::InstallQueue,
    pub confirm_dialog: Option<ConfirmAction>,
    pub confirm_cursor: usize,
    pub install_location_dialog: bool,
    pub install_location_cursor: usize,
    /// Context line shown inside the install dialog, e.g. after adding from the browser
    pub install_dialog_note: Option<String>,
    pub available_install_methods: Vec<InstallLocation>,
    pub computed_app_list: Vec<String>,
    pub left_visible_rows: Vec<LeftRowKind>,
    pub collapsed_store_categories: HashSet<String>,
    pub install_statuses: HashMap<String, InstallStatus>,
    pub right_action_cursor: usize,
    pub in_right_actions: bool,
    /// Details pane geometry recorded by the renderer, so key handling can keep
    /// the focused action button on screen.
    pub right_action_lines: std::cell::RefCell<Vec<usize>>,
    pub right_view_height: std::cell::Cell<usize>,
    pub right_total_lines: std::cell::Cell<usize>,
    /// Shared output buffer for background install/uninstall thread
    pub thread_output: Arc<Mutex<Vec<String>>>,
    /// Signals that the background operation has completed
    pub thread_done: Arc<AtomicBool>,
    /// Signals whether the operation succeeded
    pub thread_success: Arc<AtomicBool>,
    /// Warning raised by the background operation, e.g. files left behind because
    /// they were still in use. Shown as a popup when the operation finishes.
    pub thread_warning: Arc<Mutex<Option<String>>>,
    /// App queued for installation from the browser, independent of the left-pane cursor
    pub pending_install_key: Option<String>,
    /// Cargo bin directory contents captured before the running install started
    pub pre_install_bins: HashSet<String>,
    pub failed_installs: HashSet<String>,
    /// Awesome Ratatui browser state
    pub browser: super::awesome_ratatui_manager::BrowserState,
}

impl AppStoreState {
    pub fn new() -> Self {
        Self {
            focus: AppStoreFocus::LeftPane,
            left_cursor: 0,
            left_scroll: 0,
            right_scroll: 0,
            sort_mode: SortMode::AtoZ,
            group_by_category: false,
            filter_panel_open: false,
            filter_panel_cursor: 0,
            sort_dropdown_open: false,
            sort_dropdown_cursor: 0,
            filter_show_installed: true,
            filter_show_uninstalled: true,
            filter_categories: HashSet::new(),
            search_query: String::new(),
            search_active: false,
            terminal_scroll: 0,
            terminal_follow: true,
            terminal_view_height: std::cell::Cell::new(10),
            viewing_item: None,
            queue: super::queue::InstallQueue::new(),
            confirm_dialog: None,
            confirm_cursor: 0,
            install_location_dialog: false,
            install_location_cursor: 0,
            install_dialog_note: None,
            available_install_methods: Vec::new(),
            computed_app_list: Vec::new(),
            left_visible_rows: Vec::new(),
            collapsed_store_categories: HashSet::new(),
            install_statuses: HashMap::new(),
            right_action_cursor: 0,
            in_right_actions: false,
            right_action_lines: std::cell::RefCell::new(Vec::new()),
            right_view_height: std::cell::Cell::new(20),
            right_total_lines: std::cell::Cell::new(0),
            thread_output: Arc::new(Mutex::new(Vec::new())),
            thread_done: Arc::new(AtomicBool::new(false)),
            thread_success: Arc::new(AtomicBool::new(false)),
            thread_warning: Arc::new(Mutex::new(None)),
            pending_install_key: None,
            pre_install_bins: HashSet::new(),
            failed_installs: HashSet::new(),
            browser: super::awesome_ratatui_manager::BrowserState::new(),
        }
    }

    /// Filter-panel cursor index of the first per-category checkbox. Rows 1..4 are
    /// Sort, Organise by Category, Show Installed, Show Uninstalled.
    pub const CATEGORY_FILTER_BASE: usize = 5;

    /// Get the cursor index for the refresh button (always after panel items).
    pub fn refresh_cursor_idx(&self, registered: &HashMap<String, Value>) -> usize {
        if self.filter_panel_open {
            let cats = Self::all_categories(registered);
            Self::CATEGORY_FILTER_BASE + cats.len()
        } else {
            1
        }
    }

    /// Max cursor value for the filter panel area (including refresh).
    pub fn filter_max_cursor(&self, registered: &HashMap<String, Value>) -> usize {
        self.refresh_cursor_idx(registered)
    }

    /// Sync output from the background thread into the running item's log.
    /// Returns true if the operation just completed.
    pub fn poll_operation(&mut self) -> bool {
        if self.queue.running_index().is_none() {
            return false;
        }
        let mut drained: Vec<String> = Vec::new();
        if let Ok(mut buf) = self.thread_output.try_lock() {
            drained.append(&mut *buf);
        }
        let finished = self.thread_done.load(Ordering::Relaxed);
        if finished {
            // Final sync — the thread may have written more between the two locks
            if let Ok(mut buf) = self.thread_output.try_lock() {
                drained.append(&mut *buf);
            }
        }
        if !drained.is_empty() {
            if let Some(item) = self.queue.running_mut() {
                item.extend_log(drained);
            }
        }
        if finished {
            return true;
        }
        false
    }

    /// Prepare the shared channel for the next job and mark it running.
    /// Returns the id of the job that was started.
    pub fn start_next_job(&mut self) -> Option<u64> {
        let id = self.queue.start_next()?;
        let is_install = self
            .queue
            .item_by_id(id)
            .map(|i| i.op.is_install())
            .unwrap_or(false);
        self.pre_install_bins = if is_install {
            super::actions::snapshot_cargo_bin()
        } else {
            HashSet::new()
        };
        self.thread_output = Arc::new(Mutex::new(Vec::new()));
        self.thread_done = Arc::new(AtomicBool::new(false));
        self.thread_success = Arc::new(AtomicBool::new(false));
        self.thread_warning = Arc::new(Mutex::new(None));
        // Snap the terminal back onto whatever is actually running
        self.viewing_item = None;
        self.terminal_follow = true;
        self.terminal_scroll = 0;
        Some(id)
    }
    /// True while a job is running or waiting to run — drives the UI lock.
    pub fn ui_locked(&self) -> bool {
        self.queue.is_active()
    }

    /// The terminal and queue panes appear together, and only when there is
    /// something in the queue to show.
    pub fn queue_visible(&self) -> bool {
        !self.queue.is_empty()
    }

    pub fn terminal_visible(&self) -> bool {
        self.queue_visible()
    }

    /// The queue item the terminal pane is currently showing.
    pub fn viewed_item(&self) -> Option<&super::queue::QueueItem> {
        match self.viewing_item {
            Some(id) => self.queue.item_by_id(id).or_else(|| self.queue.running()),
            None => self.queue.running().or_else(|| self.queue.items.last()),
        }
    }

    pub fn visible_log(&self) -> &[String] {
        self.viewed_item().map(|i| i.log.as_slice()).unwrap_or(&[])
    }

    pub fn terminal_max_scroll(&self) -> usize {
        self.visible_log()
            .len()
            .saturating_sub(self.terminal_view_height.get().max(1))
    }

    /// Scroll offset actually used for drawing — follow mode pins it to the tail.
    pub fn effective_terminal_scroll(&self) -> usize {
        let max = self.terminal_max_scroll();
        if self.terminal_follow {
            max
        } else {
            self.terminal_scroll.min(max)
        }
    }

    /// Snap the terminal back to the tail so new output is visible next time it
    /// is focused. Called whenever focus leaves the terminal pane.
    pub fn snap_terminal_to_bottom(&mut self) {
        self.terminal_follow = true;
        self.terminal_scroll = self.terminal_max_scroll();
    }

    pub fn reset_right_pane(&mut self) {
        self.right_scroll = 0;
        self.right_action_cursor = 0;
        self.in_right_actions = false;
    }

    /// Scroll the details pane just enough to bring the focused action into view.
    pub fn ensure_action_visible(&mut self) {
        let target = match self.right_action_lines.borrow().get(self.right_action_cursor) {
            Some(&t) => t,
            None => return,
        };
        let height = self.right_view_height.get().max(1);
        let max_scroll = self.right_total_lines.get().saturating_sub(height);
        if target < self.right_scroll {
            self.right_scroll = target;
        } else if target >= self.right_scroll + height {
            self.right_scroll = target + 1 - height;
        }
        self.right_scroll = self.right_scroll.min(max_scroll);
    }

    /// Build filtered and sorted list from registered apps.
    pub fn recompute_app_list(&mut self, registered: &HashMap<String, Value>) {
        let mut keys: Vec<String> = registered
            .keys()
            .filter(|key| {
                let meta = &registered[key.as_str()];

                // Search filter (name only)
                if !self.search_query.is_empty() {
                    let label = meta.get("label").and_then(|v| v.as_str()).unwrap_or(key);
                    let query_lower = self.search_query.to_lowercase();
                    if !label.to_lowercase().contains(&query_lower)
                        && !key.to_lowercase().contains(&query_lower)
                    {
                        return false;
                    }
                }

                // Installed / uninstalled filter
                let is_installed = matches!(
                    self.install_statuses.get(key.as_str()),
                    Some(InstallStatus::Global(_))
                        | Some(InstallStatus::Local(_))
                        | Some(InstallStatus::Both(_, _))
                );
                if is_installed && !self.filter_show_installed {
                    return false;
                }
                if !is_installed && !self.filter_show_uninstalled {
                    return false;
                }

                // Category filter (set contains hidden categories)
                if !self.filter_categories.is_empty() {
                    let cat = meta
                        .get("category")
                        .and_then(|v| v.as_str())
                        .unwrap_or("Other")
                        .to_string();
                    if self.filter_categories.contains(&cat) {
                        return false;
                    }
                }

                true
            })
            .cloned()
            .collect();

        // Sort
        let install_times = if self.sort_mode == SortMode::RecentlyInstalled {
            super::actions::load_install_times()
        } else {
            HashMap::new()
        };
        keys.sort_by(|a, b| {
            let ma = &registered[a.as_str()];
            let mb = &registered[b.as_str()];
            let la = ma.get("label").and_then(|v| v.as_str()).unwrap_or(a);
            let lb = mb.get("label").and_then(|v| v.as_str()).unwrap_or(b);
            let by_name = || la.to_lowercase().cmp(&lb.to_lowercase());
            match self.sort_mode {
                SortMode::AtoZ => by_name(),
                SortMode::Author => {
                    let aa = ma.get("author").and_then(|v| v.as_str()).unwrap_or("");
                    let ab = mb.get("author").and_then(|v| v.as_str()).unwrap_or("");
                    aa.to_lowercase().cmp(&ab.to_lowercase()).then_with(by_name)
                }
                SortMode::RecentlyInstalled => {
                    // Newest first; never-installed apps (no stamp) fall to the bottom.
                    let ta = install_times.get(a.as_str()).copied().unwrap_or(0);
                    let tb = install_times.get(b.as_str()).copied().unwrap_or(0);
                    tb.cmp(&ta).then_with(by_name)
                }
            }
        });

        // Build visible rows, optionally grouped under category headers.
        self.left_visible_rows.clear();
        if self.group_by_category {
            let category_of = |key: &str| -> String {
                registered
                    .get(key)
                    .and_then(|m| m.get("category"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("Other")
                    .to_string()
            };

            let mut categories: Vec<String> = keys
                .iter()
                .map(|k| category_of(k))
                .collect::<HashSet<_>>()
                .into_iter()
                .collect();
            categories.sort_by_key(|c| c.to_lowercase());

            let mut ordered: Vec<String> = Vec::with_capacity(keys.len());
            for (ci, cat) in categories.iter().enumerate() {
                if ci > 0 {
                    self.left_visible_rows.push(LeftRowKind::Spacer);
                }
                self.left_visible_rows.push(LeftRowKind::CategoryHeader(cat.clone()));
                let collapsed = self.collapsed_store_categories.contains(cat);
                for key in keys.iter().filter(|k| category_of(k) == *cat) {
                    ordered.push(key.clone());
                    if !collapsed {
                        self.left_visible_rows.push(LeftRowKind::App(key.clone()));
                    }
                }
            }
            self.computed_app_list = ordered;
        } else {
            for key in &keys {
                self.left_visible_rows.push(LeftRowKind::App(key.clone()));
            }
            self.computed_app_list = keys;
        }

        // Clamp cursor
        if !self.left_visible_rows.is_empty() {
            self.left_cursor = self.left_cursor.min(self.left_visible_rows.len() - 1);
        } else {
            self.left_cursor = 0;
        }
    }

    /// Get the app key at the current left cursor position.
    pub fn selected_app_key(&self) -> Option<&String> {
        match self.left_visible_rows.get(self.left_cursor) {
            Some(LeftRowKind::App(key)) => Some(key),
            _ => None,
        }
    }

    /// Collect all unique categories from registered apps.
    pub fn all_categories(registered: &HashMap<String, Value>) -> Vec<String> {
        let mut cats: Vec<String> = registered
            .values()
            .filter_map(|v| v.get("category").and_then(|c| c.as_str()))
            .map(|s| s.to_string())
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();
        cats.sort();
        cats
    }
}
