/// Running apps and dashboards.
///
/// Everything the user opens becomes a session. Switching away (navbar, another app,
/// a page, the switcher) leaves it running in the background; it only ends when the
/// user closes it, it quits by itself, or the Task Manager stops it.

use std::collections::HashSet;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use chrono::{DateTime, Local};
use ratatui::{layout::Rect, style::{Color, Style}, Frame};

use crate::app_store::actions;
use crate::applications::character_set::{AppAction, CharacterSetApp};
use crate::applications::file_explorer::{FileExplorerAction, FileExplorerApp};
use crate::applications::notes::{NotesApp, NotesKeyResult};
use crate::applications::text_editor::{TextEditorAction, TextEditorApp};
use crate::dashboards::installed_runner::InstalledDashboard;
use crate::task_manager::{config, metrics};
use crate::utilities::logging;

use super::proc_control;

pub type SessionId = u64;

/// How long to look for the PID of an app launched in a new terminal window.
const DISCOVERY_WINDOW: Duration = Duration::from_secs(20);
/// After a forced kill, how long to wait for the exit before giving up on the row.
const KILL_GRACE: Duration = Duration::from_secs(3);

// ── Built-in apps ────────────────────────────────────────────────────────────

pub enum InternalApp {
    CharacterSet(CharacterSetApp),
    FileExplorer(FileExplorerApp),
    Notes(NotesApp),
    TextEditor(TextEditorApp),
}

impl InternalApp {
    /// Create and start the built-in app for a registry key.
    pub fn launch(key: &str) -> Self {
        match key {
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
            "Notes" => {
                let mut app = NotesApp::new();
                app.start();
                InternalApp::Notes(app)
            }
            _ => {
                let mut app = CharacterSetApp::new();
                app.start();
                InternalApp::CharacterSet(app)
            }
        }
    }

    pub fn render(&mut self, frame: &mut Frame, area: Rect, border_style: Style) {
        match self {
            InternalApp::CharacterSet(app) => app.render(frame, area, border_style),
            InternalApp::FileExplorer(app) => app.render(frame, area, border_style),
            InternalApp::Notes(app) => app.render(frame, area, border_style),
            InternalApp::TextEditor(app) => app.render(frame, area, border_style),
        }
    }

    /// Returns true if the app wants to close.
    pub fn handle_key(&mut self, code: &str) -> bool {
        match self {
            InternalApp::CharacterSet(app) => matches!(app.handle_key(code), Some(AppAction::Back)),
            InternalApp::FileExplorer(app) => {
                matches!(app.handle_key(code), Some(FileExplorerAction::Back))
            }
            InternalApp::Notes(app) => app.handle_key(code) == NotesKeyResult::Close,
            InternalApp::TextEditor(app) => matches!(app.handle_key(code), Some(TextEditorAction::Back)),
        }
    }

    pub fn stop(&mut self) {
        match self {
            InternalApp::CharacterSet(app) => app.stop(),
            InternalApp::FileExplorer(app) => app.stop(),
            InternalApp::Notes(app) => app.stop(),
            InternalApp::TextEditor(app) => app.stop(),
        }
    }

    /// Returns true if this app uses the OSK and has it active,
    /// meaning WASD should NOT be mapped to navigation.
    pub fn wants_raw_wasd(&self) -> bool {
        match self {
            // Notes decides itself what Back/Q means (close dialog, leave note, quit)
            InternalApp::Notes(_) => true,
            InternalApp::TextEditor(app) => app.keyboard_active(),
            _ => false,
        }
    }
}

// ── Sessions ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    App,
    Dashboard,
}

/// An app in its own terminal window. tuiOS only launched the terminal, so the app's
/// PID is found afterwards by its executable name.
pub struct External {
    pub pid: Option<u32>,
    start_time: Option<u64>,
    stem: String,
    launched_unix: u64,
    launched_at: Instant,
    /// The PID never turned up; the window cannot be managed.
    pub lost: bool,
}

pub enum Body {
    Internal(InternalApp),
    Pty(InstalledDashboard),
    External(External),
}

/// What happens once a stopping session has exited.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum After {
    Remove,
    Restart,
    ToWindow,
    ToContainer,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Phase {
    Running,
    Suspended,
    Stopping { deadline: Instant, then: After, forced: bool },
    Exited { code: Option<u32>, at: DateTime<Local> },
}

