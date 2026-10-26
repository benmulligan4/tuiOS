/// Install queue — serialised install/uninstall jobs with per-item output logs.
///
/// Jobs run one at a time. Each item keeps its own captured terminal output so the
/// user can select a finished job and re-read what happened.

use super::state::InstallLocation;

/// Per-item log cap. Long `cargo build` runs can produce tens of thousands of
/// lines, and every item keeps its own copy.
pub const MAX_LOG_LINES: usize = 2000;

/// Where an operation acts. Two apps can be installed from different sources but
/// only ever land in one of these two places, so this is what conflicts are keyed on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueueTarget {
    Path,
    Downloads,
}

impl QueueTarget {
    pub fn label(&self) -> &'static str {
        match self {
            QueueTarget::Path => "PATH",
            QueueTarget::Downloads => "Downloads",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueueOp {
    Install(InstallLocation),
    Uninstall { path: bool, downloads: bool },
}

impl QueueOp {
    pub fn is_install(&self) -> bool {
        matches!(self, QueueOp::Install(_))
    }

    pub fn verb(&self) -> &'static str {
        if self.is_install() { "Install" } else { "Uninstall" }
    }

    pub fn targets(&self) -> Vec<QueueTarget> {
        match self {
            QueueOp::Install(InstallLocation::Local) => vec![QueueTarget::Downloads],
            QueueOp::Install(_) => vec![QueueTarget::Path],
            QueueOp::Uninstall { path, downloads } => {
                let mut v = Vec::new();
                if *path { v.push(QueueTarget::Path); }
                if *downloads { v.push(QueueTarget::Downloads); }
                v
            }
        }
    }

    /// Short destination text for the queue row, e.g. "PATH (Git)".
    pub fn target_label(&self) -> String {
        match self {
            QueueOp::Install(InstallLocation::Global) => "PATH".to_string(),
            QueueOp::Install(InstallLocation::Git) => "PATH (Git)".to_string(),
            QueueOp::Install(InstallLocation::Local) => "Downloads".to_string(),
            QueueOp::Uninstall { .. } => self
                .targets()
                .iter()
                .map(|t| t.label())
                .collect::<Vec<_>>()
                .join(" + "),
        }
    }

    /// Arrow shown between the app name and the destination.
    pub fn arrow(&self) -> &'static str {
        if self.is_install() { "→" } else { "←" }
    }

    /// "Installing to PATH" / "Uninstalling from Downloads"
    pub fn destination_phrase(&self) -> String {
        if self.is_install() {
            format!("Installing to {}", self.target_label())
        } else {
            format!("Uninstalling from {}", self.target_label())
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueueStatus {
    Pending,
    Running,
    Done,
    Failed,
}

impl QueueStatus {
    pub fn label(&self) -> &'static str {
        match self {
            QueueStatus::Pending => "pending",
            QueueStatus::Running => "running",
            QueueStatus::Done => "done",
            QueueStatus::Failed => "failed",
        }
    }

    pub fn glyph(&self) -> &'static str {
        match self {
            QueueStatus::Pending => "○",
            QueueStatus::Running => "▸",
            QueueStatus::Done => "✓",
            QueueStatus::Failed => "✗",
        }
    }

    pub fn is_finished(&self) -> bool {
        matches!(self, QueueStatus::Done | QueueStatus::Failed)
    }
}

#[derive(Debug, Clone)]
pub struct QueueItem {
    pub id: u64,
    pub key: String,
    pub label: String,
    pub op: QueueOp,
    pub status: QueueStatus,
    pub log: Vec<String>,
    /// Failure reason or "files left behind" warning, shown under the row.
    pub note: Option<String>,
    /// Already included in a drain summary, so the next summary skips it.
    pub summarised: bool,
}

impl QueueItem {
    pub fn push_log(&mut self, line: String) {
        self.log.push(line);
        self.trim_log();
    }

    pub fn extend_log(&mut self, lines: impl IntoIterator<Item = String>) {
        self.log.extend(lines);
        self.trim_log();
    }

    fn trim_log(&mut self) {
        if self.log.len() > MAX_LOG_LINES {
            let overflow = self.log.len() - MAX_LOG_LINES;
            self.log.drain(0..overflow);
            self.log.insert(0, format!("… {} earlier lines trimmed …", overflow));
        }
    }
}

/// One entry in the menu that opens under a selected queue row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueueItemAction {
    ViewLog,
    Retry,
    Cancel,
    MoveUp,
    MoveDown,
}

impl QueueItemAction {
    pub fn label(&self) -> &'static str {
        match self {
            QueueItemAction::ViewLog => "View Log",
            QueueItemAction::Retry => "Retry Job",
            QueueItemAction::Cancel => "Cancel Job",
            QueueItemAction::MoveUp => "Move Up",
            QueueItemAction::MoveDown => "Move Down",
        }
    }
}

