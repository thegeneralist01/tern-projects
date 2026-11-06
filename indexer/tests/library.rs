use serde_json::{json, Value};
use std::{
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
};
const BIN: &str = env!("CARGO_BIN_EXE_tern-projects");

fn command(db: &Path, home: &Path, args: &[&str], request: Option<&Value>) -> Output {
    let mut cmd = Command::new(BIN);
    cmd.env("TERN_PROJECTS_HOME", home)
        .arg("--db")
        .arg(db)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if request.is_some() {
        cmd.stdin(Stdio::piped());
    }
    let mut child = cmd.spawn().unwrap();
    if let Some(request) = request {
        child
            .stdin
            .take()
            .unwrap()
            .write_all(request.to_string().as_bytes())
            .unwrap();
    }
    child.wait_with_output().unwrap()
}
fn run(db: &Path, home: &Path, args: &[&str]) -> Value {
    let out = command(db, home, args, None);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).unwrap()
}
fn library(db: &Path, home: &Path, request: Value) -> Value {
    let out = command(db, home, &["library"], Some(&request));
    assert!(
        out.status.success(),
        "request {request}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).unwrap()
}
fn fails(db: &Path, home: &Path, request: Value) -> String {
    let out = command(db, home, &["library"], Some(&request));
    assert!(
        !out.status.success(),
        "unexpected success: {}",
        String::from_utf8_lossy(&out.stdout)
    );
    String::from_utf8(out.stderr).unwrap()
}
fn seed(home: &Path, id: &str, cwd: &Path, updated: i64) -> PathBuf {
    let dir = home.join(".omp/agent/sessions/fixture");
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join(format!("{id}.jsonl"));
    let records = [
        json!({"type":"session","id":id,"cwd":cwd,"timestamp":updated-1}),
        json!({"type":"message","timestamp":updated,"message":{"role":"user","content":[{"type":"text","text":"shared searchable needle"}]}}),
    ];
    std::fs::write(
        &file,
        records
            .iter()
            .map(Value::to_string)
            .collect::<Vec<_>>()
            .join("\n")
            + "\n",
    )
    .unwrap();
    file
}
fn snapshot(db: &Path, home: &Path) -> Value {
    library(
        db,
        home,
        json!({"op":"snapshot","view":"mine","per":25,"recent":15}),
    )
}
fn project<'a>(snapshot: &'a Value, id: &str) -> &'a Value {
    snapshot["projects"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["id"] == id)
        .unwrap()
}
fn create(db: &Path, home: &Path, name: &str, folders: Value) -> String {
    library(
        db,
        home,
        json!({"op":"project-save","name":name,"icon":"folder","folders":folders}),
    )["project"]["id"]
        .as_str()
        .unwrap()
        .to_owned()
}
fn uids(value: &Value) -> Vec<&str> {
    value
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["uid"].as_str().unwrap())
        .collect()
}