pub struct Session {
    pub id: SessionId,
    pub key: String,
    /// Display name, with "(2)" etc. for extra instances.
    pub label: String,
    pub kind: Kind,
    pub instance: u32,
    pub cmd: Vec<String>,
    pub body: Body,
    pub phase: Phase,
    pub started: DateTime<Local>,
    started_at: Instant,
    pub last_focus: Instant,
    pub restarts: u32,
}

impl Session {
    pub fn pid(&self) -> Option<u32> {
        match &self.body {
            Body::Pty(p) => p.pid(),
            Body::External(e) => e.pid,
            Body::Internal(_) => None,
        }
    }

    pub fn is_builtin(&self) -> bool {
        matches!(self.body, Body::Internal(_))
    }

    pub fn is_external(&self) -> bool {
        matches!(self.body, Body::External(_))
    }

    pub fn is_pty(&self) -> bool {
        matches!(self.body, Body::Pty(_))
    }

    /// Still running in some form (not exited).
    pub fn is_live(&self) -> bool {
        !matches!(self.phase, Phase::Exited { .. })
    }

    pub fn is_stopping(&self) -> bool {
        matches!(self.phase, Phase::Stopping { .. })
    }

    pub fn uptime(&self) -> Duration {
        match self.phase {
            Phase::Exited { .. } => Duration::ZERO,
            _ => self.started_at.elapsed(),
        }
    }

    pub fn mode_label(&self) -> &'static str {
        match self.body {
            Body::Internal(_) => "Built-in",
            Body::Pty(_) => "Container",
            Body::External(_) => "New Window",
        }
    }

    pub fn kind_label(&self) -> &'static str {
        match self.kind {
            Kind::App => "App",
            Kind::Dashboard => "Dashboard",
        }
    }

    /// Short status for lists. `shown` = currently on screen in the container.
    pub fn status_label(&self, shown: bool) -> String {
        match &self.phase {
            Phase::Exited { code: Some(code), .. } => format!("Exited ({})", code),
            Phase::Exited { code: None, .. } => "Exited".into(),
            Phase::Stopping { .. } => "Stopping…".into(),
            Phase::Suspended => "Suspended".into(),
            Phase::Running => match &self.body {
                Body::External(e) if e.lost => "Not found".into(),
                Body::External(e) if e.pid.is_none() => "Locating…".into(),
                Body::External(_) => "Own window".into(),
                _ if shown => "Foreground".into(),
                _ => "Background".into(),
            },
        }
    }
}

pub enum SessionEvent {
    Toast(String),
    /// The session that was on screen is gone or left the container.
    LeftForeground,
    /// A running installed dashboard ended; switch home if it was showing.
    DashboardGone(String),
    /// Show this session in the container.
    Foreground(SessionId),
    /// No window system: hand tuiOS's own terminal to this command.
    RunForeground(Vec<String>),
}

pub struct Sessions {
    list: Vec<Session>,
    next_id: SessionId,
    /// App currently drawn in the container (mirrors `TuiosState::active_app`).
    pub foreground: Option<SessionId>,
    /// Inner size of the main container, so new PTYs start at the right size.
    pub container_size: (u16, u16),
}

fn now_unix() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

impl Sessions {
    pub fn new() -> Self {
        Self { list: Vec::new(), next_id: 1, foreground: None, container_size: (80, 24) }
    }

    pub fn iter(&self) -> impl Iterator<Item = &Session> {
        self.list.iter()
    }

    pub fn get(&self, id: SessionId) -> Option<&Session> {
        self.list.iter().find(|s| s.id == id)
    }

    pub fn get_mut(&mut self, id: SessionId) -> Option<&mut Session> {
        self.list.iter_mut().find(|s| s.id == id)
    }

    fn index(&self, id: SessionId) -> Option<usize> {
        self.list.iter().position(|s| s.id == id)
    }

    // ── Foreground ───────────────────────────────────────────────────────

    pub fn foreground_internal_mut(&mut self) -> Option<&mut InternalApp> {
        let id = self.foreground?;
        match &mut self.get_mut(id)?.body {
            Body::Internal(app) => Some(app),
            _ => None,
        }
    }

    pub fn foreground_pty_mut(&mut self) -> Option<&mut InstalledDashboard> {
        let id = self.foreground?;
        match &mut self.get_mut(id)?.body {
            Body::Pty(pty) => Some(pty),
            _ => None,
        }
    }

