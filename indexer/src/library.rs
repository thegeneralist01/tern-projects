use crate::model::Assignment;
use crate::text::plain_path;
use anyhow::{bail, Context, Result};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::path::{Component, Path, PathBuf};

#[derive(Debug, Deserialize)]
#[serde(tag = "op", rename_all = "kebab-case", deny_unknown_fields)]
pub enum Request {
    Snapshot {
        view: String,
        per: i64,
        recent: i64,
    },
    ProjectSave {
        id: Option<String>,
        name: String,
        icon: String,
        folders: Vec<String>,
        #[serde(default)]
        cwd: String,
    },
    ProjectRemove {
        id: String,
    },
    ChatUpdate {
        uid: String,
        pinned: Option<bool>,
        archived: Option<bool>,
        assignment: Option<Assignment>,
    },
    Archived {
        limit: i64,
        offset: i64,
    },
    SettingsUpdate {
        #[serde(default)]
        default_harness: Patch<Value>,
        icon_style: Option<String>,
    },
    Folders {
        path: String,
        cwd: String,
        offset: i64,
        limit: i64,
    },
    Harnesses,
    LaunchPlan {
        harness: String,
        project_id: Option<String>,
        cwd: Option<String>,
    },
    DeletePreview {
        uid: String,
    },
    Delete {
        uid: String,
        expected_source: String,
        expected_version: String,
    },
}

// Option<Value> cannot distinguish an absent field from an explicit JSON null.
#[derive(Debug, Default)]
pub enum Patch<T> {
    #[default]
    Missing,
    Present(T),
}
impl<'de, T: Deserialize<'de>> Deserialize<'de> for Patch<T> {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        T::deserialize(d).map(Self::Present)
    }
}

#[derive(Debug, Serialize)]
pub struct Project {
    pub id: String,
    pub name: String,
    pub icon: String,
    pub folders: Vec<String>,
    pub created_ms: i64,
}
pub struct Library {
    pub conn: Connection,
}

