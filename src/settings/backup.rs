/// Backup & Restore — export the config files into one JSON bundle in `backups/`, and
/// import one back by merging it into this device's settings.
///
/// Importing never copies apps.json / dashboards.json (they describe binaries on the
/// other device); they only tell us which apps to offer for reinstalling here.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::PathBuf;

use serde_json::{Map, Value};

use crate::app_store::actions as store;
use crate::app_store::state::InstallLocation;
use crate::settings::persistence;
use crate::task_manager::clean_text;
use crate::tuios::registry;

const FORMAT: u64 = 1;
const FILES: &[&str] = &["settings.json", "apps.json", "dashboards.json", "registered-apps.json"];

/// Settings that only make sense on the device they were made on; never imported.
const DEVICE_SPECIFIC: &[&str] = &[
    "display.fullscreen",
    "appearance.new_window_keeps_tuios_open",
    "task_manager.nvidia_stats",
    "task_manager.startup_apps",
    "task_manager.startup_modes",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackupKind {
    Export,
    /// Made automatically before an import or Restore Defaults.
    Auto,
}

#[derive(Debug, Clone)]
pub struct BackupInfo {
    pub path: PathBuf,
    pub kind: BackupKind,
    pub label: String,
    pub device: String,
    pub os: String,
    pub created: String,
    created_unix: i64,
}

impl BackupInfo {
    pub fn title(&self) -> String {
        match self.kind {
            BackupKind::Auto => "Automatic backup".into(),
            BackupKind::Export if self.label.is_empty() => "Export".into(),
            BackupKind::Export => self.label.clone(),
        }
    }

    pub fn file_name(&self) -> String {
        self.path.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default()
    }
}

pub fn backups_dir() -> PathBuf {
    std::env::current_dir().unwrap_or_default().join("backups")
}

fn device_name() -> String {
    sysinfo::System::host_name()
        .map(|h| clean_text(h.trim()))
        .filter(|h| !h.is_empty())
        .unwrap_or_else(|| "tuiOS".into())
}

/// Filename-safe version of a label: letters, digits, '-' and '_' only.
fn slug(text: &str, max: usize) -> String {
    let mut out = String::new();
    for c in text.chars() {
        if c.is_ascii_alphanumeric() || c == '_' {
            out.push(c);
        } else if (c == ' ' || c == '-') && !out.ends_with('-') {
            out.push('-');
        }
    }
    out.trim_matches('-').chars().take(max).collect()
}

fn read_info(path: PathBuf, bundle: &Value) -> Option<BackupInfo> {
    bundle.get("tuios_backup")?.as_u64()?;
    if !bundle.get("files")?.is_object() {
        return None;
    }
    let text = |key: &str| bundle.get(key).and_then(|v| v.as_str()).map(clean_text).unwrap_or_default();
    Some(BackupInfo {
        kind: if text("type") == "auto" { BackupKind::Auto } else { BackupKind::Export },
        label: text("label"),
        device: text("device"),
        os: text("os"),
        created: text("created"),
        created_unix: bundle.get("created_unix").and_then(|v| v.as_i64()).unwrap_or(0),
        path,
    })
}

fn load_bundle(path: &PathBuf) -> Result<Value, String> {
    let text = fs::read_to_string(path).map_err(|e| format!("Could not read backup: {}", e))?;
    serde_json::from_str(&text).map_err(|_| "That backup file is damaged or not a tuiOS backup".to_string())
}

/// Write every config file into one bundle in the backups folder.
pub fn export(label: &str, kind: BackupKind) -> Result<BackupInfo, String> {
    let dir = backups_dir();
    fs::create_dir_all(&dir).map_err(|e| format!("Could not create {}: {}", dir.display(), e))?;

    let mut files = Map::new();
    files.insert("settings.json".into(), persistence::load());
    for name in FILES.iter().filter(|n| **n != "settings.json") {
        let parsed = fs::read_to_string(registry::config_dir().join(name))
            .ok()
            .and_then(|t| serde_json::from_str::<Value>(&t).ok());
        if let Some(value) = parsed {
            files.insert(name.to_string(), value);
        }
    }

    let now = chrono::Local::now();
    let device = device_name();
    let label = clean_text(label.trim());
    let prefix = if kind == BackupKind::Auto { "auto-backup" } else { "tuios-settings" };
    let device_slug = slug(&device, 24);
    let mut name = format!(
        "{}_{}_{}",
        prefix,
        if device_slug.is_empty() { "device".to_string() } else { device_slug },
        now.format("%Y-%m-%d_%H%M%S")
    );
    let label_slug = slug(&label, 32);
    if !label_slug.is_empty() {
        name.push('_');
        name.push_str(&label_slug);
    }
    let path = dir.join(format!("{}.json", name));

    let bundle = serde_json::json!({
        "tuios_backup": FORMAT,
        "type": if kind == BackupKind::Auto { "auto" } else { "export" },
        "label": label,
        "device": device,
        "os": std::env::consts::OS,
        "created": now.format("%Y-%m-%d %H:%M").to_string(),
        "created_unix": now.timestamp(),
        "files": files,
    });
    let text = serde_json::to_string_pretty(&bundle).map_err(|e| e.to_string())?;
    fs::write(&path, text).map_err(|e| format!("Could not write {}: {}", path.display(), e))?;
    crate::utilities::logging::settings(&format!("Settings backed up to {}", path.display()));
    read_info(path, &bundle).ok_or_else(|| "Backup written but unreadable".into())
}

/// Every backup in the folder, newest first. Files that are not tuiOS backups are ignored.
pub fn list() -> Vec<BackupInfo> {
    let Ok(entries) = fs::read_dir(backups_dir()) else { return Vec::new() };
    let mut out: Vec<BackupInfo> = entries
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().map_or(false, |x| x == "json"))
        .filter_map(|p| {
            let bundle = load_bundle(&p).ok()?;
            read_info(p, &bundle)
        })
        .collect();
    out.sort_by(|a, b| b.created_unix.cmp(&a.created_unix).then_with(|| b.path.cmp(&a.path)));
    out
}