    /// Draw the foreground app. False when there is nothing drawable in the container.
    pub fn render_foreground(&mut self, frame: &mut Frame, area: Rect, border_style: Style) -> bool {
        let Some(id) = self.foreground else { return false };
        let Some(s) = self.get_mut(id) else { return false };
        match &mut s.body {
            Body::Internal(app) => app.render(frame, area, border_style),
            Body::Pty(pty) => pty.render(frame, area, border_style),
            Body::External(_) => return false,
        }
        true
    }

    /// Draw a live preview of the session into `area` (Task Manager details pane).
    /// Built-ins lay themselves out for the small area; embedded apps keep their real
    /// size, so they are copied 1:1 when they fit and shown as a minimap otherwise.
    /// False when there is nothing to show (own-window apps).
    pub fn render_preview(&mut self, id: SessionId, frame: &mut Frame, area: Rect) -> bool {
        let Some(s) = self.get_mut(id) else { return false };
        match &mut s.body {
            Body::Pty(pty) => {
                let screen = pty.screen_buffer();
                crate::task_manager::graphs::blit_minimap(&screen, frame.buffer_mut(), area);
                true
            }
            Body::Internal(app) => {
                app.render(frame, area, Style::default().fg(Color::DarkGray));
                true
            }
            Body::External(_) => false,
        }
    }

    pub fn pty_mut(&mut self, id: SessionId) -> Option<&mut InstalledDashboard> {
        match &mut self.get_mut(id)?.body {
            Body::Pty(pty) => Some(pty),
            _ => None,
        }
    }

    /// Bring a session into the container, waking it if it was suspended.
    pub fn set_foreground(&mut self, id: SessionId) {
        if matches!(self.get(id).map(|s| &s.phase), Some(Phase::Suspended)) {
            self.resume(id);
        }
        if let Some(s) = self.get_mut(id) {
            s.last_focus = Instant::now();
            if s.kind == Kind::App {
                self.foreground = Some(id);
            }
        }
    }

    /// Record that a dashboard session is now the one on the home screen.
    pub fn touch(&mut self, id: SessionId) {
        if let Some(s) = self.get_mut(id) {
            s.last_focus = Instant::now();
        }
    }

    // ── Dashboards ───────────────────────────────────────────────────────

    pub fn dashboard(&self, key: &str) -> Option<&Session> {
        self.list.iter().find(|s| s.kind == Kind::Dashboard && s.key == key && s.is_live())
    }

    pub fn dashboard_pty_mut(&mut self, key: &str) -> Option<&mut InstalledDashboard> {
        let s = self
            .list
            .iter_mut()
            .find(|s| s.kind == Kind::Dashboard && s.key == key && s.is_live())?;
        match &mut s.body {
            Body::Pty(pty) => Some(pty),
            _ => None,
        }
    }

    // ── Lookup ───────────────────────────────────────────────────────────

    /// Most recently used live instance of `key`.
    pub fn find_live(&self, key: &str, kind: Kind) -> Option<SessionId> {
        self.list
            .iter()
            .filter(|s| s.key == key && s.kind == kind && s.is_live() && !s.is_stopping())
            .max_by_key(|s| s.last_focus)
            .map(|s| s.id)
    }

    /// Keys of everything still running, for the navbar's ● markers.
    pub fn running_keys(&self) -> HashSet<String> {
        self.list.iter().filter(|s| s.is_live()).map(|s| s.key.clone()).collect()
    }

    pub fn live_count(&self) -> usize {
        self.list.iter().filter(|s| s.is_live()).count()
    }

    /// Labels of sessions that end when tuiOS exits. Apps in their own windows survive.
    pub fn ending_with_tuios(&self) -> Vec<String> {
        self.list
            .iter()
            .filter(|s| s.is_live() && !s.is_external())
            .map(|s| s.label.clone())
            .collect()
    }

    /// PIDs the sampler should measure, and whether it should index processes to
    /// find newly opened windows.
    pub fn watch_list(&self) -> (Vec<(u32, bool)>, bool) {
        let mut pids: Vec<(u32, bool)> = self
            .list
            .iter()
            .filter(|s| s.is_live())
            .filter_map(|s| s.pid().map(|p| (p, true)))
            .collect();
        let me = metrics::read().self_pid;
        if me != 0 {
            pids.push((me, false));
        }
        let discover = self
            .list
            .iter()
            .any(|s| matches!(&s.body, Body::External(e) if e.pid.is_none() && !e.lost) && s.is_live());
        (pids, discover)
    }

