#!/usr/bin/env python3
"""Exercise the Projects Recents This folder/All scope without a model or network."""
import argparse
from pathlib import Path
import shutil
import subprocess
import tempfile


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--runtime", default="luau")
    args = parser.parse_args()
    runtime = shutil.which(args.runtime)
    if runtime is None:
        parser.error(f"Luau runtime not found: {args.runtime}")
    root = Path(__file__).resolve().parent.parent
    fixture = r'''
local assertions = 0
local function check(value, message)
    assert(value, message)
    assertions += 1
end
local function node(kind, props, children)
    return {kind = kind, props = props or {}, children = children or {}}
end
local kv = {}
tern = {
    ui = {node = node, el = node, col = function(children) return node("col", nil, children) end, span = function(text) return node("span", {text = text}) end},
    json = {array = function(value) return value end, null = false, decode = function(value) return value end},
    fs = {exists = function() return true end, write = function() end},
    pane = {list = function() return {{pane = 1}} end},
    plugin = {data = "/tmp/recents-test/plugin"},
    runtime = {os = "linux"},
    getenv = function() return nil end,
    now = function() return 1 end,
    timer = function(_, _) return {cancel = function() end} end,
    kv = {get = function(key) return kv[key] end, set = function(key, value) kv[key] = value end},
}
local global_recent = {{uid = "all-1", title = "All one"}, {uid = "all-2", title = "All two"}}
local scoped_recent = {{uid = "repo-1", title = "Repo one"}}
local projects, snapshot_error = {}, nil
local calls, pending = {}, {}
local modules = {picker = {}, resume = {}, updates = {get = function() return {} end, subscribe = function() return function() end end}}
modules.cli = {
    status = function() return nil end,
    run = function(_, _, cb) if cb then cb({}) end end,
    request = function(payload, cb) table.insert(calls, payload); table.insert(pending, {payload = payload, cb = cb}) end,
}
local function require(path)
    local name = string.match(path, "([^/]+)$")
    return assert(modules[name], "unknown fixture dependency: " .. path)
end
__MODULES__
local panel = modules.panel
local function answer(job)
    local p = job.payload
    if p.op == "snapshot" then
        if snapshot_error then job.cb(nil, snapshot_error); return end
        job.cb({projects = projects, settings = {}, pinned = {}, recent = if p.recent_cwd then table.clone(scoped_recent) else table.clone(global_recent)}, nil)
    else
        job.cb({}, nil)
    end
end
local function settle()
    local job = table.remove(pending, 1)
    if job then answer(job) end
end
local function flush()
    local count = 0
    while #pending > 0 do
        count += 1
        assert(count < 200, "unexpected request loop")
        settle()
    end
end
local opened, runs = {}, {}
modules.cli.run = function(args, _, cb) table.insert(runs, args); if cb then cb({}) end end
local cx = {pane = 1, render = function() end, frame = function() end, toast = function() end, open = function(_, url) table.insert(opened, url) end, copy = function() end}
local function action(s, act, value)
    panel.event(s, {ev = "action", act = act, value = value}, cx)
end
local function last_snapshot()
    for i = #calls, 1, -1 do if calls[i].op == "snapshot" then return calls[i] end end
end
local function uids(list)
    local out = {}
    for _, chat in list do table.insert(out, chat.uid) end
    return table.concat(out, ",")
end
local function shows(s, uid) return s.rows["recent:" .. uid] ~= nil end
local function note(s, key) return s.rows[key] and s.rows[key].ref.label end
local function start(saved)
    return panel.init(cx, {"projects", "/tmp/recents/repo"}, saved)
end
-- A new panel waits for its first snapshot instead of showing the previous scope.
local s = start(nil)
check(s.recent_filter == "folder", "Recents defaults to This folder")
check(#s.recent == 0 and note(s, "recent:wait") == "Loading chats…", "a new panel shows a loading state")
flush()
check(last_snapshot().recent_cwd == "/tmp/recents/repo", "This folder scopes the snapshot to the panel path")
check(uids(s.recent) == "repo-1", "This folder shows only its chats")
check(shows(s, "repo-1") and not shows(s, "all-1"), "This folder rows come from the folder list")
-- Switching scope drops the old rows at once and shows the loading state.
action(s, "recent", "all")
check(#s.recent == 0 and note(s, "recent:wait") == "Loading chats…", "switching scope shows a loading state")
check(not shows(s, "repo-1"), "the previous scope's chats are not shown while loading")
flush()
check(s.recent_filter == "all", "All is selected")
check(last_snapshot().recent_cwd == nil, "All sends the global snapshot without a folder")
check(uids(s.recent) == "all-1,all-2", "All keeps the unchanged newest-first list")
check(shows(s, "all-1") and not shows(s, "repo-1"), "All replaces the folder rows")
check(kv["recents-filter:v2"] == "all", "the choice is remembered outside the panel state")
-- A reopened panel has no saved state and still keeps All.
local reopened = start(nil)
check(reopened.recent_filter == "all", "reopening the panel keeps All")
flush()
check(last_snapshot().recent_cwd == nil, "the reopened panel requests the global list")
check(uids(reopened.recent) == "all-1,all-2", "the reopened panel lists every folder")
-- Saved panel state still wins over the remembered choice.
local saved_folder = start({recents = "folder"})
check(saved_folder.recent_filter == "folder", "saved panel state restores its own scope")
flush()
check(last_snapshot().recent_cwd == "/tmp/recents/repo", "the restored scope decides the request")
-- Values written under the old All default are not read back.
kv["recents-filter:v2"] = nil
kv["recents-filter:v1"] = "all"
local legacy = start({recent_filter = "all"})
check(legacy.recent_filter == "folder", "old automatic All values fall back to This folder")
flush()
-- An empty folder says so instead of showing chats from elsewhere.
local folder = start({recents = "folder"})
check(#folder.recent == 0 and note(folder, "recent:wait") == "Loading chats…", "a restored folder waits for its own list")
flush()
scoped_recent = {}
action(folder, "reload"); flush()
check(note(folder, "recent:none") == "No chats in this folder yet", "an empty folder shows its empty state")
-- A different panel path follows the scope, and the saved state carries it across reload.
scoped_recent = {{uid = "other-1", title = "Other one"}}
action(folder, "open=projects:/tmp/recents/other")
check(#folder.recent == 0 and note(folder, "recent:wait") == "Loading chats…", "a new panel path drops the old folder's rows")
flush()
check(last_snapshot().recent_cwd == "/tmp/recents/other", "changing the panel path scopes This folder to it")
check(uids(folder.recent) == "other-1", "changing the panel path refreshes the folder list")
local carried = panel.save(folder)
check(carried.ctx_cwd == "/tmp/recents/other", "the panel saves the path it was retargeted to")
local retargeted = start(carried)
check(retargeted.ctx_cwd == "/tmp/recents/other", "a reloaded panel restores the retargeted path")
flush()
check(last_snapshot().recent_cwd == "/tmp/recents/other", "the restored path scopes the snapshot")
check(uids(retargeted.recent) == "other-1", "the restored path lists its own chats")
-- A superseded scope's response never replaces the current scope's rows.
action(retargeted, "recent", "all")          -- request 1: all
local stale = table.remove(pending, 1)
scoped_recent = {{uid = "fresh-1", title = "Fresh one"}}
action(retargeted, "recent", "folder")       -- request 2: folder
local fresh = table.remove(pending, 1)
stale.cb({projects = {}, settings = {}, pinned = {}, recent = {{uid = "stale-1"}}}, nil)
check(#retargeted.recent == 0 and not shows(retargeted, "stale-1"), "a superseded scope's response is ignored")
fresh.cb({projects = {}, settings = {}, pinned = {}, recent = {{uid = "fresh-1"}}}, nil)
check(uids(retargeted.recent) == "fresh-1", "the current scope's response applies")
-- Switching away and back before either answer lands stays loading through later rebuilds.
action(retargeted, "recent", "all")
action(retargeted, "recent", "folder")
action(retargeted, "mode", "projects")
check(note(retargeted, "recent:wait") == "Loading chats…", "switching back before either answer lands stays loading after a rebuild")
flush()
check(uids(retargeted.recent) == "fresh-1", "the folder's own answer then replaces the loading state")
-- A failed snapshot shows an error and never the previous list.
snapshot_error = "backend unavailable"
action(retargeted, "reload")
flush()
check(retargeted.error == "backend unavailable", "the failure is reported")
check(#retargeted.recent == 0 and not shows(retargeted, "fresh-1"), "a failed reload drops the old rows")
check(note(retargeted, "recent:wait") == "Couldn't load chats", "the recents area reports the failure instead of chats")
snapshot_error = nil
action(retargeted, "reload"); flush()
check(uids(retargeted.recent) == "fresh-1", "a later successful snapshot recovers")
-- Changing scope keeps the page that holds the Recents header and its toggle.
projects = {}
for i = 1, 160 do projects[i] = {id = tostring(i), name = "P" .. i, icon = "folder", folders = {"/tmp/p" .. i}, count = 0} end
action(retargeted, "reload"); flush()
local index = table.find(retargeted.order, "h:recent")
check(index ~= nil, "the Recents header is in the list")
local recents_page = math.floor((index - 1) / 150)
check(recents_page > 0, "the fixture puts Recents on a later page")
retargeted.page = recents_page
action(retargeted, "recent", if retargeted.recent_filter == "folder" then "all" else "folder")
check(retargeted.page == recents_page, "changing scope keeps the Recents page visible")
flush()
check(retargeted.page == recents_page, "the Recents page stays visible after the snapshot lands")
-- The followed terminal's folder retargets the panel in place.
local function follow(s, path) panel.event(s, {ev = "action", id = "main", act = "follow-cwd", value = path}, cx) end
if retargeted.recent_filter ~= "folder" then action(retargeted, "recent", "folder"); flush() end
scoped_recent = {{uid = "cd-1", title = "After cd"}}
retargeted.page = recents_page
retargeted.menu = {ids = {"refresh"}, selected = "refresh"}
retargeted.editor = modules["project-editor"].new({name = "", folders = {}})
follow(retargeted, "/tmp/recents/cd")
check(retargeted.ctx_cwd == "/tmp/recents/cd", "the followed terminal's folder retargets the panel")
check(retargeted.menu ~= nil and retargeted.mode == "projects", "following a folder keeps the open menu and the mode")
check(retargeted.editor ~= nil, "following a folder leaves an open dialog alone")
retargeted.editor = nil
flush()
check(last_snapshot().recent_cwd == "/tmp/recents/cd", "This folder follows the terminal's folder")
check(uids(retargeted.recent) == "cd-1", "the followed folder's chats replace the old ones")
check(retargeted.page == recents_page, "following a folder keeps the Recents page")
local count = #calls
follow(retargeted, "/tmp/recents/cd")
follow(retargeted, "")
check(#calls == count and retargeted.ctx_cwd == "/tmp/recents/cd", "the same or an unknown folder changes nothing")
-- A selected chat from the old folder and its row menu leave with it; the page stays.
check(retargeted.rows["recent:cd-1"] ~= nil, "the fixture shows the followed folder's chat")
retargeted.sel = "recent:cd-1"
retargeted.menu = {target = "recent:cd-1", ids = {"resume"}, selected = "resume"}
scoped_recent = {{uid = "cd-2", title = "Next folder"}}
follow(retargeted, "/tmp/recents/cd2")
check(retargeted.menu == nil and retargeted.sel == nil, "the old folder's selected chat and its row menu are dropped")
flush()
check(retargeted.page == recents_page and uids(retargeted.recent) == "cd-2", "a selected chat from the old folder does not move the page")
action(retargeted, "recent", "all"); flush()
count = #calls
follow(retargeted, "/tmp/recents/elsewhere")
check(retargeted.ctx_cwd == "/tmp/recents/elsewhere" and #calls == count, "with All the folder only relabels the panel")
check(panel.save(retargeted).ctx_cwd == "/tmp/recents/elsewhere", "a followed folder survives reload")
-- Search's folder scopes follow too, from their first page.
action(retargeted, "open=search:"); flush()
action(retargeted, "scope", "workspace"); flush()
retargeted.offset = 40
follow(retargeted, "/tmp/recents/search"); flush()
check(retargeted.offset == 0 and runs[#runs][5] == "/tmp/recents/search", "Worktree search follows the terminal's folder")
-- Refresh asks the window for the followed folder again.
table.clear(opened)
action(retargeted, "refresh"); flush()
check(table.find(opened, "tern-projects://follow?") ~= nil, "Refresh asks the window for the followed folder")
print("Recents scope: " .. tostring(assertions) .. " assertions passed (no model or network)")
'''
    names = ("textfield", "fmt", "input", "ui", "project-editor", "picker", "resume", "chat-launch", "panel")
    modules = []
    for name in names:
        path = root / ("blocks/panel.luau" if name == "panel" else f"lib/{name}.luau")
        source = path.read_text(encoding="utf-8")
        modules.append(f'modules["{name}"] = (function()\n{source}\nend)()')
    fixture = fixture.replace("__MODULES__", "\n".join(modules))
    with tempfile.TemporaryDirectory(prefix="recents-") as directory:
        file = Path(directory) / "recents.luau"
        file.write_text(fixture, encoding="utf-8")
        return subprocess.run([runtime, str(file)], check=False).returncode


if __name__ == "__main__":
    raise SystemExit(main())