pub fn delete(info: &BackupInfo) -> Result<(), String> {
    fs::remove_file(&info.path).map_err(|e| format!("Could not delete {}: {}", info.file_name(), e))?;
    crate::utilities::logging::settings(&format!("Deleted backup {}", info.file_name()));
    Ok(())
}

/// Back up the current settings, then reset settings.json to the defaults.
pub fn restore_defaults() -> Result<String, String> {
    export("", BackupKind::Auto).map_err(|e| format!("Not restored — could not back up first: {}", e))?;
    persistence::save(&persistence::default_settings());
    crate::utilities::logging::settings("Settings restored to defaults");
    Ok("Settings restored to defaults. Your previous settings were saved as an automatic backup.".into())
}

// ── Import ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct Change {
    keys: Vec<String>,
    value: Value,
    /// "Appearance › Accent Color"
    pub name: String,
    pub old: String,
    pub new: String,
    /// Points at this app/dashboard, which is only applied if it gets queued for install.
    pub needs: Option<String>,
}

#[derive(Debug, Clone)]
pub struct MissingApp {
    pub key: String,
    pub label: String,
    pub dashboard: bool,
    /// How to install it here; None = this App Store has no entry for it.
    pub location: Option<InstallLocation>,
    pub repository: String,
    /// App Store entry to add first, when this device's store has never heard of the app.
    new_entry: Option<Value>,
}

impl MissingApp {
    pub fn unknown_here(&self) -> bool {
        self.new_entry.is_some()
    }
}

#[derive(Debug, Clone)]
pub struct ImportPlan {
    pub info: BackupInfo,
    pub changes: Vec<Change>,
    /// Settings left alone because they belong to the other device.
    pub device_specific: Vec<String>,
    /// Settings left alone because the app they point at cannot be installed here.
    pub unavailable: Vec<String>,
    /// Installed on the other device, not here.
    pub missing: Vec<MissingApp>,
}

impl ImportPlan {
    pub fn queue_count(&self) -> usize {
        self.missing.iter().filter(|m| m.location.is_some()).count()
    }

    pub fn is_empty(&self) -> bool {
        self.changes.is_empty() && self.queue_count() == 0
    }
}

/// Result of applying an import, for the main loop to finish (it owns the install queue).
pub struct Applied {
    pub settings: usize,
    pub jobs: Vec<(String, String, InstallLocation)>,
}

fn flatten(value: &Value, keys: &mut Vec<String>, out: &mut Vec<(Vec<String>, Value)>) {
    match value {
        Value::Object(map) => {
            for (k, v) in map {
                keys.push(k.clone());
                flatten(v, keys, out);
                keys.pop();
            }
        }
        _ if !keys.is_empty() => out.push((keys.clone(), value.clone())),
        _ => {}
    }
}

fn get_path<'a>(value: &'a Value, keys: &[String]) -> Option<&'a Value> {
    keys.iter().try_fold(value, |v, k| v.get(k))
}

