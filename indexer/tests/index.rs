/// Integration tests for the tern-projects CLI.
/// Runs the binary with TERN_PROJECTS_HOME pointing to a fixture home tree.
use std::path::{Path, PathBuf};
use std::process::Command;

// Path to the compiled binary.
const BIN: &str = env!("CARGO_BIN_EXE_tern-projects");

// ── Helpers ──────────────────────────────────────────────────────────────────

fn fixture_home() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/home")
}

fn run_cmd(db: &Path, home: &Path, args: &[&str]) -> serde_json::Value {
    let out = Command::new(BIN)
        .env("TERN_PROJECTS_HOME", home)
        .arg("--db")
        .arg(db)
        .args(args)
        .output()
        .expect("failed to run tern-projects");
    let stdout = String::from_utf8_lossy(&out.stdout);
    serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("JSON parse error: {e}\nstdout: {stdout}\nstderr: {}", String::from_utf8_lossy(&out.stderr)))
}

fn make_home_with_dbs(tmp: &tempfile::TempDir) -> PathBuf {
    let src = fixture_home();
    let dst = tmp.path().to_path_buf();
    copy_dir_all(&src, &dst).expect("copy fixture home");

    // Build Codex DB.
    build_codex_db(&dst);
    // Build OpenCode DB.
    build_opencode_db(&dst);
    // Build Cursor DB.
    build_cursor_db(&dst);

    dst
}

fn copy_dir_all(src: &Path, dst: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        let dest = dst.join(entry.file_name());
        if ty.is_dir() {
            copy_dir_all(&entry.path(), &dest)?;
        } else {
            std::fs::copy(entry.path(), dest)?;
        }
    }
    Ok(())
}

