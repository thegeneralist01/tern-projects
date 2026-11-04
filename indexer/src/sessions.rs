//! One projection/filter path for search, discovered folders and the curated library.
use crate::{
    db, git,
    library::Library,
    model::{project_label, Assignment, SessionJson},
};
use anyhow::{Context, Result};
use rusqlite::{params_from_iter, types::Value as SqlValue, Connection};
use serde_json::{json, Value};
use std::path::Path;

const COLUMNS:&str = "s.uid,s.agent,s.id,s.title,s.cwd,s.project_root,s.worktree_root,s.branch,s.model,s.created_ms,s.updated_ms,s.messages,s.last_assistant,s.resume";
const JOIN: &str = " LEFT JOIN library.chat_state cs ON cs.uid=s.uid ";
const VISIBLE: &str = "cs.archived_ms IS NULL";

pub fn open(db_path: &Path) -> Result<(Connection, Library)> {
    let lib = Library::open(db_path)?;
    let conn = db::open(db_path)?;
    conn.execute(
        "ATTACH DATABASE ? AS library",
        [crate::library::path(db_path).to_string_lossy().as_ref()],
    )?;
    Ok((conn, lib))
}
fn rows(conn: &Connection, sql: &str, values: &[SqlValue], home: &str) -> Result<Vec<SessionJson>> {
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt
        .query_map(params_from_iter(values), |r| {
            let root: String = r.get(5)?;
            let assigned: Option<i64> = r.get(18)?;
            let raw: String = r.get(13)?;
            let resume = serde_json::from_str(&raw).map_err(|e| {
                rusqlite::Error::FromSqlConversionFailure(
                    13,
                    rusqlite::types::Type::Text,
                    Box::new(e),
                )
            })?;
            Ok(SessionJson {
                uid: r.get(0)?,
                agent: r.get(1)?,
                id: r.get(2)?,
                title: r.get(3)?,
                cwd: r.get(4)?,
                project_label: project_label(&root, home),
                project_root: root,
                worktree_root: r.get(6)?,
                branch: r.get(7)?,
                model: r.get(8)?,
                created_ms: r.get(9)?,
                updated_ms: r.get(10)?,
                messages: r.get(11)?,
                last_assistant: r.get(12)?,
                resume,
                snippet: r.get(14)?,
                pinned: r.get::<_, Option<i64>>(15)?.is_some(),
                archived: r.get::<_, Option<i64>>(16)?.is_some(),
                assignment: Assignment {
                    mode: r.get(17)?,
                    project_id: assigned.map(|i| i.to_string()),
                },
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}
fn select(snippet: &str) -> String {
    format!("SELECT {COLUMNS},{snippet},cs.pinned_ms,cs.archived_ms,COALESCE(cs.assignment,'automatic'),cs.project_id FROM sessions s")
}
fn page(limit: i64, offset: i64) -> Result<()> {
    anyhow::ensure!(
        (1..=1000).contains(&limit) && offset >= 0,
        "invalid page: limit must be 1–1000 and offset nonnegative"
    );
    Ok(())
}
fn members(conn: &Connection, lib: &Library, project_id: &str) -> Result<()> {
    lib.project(project_id)?;
    let mut membership = lib.membership()?;
    conn.execute_batch("CREATE TEMP TABLE project_members(uid TEXT PRIMARY KEY);")?;
    let mut stmt = conn.prepare(&format!("SELECT s.uid,s.cwd,COALESCE(cs.assignment,'automatic'),cs.project_id FROM sessions s{JOIN}WHERE {VISIBLE}"))?;
    let mut insert = conn.prepare("INSERT INTO project_members(uid) VALUES(?)")?;
    let mut records = stmt.query([])?;
    while let Some(r) = records.next()? {
        let uid: String = r.get(0)?;
        let cwd: String = r.get(1)?;
        let assignment: String = r.get(2)?;
        if membership.matches(project_id, &uid, &cwd, &assignment, r.get(3)?) {
            insert.execute([uid])?;
        }
    }
    Ok(())
}
pub struct Query<'a> {
    pub text: &'a str,
    pub scope: &'a str,
    pub cwd: &'a str,
    pub sort: &'a str,
    pub limit: i64,
    pub offset: i64,
    pub project_id: Option<&'a str>,
}
pub fn query(db_path: &Path, home: &str, q: Query<'_>) -> Result<Value> {
    page(q.limit, q.offset)?;
    anyhow::ensure!(
        ["workspace", "repository", "all"].contains(&q.scope),
        "invalid scope: use workspace, repository or all"
    );
    anyhow::ensure!(["updated", "relevance"].contains(&q.sort), "invalid sort");
    let (conn, lib) = open(db_path)?;
    if let Some(project_id) = q.project_id {
        members(&conn, &lib, project_id)?;
    }
    let tokens: Vec<String> = q
        .text
        .split_whitespace()
        .map(|s| s.replace('"', ""))
        .filter(|s| !s.is_empty())
        .collect();
    let fts = tokens
        .iter()
        .map(|s| format!("\"{s}\"*"))
        .collect::<Vec<_>>()
        .join(" ");
    let mut joins = JOIN.to_owned();
    let mut conditions = vec![VISIBLE.to_owned()];
    let mut values: Vec<SqlValue> = vec![];
    if !fts.is_empty() {
        joins.push_str(" JOIN sessions_fts f ON f.rowid=s.rowid ");
        conditions.push("sessions_fts MATCH ?".into());
        values.push(fts.clone().into());
    }
    if q.scope != "all" {
        let effective = if q.cwd.is_empty() { home } else { q.cwd };
        let info = git::resolve(effective, &mut std::collections::HashMap::new());
        let (column, root) = if q.scope == "workspace" {
            ("s.worktree_root", info.worktree_root)
        } else {
            ("s.project_root", info.project_root)
        };
        conditions.push(format!("{column}=?"));
        values.push(root.into());
    }
    if q.project_id.is_some() {
        joins.push_str(" JOIN project_members pm ON pm.uid=s.uid ");
    }
    let where_clause = conditions.join(" AND ");
    let from = format!("{joins} WHERE {where_clause}");
    let total: i64 = conn.query_row(
        &format!("SELECT count(*) FROM sessions s{from}"),
        params_from_iter(&values),
        |r| r.get(0),
    )?;
    let snippet = if fts.is_empty() {
        "NULL"
    } else {
        "snippet(sessions_fts,1,char(1),char(2),'…',12)"
    };
    let order = if q.sort == "relevance" && !fts.is_empty() {
        "bm25(sessions_fts,10.0,1.0),s.updated_ms DESC,s.uid"
    } else {
        "s.updated_ms DESC,s.uid"
    };
    values.push(q.limit.into());
    values.push(q.offset.into());
    let sessions = rows(
        &conn,
        &format!(
            "{}{from} ORDER BY {order} LIMIT ? OFFSET ?",
            select(snippet)
        ),
        &values,
        home,
    )?;
    Ok(json!({"sessions":sessions,"total":total,"offset":q.offset}))
}
fn visible(conn: &Connection, home: &str) -> Result<Vec<SessionJson>> {
    rows(
        conn,
        &format!(
            "{}{JOIN}WHERE {VISIBLE} ORDER BY s.updated_ms DESC,s.uid",
            select("NULL")
        ),
        &[],
        home,
    )
}
pub fn snapshot(db_path: &Path, home: &str, view: &str, per: i64, recent: i64) -> Result<Value> {
    anyhow::ensure!(
        ["mine", "discovered"].contains(&view),
        "invalid project view"
    );
    anyhow::ensure!(
        (0..=1000).contains(&per) && (0..=1000).contains(&recent),
        "invalid snapshot limits"
    );
    let (conn, lib) = open(db_path)?;
    let sessions = visible(&conn, home)?;
    let recents: Vec<_> = sessions.iter().take(recent as usize).collect();
    let pinned = rows(
        &conn,
        &format!(
            "{}{JOIN}WHERE {VISIBLE} AND cs.pinned_ms IS NOT NULL ORDER BY cs.pinned_ms DESC,s.uid",
            select("NULL")
        ),
        &[],
        home,
    )?;
    let projects = if view == "mine" {
        let projects = lib.projects()?;
        let mut membership = lib.membership()?;
        let mut result = Vec::with_capacity(projects.len());
        for p in projects {
            let matching: Vec<_> = sessions
                .iter()
                .filter(|s| {
                    membership.matches(
                        &p.id,
                        &s.uid,
                        &s.cwd,
                        &s.assignment.mode,
                        s.assignment
                            .project_id
                            .as_deref()
                            .and_then(|s| s.parse().ok()),
                    )
                })
                .collect();
            let updated = matching.first().map(|s| s.updated_ms).unwrap_or(0);
            result.push((!matching.is_empty(),updated,p.created_ms,p.id.parse::<i64>()?,json!({"id":p.id,"name":p.name,"icon":p.icon,"folders":p.folders,"count":matching.len(),"updated_ms":updated,"sessions":matching.iter().take(per as usize).collect::<Vec<_>>()})));
        }
        result.sort_by(|a, b| {
            b.0.cmp(&a.0)
                .then(b.1.cmp(&a.1))
                .then(b.2.cmp(&a.2))
                .then(a.3.cmp(&b.3))
        });
        result.into_iter().map(|r| r.4).collect::<Vec<_>>()
    } else {
        let mut groups: Vec<Value> = Vec::new();
        let mut positions = std::collections::HashMap::new();
        for s in &sessions {
            let pos=*positions.entry(s.project_root.as_str()).or_insert_with(|| {
                let pos=groups.len(); groups.push(json!({"root":s.project_root,"label":s.project_label,"count":0,"updated_ms":s.updated_ms,"sessions":[]})); pos
            });
            let g = &mut groups[pos];
            let count = g["count"].as_i64().context("invalid group count")?;
            g["count"] = json!(count + 1);
            if count < per {
                g["sessions"]
                    .as_array_mut()
                    .context("invalid group sessions")?
                    .push(serde_json::to_value(s)?);
            }
        }
        groups
    };
    Ok(json!({"projects":projects,"pinned":pinned,"recent":recents,"settings":lib.settings()?}))
}
pub fn archived(db_path: &Path, home: &str, limit: i64, offset: i64) -> Result<Value> {
    page(limit, offset)?;
    let (conn, _) = open(db_path)?;
    let total: i64 = conn.query_row(
        &format!("SELECT count(*) FROM sessions s{JOIN}WHERE cs.archived_ms IS NOT NULL"),
        [],
        |r| r.get(0),
    )?;
    let sessions=rows(&conn,&format!("{}{JOIN}WHERE cs.archived_ms IS NOT NULL ORDER BY cs.archived_ms DESC,s.uid LIMIT ? OFFSET ?",select("NULL")),&[limit.into(),offset.into()],home)?;
    Ok(json!({"sessions":sessions,"total":total,"offset":offset}))
}
pub fn saved_project(db_path: &Path, home: &str, project_id: &str) -> Result<Value> {
    let snapshot = snapshot(db_path, home, "mine", 25, 15)?;
    snapshot["projects"]
        .as_array()
        .and_then(|projects| projects.iter().find(|p| p["id"] == project_id))
        .cloned()
        .map(|project| json!({"project":project}))
        .context("project no longer exists")
}