fn set_path(value: &mut Value, keys: &[String], new: Value) {
    let Some((last, parents)) = keys.split_last() else { return };
    let mut current = value;
    for key in parents {
        if !current.get(key).map_or(false, |v| v.is_object()) {
            current[key.as_str()] = Value::Object(Map::new());
        }
        current = current.get_mut(key.as_str()).expect("just inserted");
    }
    current[last.as_str()] = new;
}

fn title_case(segment: &str) -> String {
    segment
        .split('_')
        .filter(|w| !w.is_empty())
        .map(|w| {
            let mut c = w.chars();
            c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// "appearance.accent_color" → "Appearance › Accent Color"; app keys are kept as written.
fn pretty(keys: &[String]) -> String {
    keys.iter()
        .enumerate()
        .map(|(i, k)| if keys[0] == "app_store" && i == 2 { clean_text(k) } else { title_case(k) })
        .collect::<Vec<_>>()
        .join(" › ")
}

fn show(value: Option<&Value>) -> String {
    match value {
        None | Some(Value::Null) => "None".into(),
        Some(Value::Bool(b)) => if *b { "On".into() } else { "Off".into() },
        Some(Value::String(s)) if s.is_empty() => "(empty)".into(),
        Some(Value::String(s)) => clean_text(s),
        Some(Value::Number(n)) => n.to_string(),
        Some(Value::Array(items)) => {
            let words: Vec<String> = items.iter().filter_map(|v| v.as_str()).map(clean_text).collect();
            let joined = words.join(", ");
            if words.len() == items.len() && joined.chars().count() <= 40 {
                if joined.is_empty() { "(empty)".into() } else { joined }
            } else {
                format!("{} items", items.len())
            }
        }
        Some(Value::Object(_)) => "…".into(),
    }
}

fn is_device_specific(path: &str) -> bool {
    DEVICE_SPECIFIC.iter().any(|d| path == *d || path.starts_with(&format!("{}.", d)))
}

/// The app or dashboard a setting points at, if any.
fn referenced(keys: &[String], value: &Value) -> Option<String> {
    let first = keys.first()?.as_str();
    match (first, keys.len()) {
        ("hotkeys", 2) => {
            let s = value.as_str()?;
            s.strip_prefix("app:").or_else(|| s.strip_prefix("dashboard:")).map(str::to_string)
        }
        ("default_dashboard", 1) => value.as_str().map(str::to_string),
        ("app_store", 3) if keys[1] == "window_modes" => Some(keys[2].clone()),
        _ => None,
    }
}

/// Where to reinstall: the method the other device used, if this store offers it.
fn pick_location(meta: &Value, source: Option<&str>) -> InstallLocation {
    let methods = store::install_methods_for(meta);
    let wanted = match source {
        Some("global") => Some(InstallLocation::Global),
        Some("git") => Some(InstallLocation::Git),
        Some("local") => Some(InstallLocation::Local),
        _ => None,
    };
    wanted
        .filter(|w| methods.contains(w))
        .or_else(|| methods.first().copied())
        .unwrap_or(InstallLocation::Global)
}

/// Work out what importing `info` would change, without touching anything.
pub fn plan(info: &BackupInfo) -> Result<ImportPlan, String> {
    let bundle = load_bundle(&info.path)?;
    let empty = Value::Object(Map::new());
    let files = bundle.get("files").unwrap_or(&empty);
    let theirs_registered = files.get("registered-apps.json").unwrap_or(&empty);

    let apps_here = registry::load_apps();
    let dashboards_here = registry::load_dashboards();
    let registered_here = registry::load_registered_apps();
    let installed = |key: &str| apps_here.contains_key(key) || dashboards_here.contains_key(key);

    let mut missing = Vec::new();
    for (file, dashboard) in [("apps.json", false), ("dashboards.json", true)] {
        let Some(entries) = files.get(file).and_then(|v| v.as_object()) else { continue };
        let here: &HashMap<String, Value> = if dashboard { &dashboards_here } else { &apps_here };
        for (key, entry) in entries {
            let internal = entry.get("type").and_then(|v| v.as_str()) == Some("internal");
            if internal || registry::is_builtin_store_key(key) || here.contains_key(key) {
                continue;
            }
            let theirs = theirs_registered.get(key).filter(|v| v.is_object()).cloned();
            let new_entry = if registered_here.contains_key(key) {
                None
            } else {
                theirs.map(|mut e| {
                    if e.get("approved").is_none() {
                        e["approved"] = Value::Bool(false);
                    }
                    e
                })
            };
            let meta = registered_here.get(key).or(new_entry.as_ref());
            let text = |v: Option<&Value>, field: &str| {
                v.and_then(|m| m.get(field)).and_then(|s| s.as_str()).map(clean_text)
            };
            missing.push(MissingApp {
                key: key.clone(),
                label: text(meta, "label").or_else(|| text(Some(entry), "label")).unwrap_or_else(|| clean_text(key)),
                dashboard,
                location: meta.map(|m| pick_location(m, entry.get("source").and_then(|v| v.as_str()))),
                repository: text(meta, "repository").unwrap_or_default(),
                new_entry,
            });
        }
    }
    missing.sort_by_key(|m| m.label.to_lowercase());
    let queueable: HashSet<String> =
        missing.iter().filter(|m| m.location.is_some()).map(|m| m.key.clone()).collect();

    let current = persistence::load();
    let mut leaves = Vec::new();
    flatten(files.get("settings.json").unwrap_or(&empty), &mut Vec::new(), &mut leaves);
    let mut changes = Vec::new();
    let mut device_specific = Vec::new();
    let mut unavailable = Vec::new();
    for (keys, value) in leaves {
        let old = get_path(&current, &keys);
        if old == Some(&value) || (old.is_none() && value.is_null()) {
            continue;
        }
        let name = pretty(&keys);
        if is_device_specific(&keys.join(".")) {
            device_specific.push(name);
            continue;
        }
        let needs = match referenced(&keys, &value) {
            Some(target) if !installed(&target) => {
                if !queueable.contains(&target) {
                    unavailable.push(name);
                    continue;
                }
                Some(target)
            }
            _ => None,
        };
        changes.push(Change { old: show(old), new: show(Some(&value)), keys, value, name, needs });
    }

    Ok(ImportPlan { info: info.clone(), changes, device_specific, unavailable, missing })
}

/// Back up the current setup, merge the plan into settings.json and, when `queue_apps`,
/// add unknown apps to this App Store. Returns the install jobs for the caller to queue.
pub fn apply(plan: &ImportPlan, queue_apps: bool) -> Result<Applied, String> {
    export("", BackupKind::Auto).map_err(|e| format!("Import cancelled — could not back up first: {}", e))?;

    let mut settings = persistence::load();
    let mut count = 0;
    for change in plan.changes.iter().filter(|c| c.needs.is_none() || queue_apps) {
        set_path(&mut settings, &change.keys, change.value.clone());
        count += 1;
    }
    persistence::save(&settings);

    let mut jobs = Vec::new();
    if queue_apps {
        let mut registered = registry::load_registered_apps();
        let mut added = false;
        for app in &plan.missing {
            let Some(location) = app.location else { continue };
            if let Some(entry) = &app.new_entry {
                registered.insert(app.key.clone(), entry.clone());
                added = true;
            }
            jobs.push((app.key.clone(), app.label.clone(), location));
        }
        if added {
            registry::save_config("registered-apps.json", &registered);
        }
    }
    crate::utilities::logging::settings(&format!(
        "Imported {} settings from {} ({} apps to install)",
        count,
        plan.info.file_name(),
        jobs.len()
    ));
    Ok(Applied { settings: count, jobs })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys(path: &str) -> Vec<String> {
        path.split('.').map(str::to_string).collect()
    }

    #[test]
    fn flatten_and_set_round_trip() {
        let src = serde_json::json!({ "a": { "b": 1, "c": [1, 2] }, "d": "x" });
        let mut leaves = Vec::new();
        flatten(&src, &mut Vec::new(), &mut leaves);
        let mut rebuilt = serde_json::json!({});
        for (k, v) in leaves {
            set_path(&mut rebuilt, &k, v);
        }
        assert_eq!(rebuilt, src);
    }

    #[test]
    fn device_specific_and_references() {
        assert!(is_device_specific("display.fullscreen"));
        assert!(is_device_specific("task_manager.startup_modes.app:btop"));
        assert!(!is_device_specific("display.fullscreen_extra"));
        assert_eq!(referenced(&keys("hotkeys.key_1"), &Value::from("app:btop")), Some("btop".into()));
        assert_eq!(referenced(&keys("app_store.window_modes.btop"), &Value::from("fullscreen")), Some("btop".into()));
        assert_eq!(referenced(&keys("appearance.accent_color"), &Value::from("Red")), None);
    }

    #[test]
    fn labels_and_names() {
        assert_eq!(slug("My Pi: living room!", 32), "My-Pi-living-room");
        assert_eq!(pretty(&keys("appearance.accent_color")), "Appearance › Accent Color");
        assert_eq!(show(Some(&Value::Bool(true))), "On");
        assert_eq!(show(Some(&serde_json::json!(["Games", "Tools"]))), "Games, Tools");
    }
}
