/// Awesome Ratatui browser — fetch, parse, cache the awesome-ratatui README,
/// and provide state/rendering for browsing and adding apps.

use std::collections::HashMap;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::utilities::logging;

const README_URL: &str =
    "https://raw.githubusercontent.com/ratatui/awesome-ratatui/main/README.md";
pub const REPO_URL: &str = "https://github.com/ratatui/awesome-ratatui";
const CACHE_FILE: &str = "awesome-ratatui-cache.json";
const CACHE_TTL_SECS: u64 = 3600;
/// Bumped whenever the parse output shape changes, so stale caches are discarded.
const CACHE_VERSION: u32 = 2;

// ── Data types ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AwesomeApp {
    pub name: String,
    pub repo_url: String,
    pub description: String,
    pub category: String,
    /// Empty when the app sits directly under its category.
    #[serde(default)]
    pub subcategory: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Cache {
    #[serde(default)]
    version: u32,
    fetched_at: u64,
    apps: Vec<AwesomeApp>,
}

// ── Browser state ───────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BrowserFocus {
    SearchBar,
    RepoButton,
    CollapseToggle,
    DescriptionToggle,
    List,
    Actions,
}

#[derive(Debug, Clone)]
pub struct BrowserState {
    pub active: bool,
    pub focus: BrowserFocus,
    pub apps: Vec<AwesomeApp>,
    pub categories: Vec<String>,
    pub collapsed: std::collections::HashSet<String>,
    pub search_query: String,
    pub cursor: usize,
    pub scroll: usize,
    /// Visible rows after filtering/collapsing (category headers + apps)
    pub visible_rows: Vec<RowKind>,
    pub fetched_at: Option<u64>,
    pub action_cursor: usize,
    pub loading: bool,
    pub error: Option<String>,
    pub show_descriptions: bool,
    /// Lazily populated install status per repo key, for apps that are not registered.
    pub probe_cache: HashMap<String, super::state::InstallStatus>,
}

#[derive(Debug, Clone)]
pub enum RowKind {
    CategoryHeader(String),
    SubcategoryHeader { category: String, subcategory: String },
    App(usize),
    Spacer,
}

/// Key used in `BrowserState::collapsed` for a subcategory row.
pub fn subcategory_key(category: &str, subcategory: &str) -> String {
    format!("{}/{}", category, subcategory)
}

impl BrowserState {
    pub fn new() -> Self {
        Self {
            active: false,
            focus: BrowserFocus::List,
            apps: Vec::new(),
            categories: Vec::new(),
            collapsed: std::collections::HashSet::new(),
            search_query: String::new(),
            cursor: 0,
            scroll: 0,
            visible_rows: Vec::new(),
            fetched_at: None,
            action_cursor: 0,
            loading: false,
            error: None,
            show_descriptions: true,
            probe_cache: HashMap::new(),
        }
    }

    pub fn selected_app(&self) -> Option<&AwesomeApp> {
        if let Some(RowKind::App(idx)) = self.visible_rows.get(self.cursor) {
            self.apps.get(*idx)
        } else {
            None
        }
    }

