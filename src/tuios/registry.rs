/// tuiOS — load app/dashboard/settings registries from config/ JSON files.

use serde_json::Value;
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

pub fn config_dir() -> PathBuf {
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.to_path_buf()));

    // Try relative to CWD first (development), then relative to executable
    let cwd_config = PathBuf::from("config");
    if cwd_config.exists() {
        return cwd_config;
    }

    if let Some(exe) = exe_dir {
        let exe_config = exe.join("../../config");
        if exe_config.exists() {
            return exe_config;
        }
    }

    cwd_config
}

/// Apps that ship with tuiOS. They live in code rather than only in apps.json so a
/// deleted or regenerated config can never drop them from the navbar.
const BUILTIN_APPS: &[(&str, &str, &str)] = &[
    ("CharacterSet", "Character Set", "applications::character_set"),
    ("FileExplorer", "File Explorer", "applications::file_explorer"),
    ("Notes", "Notes", "applications::notes"),
    ("TextEditor", "Text Editor", "applications::text_editor"),
];

const BUILTIN_DASHBOARDS: &[(&str, &str, &str)] = &[
    (
        crate::dashboards::dashboard_1::KEY,
        crate::dashboards::dashboard_1::LABEL,
        "dashboards::dashboard_1",
    ),
    ("Dashboard-2", "Dashboard 2 — Clock & Date", "dashboards::dashboard_2"),
    ("Dashboard-3", "Dashboard 3 — System Stats", "dashboards::dashboard_3"),
];

/// App Store categories reserved for the apps and dashboards that ship with tuiOS.
pub const BUILTIN_APP_CATEGORY: &str = "tuiOS Apps";
pub const BUILTIN_DASHBOARD_CATEGORY: &str = "tuiOS Dashboards";

/// One-line descriptions for the App Store preview pane, keyed by built-in name.
const BUILTIN_DESCRIPTIONS: &[(&str, &str)] = &[
    ("CharacterSet", "Browse and copy Unicode characters, box-drawing glyphs and symbols."),
    ("FileExplorer", "Navigate the filesystem, preview files and open them in the text editor."),
    ("TextEditor", "A lightweight editor for quick edits without leaving tuiOS."),
    (
        crate::dashboards::dashboard_1::KEY,
        crate::dashboards::dashboard_1::DESCRIPTION,
    ),
    ("Dashboard-2", "A large clock, the date and a system greeting."),
    ("Dashboard-3", "Live CPU, memory and disk statistics at a glance."),
];

pub fn is_builtin_store_key(key: &str) -> bool {
    BUILTIN_APPS.iter().chain(BUILTIN_DASHBOARDS).any(|(k, _, _)| *k == key)
}

/// True when the key is a built-in dashboard rather than a built-in app.
pub fn is_builtin_dashboard(key: &str) -> bool {
    BUILTIN_DASHBOARDS.iter().any(|(k, _, _)| *k == key)
}

/// Registered-app shaped entries for everything that ships with tuiOS. These are
/// merged into `load_registered_apps()` so the App Store can list them, but they are
/// never written back to registered-apps.json.
pub fn builtin_store_entries() -> HashMap<String, Value> {
    let mut out = HashMap::new();
    let groups = [
        (BUILTIN_APPS, BUILTIN_APP_CATEGORY),
        (BUILTIN_DASHBOARDS, BUILTIN_DASHBOARD_CATEGORY),
    ];
    for (table, category) in groups {
        for (key, label, module) in table {
            let description = BUILTIN_DESCRIPTIONS
                .iter()
                .find(|(k, _)| k == key)
                .map(|(_, d)| *d)
                .unwrap_or("");
            out.insert(
                (*key).to_string(),
                serde_json::json!({
                    "label": label,
                    "category": category,
                    "author": "tuiOS",
                    "version": "built-in",
                    "license": "Ships with tuiOS",
                    "repository": "",
                    "description": description,
                    "module": module,
                    "builtin": true,
                    "approved": true,
                }),
            );
        }
    }
    out
}

fn builtins_for(filename: &str) -> &'static [(&'static str, &'static str, &'static str)] {
    match filename {
        "apps.json" => BUILTIN_APPS,
        "dashboards.json" => BUILTIN_DASHBOARDS,
        _ => &[],
    }
}

fn load(filename: &str) -> HashMap<String, Value> {
    let path = config_dir().join(filename);
    let content = fs::read_to_string(&path).unwrap_or_else(|_| "{}".to_string());
    let mut map: HashMap<String, Value> = serde_json::from_str(&content).unwrap_or_default();
    for (key, label, module) in builtins_for(filename) {
        map.entry((*key).to_string()).or_insert_with(|| {
            serde_json::json!({ "label": label, "type": "internal", "module": module })
        });
    }
    map
}

pub fn load_settings() -> HashMap<String, Value> {
    load("settings.json")
}

/// Return dict of {name: {label, module}} from dashboards.json.
pub fn load_dashboards() -> HashMap<String, Value> {
    load("dashboards.json")
}

/// Return dict of {name: {label, type, module/cmd}} from apps.json.
pub fn load_apps() -> HashMap<String, Value> {
    load("apps.json")
}

/// Return dict of {name: {label, category, ...}} from registered-apps.json, with the
/// built-in apps and dashboards merged in so the App Store can list them too.
pub fn load_registered_apps() -> HashMap<String, Value> {
    let mut map = load("registered-apps.json");
    for (key, entry) in builtin_store_entries() {
        map.insert(key, entry);
    }
    map
}

/// Write an updated registry back to the config file.
///
/// Keys are written in a fixed order — built-in entries first, then everything
/// else alphabetically — so adding one app does not reshuffle the whole file.
pub fn save_config(filename: &str, data: &HashMap<String, Value>) {
    let dir = config_dir();
    let _ = fs::create_dir_all(&dir);

    let mut ordered = serde_json::Map::with_capacity(data.len());
    for (key, _, _) in builtins_for(filename) {
        if let Some(entry) = data.get(*key) {
            ordered.insert((*key).to_string(), entry.clone());
        }
    }
    // Built-ins live in code, so never let them leak into registered-apps.json
    let skip_builtins = filename == "registered-apps.json";
    let mut rest: Vec<&String> = data
        .keys()
        .filter(|k| !ordered.contains_key(k.as_str()))
        .filter(|k| !(skip_builtins && is_builtin_store_key(k)))
        .collect();
    rest.sort_by_key(|k| k.to_lowercase());
    for key in rest {
        ordered.insert(key.clone(), data[key].clone());
    }

    if let Ok(json) = serde_json::to_string_pretty(&Value::Object(ordered)) {
        let _ = fs::write(dir.join(filename), json);
    }
}
