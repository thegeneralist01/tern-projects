use serde_json::{json, Value};
use std::{
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};
const BIN: &str = env!("CARGO_BIN_EXE_tern-projects");
fn call(db: &Path, home: &Path, request: Value) -> std::process::Output {
    let mut child = Command::new(BIN)
        .env("TERN_PROJECTS_HOME", home)
        .arg("--db")
        .arg(db)
        .arg("library")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(request.to_string().as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
}
fn library(db: &Path, home: &Path, request: Value) -> Value {
    let output = call(db, home, request.clone());
    assert!(
        output.status.success(),
        "{request}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}
fn index(db: &Path, home: &Path) {
    let out = Command::new(BIN)
        .env("TERN_PROJECTS_HOME", home)
        .arg("--db")
        .arg(db)
        .arg("index")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}
fn seed(home: &Path, agent: &str, id: &str) -> PathBuf {
    let folder = home.join(format!(".{agent}/agent/sessions/fixture"));
    std::fs::create_dir_all(&folder).unwrap();
    let file = folder.join(format!("{id}.jsonl"));
    std::fs::write(&file,format!("{}\n{}\n",json!({"type":"session","id":id,"cwd":home,"timestamp":1000}),json!({"type":"message","timestamp":1001,"message":{"role":"user","content":[{"type":"text","text":"Synthetic deletion fixture"}]}}))).unwrap();
    file
}
fn preview(db: &Path, home: &Path, uid: &str) -> Value {
    library(db, home, json!({"op":"delete-preview","uid":uid}))
}
fn deletion(preview: &Value) -> Value {
    json!({"op":"delete","uid":preview["uid"],"expected_source":preview["source"],"expected_version":preview["version"]})
}
fn blocked_or_failure(db: &Path, home: &Path, uid: &str) {
    let p = preview(db, home, uid);
    if p["blocked_reason"].is_null() {
        assert!(!call(db, home, deletion(&p)).status.success());
    }
}
#[test]
fn omp_deletes_owned_artifacts_but_pi_unlinks_only_transcript() {
    for agent in ["omp", "pi"] {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("home");
        let db = temp.path().join("index.sqlite");
        let target = seed(&home, agent, "target");
        let neighbor = seed(&home, agent, "neighbor");
        let neighbor_bytes = std::fs::read(&neighbor).unwrap();
        let artifacts = target.with_extension("");
        std::fs::create_dir_all(&artifacts).unwrap();
        std::fs::write(artifacts.join("owned.txt"), "artifact").unwrap();
        let backup = target.with_file_name("target.jsonl.rewrite.bak");
        std::fs::write(&backup, "backup").unwrap();
        let near_backup = target.with_file_name("target-other.jsonl.rewrite.bak");
        std::fs::write(&near_backup, "neighbor backup").unwrap();
        let work = home.join("work.txt");
        std::fs::write(&work, "working source").unwrap();
        if agent == "omp" {
            let conn = rusqlite::Connection::open(home.join(".omp/agent/history.db")).unwrap();
            conn.execute_batch("CREATE TABLE session_titles(session_id TEXT PRIMARY KEY,title TEXT,updated_at INTEGER);CREATE TABLE session_recaps(id INTEGER PRIMARY KEY,session_id TEXT,cwd TEXT,recap TEXT,created_at INTEGER);INSERT INTO session_titles VALUES('target','target',1),('neighbor','neighbor',1);INSERT INTO session_recaps VALUES(1,'target','','target',1),(2,'neighbor','','neighbor',1);").unwrap();
        }
        index(&db, &home);
        library(
            &db,
            &home,
            json!({"op":"chat-update","uid":format!("{agent}:target"),"pinned":true}),
        );
        let p = preview(&db, &home, &format!("{agent}:target"));
        assert!(p["blocked_reason"].is_null(), "{p}");
        let result = library(&db, &home, deletion(&p));
        assert_eq!(result["ok"], true);
        assert!(!target.exists());
        assert_eq!(std::fs::read(&neighbor).unwrap(), neighbor_bytes);
        assert_eq!(std::fs::read_to_string(&work).unwrap(), "working source");
        assert!(near_backup.exists());
        if agent == "omp" {
            assert!(!artifacts.exists());
            assert!(!backup.exists());
            let conn = rusqlite::Connection::open(home.join(".omp/agent/history.db")).unwrap();
            assert_eq!(
                conn.query_row("SELECT session_id FROM session_titles", [], |r| r
                    .get::<_, String>(0))
                    .unwrap(),
                "neighbor"
            );
        } else {
            assert!(artifacts.exists());
            assert!(backup.exists());
        }
        index(&db, &home);
        let query = Command::new(BIN)
            .env("TERN_PROJECTS_HOME", &home)
            .arg("--db")
            .arg(&db)
            .arg("query")
            .output()
            .unwrap();
        let query: Value = serde_json::from_slice(&query.stdout).unwrap();
        assert!(!query["sessions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s["uid"] == format!("{agent}:target")));
        let conn = rusqlite::Connection::open(temp.path().join("library.sqlite")).unwrap();
        assert_eq!(
            conn.query_row("SELECT count(*) FROM chat_state", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
}

#[test]
fn omp_deletion_removes_only_owned_prompt_history_and_fts_entries() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let db = temp.path().join("index.sqlite");
    let target = seed(&home, "omp", "target");
    let neighbor = seed(&home, "omp", "neighbor");
    let neighbor_bytes = std::fs::read(&neighbor).unwrap();
    let history_path = home.join(".omp/agent/history.db");
    let conn = rusqlite::Connection::open(&history_path).unwrap();
    conn.execute_batch(
        "CREATE TABLE history(id INTEGER PRIMARY KEY, prompt TEXT NOT NULL UNIQUE, created_at INTEGER NOT NULL DEFAULT 0, cwd TEXT, session_id TEXT, use_count INTEGER NOT NULL DEFAULT 1);
         CREATE VIRTUAL TABLE history_fts USING fts5(prompt, content='history', content_rowid='id');
         CREATE TRIGGER history_ai AFTER INSERT ON history BEGIN INSERT INTO history_fts(rowid, prompt) VALUES (new.id, new.prompt); END;
         INSERT INTO history(prompt, session_id) VALUES('removedneedle target prompt', 'target'), ('keepneedle neighbor prompt', 'neighbor'), ('legacyneedle unattributed prompt', NULL);
         INSERT INTO history(prompt, session_id, use_count) VALUES('sharedneedle reused prompt', 'neighbor', 2);",
    )
    .unwrap();
    drop(conn);

    index(&db, &home);
    let p = preview(&db, &home, "omp:target");
    assert!(p["blocked_reason"].is_null(), "{p}");
    assert_eq!(library(&db, &home, deletion(&p))["ok"], true);
    assert!(!target.exists());
    assert_eq!(std::fs::read(&neighbor).unwrap(), neighbor_bytes);

    let conn = rusqlite::Connection::open(&history_path).unwrap();
    let remaining: Vec<(String, Option<String>, i64)> = conn
        .prepare("SELECT prompt, session_id, use_count FROM history ORDER BY prompt")
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(
        remaining,
        vec![
            ("keepneedle neighbor prompt".into(), Some("neighbor".into()), 1),
            ("legacyneedle unattributed prompt".into(), None, 1),
            ("sharedneedle reused prompt".into(), Some("neighbor".into()), 2),
        ]
    );
    for (term, expected) in [("removedneedle", 0), ("keepneedle", 1), ("sharedneedle", 1), ("legacyneedle", 1)] {
        let count: i64 = conn.query_row("SELECT count(*) FROM history_fts WHERE history_fts MATCH ?1", [term], |row| row.get(0)).unwrap();
        assert_eq!(count, expected, "history search for {term}");
    }
    conn.execute("INSERT INTO history_fts(history_fts, rank) VALUES('integrity-check', 1)", []).unwrap();
}

#[test]
fn omp_rejects_incompatible_history_fts_before_removing_the_session() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let db = temp.path().join("index.sqlite");
    let target = seed(&home, "omp", "target");
    let original = std::fs::read(&target).unwrap();
    let history_path = home.join(".omp/agent/history.db");
    let conn = rusqlite::Connection::open(&history_path).unwrap();
    conn.execute_batch(
        "CREATE TABLE history(id INTEGER PRIMARY KEY, prompt TEXT NOT NULL, session_id TEXT);
         CREATE VIRTUAL TABLE history_fts USING fts5(prompt);
         CREATE TRIGGER history_ad AFTER DELETE ON history WHEN old.session_id='neighbor' BEGIN DELETE FROM history_fts WHERE rowid=old.id; END;
         INSERT INTO history VALUES(1, 'preserveneedle prompt', 'target');
         INSERT INTO history_fts(rowid, prompt) VALUES(1, 'preserveneedle prompt');",
    )
    .unwrap();
    drop(conn);

    index(&db, &home);
    blocked_or_failure(&db, &home, "omp:target");
    assert_eq!(std::fs::read(&target).unwrap(), original);
    let conn = rusqlite::Connection::open(&history_path).unwrap();
    let count: i64 = conn
        .query_row("SELECT count(*) FROM history WHERE session_id='target'", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 1);
    let matches: i64 = conn
        .query_row("SELECT count(*) FROM history_fts WHERE history_fts MATCH 'preserveneedle'", [], |r| r.get(0))
        .unwrap();
    assert_eq!(matches, 1);
}

#[test]
fn omp_deletion_keeps_fts_consistent_when_history_has_a_delete_trigger() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let db = temp.path().join("index.sqlite");
    let target = seed(&home, "omp", "target");
    let history_path = home.join(".omp/agent/history.db");
    let conn = rusqlite::Connection::open(&history_path).unwrap();
    conn.execute_batch(
        "CREATE TABLE history(id INTEGER PRIMARY KEY, prompt TEXT NOT NULL, session_id TEXT);
         CREATE VIRTUAL TABLE history_fts USING fts5(prompt, content='history', content_rowid='id');
         CREATE TRIGGER history_ai AFTER INSERT ON history BEGIN INSERT INTO history_fts(rowid, prompt) VALUES(new.id, new.prompt); END;
         CREATE TRIGGER history_ad AFTER DELETE ON history BEGIN INSERT INTO history_fts(history_fts, rowid, prompt) VALUES('delete', old.id, old.prompt); END;
         INSERT INTO history VALUES(1, 'removedtriggerword prompt', 'target'), (2, 'kepttriggerword prompt', 'neighbor');",
    )
    .unwrap();
    drop(conn);

    index(&db, &home);
    let p = preview(&db, &home, "omp:target");
    assert!(p["blocked_reason"].is_null(), "{p}");
    assert_eq!(library(&db, &home, deletion(&p))["ok"], true);
    assert!(!target.exists());
    let conn = rusqlite::Connection::open(&history_path).unwrap();
    for (term, expected) in [("removedtriggerword", 0), ("kepttriggerword", 1)] {
        let count: i64 = conn.query_row("SELECT count(*) FROM history_fts WHERE history_fts MATCH ?1", [term], |row| row.get(0)).unwrap();
        assert_eq!(count, expected, "history search for {term}");
    }
    conn.execute("INSERT INTO history_fts(history_fts, rank) VALUES('integrity-check', 1)", []).unwrap();
}

#[test]
fn omp_deletion_repairs_a_previously_desynchronized_history_index() {
    for with_delete_trigger in [false, true] {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("home");
        let db = temp.path().join("index.sqlite");
        let target = seed(&home, "omp", "target");
        let history_path = home.join(".omp/agent/history.db");
        let conn = rusqlite::Connection::open(&history_path).unwrap();
        conn.execute_batch(
            "CREATE TABLE history(id INTEGER PRIMARY KEY, prompt TEXT NOT NULL, session_id TEXT);
             CREATE VIRTUAL TABLE history_fts USING fts5(prompt, content='history', content_rowid='id');
             CREATE TRIGGER history_ai AFTER INSERT ON history BEGIN INSERT INTO history_fts(rowid, prompt) VALUES(new.id, new.prompt); END;
             INSERT INTO history VALUES(1, 'missingneedle prompt', 'target'), (2, 'neighborneedle prompt', 'neighbor');
             INSERT INTO history_fts(history_fts, rowid, prompt) VALUES('delete', 1, 'missingneedle prompt');
             INSERT INTO history_fts(rowid, prompt) VALUES(99, 'orphanneedle prompt');",
        )
        .unwrap();
        if with_delete_trigger {
            conn.execute_batch("CREATE TRIGGER history_ad AFTER DELETE ON history BEGIN INSERT INTO history_fts(history_fts, rowid, prompt) VALUES('delete', old.id, old.prompt); END;").unwrap();
        }
        drop(conn);

        index(&db, &home);
        let p = preview(&db, &home, "omp:target");
        assert!(p["blocked_reason"].is_null(), "{p}");
        assert_eq!(library(&db, &home, deletion(&p))["ok"], true);
        assert!(!target.exists());
        let conn = rusqlite::Connection::open(&history_path).unwrap();
        for (term, expected) in [("missingneedle", 0), ("neighborneedle", 1), ("orphanneedle", 0)] {
            let count: i64 = conn.query_row("SELECT count(*) FROM history_fts WHERE history_fts MATCH ?1", [term], |row| row.get(0)).unwrap();
            assert_eq!(count, expected, "history search for {term}");
        }
        conn.execute("INSERT INTO history_fts(history_fts, rank) VALUES('integrity-check', 1)", []).unwrap();
    }
}
#[test]
fn source_change_wrong_identity_and_symlink_refuse_without_removal() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let db = temp.path().join("index.sqlite");
    let target = seed(&home, "pi", "target");
    index(&db, &home);
    let p = preview(&db, &home, "pi:target");
    std::fs::OpenOptions::new()
        .append(true)
        .open(&target)
        .unwrap()
        .write_all(b"\n")
        .unwrap();
    let bytes = std::fs::read(&target).unwrap();
    let output = call(&db, &home, deletion(&p));
    assert!(!output.status.success());
    assert_eq!(std::fs::read(&target).unwrap(), bytes);
    // Index identity disagrees with a replaced header; a fresh preview cannot authorize it.
    let text = String::from_utf8(bytes)
        .unwrap()
        .replace("\"id\":\"target\"", "\"id\":\"different\"");
    std::fs::write(&target, &text).unwrap();
    blocked_or_failure(&db, &home, "pi:target");
    assert_eq!(std::fs::read_to_string(&target).unwrap(), text);
    #[cfg(unix)]
    {
        let outside = temp.path().join("outside.jsonl");
        let matching = text.replace("\"id\":\"different\"", "\"id\":\"target\"");
        std::fs::write(&target, &matching).unwrap();
        std::fs::rename(&target, &outside).unwrap();
        std::os::unix::fs::symlink(&outside, &target).unwrap();
        index(&db, &home);
        blocked_or_failure(&db, &home, "pi:target");
        assert!(target.is_symlink());
        assert_eq!(std::fs::read_to_string(&outside).unwrap(), matching);
    }
}
#[test]
fn incompatible_optional_history_schema_refuses_entire_omp_delete() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let db = temp.path().join("index.sqlite");
    let target = seed(&home, "omp", "target");
    let bytes = std::fs::read(&target).unwrap();
    let conn = rusqlite::Connection::open(home.join(".omp/agent/history.db")).unwrap();
    conn.execute_batch(
        "CREATE TABLE session_titles(unknown TEXT);INSERT INTO session_titles VALUES('preserve');",
    )
    .unwrap();
    drop(conn);
    index(&db, &home);
    blocked_or_failure(&db, &home, "omp:target");
    assert_eq!(std::fs::read(&target).unwrap(), bytes);
}

fn cursor_user(home: &Path) -> PathBuf {
    #[cfg(target_os = "macos")]
    {
        home.join("Library/Application Support/Cursor/User")
    }
    #[cfg(not(target_os = "macos"))]
    {
        home.join(".config/Cursor/User")
    }
}

fn cursor_fixture(home: &Path, malformed: bool) -> (PathBuf, PathBuf) {
    let user = cursor_user(home);
    let global = user.join("globalStorage/state.vscdb");
    let workspace = user.join("workspaceStorage/fixture/state.vscdb");
    for file in [&global, &workspace] {
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        let conn = rusqlite::Connection::open(file).unwrap();
        conn.execute_batch("CREATE TABLE ItemTable(key TEXT PRIMARY KEY,value TEXT);")
            .unwrap();
        let header = if malformed && file == &workspace {
            json!({"allComposers":"unknown"})
        } else {
            json!({"allComposers":[{"composerId":"target"},{"composerId":"child"},{"composerId":"target-other"}],"selectedComposerIds":["target","target-other"],"lastFocusedComposerIds":["child","target-other"],"unrelated":"preserve"})
        };
        for key in ["composer.composerHeaders", "composer.composerData"] {
            conn.execute(
                "INSERT INTO ItemTable VALUES(?,?)",
                rusqlite::params![key, header.to_string()],
            )
            .unwrap();
        }
        conn.execute("INSERT INTO ItemTable VALUES('unrelated','keep')", [])
            .unwrap();
    }
    let conn = rusqlite::Connection::open(&global).unwrap();
    conn.execute_batch("CREATE TABLE cursorDiskKV(key TEXT PRIMARY KEY,value TEXT);")
        .unwrap();
    for id in ["target", "child", "target-other"] {
        let mut value = json!({"composerId":id,"name":id,"createdAt":1000,"lastUpdatedAt":1001,"conversation":[{"type":1,"text":"Synthetic Cursor deletion fixture"}]});
        if id == "target" {
            value["subComposerIds"] = json!(["child"]);
        }
        if id == "child" {
            value["isBestOfNSubcomposer"] = json!(true);
            value["subagentInfo"] = json!({"parentComposerId":"target"});
        }
        conn.execute(
            "INSERT INTO cursorDiskKV VALUES(?,?)",
            rusqlite::params![format!("composerData:{id}"), value.to_string()],
        )
        .unwrap();
        for prefix in [
            "checkpointId",
            "codeBlockDiff",
            "bubbleId",
            "messageRequestContext",
            "codeBlockPartialInlineDiffFates",
            "ofsContent",
        ] {
            conn.execute(
                "INSERT INTO cursorDiskKV VALUES(?,'owned')",
                [format!("{prefix}:{id}:part")],
            )
            .unwrap();
        }
    }
    conn.execute(
        "INSERT INTO cursorDiskKV VALUES('agentKv:blob:shared','keep')",
        [],
    )
    .unwrap();
    (global, workspace)
}

#[test]
fn cursor_removes_only_owned_keys_and_explicit_descendants() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let db = temp.path().join("index.sqlite");
    let (global, workspace) = cursor_fixture(&home, false);
    let mut databases = vec![global.clone(), workspace.clone()];
    for i in 0..14 {
        let copy = cursor_user(&home).join(format!("workspaceStorage/extra-{i}/state.vscdb"));
        std::fs::create_dir_all(copy.parent().unwrap()).unwrap();
        std::fs::copy(&workspace, &copy).unwrap();
        databases.push(copy);
    }
    index(&db, &home);
    let p = preview(&db, &home, "cursor:target");
    assert!(p["blocked_reason"].is_null(), "{p}");
    assert!(p["dependent_ids"]
        .as_array()
        .unwrap()
        .iter()
        .any(|v| v == "child"));
    library(&db, &home, deletion(&p));
    let conn = rusqlite::Connection::open(&global).unwrap();
    let keys: Vec<String> = conn
        .prepare("SELECT key FROM cursorDiskKV ORDER BY key")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    assert!(keys.contains(&"composerData:target-other".to_owned()));
    assert!(keys.contains(&"agentKv:blob:shared".to_owned()));
    assert!(!keys.iter().any(|key| key == "composerData:target"
        || key == "composerData:child"
        || key.contains(":target:")
        || key.contains(":child:")));
    for file in databases {
        let conn = rusqlite::Connection::open(file).unwrap();
        for key in ["composer.composerHeaders", "composer.composerData"] {
            let raw: String = conn
                .query_row("SELECT value FROM ItemTable WHERE key=?", [key], |r| {
                    r.get(0)
                })
                .unwrap();
            let header: Value = serde_json::from_str(&raw).unwrap();
            assert_eq!(
                header["allComposers"],
                json!([{"composerId":"target-other"}])
            );
            assert_eq!(header["selectedComposerIds"], json!(["target-other"]));
            assert_eq!(header["lastFocusedComposerIds"], json!(["target-other"]));
            assert_eq!(header["unrelated"], "preserve");
        }
        assert_eq!(
            conn.query_row(
                "SELECT value FROM ItemTable WHERE key='unrelated'",
                [],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
            "keep"
        );
    }
}

#[test]
fn cursor_unknown_header_prevents_all_database_mutation() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let db = temp.path().join("index.sqlite");
    let (global, workspace) = cursor_fixture(&home, true);
    index(&db, &home);
    let before_global = std::fs::read(&global).unwrap();
    let before_workspace = std::fs::read(&workspace).unwrap();
    blocked_or_failure(&db, &home, "cursor:target");
    assert_eq!(std::fs::read(global).unwrap(), before_global);
    assert_eq!(std::fs::read(workspace).unwrap(), before_workspace);
}

#[test]
fn cursor_child_delete_repairs_surviving_parent() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let db = temp.path().join("index.sqlite");
    let (global, _) = cursor_fixture(&home, false);
    let conn = rusqlite::Connection::open(&global).unwrap();
    let raw: String = conn
        .query_row(
            "SELECT value FROM cursorDiskKV WHERE key='composerData:target'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let mut parent: Value = serde_json::from_str(&raw).unwrap();
    parent["selectedSubComposerId"] = json!("child");
    conn.execute(
        "UPDATE cursorDiskKV SET value=? WHERE key='composerData:target'",
        [parent.to_string()],
    )
    .unwrap();
    drop(conn);
    index(&db, &home);
    let p = preview(&db, &home, "cursor:child");
    assert!(p["blocked_reason"].is_null(), "{p}");
    library(&db, &home, deletion(&p));
    let conn = rusqlite::Connection::open(global).unwrap();
    let raw: String = conn
        .query_row(
            "SELECT value FROM cursorDiskKV WHERE key='composerData:target'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let parent: Value = serde_json::from_str(&raw).unwrap();
    assert_eq!(parent["subComposerIds"], json!([]));
    assert_eq!(parent["selectedSubComposerId"], "target");
}

#[test]
fn claude_deletes_exact_session_history_without_touching_neighbors() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let db = temp.path().join("index.sqlite");
    let root = home.join(".claude");
    let project = root.join("projects/fixture");
    std::fs::create_dir_all(&project).unwrap();
    let target = project.join("target.jsonl");
    let neighbor = project.join("neighbor.jsonl");
    for (id, file) in [("target", &target), ("neighbor", &neighbor)] {
        std::fs::write(file,format!("{}\n",json!({"type":"user","sessionId":id,"cwd":home,"timestamp":"2024-06-01T12:00:00Z","message":{"role":"user","content":"Synthetic Claude fixture"}}))).unwrap();
    }
    let neighbor_bytes = std::fs::read(&neighbor).unwrap();
    let history = root.join("history.jsonl");
    let preserved = " {\"sessionId\":\"neighbor\",\"display\":\"preserve byte spacing\"}\r\n";
    std::fs::write(
        &history,
        format!("{{\"sessionId\":\"target\",\"display\":\"delete\"}}\n{preserved}"),
    )
    .unwrap();
    for path in [
        project.join("target"),
        root.join("file-history/target"),
        root.join("session-env/target"),
        root.join("image-cache/target"),
        root.join("uploads/target"),
    ] {
        std::fs::create_dir_all(&path).unwrap();
        std::fs::write(path.join("artifact"), "owned").unwrap();
    }
    std::fs::create_dir_all(root.join("debug")).unwrap();
    std::fs::write(root.join("debug/target.txt"), "owned").unwrap();
    let tasks = root.join("tasks/target");
    std::fs::create_dir_all(&tasks).unwrap();
    std::fs::write(tasks.join("keep"), "shared task list").unwrap();
    index(&db, &home);
    let p = preview(&db, &home, "claude:target");
    assert!(p["blocked_reason"].is_null(), "{p}");
    library(&db, &home, deletion(&p));
    assert!(!target.exists());
    assert!(!project.join("target").exists());
    assert!(!root.join("debug/target.txt").exists());
    assert_eq!(std::fs::read(&neighbor).unwrap(), neighbor_bytes);
    assert_eq!(std::fs::read_to_string(history).unwrap(), preserved);
    assert_eq!(
        std::fs::read_to_string(tasks.join("keep")).unwrap(),
        "shared task list"
    );
}

#[cfg(unix)]
#[test]
fn active_process_with_matching_store_refuses_direct_deletion() {
    use std::os::unix::process::CommandExt;
    struct Process(std::process::Child);
    impl Drop for Process {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let db = temp.path().join("index.sqlite");
    let source = seed(&home, "pi", "target");
    let bytes = std::fs::read(&source).unwrap();
    index(&db, &home);
    // Preserve the system executable's signature on macOS. Copying /bin/sleep to a
    // different name can be killed by code-signing enforcement between the two checks.
    // The harness identity is in argv[0], as it is for a script-based CLI.
    let mut process = Process(
        Command::new("/bin/sleep")
            .arg0(temp.path().join("pi"))
            .arg("30")
            .env("HOME", &home)
            .env("PI_CODING_AGENT_DIR", home.join(".pi/agent"))
            .spawn()
            .unwrap(),
    );
    let p = preview(&db, &home, "pi:target");
    assert!(
        p["blocked_reason"]
            .as_str()
            .is_some_and(|reason| reason.contains("running")
                || reason.contains("Stop")
                || reason.contains("stop")),
        "{p}"
    );
    assert!(
        process.0.try_wait().unwrap().is_none(),
        "fixture process exited before deletion"
    );
    assert!(!call(&db, &home, deletion(&p)).status.success());
    assert_eq!(std::fs::read(source).unwrap(), bytes);
}