#[test]
fn membership_overrides_pins_archive_and_rebuild_are_consistent() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let db = temp.path().join("cache/index.sqlite");
    let app = home.join("app");
    let sub = app.join("sub");
    let other = home.join("app-old");
    let backend = home.join("backend");
    let empty = home.join("empty");
    for path in [&sub, &other, &backend, &empty] {
        std::fs::create_dir_all(path).unwrap();
    }
    let source = seed(&home, "app", &app, 100);
    seed(&home, "sub", &sub, 200);
    seed(&home, "other", &other, 300);
    seed(&home, "back", &backend, 400);
    let original = std::fs::read(&source).unwrap();
    run(&db, &home, &["index"]);
    let a = create(&db, &home, "A", json!([app, app.join("."), backend]));
    let b = create(&db, &home, "B", json!([sub]));
    let c = create(&db, &home, "C", json!([empty]));
    let snap = snapshot(&db, &home);
    let folders = project(&snap, &a)["folders"].as_array().unwrap();
    assert!(
        folders.iter()
            .map(|folder| std::fs::canonicalize(folder.as_str().unwrap()).unwrap())
            .eq([app.canonicalize().unwrap(), backend.canonicalize().unwrap()]),
        "project folders must resolve to app, then backend: {folders:?}"
    );
    assert_eq!(
        uids(&project(&snap, &a)["sessions"]),
        ["omp:back", "omp:sub", "omp:app"]
    );
    assert_eq!(uids(&project(&snap, &b)["sessions"]), ["omp:sub"]);
    assert_eq!(project(&snap, &c)["count"], 0);
    let before = run(&db, &home, &["query", "--q", "needle"])["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["uid"] == "omp:sub")
        .unwrap()
        .clone();
    library(
        &db,
        &home,
        json!({"op":"chat-update","uid":"omp:sub","assignment":{"mode":"project","project_id":c}}),
    );
    let snap = snapshot(&db, &home);
    assert_eq!(project(&snap, &a)["count"], 2);
    assert_eq!(project(&snap, &b)["count"], 0);
    let moved = &project(&snap, &c)["sessions"][0];
    assert_eq!(moved["cwd"], before["cwd"]);
    assert_eq!(moved["resume"], before["resume"]);
    let filtered = run(&db, &home, &["query", "--project-id", &c, "--q", "needle"]);
    assert_eq!(filtered["total"], 1);
    library(
        &db,
        &home,
        json!({"op":"chat-update","uid":"omp:sub","assignment":{"mode":"none"}}),
    );
    let snap = snapshot(&db, &home);
    assert_eq!(project(&snap, &c)["count"], 0);
    assert!(uids(&snap["recent"]).contains(&"omp:sub"));
    library(
        &db,
        &home,
        json!({"op":"chat-update","uid":"omp:sub","assignment":{"mode":"automatic"}}),
    );
    library(
        &db,
        &home,
        json!({"op":"chat-update","uid":"omp:app","pinned":true}),
    );
    library(
        &db,
        &home,
        json!({"op":"settings-update","default_harness":"omp","icon_style":"monochrome"}),
    );
    let snap = snapshot(&db, &home);
    assert_eq!(uids(&snap["pinned"]), ["omp:app"]);
    assert!(uids(&snap["recent"]).contains(&"omp:app"));
    library(
        &db,
        &home,
        json!({"op":"chat-update","uid":"omp:app","archived":true}),
    );
    let snap = snapshot(&db, &home);
    assert!(uids(&snap["pinned"]).is_empty());
    assert!(!uids(&snap["recent"]).contains(&"omp:app"));
    assert_eq!(project(&snap, &a)["count"], 2);
    assert_eq!(run(&db, &home, &["query", "--q", "needle"])["total"], 3);
    assert_eq!(
        uids(&library(&db, &home, json!({"op":"archived","limit":50,"offset":0}))["sessions"]),
        ["omp:app"]
    );
    assert_eq!(std::fs::read(&source).unwrap(), original);
    // Destroy only the cache, then rebuild. Durable annotations must remain.
    std::fs::remove_file(&db).unwrap();
    run(&db, &home, &["index"]);
    let snap = snapshot(&db, &home);
    assert_eq!(snap["settings"]["default_harness"], "omp");
    assert_eq!(snap["settings"]["icon_style"], "monochrome");
    assert_eq!(project(&snap, &a)["count"], 2);
    library(
        &db,
        &home,
        json!({"op":"chat-update","uid":"omp:app","archived":false}),
    );
    assert_eq!(uids(&snapshot(&db, &home)["pinned"]), ["omp:app"]);
    let error = fails(
        &db,
        &home,
        json!({"op":"project-save","id":a,"name":" ","icon":"folder","folders":[app]}),
    );
    assert!(error.contains("Name"));
    assert_eq!(project(&snapshot(&db, &home), &a)["name"], "A");
    library(
        &db,
        &home,
        json!({"op":"chat-update","uid":"omp:sub","assignment":{"mode":"project","project_id":c}}),
    );
    library(&db, &home, json!({"op":"project-remove","id":c}));
    assert!(source.exists());
    let after_removal = snapshot(&db, &home);
    assert_eq!(project(&after_removal, &a)["count"], 3);
    assert_eq!(project(&after_removal, &b)["count"], 1);
    library(
        &db,
        &home,
        json!({"op":"settings-update","default_harness":null}),
    );
    assert!(snapshot(&db, &home)["settings"]["default_harness"].is_null());
}

