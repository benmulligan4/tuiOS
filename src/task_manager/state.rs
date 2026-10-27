/// Task Manager page state: tabs, focus, list selection, sort / filter, graph options.

use std::collections::HashMap;

use serde_json::Value;

use crate::tuios::sessions::{Body, Kind, Phase, Session, SessionId, Sessions};

use super::metrics::Metrics;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Apps,
    Performance,
    Network,
    Startup,
}

impl Tab {
    pub const ALL: [Tab; 4] = [Tab::Apps, Tab::Performance, Tab::Network, Tab::Startup];

    pub fn label(self) -> &'static str {
        match self {
            Tab::Apps => "Apps",
            Tab::Performance => "Performance",
            Tab::Network => "Network",
            Tab::Startup => "Startup",
        }
    }

    fn index(self) -> usize {
        Tab::ALL.iter().position(|t| *t == self).unwrap_or(0)
    }

    pub fn step(self, forward: bool) -> Tab {
        let i = self.index();
        let n = Tab::ALL.len();
        Tab::ALL[if forward { (i + 1) % n } else { (i + n - 1) % n }]
    }
}

/// Where the arrow keys go.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Tabs,
    Content,
    /// The action list in the Apps tab's details pane.
    Actions,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortKey {
    Name,
    Cpu,
    Memory,
    Uptime,
    Type,
}

impl SortKey {
    pub fn label(self) -> &'static str {
        match self {
            SortKey::Name => "Name",
            SortKey::Cpu => "CPU",
            SortKey::Memory => "Memory",
            SortKey::Uptime => "Uptime",
            SortKey::Type => "Type",
        }
    }

    pub fn next(self) -> SortKey {
        match self {
            SortKey::Name => SortKey::Cpu,
            SortKey::Cpu => SortKey::Memory,
            SortKey::Memory => SortKey::Uptime,
            SortKey::Uptime => SortKey::Type,
            SortKey::Type => SortKey::Name,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TypeFilter {
    All,
    Apps,
    Dashboards,
    BuiltIn,
    Container,
    Window,
    Suspended,
    Exited,
}

impl TypeFilter {
    pub fn label(self) -> &'static str {
        match self {
            TypeFilter::All => "All",
            TypeFilter::Apps => "Apps",
            TypeFilter::Dashboards => "Dashboards",
            TypeFilter::BuiltIn => "Built-in",
            TypeFilter::Container => "Container",
            TypeFilter::Window => "New Window",
            TypeFilter::Suspended => "Suspended",
            TypeFilter::Exited => "Exited",
        }
    }

    pub fn next(self) -> TypeFilter {
        match self {
            TypeFilter::All => TypeFilter::Apps,
            TypeFilter::Apps => TypeFilter::Dashboards,
            TypeFilter::Dashboards => TypeFilter::BuiltIn,
            TypeFilter::BuiltIn => TypeFilter::Container,
            TypeFilter::Container => TypeFilter::Window,
            TypeFilter::Window => TypeFilter::Suspended,
            TypeFilter::Suspended => TypeFilter::Exited,
            TypeFilter::Exited => TypeFilter::All,
        }
    }

    fn keeps(self, s: &Session) -> bool {
        match self {
            TypeFilter::All => true,
            TypeFilter::Apps => s.kind == Kind::App,
            TypeFilter::Dashboards => s.kind == Kind::Dashboard,
            TypeFilter::BuiltIn => s.is_builtin(),
            TypeFilter::Container => s.is_pty(),
            TypeFilter::Window => s.is_external(),
            TypeFilter::Suspended => s.phase == Phase::Suspended,
            TypeFilter::Exited => !s.is_live(),
        }
    }
}

/// One row of the Apps list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowId {
    /// tuiOS itself.
    Shell,
    Session(SessionId),
    /// The "End All Background Apps" button under the list.
    EndAll,
}

/// Things that can be done to a session from its details pane.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TmAction {
    OpenInContainer,
    OpenInWindow,
    Restart,
    Suspend,
    Resume,
    Quit,
    ForceKill,
    LaunchAnother,
    ViewOutput,
    Dismiss,
}

impl TmAction {
    pub fn label(self, s: &Session) -> &'static str {
        match self {
            TmAction::OpenInContainer if s.is_external() => "Open in Container (restarts it)",
            TmAction::OpenInContainer if s.kind == Kind::Dashboard => "Show on Home Screen",
            TmAction::OpenInContainer => "Open in Container",
            TmAction::OpenInWindow => "Open in New Window (restarts it)",
            TmAction::Restart => "Restart",
            TmAction::Suspend => "Suspend",
            TmAction::Resume => "Resume",
            TmAction::Quit => "Quit",
            TmAction::ForceKill => "Force Kill",
            TmAction::LaunchAnother => "Launch Another Instance",
            TmAction::ViewOutput => "View Output",
            TmAction::Dismiss => "Dismiss",
        }
    }
}

