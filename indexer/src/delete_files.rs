use super::*;
use std::io::Write;

pub(super) struct Plan {
    root: PathBuf,
    removals: Vec<PathBuf>,
    directories: Vec<PathBuf>,
    rewrites: Vec<(PathBuf, Vec<u8>)>,
    history_db: Option<PathBuf>,
    history_tables: Vec<&'static str>,
    history_fts: bool,
    id: String,
}

pub(super) fn prepare(home: &Path, chat: &Chat) -> Result<super::Plan> {
    valid_id(&chat.id)?;
    let relative = match chat.agent.as_str() {
        "omp" => ".omp/agent",
        "pi" => ".pi/agent",
        "claude" => ".claude",
        _ => unreachable!(),
    };
    let root = home.join(relative);
    safe_path(home, &root, true)?;
    let source = PathBuf::from(
        chat.source
            .strip_prefix(&format!("{}:", chat.agent))
            .context("Invalid file source namespace")?,
    );
    let sessions = root.join(if chat.agent == "claude" {
        "projects"
    } else {
        "sessions"
    });
    safe_path(&root, &source, false)?;
    let parts = source.strip_prefix(&sessions)?.components().count();
    ensure!(
        parts == 2 && source.extension().is_some_and(|s| s == "jsonl"),
        "Expected one exact depth-two session JSONL file"
    );
    let text = std::fs::read_to_string(&source)?;
    validate_header(&text, chat)?;
    let mut p = Plan {
        root: root.clone(),
        removals: vec![source.clone()],
        directories: vec![],
        rewrites: vec![],
        history_db: None,
        history_tables: vec![],
        history_fts: false,
        id: chat.id.clone(),
    };
    let mut stamps = BTreeSet::new();
    inventory(&root, &source, &mut stamps)?;
    // Capture directory metadata too: creation/removal of a backup or sidecar invalidates confirmation.
    stamps.insert(
        source
            .parent()
            .context("Session has no parent")?
            .to_path_buf(),
    );
    if chat.agent == "omp" {
        add_optional_removal(&mut p, source.with_extension(""), &mut stamps)?;
        let prefix = format!(
            "{}.",
            source
                .file_name()
                .context("Missing filename")?
                .to_string_lossy()
        );
        let mut backups = vec![];
        for entry in std::fs::read_dir(source.parent().unwrap())? {
            let e = entry?;
            let name = e.file_name();
            let name = name.to_string_lossy();
            if name.starts_with(&prefix) && name.ends_with(".bak") && name.len() > prefix.len() + 4
            {
                safe_path(&root, &e.path(), false)?;
                backups.push(e.path());
            }
        }
        backups.sort();
        for backup in backups {
            add_optional_removal(&mut p, backup, &mut stamps)?;
        }
        let db_path = root.join("history.db");
        if db_path.try_exists()? {
            add_database(&root, &db_path, &mut stamps)?;
            let conn = crate::db::open_readonly(&db_path)?;
            for table in ["history", "session_titles", "session_recaps"] {
                if table_exists(&conn, table)? {
                    // Preparing verifies the required column before the first unlink.
                    conn.prepare(&format!("SELECT session_id FROM {table} LIMIT 0"))?;
                    p.history_tables.push(table);
                }
            }
            if table_exists(&conn, "history_fts")? {
                ensure!(p.history_tables.contains(&"history"), "OMP history_fts has no history table");
                let sql: String = conn.query_row(
                    "SELECT sql FROM sqlite_master WHERE type='table' AND name='history_fts'",
                    [],
                    |row| row.get(0),
                )?;
                let schema: String = sql
                    .chars()
                    .filter(|c| !c.is_ascii_whitespace() && !['\'', '"', '`', '[', ']'].contains(c))
                    .map(|c| c.to_ascii_lowercase())
                    .collect();
                let options = schema
                    .strip_prefix("createvirtualtablehistory_ftsusingfts5(")
                    .and_then(|s| s.strip_suffix(')'))
                    .context("Unsupported OMP history_fts schema")?;
                ensure!(
                    ["prompt", "content=history", "content_rowid=id"]
                        .iter()
                        .all(|expected| options.split(',').any(|option| option == *expected)),
                    "Unsupported OMP history_fts schema"
                );
                conn.prepare("SELECT id, prompt FROM history LIMIT 0")?;
                conn.prepare("SELECT rowid, prompt FROM history_fts LIMIT 0")?;
                p.history_fts = true;
            }
            p.history_db = Some(db_path);
        }
    } else if chat.agent == "claude" {
        ensure!(
            source.file_stem().and_then(|s| s.to_str()) == Some(chat.id.as_str()),
            "Claude transcript filename does not match indexed ID"
        );
        add_optional_removal(&mut p, source.with_extension(""), &mut stamps)?;
        for rel in [
            format!("file-history/{}", chat.id),
            format!("debug/{}.txt", chat.id),
            format!("session-env/{}", chat.id),
            format!("image-cache/{}", chat.id),
            format!("uploads/{}", chat.id),
        ] {
            let target = root.join(rel);
            add_optional_removal(&mut p, target, &mut stamps)?;
        }
        let history = root.join("history.jsonl");
        if history.try_exists()? {
            inventory(&root, &history, &mut stamps)?;
            let bytes = std::fs::read(&history)?;
            let mut retained = Vec::with_capacity(bytes.len());
            for line in bytes.split_inclusive(|b| *b == b'\n') {
                if line.iter().all(u8::is_ascii_whitespace) {
                    retained.extend_from_slice(line);
                    continue;
                }
                let value: Value = serde_json::from_slice(line).context("Unsupported Claude history.jsonl record; refusing deletion before changing any files")?;
                let id = value
                    .get("sessionId")
                    .and_then(Value::as_str)
                    .context("Unsupported Claude history.jsonl sessionId schema")?;
                if id != chat.id {
                    retained.extend_from_slice(line);
                }
            }
            if retained != bytes {
                p.rewrites.push((history, retained));
            }
        }
        let index = source.parent().unwrap().join("sessions-index.json");
        if index.try_exists()? {
            // No installed sessions-index cache schema has been verified. Never guess a writer.
            bail!("Unsupported Claude sessions-index.json cache at {}; this installed cache layout must be verified before permanent deletion",index.display());
        }
    }
    let mut targets = p
        .removals
        .iter()
        .map(|p| p.to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    targets.extend(
        p.rewrites
            .iter()
            .map(|(p, _)| format!("{} (remove exact sessionId records)", p.display())),
    );
    if let Some(db) = &p.history_db {
        targets.extend(
            p.history_tables
                .iter()
                .map(|table| format!("{}: {table}[session_id={}]", db.display(), chat.id)),
        );
    }
    let mut ids = vec![chat.id.clone()];
    // Child transcript headers are read only inside an owned artifacts directory, never elsewhere.
    if chat.agent != "pi" {
        for path in &stamps {
            let owned_child = p
                .removals
                .iter()
                .any(|removed| removed.is_dir() && path.starts_with(removed));
            if owned_child && path.extension().is_some_and(|e| e == "jsonl") && path.is_file() {
                targets.push(path.to_string_lossy().into_owned());
                let content = std::fs::read_to_string(path)?;
                for line in content.lines().filter(|s| !s.trim().is_empty()) {
                    let Ok(v) = serde_json::from_str::<Value>(line) else {
                        continue;
                    };
                    let id = if chat.agent == "omp" {
                        if v.get("type").and_then(Value::as_str) == Some("session") {
                            v.get("id").and_then(Value::as_str)
                        } else {
                            None
                        }
                    } else {
                        v.get("sessionId").and_then(Value::as_str)
                    };
                    if let Some(id) = id {
                        valid_id(id)?;
                        if !ids.iter().any(|s| s == id) {
                            ids.push(id.to_string());
                        }
                        break;
                    }
                }
            }
        }
    }
    ids.sort();
    p.directories = p
        .removals
        .iter()
        .filter(|path| path.is_dir())
        .cloned()
        .collect();
    Ok(super::Plan {
        operation: Operation::Files(p),
        targets,
        ids,
        stamps,
    })
}

pub(super) fn owns_source(plan: &Plan, agent: &str, source: &str) -> bool {
    let Some(path) = source.strip_prefix(&format!("{agent}:")).map(Path::new) else {
        return false;
    };
    plan.removals.iter().any(|removed| path == removed)
        || plan
            .directories
            .iter()
            .any(|removed| path.starts_with(removed))
}

/// Enumerate only exact removed files and bounded owned-directory cache entries.
pub(super) fn cached_owned_sources(
    plan: &Plan,
    agent: &str,
    conn: &Connection,
) -> Result<Vec<String>> {
    let mut sources = plan
        .removals
        .iter()
        .filter(|path| !plan.directories.contains(path))
        .map(|path| format!("{agent}:{}", path.to_string_lossy()))
        .collect::<Vec<_>>();
    for directory in &plan.directories {
        let prefix = format!(
            "{agent}:{}{}",
            directory.to_string_lossy(),
            std::path::MAIN_SEPARATOR
        );
        let mut statement = conn.prepare(
            "SELECT source FROM sources WHERE agent=?1 AND substr(source,1,length(?2))=?2",
        )?;
        let rows = statement
            .query_map(params![agent, prefix], |r| r.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        for source in rows {
            let path = Path::new(
                source
                    .strip_prefix(&format!("{agent}:"))
                    .context("Invalid owned source namespace")?,
            );
            if path.starts_with(directory)
                && path
                    .components()
                    .all(|c| !matches!(c, Component::ParentDir | Component::CurDir))
            {
                sources.push(source);
            }
        }
    }
    sources.sort();
    sources.dedup();
    Ok(sources)
}

fn validate_header(text: &str, chat: &Chat) -> Result<()> {
    let mut lines = text.lines().filter(|s| !s.trim().is_empty());
    if chat.agent == "claude" {
        let mut found = false;
        for line in lines {
            let v: Value = serde_json::from_str(line)
                .context("Malformed Claude transcript; refusing deletion")?;
            if let Some(id) = v.get("sessionId") {
                ensure!(
                    id.as_str() == Some(chat.id.as_str()),
                    "Claude transcript contains a different sessionId"
                );
                found = true;
            }
        }
        ensure!(found, "Claude transcript has no verifiable sessionId");
    } else {
        let mut first: Value =
            serde_json::from_str(lines.next().context("Empty session transcript")?)?;
        if chat.agent == "omp" && first.get("type").and_then(Value::as_str) == Some("title") {
            first = serde_json::from_str(
                lines
                    .next()
                    .context("OMP title slot is not followed by a session header")?,
            )?;
        }
        ensure!(
            first.get("type").and_then(Value::as_str) == Some("session")
                && first.get("id").and_then(Value::as_str) == Some(chat.id.as_str()),
            "Session header ID does not match indexed chat"
        );
    }
    Ok(())
}

fn add_optional_removal(p: &mut Plan, path: PathBuf, stamps: &mut BTreeSet<PathBuf>) -> Result<()> {
    // symlink_metadata notices dangling symlinks as well; try_exists would hide them.
    match std::fs::symlink_metadata(&path) {
        Ok(_) => {
            inventory(&p.root, &path, stamps)?;
            p.removals.push(path);
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            if let Some(parent) = path.parent() {
                if parent.try_exists()? {
                    safe_path(&p.root, parent, true)?;
                    stamps.insert(parent.to_path_buf());
                }
            }
        }
        Err(e) => return Err(e.into()),
    }
    Ok(())
}

pub(super) fn execute(p: &Plan) -> Result<()> {
    let mut done = Vec::new();
    let result = (|| -> Result<()> {
        // Prepare replacement files and acquire the history write lock before any source removal.
        let mut prepared = Vec::new();
        for (path, bytes) in &p.rewrites {
            prepared.push(Replacement::new(path, bytes)?);
        }
        let mut history = if let Some(path) = &p.history_db {
            Some(Connection::open_with_flags(
                path,
                rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE,
            )?)
        } else {
            None
        };
        let transaction = if let Some(conn) = history.as_mut() {
            Some(conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?)
        } else {
            None
        };
        if let Some(tx) = transaction.as_ref() {
            for table in &p.history_tables {
                let removed = tx.execute(&format!("DELETE FROM {table} WHERE session_id=?1"), [&p.id])?;
                if *table == "history" && removed > 0 && p.history_fts {
                    // Rebuild from surviving prompts, including when the old index was inconsistent.
                    tx.execute("INSERT INTO history_fts(history_fts) VALUES('rebuild')", [])?;
                }
            }
        }
        for path in &p.removals {
            let directory = std::fs::symlink_metadata(path)?.is_dir();
            safe_path(&p.root, path, directory)?;
            if directory {
                std::fs::remove_dir_all(path)?;
            } else {
                std::fs::remove_file(path)?;
            }
            done.push(path.to_string_lossy().into_owned());
        }
        for mut replacement in prepared {
            replacement.commit()?;
            done.push(replacement.destination.to_string_lossy().into_owned());
        }
        if let Some(tx) = transaction {
            tx.commit()?;
            if let Some(path) = &p.history_db {
                done.push(path.to_string_lossy().into_owned());
            }
        }
        Ok(())
    })();
    if let Err(e) = result {
        let mut remaining = p
            .removals
            .iter()
            .filter(|p| std::fs::symlink_metadata(p).is_ok())
            .map(|p| p.display().to_string())
            .collect::<Vec<_>>();
        for path in p
            .rewrites
            .iter()
            .map(|(path, _)| path)
            .chain(p.history_db.iter())
        {
            let path = path.to_string_lossy().into_owned();
            if !done.contains(&path) {
                remaining.push(path);
            }
        }
        bail!("Permanent source cleanup failed: {e:#}. Completed paths: {}. Remaining paths: {}. No automatic retry or backup was made",done.join(", "),remaining.join(", "));
    }
    Ok(())
}

pub(super) fn verify(p: &Plan) -> Result<()> {
    for path in &p.removals {
        ensure!(
            std::fs::symlink_metadata(path)
                .is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound),
            "Source target remains: {}",
            path.display()
        );
    }
    for (path, expected) in &p.rewrites {
        ensure!(
            std::fs::read(path)? == *expected,
            "History rewrite did not persist: {}",
            path.display()
        );
    }
    if let Some(db) = &p.history_db {
        let conn = crate::db::open_readonly(db)?;
        for table in &p.history_tables {
            let count: i64 = conn.query_row(
                &format!("SELECT count(*) FROM {table} WHERE session_id=?1"),
                [&p.id],
                |r| r.get(0),
            )?;
            ensure!(count == 0, "OMP history still contains the deleted session");
        }
    }
    Ok(())
}

struct Replacement {
    destination: PathBuf,
    temporary: PathBuf,
    committed: bool,
}
impl Replacement {
    fn new(destination: &Path, bytes: &[u8]) -> Result<Self> {
        let name = destination
            .file_name()
            .context("Missing history filename")?
            .to_string_lossy();
        let temporary =
            destination.with_file_name(format!(".{name}.tern-projects-{}.tmp", std::process::id()));
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        let result = (|| -> Result<()> {
            file.set_permissions(std::fs::metadata(destination)?.permissions())?;
            file.write_all(bytes)?;
            file.sync_all()?;
            Ok(())
        })();
        if let Err(e) = result {
            let _ = std::fs::remove_file(&temporary);
            return Err(e);
        }
        Ok(Self {
            destination: destination.to_path_buf(),
            temporary,
            committed: false,
        })
    }
    fn commit(&mut self) -> Result<()> {
        std::fs::rename(&self.temporary, &self.destination)?;
        self.committed = true;
        Ok(())
    }
}
impl Drop for Replacement {
    fn drop(&mut self) {
        if !self.committed {
            let _ = std::fs::remove_file(&self.temporary);
        }
    }
}