#[test]
fn curated_filter_precedes_fts_count_and_paging() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let db = temp.path().join("index.sqlite");
    let app = home.join("app");
    let other = home.join("other");
    std::fs::create_dir_all(&app).unwrap();
    std::fs::create_dir_all(&other).unwrap();
    for i in 0..115 {
        seed(
            &home,
            &format!("s{i:03}"),
            if i < 60 { &other } else { &app },
            1000 - i,
        );
    }
    run(&db, &home, &["index"]);
    let project = create(&db, &home, "Paged", json!([app]));
    for text in ["", "needle"] {
        let first = run(
            &db,
            &home,
            &[
                "query",
                "--project-id",
                &project,
                "--q",
                text,
                "--limit",
                "50",
            ],
        );
        let second = run(
            &db,
            &home,
            &[
                "query",
                "--project-id",
                &project,
                "--q",
                text,
                "--limit",
                "50",
                "--offset",
                "50",
            ],
        );
        assert_eq!(first["total"], 55);
        assert_eq!(second["total"], 55);
        let first = uids(&first["sessions"]);
        let second = uids(&second["sessions"]);
        assert_eq!(first.first(), Some(&"omp:s060"));
        assert_eq!(first.last(), Some(&"omp:s109"));
        assert_eq!(
            second,
            ["omp:s110", "omp:s111", "omp:s112", "omp:s113", "omp:s114"]
        );
    }
}

#[test]
fn library_schema_and_folder_validation_fail_without_overwrite() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    let db = temp.path().join("index.sqlite");
    run(&db, &home, &["index"]);
    let error = fails(
        &db,
        &home,
        json!({"op":"project-save","name":"Invalid","icon":"folder","folders":[home.join("missing")]}),
    );
    assert!(error.contains("directory"));
    assert_eq!(snapshot(&db, &home)["projects"], json!([]));
    let lib = temp.path().join("library.sqlite");
    let conn = rusqlite::Connection::open(&lib).unwrap();
    conn.pragma_update(None, "user_version", 99).unwrap();
    drop(conn);
    assert!(fails(
        &db,
        &home,
        json!({"op":"snapshot","view":"mine","per":25,"recent":15})
    )
    .contains("version 99"));
    let conn = rusqlite::Connection::open(lib).unwrap();
    assert_eq!(
        conn.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        99
    );
}

#[test]
fn migration_includes_live_wal_and_refuses_existing_destinations() {
    let temp = tempfile::tempdir().unwrap();
    let old = temp.path().join("legacy");
    let home = temp.path().join("home");
    std::fs::create_dir_all(&old).unwrap();
    let source = old.join("index.sqlite");
    let conn = rusqlite::Connection::open(&source).unwrap();
    conn.execute_batch("PRAGMA journal_mode=WAL;PRAGMA wal_autocheckpoint=0;CREATE TABLE proof(value TEXT);INSERT INTO proof VALUES('in WAL');").unwrap();
    let db = temp.path().join("new/index.sqlite");
    run(
        &db,
        &home,
        &["migrate-data", "--from", old.to_str().unwrap()],
    );
    let dest = rusqlite::Connection::open(&db).unwrap();
    assert_eq!(
        dest.query_row("SELECT value FROM proof", [], |r| r.get::<_, String>(0))
            .unwrap(),
        "in WAL"
    );
    let out = command(
        &db,
        &home,
        &["migrate-data", "--from", old.to_str().unwrap()],
        None,
    );
    assert!(!out.status.success());
    assert_eq!(
        dest.query_row("SELECT value FROM proof", [], |r| r.get::<_, String>(0))
            .unwrap(),
        "in WAL"
    );
    assert_eq!(
        conn.query_row("SELECT value FROM proof", [], |r| r.get::<_, String>(0))
            .unwrap(),
        "in WAL"
    );
}