/// Actions offered for a session in its current state.
pub fn actions_for(s: &Session) -> Vec<TmAction> {
    use TmAction::*;
    let mut out = Vec::new();
    match &s.phase {
        Phase::Exited { .. } => {
            out.push(Restart);
            if s.is_pty() {
                out.push(ViewOutput);
            }
            out.push(Dismiss);
        }
        Phase::Stopping { .. } => {
            out.push(ForceKill);
            if s.is_pty() {
                out.push(ViewOutput);
            }
        }
        Phase::Running | Phase::Suspended => {
            if let Body::External(e) = &s.body {
                if e.pid.is_none() {
                    // Still looking for the window's process, or gave up
                    out.push(Dismiss);
                    return out;
                }
            }
            out.push(OpenInContainer);
            if s.is_pty() {
                out.push(OpenInWindow);
            }
            out.push(Restart);
            if s.pid().is_some() {
                out.push(if s.phase == Phase::Suspended { Resume } else { Suspend });
            }
            out.push(Quit);
            if s.kind == Kind::App {
                out.push(LaunchAnother);
            }
            if s.is_pty() {
                out.push(ViewOutput);
            }
        }
    }
    out
}

/// An app or dashboard that can be started at boot.
#[derive(Debug, Clone)]
pub struct StartupRow {
    pub action: String,
    pub label: String,
    pub kind: &'static str,
    /// Built-ins are drawn by tuiOS itself, so they cannot have a window of their own.
    pub builtin: bool,
    /// The app's own window mode (App Store / Settings), used until the Startup tab overrides it.
    pub default_window: bool,
}

impl StartupRow {
    pub fn in_window(&self, settings: &Value) -> bool {
        !self.builtin && super::config::startup_in_window(settings, &self.action).unwrap_or(self.default_window)
    }
}

/// Graph time windows, in seconds.
pub const WINDOWS: [u64; 5] = [30, 60, 120, 300, 900];

/// "End All" waits for confirmation; single quits do not.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Confirm {
    EndAll,
}

#[derive(Debug, Clone)]
pub struct TaskManagerState {
    pub tab: Tab,
    pub focus: Focus,
    /// Selected row in the Apps list, kept by identity so re-sorting does not move it.
    pub selected: RowId,
    pub action_cursor: usize,
    pub sort: SortKey,
    pub filter: TypeFilter,
    /// Overrides the default graph style from Settings until the page is closed.
    pub graph_line: Option<bool>,
    pub window_idx: usize,
    pub perf_cursor: usize,
    pub perf_zoom: bool,
    /// Panels per row in the Performance grid as last drawn (arrow keys follow it).
    pub perf_cols: usize,
    /// 0 = all interfaces, then one per interface.
    pub net_cursor: usize,
    pub startup_cursor: usize,
    pub startup_rows: Vec<StartupRow>,
    /// A new terminal window can be opened here (false on a Pi with no desktop).
    pub windows_ok: bool,
    pub confirm: Option<Confirm>,
    pub confirm_cursor: usize,
    /// Session whose output fills the page ("View Output").
    pub viewer: Option<SessionId>,
    /// Rows the Apps list could show last frame (for scrolling with the cursor).
    pub list_height: usize,
    /// Index of the selected row last frame, so the cursor stays put when that row goes.
    pub last_index: usize,
}

impl TaskManagerState {
    pub fn new() -> Self {
        Self {
            tab: Tab::Apps,
            focus: Focus::Content,
            selected: RowId::Shell,
            action_cursor: 0,
            sort: SortKey::Name,
            filter: TypeFilter::All,
            graph_line: None,
            window_idx: 1,
            perf_cursor: 0,
            perf_zoom: false,
            perf_cols: 2,
            net_cursor: 0,
            startup_cursor: 0,
            startup_rows: Vec::new(),
            windows_ok: true,
            confirm: None,
            confirm_cursor: 0,
            viewer: None,
            list_height: 10,
            last_index: 0,
        }
    }