    // ── Launching ────────────────────────────────────────────────────────

    /// Lowest instance number not used by a running copy of `key`.
    fn next_instance(&self, key: &str) -> u32 {
        let used: HashSet<u32> = self
            .list
            .iter()
            .filter(|s| s.key == key && s.is_live())
            .map(|s| s.instance)
            .collect();
        (1..).find(|n| !used.contains(n)).unwrap_or(1)
    }

    fn push(&mut self, key: &str, label: &str, kind: Kind, cmd: Vec<String>, body: Body) -> SessionId {
        // A fresh launch supersedes any exited rows left for the same app
        self.list.retain(|s| s.key != key || s.is_live());
        let instance = self.next_instance(key);
        let label = if instance > 1 { format!("{} ({})", label, instance) } else { label.to_string() };
        let id = self.next_id;
        self.next_id += 1;
        self.list.push(Session {
            id,
            key: key.to_string(),
            label,
            kind,
            instance,
            cmd,
            body,
            phase: Phase::Running,
            started: Local::now(),
            started_at: Instant::now(),
            last_focus: Instant::now(),
            restarts: 0,
        });
        id
    }

    fn instance_label(&self, key: &str, label: &str) -> String {
        let n = self.next_instance(key);
        if n > 1 { format!("{} ({})", label, n) } else { label.to_string() }
    }

    pub fn launch_internal(&mut self, key: &str, label: &str) -> SessionId {
        logging::info(&format!("Session start: {} (built-in)", key));
        self.push(key, label, Kind::App, Vec::new(), Body::Internal(InternalApp::launch(key)))
    }

    fn start_pty(&self, label: &str, cmd: &[String]) -> Option<InstalledDashboard> {
        let (cols, rows) = self.container_size;
        InstalledDashboard::start(label, cmd, cols, rows, config::load().scrollback)
    }

    pub fn launch_pty(&mut self, key: &str, label: &str, kind: Kind, cmd: &[String]) -> Option<SessionId> {
        let shown = self.instance_label(key, label);
        let pty = self.start_pty(&shown, cmd)?;
        logging::info(&format!(
            "Session start: {} in the tuiOS container (pid {})",
            key,
            pty.pid().map(|p| p.to_string()).unwrap_or_else(|| "?".into())
        ));
        Some(self.push(key, label, kind, cmd.to_vec(), Body::Pty(pty)))
    }

    fn open_window(cmd: &[String]) -> Option<External> {
        if cmd.is_empty() || !actions::can_open_new_window() || !actions::spawn_in_new_window(cmd) {
            return None;
        }
        Some(External {
            pid: None,
            start_time: None,
            stem: metrics::file_stem_lower(&cmd[0]),
            launched_unix: now_unix(),
            launched_at: Instant::now(),
            lost: false,
        })
    }

    /// Open the app in a new terminal window and track it. None = no window system.
    pub fn launch_external(&mut self, key: &str, label: &str, kind: Kind, cmd: &[String]) -> Option<SessionId> {
        let ext = Self::open_window(cmd)?;
        logging::info(&format!("Session start: {} in a new window", key));
        Some(self.push(key, label, kind, cmd.to_vec(), Body::External(ext)))
    }

    /// Start another copy of an app in the same mode as `id`.
    pub fn launch_another(&mut self, id: SessionId) -> Option<SessionId> {
        let s = self.get(id)?;
        if s.kind != Kind::App {
            return None;
        }
        let (key, cmd) = (s.key.clone(), s.cmd.clone());
        let base = base_label(&s.label, s.instance);
        let (internal, external) = (s.is_builtin(), s.is_external());
        if internal {
            Some(self.launch_internal(&key, &base))
        } else if external {
            self.launch_external(&key, &base, Kind::App, &cmd)
        } else {
            self.launch_pty(&key, &base, Kind::App, &cmd)
        }
    }

    // ── Stopping ─────────────────────────────────────────────────────────

    /// The user closed the app (q / Backspace / Quit in the Task Manager).
    pub fn close(&mut self, id: SessionId) -> Vec<SessionEvent> {
        self.begin_stop(id, After::Remove)
    }