#[test]
fn project_folder_moves_preserve_chats_search_and_explicit_choices() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let db = temp.path().join("index.sqlite");
    let old = home.join("original");
    let sub = old.join("sub");
    let moved = home.join("renamed");
    let other = home.join("other");
    for folder in [&sub, &other] {
        std::fs::create_dir_all(folder).unwrap();
    }
    #[cfg(unix)]
    let transcript_cwd = {
        let alias = temp.path().join("home-alias");
        std::os::unix::fs::symlink(&home, &alias).unwrap();
        alias.join("original")
    };
    #[cfg(not(unix))]
    let transcript_cwd = old.clone();
    let source = seed(&home, "original", &transcript_cwd, 100);
    let original_bytes = std::fs::read(&source).unwrap();
    seed(&home, "child", &sub, 200);
    seed(&home, "archived", &old, 300);
    seed(&home, "excluded", &old, 400);
    seed(&home, "assigned-away", &old, 500);
    run(&db, &home, &["index"]);
    let a = create(&db, &home, "Original", json!([old]));
    let b = create(&db, &home, "Overlapping", json!([sub]));
    let c = create(&db, &home, "Other", json!([other]));
    for (uid, patch) in [
        ("omp:original", json!({"pinned":true})),
        ("omp:archived", json!({"archived":true})),
        ("omp:excluded", json!({"assignment":{"mode":"none"}})),
        ("omp:assigned-away", json!({"assignment":{"mode":"project","project_id":c}})),
    ] {
        let mut request = patch;
        request["op"] = json!("chat-update");
        request["uid"] = json!(uid);
        library(&db, &home, request);
    }
    library(&db, &home, json!({
        "op":"project-save","id":a,"name":"New label","icon":"code","folders":[old]
    }));
    assert_eq!(uids(&project(&snapshot(&db, &home), &a)["sessions"]), ["omp:child", "omp:original"]);
    std::fs::rename(&old, &moved).unwrap();
    // Edit both overlapping projects: retention must not turn automatic membership
    // into a single-project override.
    for (id, folders) in [(&a, json!([moved])), (&b, json!([moved.join("sub")]))] {
        library(&db, &home, json!({
            "op":"project-save","id":id,"name":"Moved","icon":"folder","folders":folders
        }));
    }
    // Reusing the old path must not attach unrelated new chats to the moved project.
    std::fs::create_dir_all(&old).unwrap();
    seed(&home, "reused-old-path", &old, 600);
    seed(&home, "new-location", &moved, 700);
    run(&db, &home, &["index"]);
    for rebuild in [false, true] {
        if rebuild {
            let conn = rusqlite::Connection::open(&db).unwrap();
            conn.pragma_update(None, "user_version", 99).unwrap();
            drop(conn);
            run(&db, &home, &["index"]);
        }
        let snap = snapshot(&db, &home);
        assert_eq!(uids(&project(&snap, &a)["sessions"]), ["omp:new-location", "omp:child", "omp:original"]);
        assert_eq!(uids(&project(&snap, &b)["sessions"]), ["omp:child"]);
        assert_eq!(uids(&project(&snap, &c)["sessions"]), ["omp:assigned-away"]);
        assert_eq!(uids(&snap["pinned"]), ["omp:original"]);
        for text in ["", "needle"] {
            let result = run(&db, &home, &["query", "--project-id", &a, "--q", text, "--limit", "2"]);
            assert_eq!(result["total"], 3);
            assert_eq!(uids(&result["sessions"]), ["omp:new-location", "omp:child"]);
            let tail = run(&db, &home, &["query", "--project-id", &a, "--q", text, "--limit", "2", "--offset", "2"]);
            assert_eq!(uids(&tail["sessions"]), ["omp:original"]);
        }
    }
    library(&db, &home, json!({"op":"chat-update","uid":"omp:archived","archived":false}));
    assert_eq!(project(&snapshot(&db, &home), &a)["count"], 4);
    library(&db, &home, json!({"op":"chat-update","uid":"omp:original","assignment":{"mode":"none"}}));
    assert_eq!(project(&snapshot(&db, &home), &a)["count"], 3);
    library(&db, &home, json!({"op":"chat-update","uid":"omp:original","assignment":{"mode":"automatic"}}));
    assert_eq!(project(&snapshot(&db, &home), &a)["count"], 4);
    assert_eq!(std::fs::read(source).unwrap(), original_bytes);
    library(&db, &home, json!({"op":"project-remove","id":a}));
    let conn = rusqlite::Connection::open(temp.path().join("library.sqlite")).unwrap();
    assert_eq!(conn.query_row("SELECT count(*) FROM project_chats WHERE project_id=?",
        [&a], |r| r.get::<_, i64>(0)).unwrap(), 0);
    assert_eq!(uids(&project(&snapshot(&db, &home), &b)["sessions"]), ["omp:child"]);
}

