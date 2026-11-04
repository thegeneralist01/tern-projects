use std::io::Write;
use std::path::Path;
use std::process::{Command, Output, Stdio};
use std::sync::{Arc, Barrier};

use serde_json::{json, Value};

const BIN: &str = env!("CARGO_BIN_EXE_tern-projects");

fn command(db: &Path, home: &Path) -> Command {
    let mut command = Command::new(BIN);
    command.env("TERN_PROJECTS_HOME", home).arg("--db").arg(db);
    command
}

fn library(db: &Path, home: &Path, request: &Value) -> Output {
    let mut child = command(db, home)
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

fn success(output: Output) -> Value {
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
fn concurrent_first_index_snapshot_query_and_save_preserve_library() {
    // Both databases are absent: every CLI path must tolerate another opener's
    // index/library migration without dropping newly committed data.
    for _ in 0..4 {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("home");
        let folder = home.join("project");
        std::fs::create_dir_all(&folder).unwrap();
        let db = temp.path().join("index.sqlite");
        assert!(!temp.path().join("library.sqlite").exists());

        let barrier = Arc::new(Barrier::new(9));
        let workers: Vec<_> = (0..9).map(|worker| {
            let barrier = Arc::clone(&barrier);
            let db = db.clone();
            let home = home.clone();
            let folder = folder.clone();
            std::thread::spawn(move || {
                barrier.wait();
                let output = match worker {
                    0 => library(&db, &home, &json!({
                        "op":"project-save", "name":"Saved during first open", "icon":"rocket",
                        "folders":[folder], "cwd":home,
                    })),
                    1 => command(&db, &home).arg("index").output().unwrap(),
                    2..=4 => library(&db, &home, &json!({
                        "op":"snapshot", "view":"mine", "per":5, "recent":15,
                    })),
                    _ => command(&db, &home).args(["query", "--scope", "all"]).output().unwrap(),
                };
                success(output)
            })
        }).collect();
        let results: Vec<_> = workers
            .into_iter()
            .map(|worker| worker.join().unwrap())
            .collect();
        let saved_id = results[0]["project"]["id"].as_str().unwrap();
        let snapshot = success(library(
            &db,
            &home,
            &json!({
                "op":"snapshot", "view":"mine", "per":5, "recent":15,
            }),
        ));
        let projects = snapshot["projects"].as_array().unwrap();
        assert_eq!(projects.len(), 1);
        assert_eq!(projects[0]["id"], saved_id);
        assert_eq!(projects[0]["name"], "Saved during first open");
        assert_eq!(projects[0]["icon"], "rocket");
        assert_eq!(
            projects[0]["folders"],
            json!([std::fs::canonicalize(&folder).unwrap()])
        );
        assert_eq!(projects[0]["count"], 0);
        let conn = rusqlite::Connection::open(temp.path().join("library.sqlite")).unwrap();
        assert_eq!(
            conn.query_row("SELECT count(*) FROM projects", [], |row| row
                .get::<_, i64>(0))
                .unwrap(),
            1
        );
    }
}