    /// Look up the install status of a browser app, preferring the registered-app
    /// statuses and falling back to the lazily filled probe cache.
    pub fn status_for<'a>(
        &'a self,
        key: &str,
        registered_statuses: &'a HashMap<String, super::state::InstallStatus>,
    ) -> Option<&'a super::state::InstallStatus> {
        registered_statuses
            .get(key)
            .filter(|s| !matches!(s, super::state::InstallStatus::NotInstalled))
            .or_else(|| self.probe_cache.get(key))
    }

    /// Probe the highlighted app for an existing PATH/downloads install.
    /// Results are cached per key so scrolling stays responsive.
    pub fn probe_selected(&mut self) {
        let Some(app) = self.selected_app() else { return };
        let key = repo_to_key(&app.repo_url);
        if self.probe_cache.contains_key(&key) {
            return;
        }
        let name = app.name.clone();
        let status = super::actions::probe_install_status(&key, &name);
        self.probe_cache.insert(key, status);
    }

    /// True once every category is collapsed, which flips the button to "expand all".
    pub fn all_collapsed(&self) -> bool {
        !self.categories.is_empty()
            && self.categories.iter().all(|c| self.collapsed.contains(c))
    }

    pub fn toggle_collapse_all(&mut self) {
        if self.all_collapsed() {
            self.collapsed.clear();
        } else {
            for cat in &self.categories {
                self.collapsed.insert(cat.clone());
            }
        }
        self.cursor = 0;
        self.scroll = 0;
        self.recompute_visible();
    }

    /// Move the cursor to the next/previous category header, leaving it put if
    /// there is none in that direction.
    pub fn jump_category(&mut self, forward: bool) {
        let is_header = |r: &RowKind| matches!(r, RowKind::CategoryHeader(_));
        let target = if forward {
            self.visible_rows
                .iter()
                .enumerate()
                .skip(self.cursor + 1)
                .find(|(_, r)| is_header(r))
                .map(|(i, _)| i)
        } else {
            self.visible_rows[..self.cursor.min(self.visible_rows.len())]
                .iter()
                .enumerate()
                .rev()
                .find(|(_, r)| is_header(r))
                .map(|(i, _)| i)
        };

        let Some(idx) = target else { return };
        self.cursor = idx;

        const VISIBLE_H: usize = 20;
        if self.cursor < self.scroll {
            self.scroll = self.cursor;
        } else if self.cursor >= self.scroll + VISIBLE_H {
            self.scroll = self.cursor.saturating_sub(VISIBLE_H - 1);
        }
        self.probe_selected();
    }

    pub fn recompute_visible(&mut self) {
        let query = self.search_query.to_lowercase();
        let mut rows: Vec<RowKind> = Vec::new();
        let mut has_prev_category = false;

        let matches = |a: &AwesomeApp| {
            query.is_empty()
                || a.name.to_lowercase().contains(&query)
                || a.description.to_lowercase().contains(&query)
        };

        for cat in &self.categories {
            let cat_apps: Vec<usize> = self.apps.iter().enumerate()
                .filter(|(_, a)| a.category == *cat && matches(a))
                .map(|(i, _)| i)
                .collect();

            if cat_apps.is_empty() {
                continue;
            }

            if has_prev_category {
                rows.push(RowKind::Spacer);
            }
            has_prev_category = true;

            rows.push(RowKind::CategoryHeader(cat.clone()));
            if self.collapsed.contains(cat) {
                continue;
            }

            // Apps that sit directly under the category come before any subcategory.
            for idx in cat_apps.iter().filter(|i| self.apps[**i].subcategory.is_empty()) {
                rows.push(RowKind::App(*idx));
            }

            for sub in subcategories_for(cat) {
                let sub_apps: Vec<usize> = cat_apps.iter()
                    .copied()
                    .filter(|i| self.apps[*i].subcategory == *sub)
                    .collect();
                if sub_apps.is_empty() {
                    continue;
                }
                rows.push(RowKind::SubcategoryHeader {
                    category: cat.clone(),
                    subcategory: sub.to_string(),
                });
                if self.collapsed.contains(&subcategory_key(cat, sub)) {
                    continue;
                }
                for idx in sub_apps {
                    rows.push(RowKind::App(idx));
                }
            }
        }

        self.visible_rows = rows;
        if self.cursor >= self.visible_rows.len() {
            self.cursor = self.visible_rows.len().saturating_sub(1);
        }
    }
}

// ── Fetch & parse ───────────────────────────────────────────────────────

fn now_epoch() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn cache_path() -> std::path::PathBuf {
    let cwd_config = std::path::PathBuf::from("config");
    if cwd_config.exists() {
        return cwd_config.join(CACHE_FILE);
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let p = dir.join("../../config");
            if p.exists() {
                return p.join(CACHE_FILE);
            }
        }
    }
    cwd_config.join(CACHE_FILE)
}

fn load_cache() -> Option<Cache> {
    let data = std::fs::read_to_string(cache_path()).ok()?;
    serde_json::from_str(&data).ok()
}

fn save_cache(cache: &Cache) {
    if let Ok(json) = serde_json::to_string_pretty(cache) {
        let _ = std::fs::write(cache_path(), json);
    }
}