#[test]
fn legacy_library_migration_preserves_projects_annotations_and_settings() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let folder = home.join("project");
    std::fs::create_dir_all(&folder).unwrap();
    let db = temp.path().join("index.sqlite");
    seed(&home, "legacy", &folder, 100);
    run(&db, &home, &["index"]);
    // An actual v1 library, not a re-labeled v2 database.
    let conn = rusqlite::Connection::open(temp.path().join("library.sqlite")).unwrap();
    conn.execute_batch("
        CREATE TABLE projects(id INTEGER PRIMARY KEY AUTOINCREMENT,name TEXT NOT NULL,icon TEXT NOT NULL,created_ms INTEGER NOT NULL);
        CREATE TABLE project_folders(project_id INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,position INTEGER NOT NULL,path TEXT NOT NULL,PRIMARY KEY(project_id,position),UNIQUE(project_id,path));
        CREATE TABLE chat_state(uid TEXT PRIMARY KEY,assignment TEXT NOT NULL DEFAULT 'automatic',project_id INTEGER REFERENCES projects(id) ON DELETE SET NULL,pinned_ms INTEGER,archived_ms INTEGER);
        CREATE TABLE settings(key TEXT PRIMARY KEY,value TEXT NOT NULL);
        INSERT INTO projects VALUES(1,'Legacy','code',100);
        INSERT INTO chat_state VALUES('omp:legacy','automatic',NULL,100,NULL);
        INSERT INTO settings VALUES('default_harness','\"omp\"');
        PRAGMA user_version=1;").unwrap();
    conn.execute("INSERT INTO project_folders VALUES(1,0,?)", [folder.to_str().unwrap()]).unwrap();
    drop(conn);
    let snap = snapshot(&db, &home);
    assert_eq!(project(&snap, "1")["name"], "Legacy");
    assert_eq!(uids(&snap["pinned"]), ["omp:legacy"]);
    assert_eq!(snap["settings"]["default_harness"], "omp");
    let moved = home.join("moved");
    std::fs::rename(&folder, &moved).unwrap();
    library(&db, &home, json!({"op":"project-save","id":"1","name":"Moved","icon":"code","folders":[moved]}));
    assert_eq!(uids(&project(&snapshot(&db, &home), "1")["sessions"]), ["omp:legacy"]);
}

#[test]
fn malformed_legacy_library_is_not_migrated_or_overwritten() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    let db = temp.path().join("index.sqlite");
    let conn = rusqlite::Connection::open(temp.path().join("library.sqlite")).unwrap();
    conn.execute_batch("CREATE TABLE projects(id INTEGER PRIMARY KEY,name TEXT);
        INSERT INTO projects VALUES(1,'Keep me'); PRAGMA user_version=1;").unwrap();
    assert!(fails(&db, &home, json!({"op":"snapshot","view":"mine","per":25,"recent":15}))
        .contains("malformed library schema"));
    assert_eq!(conn.query_row("SELECT name FROM projects", [], |r| r.get::<_, String>(0)).unwrap(), "Keep me");
    assert_eq!(conn.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0)).unwrap(), 1);
    assert_eq!(conn.query_row(
        "SELECT count(*) FROM sqlite_master WHERE name='project_chats'", [],
        |r| r.get::<_, i64>(0)).unwrap(), 0);
}

#[test]
fn failed_folder_change_rolls_back_retained_membership() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let old = home.join("old");
    let new = home.join("new");
    for folder in [&old, &new] {
        std::fs::create_dir_all(folder).unwrap();
    }
    let db = temp.path().join("index.sqlite");
    seed(&home, "original", &old, 100);
    run(&db, &home, &["index"]);
    let id = create(&db, &home, "Original", json!([old]));
    let conn = rusqlite::Connection::open(temp.path().join("library.sqlite")).unwrap();
    conn.execute_batch("CREATE TRIGGER reject_folder BEFORE INSERT ON project_folders
        BEGIN SELECT RAISE(ABORT,'fixture rejection'); END;").unwrap();
    assert!(fails(&db, &home, json!({
        "op":"project-save","id":id,"name":"Changed","icon":"code","folders":[new]
    })).contains("fixture rejection"));
    let snap = snapshot(&db, &home);
    assert_eq!(project(&snap, &id)["name"], "Original");
    assert_eq!(uids(&project(&snap, &id)["sessions"]), ["omp:original"]);
    assert_eq!(conn.query_row("SELECT count(*) FROM project_chats", [], |r| r.get::<_, i64>(0)).unwrap(), 0);
}