pub fn path(db: &Path) -> PathBuf {
    db.parent().unwrap_or(Path::new(".")).join("library.sqlite")
}
pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(i64::MAX as u128) as i64
}
pub fn id(s: &str) -> Result<i64> {
    let n = s.parse::<i64>().context("invalid project ID")?;
    anyhow::ensure!(n > 0 && n.to_string() == s, "invalid project ID");
    Ok(n)
}
impl Library {
    pub fn open(db: &Path) -> Result<Self> {
        let p = path(db);
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let _initialization = crate::db::initialization_lock(&p)?;
        let mut conn = Connection::open(&p).context("opening project library")?;
        conn.busy_timeout(std::time::Duration::from_millis(5000))?;
        conn.pragma_update(None, "foreign_keys", true)?;
        let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        if version == 0 {
            // Another opener may initialize the library while we wait for this lock.
            // Only the version read under the write lock can authorize schema creation.
            let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
            let version: i64 = tx.query_row("PRAGMA user_version", [], |r| r.get(0))?;
            if version == 0 {
                let tables: i64 = tx.query_row(
                    "SELECT count(*) FROM sqlite_master WHERE name NOT LIKE 'sqlite_%'",
                    [],
                    |r| r.get(0),
                )?;
                anyhow::ensure!(
                    tables == 0,
                    "unsupported library schema: unversioned nonempty database"
                );
                tx.execute_batch("
                    CREATE TABLE projects(id INTEGER PRIMARY KEY AUTOINCREMENT,name TEXT NOT NULL,icon TEXT NOT NULL,created_ms INTEGER NOT NULL);
                    CREATE TABLE project_folders(project_id INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,position INTEGER NOT NULL,path TEXT NOT NULL,PRIMARY KEY(project_id,position),UNIQUE(project_id,path));
                    CREATE TABLE chat_state(uid TEXT PRIMARY KEY,assignment TEXT NOT NULL DEFAULT 'automatic',project_id INTEGER REFERENCES projects(id) ON DELETE SET NULL,pinned_ms INTEGER,archived_ms INTEGER);
                    CREATE TABLE settings(key TEXT PRIMARY KEY,value TEXT NOT NULL);
                    PRAGMA user_version=1;")?;
            } else {
                anyhow::ensure!(
                    version == 1,
                    "unsupported library schema version {version}; library was not overwritten"
                );
            }
            validate_library(&tx)?;
            tx.commit()?;
        } else {
            anyhow::ensure!(
                version == 1,
                "unsupported library schema version {version}; library was not overwritten"
            );
            validate_library(&conn)?;
        }
        conn.pragma_update(None, "journal_mode", "WAL")?;
        let lib = Self { conn };
        lib.settings()?;
        Ok(lib)
    }
    pub fn projects(&self) -> Result<Vec<Project>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id,name,icon,created_ms FROM projects ORDER BY created_ms DESC,id")?;
        let mut projects = stmt
            .query_map([], |r| {
                Ok(Project {
                    id: r.get::<_, i64>(0)?.to_string(),
                    name: r.get(1)?,
                    icon: r.get(2)?,
                    created_ms: r.get(3)?,
                    folders: vec![],
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut folders = self
            .conn
            .prepare("SELECT path FROM project_folders WHERE project_id=? ORDER BY position")?;
        for p in &mut projects {
            p.folders = folders
                .query_map([id(&p.id)?], |r| r.get(0))?
                .collect::<rusqlite::Result<_>>()?;
            anyhow::ensure!(
                !p.folders.is_empty(),
                "malformed library: project {} has no source folders",
                p.id
            );
        }
        Ok(projects)
    }
    pub fn project(&self, project_id: &str) -> Result<Project> {
        self.projects()?
            .into_iter()
            .find(|p| p.id == project_id)
            .context("project no longer exists")
    }
    pub fn settings(&self) -> Result<Value> {
        let get = |key: &str| -> Result<Option<Value>> {
            let raw: Option<String> = self
                .conn
                .query_row("SELECT value FROM settings WHERE key=?", [key], |r| {
                    r.get(0)
                })
                .optional()?;
            raw.map(|s| serde_json::from_str(&s).context("malformed library setting"))
                .transpose()
        };
        let default = get("default_harness")?.unwrap_or(Value::Null);
        validate_harness(&default)?;
        let style = get("icon_style")?.unwrap_or(json!("color"));
        anyhow::ensure!(
            style == "color" || style == "monochrome",
            "malformed icon_style setting"
        );
        Ok(json!({"default_harness":default,"icon_style":style}))
    }
    pub fn save_project(
        &mut self,
        project_id: Option<&str>,
        name: &str,
        icon: &str,
        folders: &[String],
        cwd: &str,
        home: &Path,
    ) -> Result<Value> {
        let name = name.trim();
        anyhow::ensure!(!name.is_empty(), "Name: enter a project name");
        anyhow::ensure!(
            ["folder", "code", "terminal", "puzzle", "wrench", "rocket"].contains(&icon),
            "Icon: choose a supported icon"
        );
        anyhow::ensure!(
            !folders.is_empty(),
            "Source folders: at least one source folder is required"
        );
        let old = project_id.map(|i| self.project(i)).transpose()?;
        let mut paths = Vec::new();
        let mut seen = HashSet::new();
        for folder in folders {
            let expanded = expand(folder, cwd, home)?;
            // Editing must not force removal of a previously configured missing folder.
            let normalized = match readable_dir(&expanded) {
                Ok(p) => p,
                Err(e)
                    if old
                        .as_ref()
                        .is_some_and(|p| p.folders.iter().any(|s| Path::new(s) == expanded)) =>
                {
                    let _ = e;
                    lexical(&expanded)
                }
                Err(e) => return Err(e).context("Source folders"),
            };
            if seen.insert(normalized.clone()) {
                paths.push(normalized.to_string_lossy().into_owned());
            }
        }
        let tx = self.conn.transaction()?;
        let project_id = if let Some(project_id) = project_id {
            let project_id = id(project_id)?;
            tx.execute(
                "UPDATE projects SET name=?,icon=? WHERE id=?",
                params![name, icon, project_id],
            )?;
            tx.execute(
                "DELETE FROM project_folders WHERE project_id=?",
                [project_id],
            )?;
            project_id
        } else {
            tx.execute(
                "INSERT INTO projects(name,icon,created_ms) VALUES(?,?,?)",
                params![name, icon, now_ms()],
            )?;
            tx.last_insert_rowid()
        };
        for (position, folder) in paths.iter().enumerate() {
            tx.execute(
                "INSERT INTO project_folders(project_id,position,path) VALUES(?,?,?)",
                params![project_id, position as i64, folder],
            )?;
        }
        tx.commit()?;
        Ok(json!({"project":self.project(&project_id.to_string())?}))
    }
    pub fn remove_project(&mut self, project_id: &str) -> Result<Value> {
        self.project(project_id)?;
        let tx = self.conn.transaction()?;
        let project_id = id(project_id)?;
        tx.execute(
            "UPDATE chat_state SET assignment='automatic',project_id=NULL WHERE project_id=?",
            [project_id],
        )?;
        tx.execute("DELETE FROM projects WHERE id=?", [project_id])?;
        tx.commit()?;
        Ok(json!({"ok":true}))
    }
    pub fn update_chat(
        &mut self,
        uid: &str,
        pinned: Option<bool>,
        archived: Option<bool>,
        assignment: Option<Assignment>,
    ) -> Result<Value> {
        anyhow::ensure!(!uid.is_empty(), "missing chat UID");
        let assigned = match &assignment {
            Some(a) => match a.mode.as_str() {
                "project" => Some(id(&self
                    .project(
                        a.project_id
                            .as_deref()
                            .context("assignment requires project_id")?,
                    )?
                    .id)?),
                "automatic" | "none" => {
                    anyhow::ensure!(
                        a.project_id.is_none(),
                        "only project assignments accept project_id"
                    );
                    None
                }
                _ => bail!("invalid assignment mode"),
            },
            None => None,
        };
        let tx = self.conn.transaction()?;
        tx.execute("INSERT OR IGNORE INTO chat_state(uid) VALUES(?)", [uid])?;
        if let Some(pinned) = pinned {
            tx.execute("UPDATE chat_state SET pinned_ms=CASE WHEN ? THEN COALESCE(pinned_ms,?) ELSE NULL END WHERE uid=?", params![pinned,now_ms(),uid])?;
        }
        if let Some(archived) = archived {
            tx.execute("UPDATE chat_state SET archived_ms=CASE WHEN ? THEN COALESCE(archived_ms,?) ELSE NULL END WHERE uid=?", params![archived,now_ms(),uid])?;
        }
        if let Some(a) = assignment {
            tx.execute(
                "UPDATE chat_state SET assignment=?,project_id=? WHERE uid=?",
                params![a.mode, assigned, uid],
            )?;
        }
        tx.commit()?;
        Ok(json!({"ok":true}))
    }
    pub fn update_settings(
        &mut self,
        default: Patch<Value>,
        style: Option<String>,
    ) -> Result<Value> {
        if let Patch::Present(v) = &default {
            validate_harness(v)?;
        }
        if let Some(style) = &style {
            anyhow::ensure!(
                style == "color" || style == "monochrome",
                "invalid icon style"
            );
        }
        let tx = self.conn.transaction()?;
        if let Patch::Present(v) = default {
            tx.execute("INSERT INTO settings(key,value) VALUES('default_harness',?) ON CONFLICT(key) DO UPDATE SET value=excluded.value", [v.to_string()])?;
        }
        if let Some(style) = style {
            tx.execute("INSERT INTO settings(key,value) VALUES('icon_style',?) ON CONFLICT(key) DO UPDATE SET value=excluded.value", [json!(style).to_string()])?;
        }
        tx.commit()?;
        Ok(json!({"settings":self.settings()?}))
    }
}

fn validate_library(conn: &Connection) -> Result<()> {
    validate_schema(conn)?;
    let malformed: i64 = conn.query_row("SELECT count(*) FROM chat_state WHERE assignment NOT IN ('automatic','project','none') OR (assignment='project' AND project_id IS NULL) OR (assignment!='project' AND project_id IS NOT NULL)", [], |r| r.get(0))?;
    anyhow::ensure!(malformed == 0, "malformed library chat assignments");
    Ok(())
}

fn validate_schema(conn: &Connection) -> Result<()> {
    let tables: &[(&str, &[(&str, &str, i64)])] = &[
        (
            "projects",
            &[
                ("id", "INTEGER", 1),
                ("name", "TEXT", 0),
                ("icon", "TEXT", 0),
                ("created_ms", "INTEGER", 0),
            ],
        ),
        (
            "project_folders",
            &[
                ("project_id", "INTEGER", 1),
                ("position", "INTEGER", 2),
                ("path", "TEXT", 0),
            ],
        ),
        (
            "chat_state",
            &[
                ("uid", "TEXT", 1),
                ("assignment", "TEXT", 0),
                ("project_id", "INTEGER", 0),
                ("pinned_ms", "INTEGER", 0),
                ("archived_ms", "INTEGER", 0),
            ],
        ),
        ("settings", &[("key", "TEXT", 1), ("value", "TEXT", 0)]),
    ];
    for (table, expected) in tables {
        let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
        let columns = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, i64>(5)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        anyhow::ensure!(
            columns.len() == expected.len()
                && columns.iter().zip(*expected).all(|(actual, expected)| {
                    actual.0 == expected.0
                        && actual.1.eq_ignore_ascii_case(expected.1)
                        && actual.2 == expected.2
                }),
            "malformed library schema: unsupported {table} columns or primary key"
        );
    }
    let mut foreign_keys = conn.prepare("PRAGMA foreign_key_check")?;
    anyhow::ensure!(
        foreign_keys.query([])?.next()?.is_none(),
        "malformed library: invalid project references"
    );
    let malformed: i64 = conn.query_row(
        "SELECT count(*) FROM projects p WHERE trim(p.name)='' OR NOT EXISTS (SELECT 1 FROM project_folders f WHERE f.project_id=p.id AND f.position=0)",
        [], |r| r.get(0))?;
    anyhow::ensure!(
        malformed == 0,
        "malformed library: projects require a name and a Primary folder"
    );
    Ok(())
}
fn validate_harness(v: &Value) -> Result<()> {
    anyhow::ensure!(
        v.is_null()
            || v.as_str().is_some_and(
                |s| ["omp", "claude", "codex", "pi", "opencode", "cursor"].contains(&s)
            ),
        "invalid default harness"
    );
    Ok(())
}
pub fn lexical(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in path.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                if matches!(out.components().next_back(), Some(Component::Normal(_))) {
                    out.pop();
                }
            }
            _ => out.push(c.as_os_str()),
        }
    }
    out
}
pub fn expand(value: &str, cwd: &str, home: &Path) -> Result<PathBuf> {
    anyhow::ensure!(!value.is_empty(), "enter a folder path");
    anyhow::ensure!(
        !value.contains(['\0', '\n', '\r']),
        "folder path contains a control character"
    );
    let path = if value == "~" {
        home.to_owned()
    } else if value.starts_with("~/") || value.starts_with("~\\") {
        home.join(&value[2..])
    } else {
        PathBuf::from(value)
    };
    let path = if path.is_absolute() {
        path
    } else {
        anyhow::ensure!(
            Path::new(cwd).is_absolute(),
            "a current working directory is required for relative paths"
        );
        Path::new(cwd).join(path)
    };
    Ok(lexical(&path))
}
pub fn readable_dir(path: &Path) -> Result<PathBuf> {
    anyhow::ensure!(
        path.is_dir(),
        "not an existing directory: {}",
        path.display()
    );
    std::fs::read_dir(path).with_context(|| format!("cannot read directory {}", path.display()))?;
    std::fs::canonicalize(path)
        .map(plain_path)
        .with_context(|| format!("cannot resolve directory {}", path.display()))
}
pub fn folders(value: &str, cwd: &str, home: &Path, offset: i64, limit: i64) -> Result<Value> {
    anyhow::ensure!(
        offset >= 0 && (1..=100).contains(&limit),
        "invalid folder page"
    );
    let path = readable_dir(&expand(value, cwd, home)?)?;
    let mut entries = Vec::new();
    for entry in std::fs::read_dir(&path)? {
        let entry = entry?;
        if entry.path().is_dir() {
            entries.push((
                entry.file_name().to_string_lossy().into_owned(),
                entry.path(),
            ));
        }
    }
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    let total = entries.len();
    let entries: Vec<_> = entries
        .into_iter()
        .skip(offset as usize)
        .take(limit as usize)
        .map(|(name, path)| json!({"name":name,"path":path}))
        .collect();
    Ok(json!({"path":path,"parent":path.parent(),"entries":entries,"total":total}))
}

pub struct Membership {
    folders: HashMap<String, Vec<PathBuf>>,
    paths: HashMap<String, PathBuf>,
}
fn normalized(path: &str) -> PathBuf {
    let path = std::fs::canonicalize(path).unwrap_or_else(|_| lexical(Path::new(path)));
    #[cfg(windows)]
    {
        PathBuf::from(path.to_string_lossy().to_lowercase())
    }
    #[cfg(not(windows))]
    {
        path
    }
}
impl Membership {
    pub fn new(projects: Vec<Project>) -> Self {
        let folders = projects
            .into_iter()
            .map(|p| (p.id, p.folders.iter().map(|f| normalized(f)).collect()))
            .collect();
        Self {
            folders,
            paths: HashMap::new(),
        }
    }
    pub fn matches(
        &mut self,
        project_id: &str,
        cwd: &str,
        assignment: &str,
        assigned: Option<i64>,
    ) -> bool {
        match assignment {
            "none" => false,
            "project" => project_id.parse::<i64>().ok() == assigned,
            _ => {
                if !self.paths.contains_key(cwd) {
                    self.paths.insert(cwd.to_owned(), normalized(cwd));
                }
                let path = &self.paths[cwd];
                path.is_absolute()
                    && self.folders.get(project_id).is_some_and(|folders| {
                        folders.iter().any(|folder| path.starts_with(folder))
                    })
            }
        }
    }
}