/// Canonical categories in display order, each with its ordered subcategories.
const CATEGORY_TABLE: &[(&str, &[&str])] = &[
    ("Development Tools", &[
        "Source Control and Collaboration",
        "Code Search, Editing, and Review",
        "APIs, Databases, Build, and Debugging",
    ]),
    ("AI and Agents", &[]),
    ("Files, Data, and Documents", &[]),
    ("Terminal Workflow", &[]),
    ("Networking and Internet", &[
        "Network Operations and Infrastructure",
        "Remote Access, APIs, and File Transfer",
        "Communications and Social",
    ]),
    ("System Administration", &[
        "Monitoring, Diagnostics, and Logs",
        "Containers and Orchestration",
        "OS, Storage, and Package Management",
        "Batch, Database, and Cluster Operations",
    ]),
    ("Hardware and Embedded", &[]),
    ("Security and Identity", &[]),
    ("Productivity and Planning", &[
        "Tasks, Projects, and Calendars",
        "Notes and Journaling",
        "Finance and Markets",
        "Focus, Habits, and Time",
    ]),
    ("Reading and Learning", &[]),
    ("Music and Media", &[
        "Music and Audio",
        "Books, Video, and Creative Media",
    ]),
    ("Games", &[]),
    ("Science, Math, and Exploration", &[]),
    ("Other", &[]),
];

pub fn subcategories_for(category: &str) -> &'static [&'static str] {
    CATEGORY_TABLE
        .iter()
        .find(|(cat, _)| *cat == category)
        .map(|(_, subs)| *subs)
        .unwrap_or(&[])
}

/// Strip the leading emoji/punctuation from a markdown heading and lowercase it.
fn normalize_heading(heading: &str) -> String {
    heading
        .chars()
        .skip_while(|c| !c.is_ascii_alphanumeric())
        .collect::<String>()
        .trim()
        .to_lowercase()
}

/// Map a README category heading onto a canonical category name.
fn map_category(raw: &str) -> &'static str {
    match normalize_heading(raw).as_str() {
        "development tools" => "Development Tools",
        "ai and agents" => "AI and Agents",
        "files, data, and documents" => "Files, Data, and Documents",
        "terminal workflow" => "Terminal Workflow",
        "networking and internet" => "Networking and Internet",
        "system administration" => "System Administration",
        "hardware and embedded" | "embedded" => "Hardware and Embedded",
        "security and identity" => "Security and Identity",
        "productivity and planning" | "productivity and utilities" => "Productivity and Planning",
        "reading and learning" => "Reading and Learning",
        "music and media" => "Music and Media",
        "games and entertainment" | "games" => "Games",
        "science, math, and exploration" => "Science, Math, and Exploration",
        _ => "Other",
    }
}

/// Map a README subcategory heading onto one of `category`'s known subcategories.
fn map_subcategory(category: &str, raw: &str) -> Option<&'static str> {
    let key = normalize_heading(raw);
    subcategories_for(category)
        .iter()
        .find(|sub| normalize_heading(sub) == key)
        .copied()
}

fn parse_readme(md: &str) -> Vec<AwesomeApp> {
    let mut apps: Vec<AwesomeApp> = Vec::new();
    let mut in_apps_section = false;
    let mut current_category: Option<&'static str> = None;
    let mut current_subcategory: Option<&'static str> = None;

    for line in md.lines() {
        let trimmed = line.trim();

        // Detect top-level sections
        if trimmed.starts_with("## ") {
            let clean = normalize_heading(trimmed.trim_start_matches("## "));
            in_apps_section = clean == "apps";
            current_category = None;
            current_subcategory = None;
            continue;
        }

        if !in_apps_section {
            continue;
        }

        if trimmed.starts_with("### ") {
            current_category = Some(map_category(trimmed.trim_start_matches("### ")));
            current_subcategory = None;
            continue;
        }

        if trimmed.starts_with("#### ") {
            current_subcategory = current_category
                .and_then(|cat| map_subcategory(cat, trimmed.trim_start_matches("#### ")));
            continue;
        }

        // Parse app entries: "- [Name](url) - Description"
        if let Some(cat) = current_category {
            if trimmed.starts_with("- [") {
                if let Some(app) = parse_app_line(trimmed, cat, current_subcategory.unwrap_or("")) {
                    apps.push(app);
                }
            }
        }
    }

    apps
}