    pub fn restart(&mut self, id: SessionId) -> Vec<SessionEvent> {
        self.begin_stop(id, After::Restart)
    }

    /// Embedded app → its own terminal window. A running process cannot change
    /// terminals, so this quits the embedded copy and launches a fresh one.
    pub fn move_to_window(&mut self, id: SessionId) -> Vec<SessionEvent> {
        self.begin_stop(id, After::ToWindow)
    }

    /// Own-window app → the tuiOS container (quit the window copy, relaunch embedded).
    pub fn move_to_container(&mut self, id: SessionId) -> Vec<SessionEvent> {
        self.begin_stop(id, After::ToContainer)
    }

    pub fn force_kill(&mut self, id: SessionId) {
        let Some(s) = self.get_mut(id) else { return };
        let then = match s.phase {
            Phase::Stopping { then, .. } => then,
            _ => After::Remove,
        };
        if let Some(pid) = s.pid() {
            proc_control::resume(pid, s.is_pty());
        }
        match &mut s.body {
            Body::Pty(p) => p.kill(),
            Body::External(e) => {
                if let Some(pid) = e.pid {
                    proc_control::kill(pid);
                }
            }
            Body::Internal(_) => {}
        }
        logging::info(&format!("Session force-killed: {}", s.label));
        s.phase = Phase::Stopping { deadline: Instant::now() + KILL_GRACE, then, forced: true };
    }

    /// Remove an exited (or unmanageable) row.
    pub fn dismiss(&mut self, id: SessionId) -> Vec<SessionEvent> {
        self.remove(id)
    }

    fn remove(&mut self, id: SessionId) -> Vec<SessionEvent> {
        let mut events = Vec::new();
        if let Some(i) = self.index(id) {
            let s = self.list.remove(i);
            if self.foreground == Some(id) {
                self.foreground = None;
                events.push(SessionEvent::LeftForeground);
            }
            if s.kind == Kind::Dashboard {
                events.push(SessionEvent::DashboardGone(s.key.clone()));
            }
        }
        events
    }

    fn begin_stop(&mut self, id: SessionId, then: After) -> Vec<SessionEvent> {
        let timeout = config::load().kill_timeout;
        let Some(s) = self.get_mut(id) else { return Vec::new() };
        logging::info(&format!("Session stop requested: {} ({:?})", s.label, then));

        if let Phase::Exited { .. } = s.phase {
            return self.finish(id, then, None);
        }
        if let Some(pid) = s.pid() {
            if s.phase == Phase::Suspended {
                proc_control::resume(pid, s.is_pty());
            }
        }

        match &mut s.body {
            Body::Internal(app) => {
                app.stop();
                return self.finish(id, then, None);
            }
            Body::Pty(p) => {
                if p.poll_exit().is_some() {
                    return self.finish(id, then, None);
                }
                p.interrupt();
                if let Some(pid) = p.pid() {
                    proc_control::terminate(pid);
                }
                s.phase = Phase::Stopping { deadline: Instant::now() + timeout, then, forced: false };
            }
            Body::External(e) => {
                let Some(pid) = e.pid else {
                    // Nothing to signal yet — the window has to be closed by hand
                    return self.finish(id, then, None);
                };
                let forced = !proc_control::has_graceful_terminate();
                if forced {
                    proc_control::kill(pid);
                } else {
                    proc_control::terminate(pid);
                }
                let wait = if forced { KILL_GRACE } else { timeout };
                s.phase = Phase::Stopping { deadline: Instant::now() + wait, then, forced };
            }
        }

        // Leaving the container right away feels responsive; the exit finishes in the background
        let mut events = Vec::new();
        if self.foreground == Some(id) && matches!(then, After::Remove | After::ToWindow) {
            self.foreground = None;
            events.push(SessionEvent::LeftForeground);
        }
        events
    }