#[derive(Debug, Clone)]
pub struct InstallQueue {
    pub items: Vec<QueueItem>,
    pub cursor: usize,
    /// The row at `cursor` is selected and showing its per-job options.
    pub item_focused: bool,
    /// Position within the selected row's option menu.
    pub option_cursor: usize,
    /// When paused the current item finishes but nothing new starts.
    pub paused: bool,
    next_id: u64,
}

impl InstallQueue {
    pub fn new() -> Self {
        Self {
            items: Vec::new(),
            cursor: 0,
            item_focused: false,
            option_cursor: 0,
            paused: false,
            next_id: 1,
        }
    }

    /// Options offered for a row, in menu order.
    pub fn item_actions(&self, index: usize) -> Vec<QueueItemAction> {
        let Some(item) = self.items.get(index) else { return Vec::new() };
        let mut out = vec![QueueItemAction::ViewLog];
        if item.status == QueueStatus::Failed {
            out.push(QueueItemAction::Retry);
        }
        if item.status == QueueStatus::Pending {
            out.push(QueueItemAction::Cancel);
        }
        if self.can_move(index, true) {
            out.push(QueueItemAction::MoveUp);
        }
        if self.can_move(index, false) {
            out.push(QueueItemAction::MoveDown);
        }
        out
    }

    pub fn clamp_option_cursor(&mut self) {
        let len = self.item_actions(self.cursor).len();
        if self.option_cursor >= len {
            self.option_cursor = len.saturating_sub(1);
        }
    }

    /// Queue a job, or explain why it clashes with something already queued.
    pub fn enqueue(&mut self, key: &str, label: &str, op: QueueOp) -> Result<u64, String> {
        if let Some(msg) = self.conflict(key, label, op) {
            return Err(msg);
        }
        let id = self.next_id;
        self.next_id += 1;
        self.items.push(QueueItem {
            id,
            key: key.to_string(),
            label: label.to_string(),
            op,
            status: QueueStatus::Pending,
            log: Vec::new(),
            note: None,
            summarised: false,
        });
        Ok(id)
    }