    /// Called whenever the page opens: rebuild the Startup list, drop stale overlays.
    pub fn on_open(&mut self, apps: &HashMap<String, Value>, dashboards: &HashMap<String, Value>) {
        self.confirm = None;
        self.viewer = None;
        self.perf_zoom = false;
        if self.focus == Focus::Actions {
            self.focus = Focus::Content;
        }
        self.startup_rows = startup_rows(apps, dashboards);
        self.startup_cursor = self.startup_cursor.min(self.startup_rows.len().saturating_sub(1));
        // Probes for terminal emulators on Linux, so check once per opening, not per frame
        self.windows_ok = crate::app_store::actions::can_open_new_window();
    }

    pub fn window_secs(&self) -> u64 {
        WINDOWS[self.window_idx.min(WINDOWS.len() - 1)]
    }

    pub fn line_graphs(&self, default_line: bool) -> bool {
        self.graph_line.unwrap_or(default_line)
    }
}

fn label_of(key: &str, meta: &Value) -> String {
    meta.get("label").and_then(|v| v.as_str()).unwrap_or(key).to_string()
}

/// Every app plus every installed dashboard (built-in dashboards have nothing to start).
fn startup_rows(apps: &HashMap<String, Value>, dashboards: &HashMap<String, Value>) -> Vec<StartupRow> {
    let registered = crate::tuios::registry::load_registered_apps();
    let in_window = |key: &str| {
        crate::app_store::actions::get_window_mode(key, registered.get(key)) == "fullscreen"
    };
    let mut rows: Vec<StartupRow> = Vec::new();
    for (key, meta) in apps {
        let builtin = meta.get("type").and_then(|v| v.as_str()) == Some("internal");
        if crate::tuios::registry::is_builtin_store_key(key)
            && !crate::app_store::actions::get_builtin_enabled(key)
        {
            continue;
        }
        rows.push(StartupRow {
            action: format!("app:{}", key),
            label: label_of(key, meta),
            kind: "App",
            builtin,
            default_window: !builtin && in_window(key),
        });
    }
    for (key, meta) in dashboards {
        if meta.get("type").and_then(|v| v.as_str()) != Some("installed") {
            continue;
        }
        rows.push(StartupRow {
            action: format!("dashboard:{}", key),
            label: label_of(key, meta),
            kind: "Dashboard",
            builtin: false,
            default_window: in_window(key),
        });
    }
    rows.sort_by(|a, b| a.kind.cmp(b.kind).then_with(|| a.label.to_lowercase().cmp(&b.label.to_lowercase())));
    rows
}

/// CPU % and memory of a session, from the sampler (built-ins live inside tuiOS).
pub fn usage(s: &Session, m: &Metrics) -> Option<(f32, u64)> {
    let pid = s.pid()?;
    if !s.is_live() {
        return None;
    }
    m.procs.get(&pid).map(|p| (p.cpu, p.mem))
}

/// The Apps list in display order: tuiOS, the filtered + sorted sessions, then End All.
pub fn rows(sessions: &Sessions, tm: &TaskManagerState, m: &Metrics) -> Vec<RowId> {
    let mut list: Vec<&Session> = sessions.iter().filter(|s| tm.filter.keeps(s)).collect();
    let name = |s: &Session| s.label.to_lowercase();
    match tm.sort {
        SortKey::Name => list.sort_by_key(|s| name(s)),
        SortKey::Cpu => list.sort_by(|a, b| {
            let ca = usage(a, m).map_or(-1.0, |u| u.0);
            let cb = usage(b, m).map_or(-1.0, |u| u.0);
            cb.partial_cmp(&ca).unwrap_or(std::cmp::Ordering::Equal).then_with(|| name(a).cmp(&name(b)))
        }),
        SortKey::Memory => list.sort_by(|a, b| {
            let ma = usage(a, m).map_or(0, |u| u.1);
            let mb = usage(b, m).map_or(0, |u| u.1);
            mb.cmp(&ma).then_with(|| name(a).cmp(&name(b)))
        }),
        SortKey::Uptime => list.sort_by(|a, b| b.uptime().cmp(&a.uptime()).then_with(|| name(a).cmp(&name(b)))),
        SortKey::Type => list.sort_by(|a, b| {
            (a.kind_label(), a.mode_label(), name(a)).cmp(&(b.kind_label(), b.mode_label(), name(b)))
        }),
    }
    let mut out = vec![RowId::Shell];
    out.extend(list.iter().map(|s| RowId::Session(s.id)));
    if sessions.iter().any(|s| s.is_live()) {
        out.push(RowId::EndAll);
    }
    out
}

/// Index of the selected row; when it has gone, the row now at its old position.
pub fn selected_index(rows: &[RowId], selected: RowId, last_index: usize) -> usize {
    rows.iter()
        .position(|r| *r == selected)
        .unwrap_or_else(|| last_index.min(rows.len().saturating_sub(1)))
}