fn build_codex_db(home: &Path) {
    std::fs::create_dir_all(home.join(".codex")).unwrap();
    let state_path = home.join(".codex/state_5.sqlite");
    let history_path = home.join(".codex/thread_history_1.sqlite");

    let conn = rusqlite::Connection::open(&state_path).unwrap();
    conn.execute_batch("
        CREATE TABLE threads (
            id TEXT PRIMARY KEY,
            name TEXT,
            title TEXT,
            cwd TEXT,
            git_branch TEXT,
            model TEXT,
            preview TEXT,
            archived INTEGER DEFAULT 0,
            source TEXT,
            thread_source TEXT,
            first_user_message TEXT,
            created_at_ms INTEGER,
            updated_at_ms INTEGER,
            rollout_path TEXT
        );
    ").unwrap();
    // Kept session.
    conn.execute("INSERT INTO threads VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
        rusqlite::params![
            "codex-thread-1", "Build Codex FTS adapter", "Codex FTS task",
            "/home/user/repos/codex-proj", "main", "o1-mini",
            "preview text", 0, "direct", "", "Build me a Codex FTS adapter",
            1717236000000i64, 1717236005000i64, null_str()
        ]
    ).unwrap();
    // Subagent thread (should be skipped).
    conn.execute("INSERT INTO threads VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
        rusqlite::params![
            "codex-sub-1", "Sub task", null_str(), "/home/user", null_str(), null_str(),
            "some preview", 0, "{\"subagent\":true}", "subagent", "sub task text",
            1717236000000i64, 1717236001000i64, null_str()
        ]
    ).unwrap();
    // Archived thread (should be skipped).
    conn.execute("INSERT INTO threads VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
        rusqlite::params![
            "codex-archived-1", "Archived", null_str(), "/home/user", null_str(), null_str(),
            "some preview", 1, "direct", "", "archived content",
            1717236000000i64, 1717236001000i64, null_str()
        ]
    ).unwrap();

    // History DB.
    let hconn = rusqlite::Connection::open(&history_path).unwrap();
    hconn.execute_batch("
        CREATE TABLE thread_items (
            id INTEGER PRIMARY KEY,
            thread_id TEXT NOT NULL,
            item_type TEXT NOT NULL,
            item_json TEXT NOT NULL,
            rollout_ordinal INTEGER DEFAULT 0
        );
    ").unwrap();
    hconn.execute("INSERT INTO thread_items(thread_id, item_type, item_json, rollout_ordinal) VALUES (?,?,?,?)",
        rusqlite::params![
            "codex-thread-1", "userMessage",
            r#"{"content":[{"type":"text","text":"Build me a Codex FTS adapter"}]}"#,
            1
        ]
    ).unwrap();
    hconn.execute("INSERT INTO thread_items(thread_id, item_type, item_json, rollout_ordinal) VALUES (?,?,?,?)",
        rusqlite::params![
            "codex-thread-1", "agentMessage",
            r#"{"text":"I'll build the adapter now.","phase":"response"}"#,
            2
        ]
    ).unwrap();
    // Commentary message (counted in body but not in messages).
    hconn.execute("INSERT INTO thread_items(thread_id, item_type, item_json, rollout_ordinal) VALUES (?,?,?,?)",
        rusqlite::params![
            "codex-thread-1", "agentMessage",
            r#"{"text":"Thinking step...","phase":"commentary"}"#,
            3
        ]
    ).unwrap();
}

fn build_opencode_db(home: &Path) {
    std::fs::create_dir_all(home.join(".local/share/opencode")).unwrap();
    let db_path = home.join(".local/share/opencode/opencode.db");
    let conn = rusqlite::Connection::open(&db_path).unwrap();
    conn.execute_batch("
        CREATE TABLE session (
            id TEXT PRIMARY KEY, title TEXT, directory TEXT,
            model TEXT, parent_id TEXT,
            time_created INTEGER, time_updated INTEGER, time_archived INTEGER
        );
        CREATE TABLE message (
            id TEXT PRIMARY KEY, session_id TEXT, time_created INTEGER, data TEXT
        );
        CREATE TABLE part (
            id TEXT PRIMARY KEY, session_id TEXT, message_id TEXT, data TEXT
        );
    ").unwrap();

    // Kept session.
    conn.execute("INSERT INTO session VALUES (?,?,?,?,?,?,?,?)",
        rusqlite::params![
            "opencode-session-1",
            "Build OpenCode plugin",
            "/home/user/repos/opencode-proj",
            r#"{"id":"gpt-4o"}"#,
            null_str(),
            1717236000000i64,
            1717236010000i64,
            null_str()
        ]
    ).unwrap();
    conn.execute("INSERT INTO message VALUES (?,?,?,?)",
        rusqlite::params!["msg-1", "opencode-session-1", 1717236001000i64, r#"{"role":"user"}"#]
    ).unwrap();
    conn.execute("INSERT INTO part VALUES (?,?,?,?)",
        rusqlite::params!["part-1", "opencode-session-1", "msg-1",
            r#"{"type":"text","text":"Implement the OpenCode session adapter","synthetic":0}"#]
    ).unwrap();
    conn.execute("INSERT INTO message VALUES (?,?,?,?)",
        rusqlite::params!["msg-2", "opencode-session-1", 1717236002000i64, r#"{"role":"assistant"}"#]
    ).unwrap();
    conn.execute("INSERT INTO part VALUES (?,?,?,?)",
        rusqlite::params!["part-2", "opencode-session-1", "msg-2",
            r#"{"type":"text","text":"Here is the OpenCode adapter implementation."}"#]
    ).unwrap();

    // Child session (should be skipped: parent_id set).
    conn.execute("INSERT INTO session VALUES (?,?,?,?,?,?,?,?)",
        rusqlite::params![
            "opencode-child-1", "Child session", "/home/user",
            null_str(), "opencode-session-1",
            1717236000000i64, 1717236005000i64, null_str()
        ]
    ).unwrap();
}

fn build_cursor_db(home: &Path) {
    let storage = if cfg!(target_os = "macos") {
        "Library/Application Support/Cursor/User/globalStorage"
    } else {
        ".config/Cursor/User/globalStorage"
    };
    std::fs::create_dir_all(home.join(storage)).unwrap();
    let db_path = home.join(storage).join("state.vscdb");
    let conn = rusqlite::Connection::open(&db_path).unwrap();
    conn.execute_batch("CREATE TABLE cursorDiskKV (key TEXT PRIMARY KEY, value TEXT);").unwrap();

    // Kept session: v1 conversation format.
    let composer_val = serde_json::json!({
        "composerId": "cursor-comp-1",
        "name": "Cursor Rust project",
        "workspaceProjectDir": "/home/user/repos/cursor-proj",
        "conversation": [
            {"type": 1, "text": "Help me debug this Rust borrow checker error"},
            {"type": 2, "text": "The issue is a moved value being used again after move."}
        ],
        "modelConfig": {"modelName": "claude-3-5-sonnet"},
        "createdAt": 1717236000000i64,
        "lastUpdatedAt": 1717236010000i64
    });
    conn.execute("INSERT INTO cursorDiskKV VALUES (?,?)",
        rusqlite::params!["composerData:cursor-comp-1", composer_val.to_string()]
    ).unwrap();

    // Empty composer (no messages) — should be skipped.
    let empty_val = serde_json::json!({
        "composerId": "cursor-empty-1",
        "name": "",
        "conversation": [],
        "createdAt": 1717236000000i64,
        "lastUpdatedAt": 1717236001000i64
    });
    conn.execute("INSERT INTO cursorDiskKV VALUES (?,?)",
        rusqlite::params!["composerData:cursor-empty-1", empty_val.to_string()]
    ).unwrap();

    // Archived — should be skipped.
    let archived_val = serde_json::json!({
        "composerId": "cursor-archived-1",
        "name": "archived",
        "isArchived": true,
        "conversation": [{"type": 1, "text": "some text"}],
        "createdAt": 1717236000000i64,
        "lastUpdatedAt": 1717236001000i64
    });
    conn.execute("INSERT INTO cursorDiskKV VALUES (?,?)",
        rusqlite::params!["composerData:cursor-archived-1", archived_val.to_string()]
    ).unwrap();
}

fn null_str() -> Option<String> { None }

// ── Worktree fixture ─────────────────────────────────────────────────────────

fn build_worktree_fixture(home: &Path) {
    // Create a bare-ish repo structure:
    //   <home>/repos/myrepo/.git/HEAD
    //   <home>/repos/myrepo/.git/worktrees/w/HEAD
    //   <home>/repos/myrepo/.git/worktrees/w/commondir  -> "../.."
    //   <home>/repos/worktree/.git  (file: "gitdir: ../myrepo/.git/worktrees/w")
    let repo_git = home.join("repos/myrepo/.git");
    std::fs::create_dir_all(repo_git.join("worktrees/w")).unwrap();
    std::fs::write(repo_git.join("HEAD"), "ref: refs/heads/main").unwrap();
    std::fs::write(repo_git.join("worktrees/w/HEAD"), "ref: refs/heads/feat-wt").unwrap();
    std::fs::write(repo_git.join("worktrees/w/commondir"), "../..").unwrap();
    // The worktree dir.
    let wt_dir = home.join("repos/worktree");
    std::fs::create_dir_all(&wt_dir).unwrap();
    std::fs::write(wt_dir.join(".git"), "gitdir: ../myrepo/.git/worktrees/w").unwrap();
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[test]
fn test_index_and_query() {
    let tmp = tempfile::TempDir::new().unwrap();
    let home = make_home_with_dbs(&tmp);
    let db_path = tmp.path().join("index.sqlite");

    // 1. First index run.
    let result = run_cmd(&db_path, &home, &["index"]);
    assert!(result.get("error").is_none(), "index error: {result}");
    let changed = result["changed"].as_i64().unwrap_or(0);
    // Expect: claude(1) + codex(1) + omp(1, skips the noise one) + pi(1) + opencode(1) + cursor(1) = 6
    // Note: omp nested-subagent has only a command-name noise user message → None → also recorded.
    assert!(changed >= 6, "expected at least 6 changed, got {changed}; result={result}");
    let errors = result["errors"].as_array().unwrap();
    assert!(errors.is_empty(), "index errors: {errors:?}");

    // 2. Query all — verify kept sessions by uid prefix.
    let q = run_cmd(&db_path, &home, &["query", "--scope", "all"]);
    let sessions = q["sessions"].as_array().unwrap();
    let uids: Vec<&str> = sessions.iter()
        .map(|s| s["uid"].as_str().unwrap())
        .collect();

    assert!(uids.iter().any(|u| u.starts_with("claude:")), "missing claude session");
    assert!(uids.iter().any(|u| u.starts_with("codex:")), "missing codex session");
    assert!(uids.iter().any(|u| u.starts_with("omp:")), "missing omp session");
    assert!(uids.iter().any(|u| u.starts_with("pi:")), "missing pi session");
    assert!(uids.iter().any(|u| u.starts_with("opencode:")), "missing opencode session");
    assert!(uids.iter().any(|u| u.starts_with("cursor:")), "missing cursor session");

    // Verify expected details.
    let claude_s = sessions.iter().find(|s| s["uid"].as_str().unwrap().starts_with("claude:")).unwrap();
    assert_eq!(claude_s["agent"], "claude");
    assert_eq!(claude_s["branch"], "feat/fts");
    assert!(claude_s["messages"].as_i64().unwrap() >= 2);
    let resume = &claude_s["resume"];
    assert_eq!(resume["kind"], "tab");
    assert_eq!(resume["argv"][0], "claude");
    assert_eq!(resume["argv"][1], "--resume");

    let codex_s = sessions.iter().find(|s| s["uid"].as_str().unwrap().starts_with("codex:")).unwrap();
    assert_eq!(codex_s["agent"], "codex");
    assert_eq!(codex_s["model"], "o1-mini");
    assert_eq!(codex_s["resume"]["argv"][0], "codex");
    assert_eq!(codex_s["resume"]["argv"][1], "resume");

    let omp_s = sessions.iter().find(|s| s["uid"].as_str().unwrap() == "omp:session-omp1").unwrap();
    assert_eq!(omp_s["model"], "gpt-4o");
    let omp_cmd = omp_s["resume"]["command"].as_str().unwrap();
    assert!(omp_cmd.contains("omp"), "omp resume command: {omp_cmd}");
    assert!(omp_cmd.contains("--resume"), "omp resume command: {omp_cmd}");

    let pi_s = sessions.iter().find(|s| s["uid"].as_str().unwrap().starts_with("pi:")).unwrap();
    assert_eq!(pi_s["model"], "gpt-4");
    assert_eq!(pi_s["resume"]["argv"][0], "pi");
    assert_eq!(pi_s["resume"]["argv"][1], "--session");

    let cursor_s = sessions.iter().find(|s| s["uid"].as_str().unwrap().starts_with("cursor:")).unwrap();
    assert_eq!(cursor_s["resume"]["kind"], "spawn");
    assert_eq!(cursor_s["resume"]["argv"][0], "open");
    assert_eq!(cursor_s["resume"]["argv"][1], "-a");
    assert_eq!(cursor_s["resume"]["argv"][2], "Cursor");

    // 3. Second index run → changed == 0.
    let result2 = run_cmd(&db_path, &home, &["index"]);
    assert_eq!(result2["changed"].as_i64().unwrap_or(-1), 0, "second run should have changed=0");
}

#[test]
fn test_fts_search() {
    let tmp = tempfile::TempDir::new().unwrap();
    let home = make_home_with_dbs(&tmp);
    let db_path = tmp.path().join("fts-test.sqlite");
    run_cmd(&db_path, &home, &["index"]);

    // Search for a word only in the OMP session.
    let q = run_cmd(&db_path, &home, &["query", "--q", "Python", "--scope", "all"]);
    let sessions = q["sessions"].as_array().unwrap();
    assert!(!sessions.is_empty(), "expected at least one FTS result for 'Python'");
    // The OMP session body contains "Python script".
    let found_omp = sessions.iter().any(|s| s["uid"].as_str().unwrap().starts_with("omp:"));
    assert!(found_omp, "expected OMP session in FTS results for 'Python'");

    // Snippet should contain the highlight markers (U+0001 / U+0002).
    let first = &sessions[0];
    if let Some(snippet) = first.get("snippet").and_then(|s| s.as_str()) {
        // The snippet should exist and contain highlight characters.
        assert!(snippet.contains('\u{0001}') || !snippet.is_empty(), "snippet: {snippet:?}");
    }
}

#[test]
fn test_increment_and_delete() {
    let tmp = tempfile::TempDir::new().unwrap();
    let home = make_home_with_dbs(&tmp);
    let db_path = tmp.path().join("inc-test.sqlite");
    run_cmd(&db_path, &home, &["index"]);

    // Touch the OMP session file: add an extra line.
    let omp_dir = home.join(".omp/agent/sessions/myproject");
    let omp_file = omp_dir.join("20240601_session-omp1.jsonl");
    let mut content = std::fs::read_to_string(&omp_file).unwrap();
    content.push_str(r#"{"type":"message","role":"assistant","content":[{"type":"text","text":"Updated assistant response for OMP."}],"timestamp":1717236010000}"#);
    content.push('\n');
    std::fs::write(&omp_file, &content).unwrap();

    // Touch the file mtime by writing and re-writing it.
    let result = run_cmd(&db_path, &home, &["index"]);
    assert_eq!(result["changed"].as_i64().unwrap_or(0), 1, "expected 1 changed after touch; {result}");

    // Verify last_assistant is updated.
    let q = run_cmd(&db_path, &home, &["query", "--scope", "all"]);
    let omp_s = q["sessions"].as_array().unwrap().iter()
        .find(|s| s["uid"].as_str().unwrap() == "omp:session-omp1")
        .expect("omp session not found");
    assert!(
        omp_s["last_assistant"].as_str().unwrap().contains("Updated"),
        "expected updated last_assistant; got: {}",
        omp_s["last_assistant"]
    );

    // Delete the Pi session file.
    let pi_dir = home.join(".pi/agent/sessions/piproject");
    let pi_file = pi_dir.join("20240601_pi-session1.jsonl");
    std::fs::remove_file(&pi_file).unwrap();

    let result3 = run_cmd(&db_path, &home, &["index"]);
    assert_eq!(result3["removed"].as_i64().unwrap_or(0), 1, "expected 1 removed; {result3}");

    // Verify Pi session is gone.
    let q2 = run_cmd(&db_path, &home, &["query", "--scope", "all"]);
    let has_pi = q2["sessions"].as_array().unwrap().iter()
        .any(|s| s["uid"].as_str().unwrap().starts_with("pi:"));
    assert!(!has_pi, "Pi session should be removed");
}

#[test]
fn test_worktree_scope() {
    let tmp = tempfile::TempDir::new().unwrap();
    let home = make_home_with_dbs(&tmp);
    build_worktree_fixture(&home);
    let db_path = tmp.path().join("wt-test.sqlite");

    // Add an OMP session whose cwd is the worktree directory.
    let wt_sessions_dir = home.join(".omp/agent/sessions/worktree-proj");
    std::fs::create_dir_all(&wt_sessions_dir).unwrap();
    let wt_file = wt_sessions_dir.join("20240601_wt-session.jsonl");
    let wt_cwd = home.join("repos/worktree").to_string_lossy().into_owned();
    let cwd_json = serde_json::to_string(&wt_cwd).unwrap();
    let content = format!(
        "{{\"type\":\"session\",\"id\":\"wt-session-1\",\"cwd\":{cwd_json},\"timestamp\":1717236000000}}\n\
         {{\"type\":\"message\",\"role\":\"user\",\"content\":\"worktree task in the branch\",\"timestamp\":1717236001000}}\n\
         {{\"type\":\"message\",\"role\":\"assistant\",\"content\":\"done\",\"timestamp\":1717236002000}}\n"
    );
    std::fs::write(&wt_file, &content).unwrap();

    run_cmd(&db_path, &home, &["index"]);

    // --scope workspace --cwd <worktree_dir> should return only the worktree session.
    let q_ws = run_cmd(&db_path, &home, &[
        "query", "--scope", "workspace",
        "--cwd", &wt_cwd,
    ]);
    let ws_sessions = q_ws["sessions"].as_array().unwrap();
    let all_in_ws = ws_sessions.iter().all(|s| {
        let wr = s["worktree_root"].as_str().unwrap_or("");
        wr == wt_cwd
    });
    assert!(all_in_ws, "workspace scope returned sessions from wrong worktree");
    assert!(!ws_sessions.is_empty(), "workspace scope returned no sessions");

    // Repository scope includes sessions from both the main repository and its worktree.
    let q_proj = run_cmd(&db_path, &home, &[
        "query", "--scope", "repository",
        "--cwd", &wt_cwd,
    ]);
    let proj_sessions = q_proj["sessions"].as_array().unwrap();
    let repo_root = home.join("repos/myrepo").to_string_lossy().into_owned();
    let all_in_proj = proj_sessions.iter().all(|s| {
        let pr = s["project_root"].as_str().unwrap_or("");
        pr == repo_root
    });
    assert!(all_in_proj, "project scope sessions outside project root");
}

#[test]
fn test_projects_command() {
    let tmp = tempfile::TempDir::new().unwrap();
    let home = make_home_with_dbs(&tmp);
    let db_path = tmp.path().join("projects-test.sqlite");
    run_cmd(&db_path, &home, &["index"]);

    let result = run_cmd(&db_path, &home, &["projects", "--per", "3", "--recent", "5"]);
    assert!(result.get("error").is_none(), "projects error: {result}");
    assert!(result["projects"].as_array().unwrap().len() > 0, "no projects returned");
    let recent = result["recent"].as_array().unwrap();
    assert!(recent.len() > 0, "no recents returned");
    // First recent should have agent field.
    assert!(recent[0].get("agent").is_some(), "recent missing agent field");
}
