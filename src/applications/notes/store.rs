/// Notes — data model and persistence (config/notes.json).
///
/// The file is only written once at least one note (active or trashed) exists,
/// and it is removed again when the last note is permanently deleted, so a
/// fresh install never ends up with an empty notes.json.

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

/// Trashed notes older than this are purged when the store is loaded.
pub const TRASH_RETENTION_DAYS: i64 = 30;

pub fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Note {
    pub id: u64,
    pub title: String,
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub pinned: bool,
    pub created_at: i64,
    pub updated_at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deleted_at: Option<i64>,
}

impl Note {
    pub fn is_trashed(&self) -> bool {
        self.deleted_at.is_some()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum SortOrder {
    #[default]
    Modified,
    Created,
    Title,
}

impl SortOrder {
    pub fn next(self) -> Self {
        match self {
            SortOrder::Modified => SortOrder::Created,
            SortOrder::Created => SortOrder::Title,
            SortOrder::Title => SortOrder::Modified,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            SortOrder::Modified => "Last edited",
            SortOrder::Created => "Date created",
            SortOrder::Title => "Title (A–Z)",
        }
    }
}

#[derive(Serialize, Deserialize, Default)]
struct NotesFile {
    #[serde(default)]
    sort: SortOrder,
    #[serde(default)]
    notes: Vec<Note>,
}

pub struct NoteStore {
    pub notes: Vec<Note>,
    pub sort: SortOrder,
    path: PathBuf,
    next_id: u64,
    /// Set when an existing notes.json could not be read; shown to the user.
    pub load_error: Option<String>,
}

impl NoteStore {
    pub fn load() -> Self {
        Self::load_from(crate::tuios::registry::config_dir().join("notes.json"))
    }

    pub fn load_from(path: PathBuf) -> Self {
        let mut load_error = None;
        let file: NotesFile = match fs::read_to_string(&path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => NotesFile::default(),
            Err(e) => {
                load_error = Some(format!("Could not read notes.json: {}", e));
                NotesFile::default()
            }
            Ok(s) => serde_json::from_str(&s).unwrap_or_else(|e| {
                // Move the unreadable file aside so a later save can't destroy it
                let backup = path.with_extension(format!("json.corrupt-{}", now()));
                let _ = fs::rename(&path, &backup);
                load_error = Some(format!("notes.json was unreadable ({}); backed up to {}", e, backup.display()));
                NotesFile::default()
            }),
        };
        let next_id = file.notes.iter().map(|n| n.id).max().unwrap_or(0) + 1;
        let mut store = Self { notes: file.notes, sort: file.sort, path, next_id, load_error };
        if store.purge_expired_trash(now()) > 0 {
            let _ = store.save();
        }
        store
    }

    /// Persist all notes. Writes atomically via a temp file; removes the file
    /// entirely when there are no notes left.
    pub fn save(&self) -> Result<(), String> {
        if self.load_error.is_some() && self.path.exists() {
            return Err("notes.json could not be read, so it was not overwritten".into());
        }
        if self.notes.is_empty() {
            if self.path.exists() {
                fs::remove_file(&self.path).map_err(|e| e.to_string())?;
            }
            return Ok(());
        }
        if let Some(dir) = self.path.parent() {
            fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        let file = NotesFile { sort: self.sort, notes: self.notes.clone() };
        let json = serde_json::to_string_pretty(&file).map_err(|e| e.to_string())?;
        let tmp = self.path.with_extension("json.tmp");
        fs::write(&tmp, json).map_err(|e| e.to_string())?;
        fs::rename(&tmp, &self.path).map_err(|e| e.to_string())
    }

    pub fn create(&mut self, title: &str) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        let ts = now();
        let title = title.trim();
        self.notes.push(Note {
            id,
            title: if title.is_empty() { "Untitled".to_string() } else { title.to_string() },
            body: String::new(),
            pinned: false,
            created_at: ts,
            updated_at: ts,
            deleted_at: None,
        });
        id
    }

    pub fn get(&self, id: u64) -> Option<&Note> {
        self.notes.iter().find(|n| n.id == id)
    }

    fn get_mut(&mut self, id: u64) -> Option<&mut Note> {
        self.notes.iter_mut().find(|n| n.id == id)
    }

    /// Update a note's body. Returns true if the content actually changed.
    pub fn set_body(&mut self, id: u64, body: String) -> bool {
        match self.get_mut(id) {
            Some(n) if n.body != body => {
                n.body = body;
                n.updated_at = now();
                true
            }
            _ => false,
        }
    }

    pub fn rename(&mut self, id: u64, title: &str) -> bool {
        let title = title.trim();
        match self.get_mut(id) {
            Some(n) if !title.is_empty() && n.title != title => {
                n.title = title.to_string();
                n.updated_at = now();
                true
            }
            _ => false,
        }
    }

    /// Toggle pinned state; returns the new state.
    pub fn toggle_pin(&mut self, id: u64) -> Option<bool> {
        let n = self.get_mut(id)?;
        n.pinned = !n.pinned;
        Some(n.pinned)
    }

    pub fn trash(&mut self, id: u64) -> bool {
        match self.get_mut(id) {
            Some(n) if n.deleted_at.is_none() => {
                n.deleted_at = Some(now());
                n.pinned = false;
                true
            }
            _ => false,
        }
    }

    pub fn restore(&mut self, id: u64) -> bool {
        match self.get_mut(id) {
            Some(n) if n.deleted_at.is_some() => {
                n.deleted_at = None;
                true
            }
            _ => false,
        }
    }

    pub fn delete_forever(&mut self, id: u64) -> bool {
        let before = self.notes.len();
        self.notes.retain(|n| n.id != id);
        self.notes.len() != before
    }

    /// Permanently delete every trashed note; returns how many were removed.
    pub fn empty_trash(&mut self) -> usize {
        let before = self.notes.len();
        self.notes.retain(|n| !n.is_trashed());
        before - self.notes.len()
    }

    pub fn purge_expired_trash(&mut self, now: i64) -> usize {
        let cutoff = now - TRASH_RETENTION_DAYS * 24 * 60 * 60;
        let before = self.notes.len();
        self.notes.retain(|n| n.deleted_at.map_or(true, |d| d > cutoff));
        before - self.notes.len()
    }

    pub fn count(&self, trash: bool) -> usize {
        self.notes.iter().filter(|n| n.is_trashed() == trash).count()
    }

    /// Ids of notes in the active list (or the trash), filtered by a
    /// case-insensitive search over title and body, in display order.
    pub fn visible(&self, trash: bool, query: &str) -> Vec<u64> {
        let q = query.trim().to_lowercase();
        let mut notes: Vec<&Note> = self
            .notes
            .iter()
            .filter(|n| n.is_trashed() == trash)
            .filter(|n| {
                q.is_empty()
                    || n.title.to_lowercase().contains(&q)
                    || n.body.to_lowercase().contains(&q)
            })
            .collect();

        if trash {
            notes.sort_by(|a, b| b.deleted_at.cmp(&a.deleted_at).then(b.id.cmp(&a.id)));
        } else {
            let sort = self.sort;
            notes.sort_by(|a, b| {
                b.pinned.cmp(&a.pinned).then_with(|| match sort {
                    SortOrder::Modified => b.updated_at.cmp(&a.updated_at).then(b.id.cmp(&a.id)),
                    SortOrder::Created => b.created_at.cmp(&a.created_at).then(b.id.cmp(&a.id)),
                    SortOrder::Title => a
                        .title
                        .to_lowercase()
                        .cmp(&b.title.to_lowercase())
                        .then(a.id.cmp(&b.id)),
                })
            });
        }
        notes.into_iter().map(|n| n.id).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_path(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("tuios-notes-test-{}-{}", name, std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir.join("notes.json")
    }

    #[test]
    fn file_only_exists_while_notes_exist() {
        let path = temp_path("lifecycle");
        let mut store = NoteStore::load_from(path.clone());
        store.save().unwrap();
        assert!(!path.exists(), "no file should be written without notes");

        let id = store.create("First");
        store.set_body(id, "hello".into());
        store.save().unwrap();
        let reloaded = NoteStore::load_from(path.clone());
        assert_eq!(reloaded.get(id).unwrap().body, "hello");

        store.trash(id);
        store.save().unwrap();
        assert!(path.exists(), "trashed notes are still persisted");

        store.empty_trash();
        store.save().unwrap();
        assert!(!path.exists(), "file is removed once no notes remain");
    }

    #[test]
    fn visible_sorts_pinned_first_and_filters() {
        let mut store = NoteStore::load_from(temp_path("sort"));
        let a = store.create("Banana");
        let b = store.create("apple");
        let c = store.create("Cherry");
        store.set_body(c, "shopping list".into());
        store.sort = SortOrder::Title;
        assert_eq!(store.visible(false, ""), vec![b, a, c]);

        store.toggle_pin(c);
        assert_eq!(store.visible(false, ""), vec![c, b, a]);
        assert_eq!(store.visible(false, "SHOPPING"), vec![c]);

        store.trash(a);
        assert_eq!(store.visible(false, ""), vec![c, b]);
        assert_eq!(store.visible(true, ""), vec![a]);
        store.restore(a);
        assert_eq!(store.count(true), 0);
    }

    #[test]
    fn corrupt_file_is_backed_up_not_destroyed() {
        let path = temp_path("corrupt");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, "{ not json").unwrap();
        let mut store = NoteStore::load_from(path.clone());
        assert!(store.load_error.is_some());
        let backups: Vec<_> = fs::read_dir(path.parent().unwrap())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().contains("corrupt"))
            .collect();
        assert_eq!(backups.len(), 1);
        assert_eq!(fs::read_to_string(backups[0].path()).unwrap(), "{ not json");
        store.create("new");
        store.save().unwrap();
        assert!(path.exists());
    }

    #[test]
    fn expired_trash_is_purged() {
        let mut store = NoteStore::load_from(temp_path("purge"));
        let old = store.create("old");
        let recent = store.create("recent");
        store.trash(old);
        store.trash(recent);
        let t = now();
        store.notes.iter_mut().find(|n| n.id == old).unwrap().deleted_at =
            Some(t - (TRASH_RETENTION_DAYS + 1) * 86_400);
        assert_eq!(store.purge_expired_trash(t), 1);
        assert!(store.get(old).is_none());
        assert!(store.get(recent).is_some());
    }
}