    /// The process is gone; do whatever the stop was for.
    fn finish(&mut self, id: SessionId, then: After, code: Option<u32>) -> Vec<SessionEvent> {
        let Some(s) = self.get(id) else { return Vec::new() };
        let (key, kind, cmd, label) = (s.key.clone(), s.kind, s.cmd.clone(), s.label.clone());
        let was_internal = s.is_builtin();
        let was_external = s.is_external();
        logging::info(&format!("Session ended: {} (exit {:?}, then {:?})", label, code, then));

        match then {
            After::Remove => self.remove(id),
            After::Restart => {
                let body = if was_internal {
                    Some(Body::Internal(InternalApp::launch(&key)))
                } else if was_external {
                    Self::open_window(&cmd).map(Body::External)
                } else {
                    self.start_pty(&label, &cmd).map(Body::Pty)
                };
                match body {
                    Some(body) => {
                        let s = self.get_mut(id).expect("session checked above");
                        s.body = body;
                        s.phase = Phase::Running;
                        s.started = Local::now();
                        s.started_at = Instant::now();
                        s.restarts += 1;
                        vec![SessionEvent::Toast(format!("Restarted {}", label))]
                    }
                    None => {
                        let mut ev = self.remove(id);
                        ev.push(SessionEvent::Toast(format!("Could not restart {}", label)));
                        ev
                    }
                }
            }
            After::ToWindow => match Self::open_window(&cmd) {
                Some(ext) => {
                    let s = self.get_mut(id).expect("session checked above");
                    s.body = Body::External(ext);
                    s.phase = Phase::Running;
                    s.started = Local::now();
                    s.started_at = Instant::now();
                    let mut ev = Vec::new();
                    if self.foreground == Some(id) {
                        self.foreground = None;
                        ev.push(SessionEvent::LeftForeground);
                    }
                    if kind == Kind::Dashboard {
                        ev.push(SessionEvent::DashboardGone(key));
                    }
                    ev.push(SessionEvent::Toast(format!("{} opened in a new window", label)));
                    ev
                }
                None => {
                    let mut ev = self.remove(id);
                    ev.push(SessionEvent::RunForeground(cmd));
                    ev
                }
            },
            After::ToContainer => match self.start_pty(&label, &cmd) {
                Some(pty) => {
                    let s = self.get_mut(id).expect("session checked above");
                    s.body = Body::Pty(pty);
                    s.phase = Phase::Running;
                    s.started = Local::now();
                    s.started_at = Instant::now();
                    vec![SessionEvent::Foreground(id)]
                }
                None => {
                    let mut ev = self.remove(id);
                    ev.push(SessionEvent::Toast(format!("Could not start {} in the container", label)));
                    ev
                }
            },
        }
    }

    // ── Suspend ──────────────────────────────────────────────────────────

    pub fn suspend(&mut self, id: SessionId) -> bool {
        let Some(s) = self.get_mut(id) else { return false };
        if s.phase != Phase::Running {
            return false;
        }
        let Some(pid) = s.pid() else { return false };
        if proc_control::suspend(pid, s.is_pty()) {
            s.phase = Phase::Suspended;
            logging::info(&format!("Session suspended: {}", s.label));
            true
        } else {
            false
        }
    }

    pub fn resume(&mut self, id: SessionId) -> bool {
        let Some(s) = self.get_mut(id) else { return false };
        if s.phase != Phase::Suspended {
            return false;
        }
        let Some(pid) = s.pid() else { return false };
        let ok = proc_control::resume(pid, s.is_pty());
        // Even if the signal failed the process is not ours to freeze any more
        s.phase = Phase::Running;
        logging::info(&format!("Session resumed: {}", s.label));
        ok
    }

    // ── Per-frame upkeep ─────────────────────────────────────────────────