    /// An app may be queued twice for different destinations, but never twice for
    /// the same one, and an install may not race an uninstall of the same place.
    fn conflict(&self, key: &str, label: &str, op: QueueOp) -> Option<String> {
        let wanted = op.targets();
        for item in &self.items {
            if item.key != key || item.status.is_finished() {
                continue;
            }
            let clash = item.op.targets().into_iter().find(|t| wanted.contains(t))?;
            let running = item.status == QueueStatus::Running;
            return Some(match (item.op.is_install(), running) {
                (true, true) => format!("{} is currently being installed to {}", label, clash.label()),
                (true, false) => format!("{} is already queued to install to {}", label, clash.label()),
                (false, true) => format!("{} is currently being uninstalled from {}", label, clash.label()),
                (false, false) => {
                    format!("{} is already queued to uninstall from {}", label, clash.label())
                }
            });
        }
        None
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Anything still to do — drives the UI lock.
    pub fn is_active(&self) -> bool {
        self.items
            .iter()
            .any(|i| matches!(i.status, QueueStatus::Pending | QueueStatus::Running))
    }

    pub fn has_active_for(&self, key: &str) -> bool {
        self.items.iter().any(|i| {
            i.key == key && matches!(i.status, QueueStatus::Pending | QueueStatus::Running)
        })
    }

    pub fn running_index(&self) -> Option<usize> {
        self.items.iter().position(|i| i.status == QueueStatus::Running)
    }

    pub fn running(&self) -> Option<&QueueItem> {
        self.running_index().map(|i| &self.items[i])
    }

    pub fn running_mut(&mut self) -> Option<&mut QueueItem> {
        self.running_index().map(move |i| &mut self.items[i])
    }

    pub fn item_by_id(&self, id: u64) -> Option<&QueueItem> {
        self.items.iter().find(|i| i.id == id)
    }

    pub fn item_by_id_mut(&mut self, id: u64) -> Option<&mut QueueItem> {
        self.items.iter_mut().find(|i| i.id == id)
    }

    pub fn pending_count(&self) -> usize {
        self.items.iter().filter(|i| i.status == QueueStatus::Pending).count()
    }

    pub fn active_count(&self) -> usize {
        self.items
            .iter()
            .filter(|i| matches!(i.status, QueueStatus::Pending | QueueStatus::Running))
            .count()
    }

    /// 1-based position of the running item among the active set, for "(2 of 5)".
    pub fn running_position(&self) -> Option<(usize, usize)> {
        let total = self.items.iter().filter(|i| !i.status.is_finished()).count();
        let idx = self
            .items
            .iter()
            .filter(|i| !i.status.is_finished())
            .position(|i| i.status == QueueStatus::Running)?;
        Some((idx + 1, total))
    }

    /// Promote the next pending job to running. Returns its id.
    pub fn start_next(&mut self) -> Option<u64> {
        if self.paused || self.running_index().is_some() {
            return None;
        }
        let idx = self.items.iter().position(|i| i.status == QueueStatus::Pending)?;
        self.items[idx].status = QueueStatus::Running;
        Some(self.items[idx].id)
    }

    /// Cancelling drops the job entirely — it never ran, so there is nothing to show.
    pub fn cancel(&mut self, index: usize) -> Result<String, String> {
        let item = self.items.get(index).ok_or_else(|| "No item selected".to_string())?;
        match item.status {
            QueueStatus::Pending => {
                let msg = format!("Cancelled {} {}", item.op.verb().to_lowercase(), item.label);
                self.items.remove(index);
                self.item_focused = false;
                self.clamp_cursor();
                Ok(msg)
            }
            QueueStatus::Running => {
                Err("This job has already started and cannot be cancelled".to_string())
            }
            _ => Err("This job has already finished".to_string()),
        }
    }

    pub fn cancel_all_pending(&mut self) -> usize {
        let before = self.items.len();
        self.items.retain(|i| i.status != QueueStatus::Pending);
        self.item_focused = false;
        self.clamp_cursor();
        before - self.items.len()
    }

    pub fn clear_finished(&mut self) -> usize {
        let before = self.items.len();
        self.items.retain(|i| !i.status.is_finished());
        self.clamp_cursor();
        before - self.items.len()
    }

    /// Re-queue a failed job at the back of the queue.
    pub fn retry(&mut self, index: usize) -> Result<String, String> {
        let (key, label, op, status) = {
            let item = self.items.get(index).ok_or_else(|| "No item selected".to_string())?;
            (item.key.clone(), item.label.clone(), item.op, item.status)
        };
        if status != QueueStatus::Failed {
            return Err("Only failed jobs can be retried".to_string());
        }
        self.enqueue(&key, &label, op)?;
        self.items.remove(index);
        self.clamp_cursor();
        Ok(format!("Re-queued {} {}", op.verb().to_lowercase(), label))
    }

    /// Reorder within the pending block. A job can never be moved above the one
    /// that is running, or above anything that has already finished.
    pub fn can_move(&self, index: usize, up: bool) -> bool {
        if self.items.get(index).map(|i| i.status) != Some(QueueStatus::Pending) {
            return false;
        }
        let other = if up {
            match index.checked_sub(1) {
                Some(o) => o,
                None => return false,
            }
        } else if index + 1 < self.items.len() {
            index + 1
        } else {
            return false;
        };
        self.items[other].status == QueueStatus::Pending
    }

    pub fn move_item(&mut self, index: usize, up: bool) -> bool {
        if !self.can_move(index, up) {
            return false;
        }
        let other = if up { index - 1 } else { index + 1 };
        self.items.swap(index, other);
        self.cursor = other;
        true
    }

    pub fn clamp_cursor(&mut self) {
        if self.items.is_empty() {
            self.cursor = 0;
            self.item_focused = false;
        } else if self.cursor >= self.items.len() {
            self.cursor = self.items.len() - 1;
        }
        self.clamp_option_cursor();
    }

    pub fn selected(&self) -> Option<&QueueItem> {
        self.items.get(self.cursor)
    }

    /// Roll up everything that finished since the last summary.
    pub fn take_summary(&mut self) -> Option<String> {
        let mut done = 0;
        let mut failed = 0;
        let mut notes: Vec<String> = Vec::new();
        for item in self.items.iter_mut().filter(|i| i.status.is_finished() && !i.summarised) {
            item.summarised = true;
            if item.status == QueueStatus::Done {
                done += 1;
            } else {
                failed += 1;
                if let Some(n) = &item.note {
                    notes.push(format!("{}: {}", item.label, n));
                }
            }
        }
        if done + failed == 0 {
            return None;
        }
        let mut parts = Vec::new();
        if done > 0 { parts.push(format!("{} succeeded", done)); }
        if failed > 0 { parts.push(format!("{} failed", failed)); }
        let mut msg = format!("Install queue finished — {}", parts.join(", "));
        for note in notes.iter().take(2) {
            msg.push_str(&format!("  •  {}", note));
        }
        Some(msg)
    }
}

impl Default for InstallQueue {
    fn default() -> Self {
        Self::new()
    }
}
