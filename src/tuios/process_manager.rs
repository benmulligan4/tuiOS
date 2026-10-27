/// tuiOS — subprocess app launcher, tracker, and log manager.

use chrono::{DateTime, Utc};
use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;

use super::models::{AppStatus, RunningApp};

const LOGS_DIR: &str = "logs";

fn ensure_logs_dir() {
    fs::create_dir_all(LOGS_DIR).ok();
}

fn log_path(name: &str) -> String {
    format!("{}/{}.log", LOGS_DIR, name)
}

fn system_log_path() -> String {
    format!("{}/apps.log", LOGS_DIR)
}

fn write_system_log(line: &str) {
    ensure_logs_dir();
    if let Ok(mut file) = OpenOptions::new()
        .create(true)
        .append(true)
        .open(system_log_path())
    {
        writeln!(file, "{}", line).ok();
    }
}

struct ProcessEntry {
    child: Child,
    app: RunningApp,
    _log_file: File,
}

lazy_static::lazy_static! {
    static ref PROCESSES: Mutex<HashMap<String, ProcessEntry>> = Mutex::new(HashMap::new());
}

/// Launch an external app as a supervised subprocess.
/// Returns the RunningApp entry. If already running, returns existing entry.
pub fn launch(name: &str, cmd: &[String]) -> Option<RunningApp> {
    ensure_logs_dir();

    let mut processes = PROCESSES.lock().ok()?;

    // Check if already running
    if let Some(entry) = processes.get_mut(name) {
        match entry.child.try_wait() {
            Ok(None) => return Some(entry.app.clone()), // still running
            _ => {} // process ended, will remove below
        }
        // Remove stale entry
        processes.remove(name);
    }

    if cmd.is_empty() {
        return None;
    }

    let log_file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path(name))
        .ok()?;

    let log_stdout = log_file.try_clone().ok()?;
    let log_stderr = log_file.try_clone().ok()?;

    let child = Command::new(&cmd[0])
        .args(&cmd[1..])
        .stdout(Stdio::from(log_stdout))
        .stderr(Stdio::from(log_stderr))
        .spawn()
        .ok()?;

    let now: DateTime<Utc> = Utc::now();
    let pid = child.id();
    let app = RunningApp {
        name: name.to_string(),
        pid: Some(pid),
        status: AppStatus::Running,
        start_time: now,
        log_path: log_path(name),
    };

    write_system_log(&format!(
        "{} | {} | START | {}",
        now.to_rfc3339(),
        name,
        pid
    ));

    let entry = ProcessEntry {
        child,
        app: app.clone(),
        _log_file: log_file,
    };
    processes.insert(name.to_string(), entry);

    Some(app)
}

/// Terminate a running subprocess by name.
#[allow(dead_code)]
pub fn stop(name: &str) {
    let mut processes = match PROCESSES.lock() {
        Ok(p) => p,
        Err(_) => return,
    };

    if let Some(mut entry) = processes.remove(name) {
        entry.child.kill().ok();
        let returncode = entry.child.wait().map(|s| s.code().unwrap_or(-1)).unwrap_or(-1);
        let now = Utc::now();
        write_system_log(&format!(
            "{} | {} | STOP | {}",
            now.to_rfc3339(),
            name,
            returncode
        ));
    }
}

/// Return the current AppStatus for a named app.
#[allow(dead_code)]
pub fn get_status(name: &str) -> AppStatus {
    let mut processes = match PROCESSES.lock() {
        Ok(p) => p,
        Err(_) => return AppStatus::Stopped,
    };

    if let Some(entry) = processes.get_mut(name) {
        match entry.child.try_wait() {
            Ok(None) => AppStatus::Running,
            _ => {
                processes.remove(name);
                AppStatus::Stopped
            }
        }
    } else {
        AppStatus::Stopped
    }
}

/// Return the last n lines from the app's log file.
pub fn log_tail(name: &str, n: usize) -> Vec<String> {
    let path = log_path(name);
    let file = match File::open(&path) {
        Ok(f) => f,
        Err(_) => return Vec::new(),
    };

    let reader = BufReader::new(file);
    let lines: Vec<String> = reader
        .lines()
        .filter_map(|l| l.ok())
        .collect();

    let start = if lines.len() > n { lines.len() - n } else { 0 };
    lines[start..].to_vec()
}

/// Return a snapshot list of all tracked apps with refreshed status.
#[allow(dead_code)]
pub fn list_running() -> Vec<RunningApp> {
    let mut processes = match PROCESSES.lock() {
        Ok(p) => p,
        Err(_) => return Vec::new(),
    };

    let mut result = Vec::new();
    let mut stale = Vec::new();

    for (name, entry) in processes.iter_mut() {
        match entry.child.try_wait() {
            Ok(None) => {
                entry.app.status = AppStatus::Running;
                result.push(entry.app.clone());
            }
            _ => {
                stale.push(name.clone());
            }
        }
    }

    for name in stale {
        processes.remove(&name);
    }

    result
}