    /// Notice exits, finish stops, find new windows' PIDs. Called every loop pass.
    pub fn pump(&mut self) -> Vec<SessionEvent> {
        let mut events = Vec::new();
        let cfg = config::load();
        let tracked: HashSet<u32> = self.list.iter().filter_map(|s| s.pid()).collect();
        let now = Instant::now();

        let mut exits: Vec<(SessionId, Option<u32>)> = Vec::new();
        for s in &mut self.list {
            if !s.is_live() {
                continue;
            }
            let exited: Option<Option<u32>> = match &mut s.body {
                Body::Internal(_) => None,
                Body::Pty(p) => p.poll_exit().map(Some),
                Body::External(e) => match e.pid {
                    None => {
                        if !e.lost {
                            if let Some((pid, start)) = metrics::find_process(&e.stem, e.launched_unix, &tracked) {
                                e.pid = Some(pid);
                                e.start_time = Some(start);
                                logging::info(&format!("Session {} found in its window (pid {})", s.label, pid));
                            } else if e.launched_at.elapsed() > DISCOVERY_WINDOW {
                                e.lost = true;
                                logging::warn(&format!("Session {}: could not find the app in its new window", s.label));
                            }
                        }
                        None
                    }
                    Some(pid) => (!proc_control::is_alive(pid)).then_some(None),
                },
            };
            if let Some(code) = exited {
                exits.push((s.id, code));
                continue;
            }
            if let Phase::Stopping { deadline, then, forced } = s.phase {
                if now >= deadline {
                    if forced {
                        // Refuses to die (or we lost track) — stop waiting on it
                        exits.push((s.id, None));
                    } else {
                        match &mut s.body {
                            Body::Pty(p) => p.kill(),
                            Body::External(e) => {
                                if let Some(pid) = e.pid {
                                    proc_control::kill(pid);
                                }
                            }
                            Body::Internal(_) => {}
                        }
                        logging::info(&format!("Session {} did not quit in time; force-killed", s.label));
                        s.phase = Phase::Stopping { deadline: now + KILL_GRACE, then, forced: true };
                    }
                }
            }
        }

        for (id, code) in exits {
            let Some(s) = self.get(id) else { continue };
            match s.phase.clone() {
                Phase::Stopping { then, .. } => events.extend(self.finish(id, then, code)),
                _ => events.extend(self.exited_on_its_own(id, code, cfg.exit_notifications)),
            }
        }
        events
    }

    fn exited_on_its_own(&mut self, id: SessionId, code: Option<u32>, notify: bool) -> Vec<SessionEvent> {
        let Some(s) = self.get(id) else { return Vec::new() };
        let (label, kind, key, external) = (s.label.clone(), s.kind, s.key.clone(), s.is_external());
        let shown = self.foreground == Some(id);
        logging::info(&format!("Session exited: {} (code {:?})", label, code));

        // Quitting from inside the app, or closing its window, is a normal close
        if external || (shown && code == Some(0)) {
            return self.remove(id);
        }

        let mut events = Vec::new();
        if shown {
            self.foreground = None;
            events.push(SessionEvent::LeftForeground);
        }
        if let Some(s) = self.get_mut(id) {
            s.phase = Phase::Exited { code, at: Local::now() };
        }
        if kind == Kind::Dashboard {
            events.push(SessionEvent::DashboardGone(key));
        }
        if notify {
            let what = match code {
                Some(0) => format!("{} finished", label),
                Some(c) => format!("{} exited with code {}", label, c),
                None => format!("{} exited", label),
            };
            events.push(SessionEvent::Toast(format!("{} — see the Task Manager to restart it", what)));
        }
        events
    }

    /// Stop everything that would otherwise be orphaned when tuiOS exits.
    pub fn shutdown_all(&mut self) {
        for s in &mut self.list {
            match &mut s.body {
                Body::Internal(app) => app.stop(),
                Body::Pty(p) => {
                    if let Some(pid) = p.pid() {
                        proc_control::resume(pid, true);
                    }
                    p.kill();
                }
                // Own-window apps outlive tuiOS, so never leave one frozen
                Body::External(e) => {
                    if let (Some(pid), Phase::Suspended) = (e.pid, &s.phase) {
                        proc_control::resume(pid, false);
                    }
                }
            }
        }
        self.list.clear();
        self.foreground = None;
    }
}

/// "Notes (2)" → "Notes" for the given instance number.
fn base_label(label: &str, instance: u32) -> String {
    label
        .strip_suffix(&format!(" ({})", instance))
        .unwrap_or(label)
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn instance_labels_round_trip() {
        assert_eq!(base_label("Notes (2)", 2), "Notes");
        assert_eq!(base_label("Notes", 1), "Notes");
    }

    #[test]
    fn built_in_sessions_close_immediately() {
        let mut s = Sessions::new();
        let id = s.launch_internal("CharacterSet", "Character Set");
        s.set_foreground(id);
        assert_eq!(s.foreground, Some(id));
        let second = s.launch_another(id).expect("second instance");
        assert_eq!(s.get(second).unwrap().label, "Character Set (2)");
        let events = s.close(id);
        assert!(s.get(id).is_none());
        assert!(events.iter().any(|e| matches!(e, SessionEvent::LeftForeground)));
        assert_eq!(s.find_live("CharacterSet", Kind::App), Some(second));
    }
}