fn parse_app_line(line: &str, category: &str, subcategory: &str) -> Option<AwesomeApp> {
    // Format: "- [Name](url) - Description" or "- [Name](url) — Description"
    let rest = line.strip_prefix("- ")?;
    let name_start = rest.find('[')? + 1;
    let name_end = rest.find(']')?;
    let name = rest[name_start..name_end].to_string();

    let url_start = rest.find("](")? + 2;
    let url_end = rest[url_start..].find(')')? + url_start;
    let repo_url = rest[url_start..url_end].to_string();

    // Description comes after ") - " or ") — "
    let after_url = &rest[url_end + 1..];
    let description = after_url
        .trim_start_matches(')')
        .trim()
        .trim_start_matches('-')
        .trim_start_matches('—')
        .trim()
        .to_string();

    Some(AwesomeApp {
        name,
        repo_url,
        description,
        category: category.to_string(),
        subcategory: subcategory.to_string(),
    })
}

/// Load apps from cache (if fresh) or fetch from GitHub.
pub fn load_or_fetch() -> Result<(Vec<AwesomeApp>, u64), String> {
    if let Some(cache) = load_cache() {
        let age = now_epoch().saturating_sub(cache.fetched_at);
        if cache.version == CACHE_VERSION && age < CACHE_TTL_SECS {
            logging::info(&format!(
                "Awesome Ratatui: loaded {} apps from cache ({}s old)",
                cache.apps.len(), age
            ));
            return Ok((cache.apps, cache.fetched_at));
        }
    }
    fetch_and_cache()
}

/// Force fetch from GitHub regardless of cache.
pub fn fetch_and_cache() -> Result<(Vec<AwesomeApp>, u64), String> {
    logging::info("Awesome Ratatui: fetching README from GitHub...");
    let body = ureq::get(README_URL)
        .call()
        .map_err(|e| format!("HTTP error: {}", e))?
        .into_string()
        .map_err(|e| format!("Read error: {}", e))?;

    let apps = parse_readme(&body);
    let ts = now_epoch();
    logging::info(&format!("Awesome Ratatui: parsed {} apps from README", apps.len()));

    let cache = Cache { version: CACHE_VERSION, fetched_at: ts, apps: apps.clone() };
    save_cache(&cache);

    Ok((apps, ts))
}

/// Build the ordered category list from parsed apps.
pub fn build_categories(apps: &[AwesomeApp]) -> Vec<String> {
    CATEGORY_TABLE
        .iter()
        .filter(|(cat, _)| apps.iter().any(|a| a.category == *cat))
        .map(|(cat, _)| cat.to_string())
        .collect()
}

// ── Add / remove from registered apps ───────────────────────────────────

/// Generate a key for a repo URL (last segment of the path, lowercased).
pub fn repo_to_key(repo_url: &str) -> String {
    repo_url
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or("unknown")
        .to_lowercase()
}

/// Extract author from a GitHub URL.
fn repo_to_author(repo_url: &str) -> String {
    let parts: Vec<&str> = repo_url
        .trim_end_matches('/')
        .rsplitn(3, '/')
        .collect();
    if parts.len() >= 2 { parts[1].to_string() } else { "unknown".to_string() }
}

/// Build a registered-app Value from an AwesomeApp.
pub fn to_registered_value(app: &AwesomeApp) -> Value {
    let key = repo_to_key(&app.repo_url);
    let author = repo_to_author(&app.repo_url);
    serde_json::json!({
        "label": app.name,
        "description": app.description,
        "category": app.category,
        "author": author,
        "version": "unknown",
        "license": "unknown",
        "repository": app.repo_url,
        "crate_name": key,
        "cmd": [key],
        "pre_installed": false,
        "install_methods": ["global", "git", "local"],
        "default_window_mode": "embedded",
        "supports_embedded": true,
        "approved": false
    })
}

/// Add an awesome-ratatui app to registered-apps.json. Returns the key used.
pub fn add_to_registered(app: &AwesomeApp) -> String {
    let key = repo_to_key(&app.repo_url);
    let value = to_registered_value(app);

    let mut registered = crate::tuios::registry::load_registered_apps();
    registered.insert(key.clone(), value);
    crate::tuios::registry::save_config("registered-apps.json", &registered);

    logging::info(&format!(
        "Awesome Ratatui: added '{}' ({}) to registered apps as '{}'",
        app.name, app.category, key
    ));
    key
}

/// Remove an app from registered-apps.json. Only succeeds if not installed.
pub fn remove_from_registered(key: &str, registered: &mut HashMap<String, Value>) -> bool {
    if registered.remove(key).is_some() {
        crate::tuios::registry::save_config("registered-apps.json", registered);
        logging::info(&format!("Awesome Ratatui: removed '{}' from registered apps", key));
        true
    } else {
        false
    }
}


