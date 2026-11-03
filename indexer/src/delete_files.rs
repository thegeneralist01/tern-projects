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
    ids: Vec<String>,
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
        ids: vec![],
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
        if db_path.try_exists()? || db_path.is_symlink() {
            add_history_database(&root, &db_path, &mut stamps)?;
            let conn = crate::db::open_readonly(&db_path)?;
            (p.history_tables, p.history_fts) = validate_history_schema(&conn)?;
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
                if chat.agent == "omp" {
                    let id = omp_session_id(&content)?;
                    if !ids.contains(&id) {
                        ids.push(id);
                    }
                } else {
                    for line in content.lines().filter(|s| !s.trim().is_empty()) {
                        let Ok(v) = serde_json::from_str::<Value>(line) else {
                            continue;
                        };
                        if let Some(id) = v.get("sessionId").and_then(Value::as_str) {
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
    }
    ids.sort();
    if let Some(db) = &p.history_db {
        for table in &p.history_tables {
            for id in &ids {
                targets.push(format!("{}: {table}[session_id={id}]", db.display()));
            }
        }
    }
    p.ids = ids.clone();
    p.directories = p
        .removals
        .iter()
        .filter(|path| path.is_dir())
        .cloned()
        .collect();
    if chat.agent == "omp" && ids.len() > 1 {
        validate_dependent_inventory(&p, &sessions, &chat.id, &mut stamps)?;
    }
    Ok(super::Plan {
        operation: Operation::Files(p),
        targets,
        ids,
        stamps,
    })
}

// Child attribution is not proved by the disposable index alone: an unindexed
// transcript elsewhere in this bounded source tree may reuse the same ID.
fn validate_dependent_inventory(
    p: &Plan,
    path: &Path,
    target: &str,
    stamps: &mut BTreeSet<PathBuf>,
) -> Result<()> {
    if p.removals.iter().any(|owned| path == owned)
        || p.directories.iter().any(|owned| path.starts_with(owned))
    {
        return Ok(());
    }
    let metadata = std::fs::symlink_metadata(path)?;
    ensure!(
        !metadata.file_type().is_symlink(),
        "Unverifiable OMP dependent inventory symlink: {}",
        path.display()
    );
    if metadata.is_dir() {
        safe_path(&p.root, path, true)?;
        stamps.insert(path.to_path_buf());
        for entry in std::fs::read_dir(path)? {
            validate_dependent_inventory(p, &entry?.path(), target, stamps)?;
        }
    } else if path.extension().is_some_and(|extension| extension == "jsonl") {
        safe_path(&p.root, path, false)?;
        stamps.insert(path.to_path_buf());
        let id = omp_session_id(&std::fs::read_to_string(path)?)?;
        ensure!(
            id == target || !p.ids.contains(&id),
            "Ambiguous child-session identity: {id} also exists outside the owned deletion targets at {}; refusing deletion",
            path.display()
        );
    }
    Ok(())
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
    } else if chat.agent == "omp" {
        ensure!(
            omp_session_id(text)? == chat.id,
            "Session header ID does not match indexed chat"
        );
    } else {
        let first: Value =
            serde_json::from_str(lines.next().context("Empty session transcript")?)?;
        ensure!(
            first.get("type").and_then(Value::as_str) == Some("session")
                && first.get("id").and_then(Value::as_str) == Some(chat.id.as_str()),
            "Session header ID does not match indexed chat"
        );
    }
    Ok(())
}

fn omp_session_id(text: &str) -> Result<String> {
    let mut lines = text.lines().filter(|s| !s.trim().is_empty());
    let mut header: Value = serde_json::from_str(lines.next().context("Empty OMP transcript")?)?;
    if header.get("type").and_then(Value::as_str) == Some("title") {
        header = serde_json::from_str(
            lines.next().context("OMP title slot is not followed by a session header")?,
        )?;
    }
    ensure!(
        header.get("type").and_then(Value::as_str) == Some("session"),
        "OMP transcript has no verifiable session header"
    );
    let id = header.get("id").and_then(Value::as_str).context("OMP transcript has no session ID")?;
    valid_id(id)?;
    for line in lines {
        let record: Value = serde_json::from_str(line).context("Malformed OMP transcript; refusing deletion")?;
        if record.get("type").and_then(Value::as_str) == Some("session") {
            ensure!(
                record.get("id").and_then(Value::as_str) == Some(id),
                "OMP transcript contains conflicting session IDs"
            );
        }
    }
    Ok(id.to_string())
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

// Deliberately recognize only verified SQL shapes. In particular, removing quotes
// from string literals would incorrectly equate different content-store ownership.
fn normalized_sql(sql: &str) -> String {
    let mut quoted = false;
    let mut normalized = sql.chars()
        .filter_map(|c| {
            if c == '\'' {
                quoted = !quoted;
                Some(c)
            } else if quoted {
                Some(c)
            } else if c.is_ascii_whitespace() {
                None
            } else {
                Some(c.to_ascii_lowercase())
            }
        })
        .collect::<String>();
    while normalized.ends_with(';') {
        normalized.pop();
    }
    normalized
}

fn validate_history_schema(conn: &Connection) -> Result<(Vec<&'static str>, bool)> {
    let mut tables = Vec::new();
    for table in ["history", "session_titles", "session_recaps"] {
        if !table_exists(conn, table)? {
            continue;
        }
        let sql: String = conn.query_row(
            "SELECT sql FROM sqlite_master WHERE type='table' AND name=?1",
            [table],
            |r| r.get(0),
        )?;
        ensure!(
            normalized_sql(&sql).starts_with(&format!("createtable{table}(")),
            "Unsupported OMP {table} table schema"
        );
        let columns = conn.prepare(&format!("PRAGMA table_xinfo({table})"))?
            .query_map([], |r| {
                Ok((r.get::<_, String>(1)?, r.get::<_, String>(2)?,
                    r.get::<_, i64>(5)?, r.get::<_, i64>(6)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        ensure!(
            columns.iter().any(|(name, ty, _, hidden)|
                name == "session_id" && ty.eq_ignore_ascii_case("TEXT") && *hidden == 0),
            "Unsupported OMP {table} session_id ownership column"
        );
        if table == "history" {
            ensure!(
                columns.iter().any(|(name, ty, pk, hidden)|
                    name == "id" && ty.eq_ignore_ascii_case("INTEGER") && *pk == 1 && *hidden == 0)
                    && columns.iter().any(|(name, ty, _, hidden)|
                        name == "prompt" && ty.eq_ignore_ascii_case("TEXT") && *hidden == 0),
                "Unsupported OMP history identity/prompt schema"
            );
            ensure!(
                columns.iter().filter(|(_, _, pk, _)| *pk != 0).count() == 1,
                "Unsupported OMP history primary key"
            );
            // INTEGER PRIMARY KEY must alias rowid, not a separate (e.g. DESC
            // or WITHOUT ROWID) index that external-content FTS cannot own.
            let separate_primary_key: bool = conn.query_row(
                "SELECT EXISTS(SELECT 1 FROM pragma_index_list('history') WHERE origin='pk')",
                [],
                |r| r.get(0),
            )?;
            ensure!(!separate_primary_key, "OMP history IDs do not alias SQLite row ownership");
        }
        tables.push(table);
    }
    ensure!(!tables.is_empty(), "Unrecognized OMP history store ownership");
    let fts = table_exists(conn, "history_fts")?;
    if fts {
        ensure!(tables.contains(&"history"), "OMP history_fts has no history table");
        let sql: String = conn.query_row(
            "SELECT sql FROM sqlite_master WHERE type='table' AND name='history_fts'",
            [],
            |r| r.get(0),
        )?;
        let normalized = normalized_sql(&sql);
        let options = normalized
            .strip_prefix("createvirtualtablehistory_ftsusingfts5(")
            .and_then(|s| s.strip_suffix(')'))
            .context("Unsupported OMP history_fts schema")?;
        let mut options = options.split(',').collect::<Vec<_>>();
        options.sort_unstable();
        ensure!(
            options == ["content='history'", "content_rowid='id'", "prompt"]
                || options == ["content=history", "content_rowid=id", "prompt"],
            "Unsupported OMP history_fts schema"
        );
        conn.prepare("SELECT rowid, prompt FROM history_fts LIMIT 0")?;
    }
    let mut statement = conn.prepare("SELECT name FROM sqlite_master WHERE type='table'")?;
    for table in statement.query_map([], |r| r.get::<_, String>(0))? {
        let table = table?;
        let sql = format!("PRAGMA foreign_key_list(\"{}\")", table.replace('"', "\"\""));
        let mut keys = conn.prepare(&sql)?;
        for referenced in keys.query_map([], |r| r.get::<_, String>(2))? {
            let referenced = referenced?;
            ensure!(
                !tables.iter().any(|target| target.eq_ignore_ascii_case(&referenced)),
                "Unsupported OMP history foreign-key side effects from {table}"
            );
        }
    }
    // A session_id predicate alone cannot bound trigger side effects. Unknown
    // triggers, including those on FTS shadow tables, must refuse before unlink.
    let mut statement = conn.prepare("SELECT name, sql FROM sqlite_master WHERE type='trigger'")?;
    for trigger in statement.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))? {
        let (name, sql) = trigger?;
        ensure!(
            fts && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'),
            "Unsupported OMP history trigger: {name}"
        );
        let prefix = format!("createtrigger{}", name.to_ascii_lowercase());
        let normalized = normalized_sql(&sql);
        let body = normalized.strip_prefix(&prefix)
            .context("Unsupported OMP history trigger declaration")?;
        ensure!(
            matches!(body,
                "afterinsertonhistorybegininsertintohistory_fts(rowid,prompt)values(new.id,new.prompt);end"
                | "afterdeleteonhistorybegininsertintohistory_fts(history_fts,rowid,prompt)values('delete',old.id,old.prompt);end"
                | "afterdeleteonhistorybegindeletefromhistory_ftswhererowid=old.id;end"),
            "Unsupported OMP history trigger: {name}"
        );
    }
    Ok((tables, fts))
}

fn add_history_database(root: &Path, path: &Path, stamps: &mut BTreeSet<PathBuf>) -> Result<()> {
    safe_path(root, path, false)?;
    stamps.insert(path.to_path_buf());
    for suffix in ["-wal", "-journal"] {
        let sidecar = PathBuf::from(format!("{}{suffix}", path.to_string_lossy()));
        match std::fs::symlink_metadata(&sidecar) {
            Ok(metadata) => {
                safe_path(root, &sidecar, false)?;
                // Opening a clean WAL store/BEGIN IMMEDIATE may create an empty
                // WAL itself. It contains no committed frames to confirm.
                if suffix != "-wal" || metadata.len() != 0 {
                    stamps.insert(sidecar);
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
    }
    Ok(())
}

/// Include newly-created WAL/journal files when revalidating confirmation, too.
pub(super) fn history_stamps(p: &Plan, stamps: &mut BTreeSet<PathBuf>) -> Result<()> {
    if let Some(path) = &p.history_db {
        add_history_database(&p.root, path, stamps)?;
    } else if p.root.ends_with(".omp/agent")
        && (p.root.join("history.db").try_exists()? || p.root.join("history.db").is_symlink())
    {
        bail!("OMP history store appeared; obtain a fresh deletion preview");
    }
    Ok(())
}

pub(super) fn execute(p: &Plan, revalidate: impl FnOnce() -> Result<()>) -> Result<()> {
    let mut done = Vec::new();
    let result = (|| -> Result<()> {
        // Prepare replacement files and acquire the history write lock before any source removal.
        let mut prepared = Vec::new();
        for (path, bytes) in &p.rewrites {
            prepared.push(Replacement::new(path, bytes)?);
        }
        let mut history = if let Some(path) = &p.history_db {
            safe_path(&p.root, path, false)?;
            let conn = Connection::open_with_flags(
                path,
                rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE,
            )?;
            conn.busy_timeout(std::time::Duration::from_millis(5000))?;
            Some(conn)
        } else {
            None
        };
        let transaction = if let Some(conn) = history.as_mut() {
            Some(conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?)
        } else {
            None
        };
        // Confirmation must still match after obtaining the history write lock.
        // No other writer can change its rows/schema between this check and our SQL.
        revalidate()?;
        if let Some(tx) = transaction.as_ref() {
            let (tables, fts) = validate_history_schema(tx)?;
            ensure!(
                tables == p.history_tables && fts == p.history_fts,
                "OMP history schema changed; obtain a fresh deletion preview"
            );
            for table in &p.history_tables {
                let mut removed = 0;
                for id in &p.ids {
                    removed += tx.execute(
                        &format!("DELETE FROM {table} WHERE session_id COLLATE BINARY=?1"),
                        [id],
                    )?;
                }
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
            for id in &p.ids {
                let count: i64 = conn.query_row(
                    &format!("SELECT count(*) FROM {table} WHERE session_id COLLATE BINARY=?1"),
                    [id],
                    |r| r.get(0),
                )?;
                ensure!(count == 0, "OMP history still contains a deleted session");
            }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_confirmation_is_revalidated_under_the_write_lock() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path();
        let root = home.join(".omp/agent");
        let source = root.join("sessions/fixture/target.jsonl");
        std::fs::create_dir_all(source.parent().unwrap()).unwrap();
        std::fs::write(&source, "{\"type\":\"session\",\"id\":\"target\"}\n").unwrap();
        let bytes = std::fs::read(&source).unwrap();
        let history = Connection::open(root.join("history.db")).unwrap();
        history.execute_batch(
            "CREATE TABLE history(id INTEGER PRIMARY KEY,prompt TEXT,session_id TEXT);
             INSERT INTO history VALUES(1,'target prompt','target');",
        ).unwrap();
        let chat = Chat {
            uid: "omp:target".into(),
            agent: "omp".into(),
            id: "target".into(),
            source: format!("omp:{}", source.display()),
            title: "target".into(),
            version: "fixture".into(),
        };
        let plan = prepare(home, &chat).unwrap();
        let expected = super::super::token(&chat, &plan).unwrap();
        // A new WAL after the first token check must be observed, even though
        // it was absent from the original inventory.
        history.execute_batch(
            "PRAGMA journal_mode=WAL;
             INSERT INTO history VALUES(2,'neighbor prompt','neighbor');",
        ).unwrap();
        history.busy_timeout(std::time::Duration::ZERO).unwrap();
        let Operation::Files(files) = &plan.operation else { unreachable!() };
        let mut checked = false;
        let result = execute(files, || {
            checked = true;
            assert!(history.execute_batch("BEGIN IMMEDIATE").is_err(), "History write lock is not held");
            ensure!(
                super::super::token(&chat, &plan)? == expected,
                "Stale confirmation"
            );
            Ok(())
        });
        assert!(checked, "Confirmation callback was never reached");
        assert!(result.is_err());
        assert_eq!(std::fs::read(&source).unwrap(), bytes);
        assert_eq!(history.query_row("SELECT count(*) FROM history", [], |r| r.get::<_, i64>(0)).unwrap(), 2);
    }
}
