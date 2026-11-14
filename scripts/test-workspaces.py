#!/usr/bin/env python3
"""Exercise workspace membership, dialogs, authorization and editor routing without a model."""
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
local timers, store, launched, links, toasts = {}, {}, {}, {}, {}
local renders = 0
local installed, executable_present, poll_state = false, false, "running"
local settings, launch_error = {}, nil
tern = {
    ui = {node = node, el = node, col = function(children) return node("col", nil, children) end, span = function(text, class) return node("span", {text = text, class = class}) end},
    json = {array = function(value) return value end, null = false, decode = function(value) return value end},
    fs = {exists = function() return executable_present end, write = function() end},
    pane = {list = function() return {{pane = 1}} end},
    plugin = {data = "/tmp/ws-test/plugin"},
    runtime = {os = "linux"},
    getenv = function() return nil end,
    now = function() return 1 end,
    timer = function(delay, cb) table.insert(timers, {delay = delay, cb = cb}); return {cancel = function() end} end,
    kv = {get = function(key) return store[key] end, set = function(key, value) store[key] = value end},
}
local projects, checkouts, pending, calls, pid = {}, {}, {}, {}, 10
local roots = {["/tmp/ws-test/repo/app"] = "/tmp/ws-test/repo"}
local root_failures, snapshot_error, roots_error = {}, nil, nil
local agent_panel, agent_snapshots = nil, {}
local active_workspace, peak_workspace = 0, 0
local modules = {
    picker = {},
    updates = {get = function() return {status = "idle", current_version = "0.2.14", command = ""} end, subscribe = function() return function() end end},
}
modules.cli = {
    status = function() return nil end,
    run = function(_, _, cb) cb({}) end,
    request = function(payload, cb, current)
        table.insert(calls, payload)
        if payload.op == "workspaces" and payload.request.action ~= "detect" then
            active_workspace += 1
            peak_workspace = math.max(peak_workspace, active_workspace)
        end
        table.insert(pending, {payload = payload, cb = cb, current = current})
    end,
}
local function require(path)
    local name = string.match(path, "([^/]+)$")
    return assert(modules[name], "unknown fixture dependency: " .. path)
end
__MODULES__
modules["chat-launch"].open = function(pane, cwd, command, resuming, live, _fallback)
    check(live(), "workspace launch retains the panel liveness boundary")
    table.insert(launched, {pane = pane, cwd = cwd, command = command, resuming = resuming})
end
local panel, workspaces, fmt, tf = modules.panel, modules.workspaces, modules.fmt, modules.textfield
local function settle(index)
    local job = table.remove(pending, index or 1)
    if job.native then
        workspaces.event(agent_panel, {ev = "action", act = "workspace-agents", value = {visible = true, nonce = job.nonce, agents = table.remove(agent_snapshots, 1) or {}}})
        return
    end
    local p, data, err = job.payload, {}, nil
    if p.op == "workspaces" and p.request.action ~= "detect" then active_workspace -= 1 end
    if job.current and not job.current() then return end
    if p.op == "snapshot" then data = {projects = projects, settings = settings, pinned = {}, recent = {}}; err = snapshot_error
    elseif p.op == "archived" then data = {sessions = {}, total = 0}
    elseif p.op == "harnesses" then data = {harnesses = {{id = "codex", label = "Codex", installed = true, multi_root = true, path = "/bin/codex"}, {id = "omp", label = "OMP", installed = false, reason = "OMP CLI was not found"}}}
    elseif p.op == "launch-plan" then data = {cwd = p.cwd, command = p.harness .. " --fixture", warnings = {}}; err = launch_error
    elseif p.op == "folders" then data = {path = p.path}
    elseif p.op == "project-save" then
        local project = {id = tostring(#projects + 1), name = p.name, folders = table.clone(p.folders)}
        table.insert(projects, project)
        data = {project = project}
    elseif p.op == "workspaces" then
        local r = p.request
        if r.action == "detect" then data = {available = installed, path = if installed then "/bin/ovnfs" else nil}
        elseif r.action == "poll" then data = {state = poll_state, checkouts = checkouts, pid = pid, error = if poll_state == "stopped" then "OVNFS is not running." elseif poll_state == "error" then "fixture permission denied" else nil}
        elseif r.action == "roots" then
            err = roots_error
            for _, path in r.paths do data[path] = if root_failures[path] then {error = root_failures[path]} else {source = roots[path] or path} end
        elseif r.action == "details" then data = {changes = {"one.txt"}, commits = {count = 2, titles = {"one", "two"}, head = "fixture-head"}, token = "detail-token"}
        elseif r.action == "preview" then data = {source = "/tmp/ws-test/other", applied = {}, merged = {}, conflicts = {}, details = {}, token = "preview-token"}
        elseif r.action == "authorize" then
            for _, c in checkouts do if c.name == r.name then data = table.clone(c) end end
            data.agent_locations = {}
            for _, agent in r.agents do if agent.cwd then data.agent_locations[agent.cwd] = if string.find(agent.cwd, "missing", 1, true) then false else agent.cwd end end
        elseif r.action == "save-before-remove" then data = {token = "saved-token"}
        elseif r.action == "resolve" then
            for _, path in r.paths do data[path] = if string.find(path, "missing", 1, true) then false else path end
        elseif r.action ~= "apply" and r.action ~= "remove" and r.action ~= "fork" then error("unexpected operation: " .. r.action) end
    end
    job.cb(data, err)
end
local function flush()
    local count = 0
    while #pending > 0 do
        count += 1
        assert(count < 500, "unexpected request loop")
        settle()
    end
end
local cx = {pane = 1, render = function() renders += 1 end, frame = function() end, toast = function(_, level, title, detail) table.insert(toasts, {level = level, title = title, detail = detail}) end, open = function(_, url)
    table.insert(links, url)
    local nonce = string.match(url, "nonce=(.+)$")
    if nonce then table.insert(pending, {native = true, nonce = fmt.unpct(nonce)}) end
end}
local function start(mode)
    local s = panel.init(cx, {mode, "/tmp/ws-test/header"})
    s.workspaces.visible = true
    workspaces.refresh(s)
    flush()
    if mode == "workspaces" and s.mode ~= mode and s.workspaces.available then
        panel.event(s, {ev = "action", act = "mode", value = mode}, cx)
    end
    if s.mode == "workspaces" then s.workspaces.visible = true; workspaces.refresh(s) end
    flush()
    return s
end
local function action(s, act, value)
    panel.event(s, {ev = "action", act = act, value = value}, cx)
end
local function workspace_action(s, verb, value)
    action(s, "ws", verb .. ":" .. fmt.pct(value or ""))
end
local function all_nodes(nodes, out)
    out = out or {}
    for _, n in nodes do
        table.insert(out, n)
        all_nodes(n.children, out)
    end
    return out
end
local function view(s) return all_nodes(workspaces.view(s)) end
local function has_text(nodes, text)
    for _, n in nodes do
        if n.props.text == text or n.props.label == text then return true end
    end
    return false
end
local function panel_nodes(s) return all_nodes({panel.view(s, cx).main}) end
local function has_tab(s)
    for _, n in panel_nodes(s) do
        if n.props.actions and n.props.actions.click == "mode=workspaces" then return true end
    end
    return false
end
local function count_workspace(verb)
    local count = 0
    for _, p in calls do if p.op == "workspaces" and (not verb or p.request.action == verb) then count += 1 end end
    return count
end
local missing = panel.init(cx, {"projects", "/tmp/ws-test/header"}, {mode = "workspaces"})
check(missing.mode == "projects" and not has_tab(missing), "missing and unknown installations never flash a restored Workspaces tab")
flush()
check(not has_tab(missing) and count_workspace() == 1 and count_workspace("detect") == 1, "missing performs only detection, never status or listing")
panel.key(missing, {name = "tab"}, cx); flush()
check(missing.mode == "projects" and missing.view == "discovered", "Tab reaches Discovered")
panel.key(missing, {name = "tab"}, cx); flush()
check(missing.mode == "search", "Tab skips missing Workspaces")
panel.key(missing, {name = "tab", shift = true}, cx); flush()
check(missing.mode == "projects" and missing.view == "discovered", "Shift+Tab also skips missing Workspaces")
action(missing, "mode", "workspaces"); flush()
check(missing.mode == "projects" and not missing.workspaces.active, "commands targeting a missing Workspaces view fall back to Projects")
action(missing, "open=workspaces:"); flush()
check(missing.mode == "projects", "window open requests also fall back")
action(missing, "mode", "settings"); flush()
check(has_text(panel_nodes(missing), "Cycle Mine / Discovered / Search / Settings"), "keyboard help omits missing Workspaces")
for _, timer in table.clone(timers) do if timer.delay == 120000 then timer.cb(cx) end end
flush()
check(count_workspace() == 1, "chat-index timer does not re-detect or poll missing OVNFS")
action(missing, "refresh"); flush()
check(count_workspace() == 2 and count_workspace("detect") == 2 and not has_tab(missing), "explicit Refresh performs one detection without flashing")
installed = true; executable_present = true
action(missing, "refresh"); flush()
check(has_tab(missing) and not missing.workspaces.active, "install then Refresh exposes the tab without reopening or polling it")
local reopened = panel.init(cx, {"projects", "/tmp/ws-test/header"}, {mode = "workspaces"})
check(has_tab(reopened) and reopened.mode == "workspaces", "positive detection restores Workspaces before later opens render")
flush(); reopened.alive = false
action(missing, "mode", "workspaces"); missing.workspaces.visible = true; workspaces.refresh(missing); flush()
check(missing.mode == "workspaces" and missing.workspaces.active, "installed Workspaces becomes active on selection")
poll_state = "stopped"; workspaces.refresh(missing); flush()
check(has_text(view(missing), "OVNFS is not running") and has_text(view(missing), "Start"), "stopped state retains its Start button")
poll_state = "error"; workspaces.refresh(missing); flush()
check(has_text(view(missing), "OVNFS error: Fixture permission denied"), "error state retains its reason")
installed = false; executable_present = false
workspace_action(missing, "refresh"); flush()
check(missing.mode == "projects" and not missing.workspaces.active and not has_tab(missing), "Refresh after removal leaves Workspaces and hides its tab")
local removed = panel.init(cx, {"projects", "/tmp/ws-test/header"}, {mode = "workspaces"})
check(removed.mode == "projects" and not has_tab(removed), "negative cache never flashes on reopen")
flush(); removed.alive = false; missing.alive = false
installed = true; executable_present = true; poll_state = "running"
-- An old positive result is also checked synchronously before rendering after uninstall.
local first_installed = panel.init(cx, {"projects", "/tmp/ws-test/header"})
check(not has_tab(first_installed), "the first installation is hidden until detection completes")
flush()
check(has_tab(first_installed), "the first detection can reveal Workspaces")
first_installed.alive = false
executable_present = false; installed = false
local stale = panel.init(cx, {"workspaces", "/tmp/ws-test/header"})
check(stale.mode == "projects" and not has_tab(stale), "a deleted cached executable cannot flash on a later open")
flush(); stale.alive = false
installed = true; executable_present = true
calls = {}
local function checkout(name, source, parent)
    return {name = name, source = source, source_location = source, location = "/mount/" .. name, path = "/mount/" .. name, identity = name, touched = "2026-11-12T00:00:00Z", parent_name = parent}
end
local function detail_calls()
    local count = 0
    for _, p in calls do if p.op == "workspaces" and p.request.action == "details" then count += 1 end end
    return count
end
local s = start("workspaces")
check(has_text(view(s), "Add a project to see its workspaces"), "no projects and no checkouts keeps the empty state")
-- Known-hidden views do no periodic bridge work, resolving or rendering.
s.workspaces.visible = false
local hidden_links, hidden_resolves, hidden_polls, hidden_renders = #links, count_workspace("resolve"), count_workspace("poll"), renders
for _, timer in table.clone(timers) do if timer.delay == 2000 then timer.cb(cx) end end
flush()
check(#links == hidden_links and count_workspace("resolve") == hidden_resolves and count_workspace("poll") == hidden_polls, "hidden timer requests neither agent snapshots nor path resolution nor OVNFS polls")
action(s, "workspace-agents", {visible = false, agents = {{cwd = "/hidden/new-agent", state = "idle", live = true}}}); flush()
check(count_workspace("resolve") == hidden_resolves and renders == hidden_renders, "hidden agent payload updates state without resolving paths or rendering")
check(s.workspaces.agents[1].cwd == "/hidden/new-agent", "hidden agent state is retained")
for _, mode in {"projects", "search", "settings"} do
    action(s, "mode", mode); flush()
    local before_render, before_resolve = renders, count_workspace("resolve")
    action(s, "workspace-agents", {visible = true, agents = {}}); flush()
    check(renders == before_render and count_workspace("resolve") == before_resolve, "agent payloads do not redraw or resolve outside Workspaces: " .. mode)
end
action(s, "mode", "workspaces"); s.workspaces.visible = true; workspaces.refresh(s); flush()
-- A non-local window can refuse the route entirely. Listing has a bounded wait.
local timeout_start, timeout_polls = #timers, count_workspace("poll")
local without_agents = panel.init(cx, {"workspaces", "/tmp/ws-test/header"})
flush()
check(without_agents.workspaces.visible == nil and without_agents.workspaces.state == "loading", "initial listing waits briefly for a native visibility snapshot")
for index = timeout_start + 1, #timers do if timers[index].delay == 3000 then timers[index].cb(cx) end end
flush()
check(count_workspace("poll") == timeout_polls + 1 and without_agents.workspaces.state == "running", "a refused window snapshot cannot leave the first OVNFS poll stuck")
check(without_agents.workspaces.agents_unknown and #without_agents.workspaces.agents == 0, "timeout uses unknown agent status instead of stale results")
check(has_text(view(without_agents), "Agent status is unavailable. Applying or removing still requires a live Tern agent check."), "unknown agent status is explicit while browsing remains available")
without_agents.alive = false
local known_hidden_start, known_hidden_polls = #timers, count_workspace("poll")
local known_hidden = panel.init(cx, {"workspaces", "/tmp/ws-test/header"})
known_hidden.workspaces.visible = false
flush()
for index = known_hidden_start + 1, #timers do if timers[index].delay == 3000 then timers[index].cb(cx) end end
flush()
check(count_workspace("poll") == known_hidden_polls and not known_hidden.workspaces.visible, "visibility timeout never overrides a known-hidden window")
known_hidden.alive = false
checkouts = {
    checkout("parent", "/tmp/ws-test/repo"),
    checkout("child", "/tmp/ws-test/repo", "parent"),
    checkout("other", "/tmp/ws-test/other"),
    checkout("orphan", "/tmp/ws-test/other", "gone"),
    checkout("unknown", "/tmp/ws-test/missing"),
}
checkouts[5].source_location = nil
workspaces.refresh(s); flush()
local nodes = view(s)
check(has_text(nodes, "Not in a project"), "zero projects still shows existing checkouts")
check(not has_text(nodes, "Add a project to see its workspaces"), "existing checkouts suppress the misleading empty state")
local add_buttons, checkout_rows = 0, {}
for _, n in nodes do
    if n.props.actions and n.props.actions.click and string.find(n.props.actions.click, "ws=add-project:", 1, true) then add_buttons += 1 end
    if n.kind == "item" and string.sub(n.props.key, 1, 9) == "checkout:" then checkout_rows[n.props.key] = n end
end
check(add_buttons == 3, "one Add as project button per source, including an unresolved source")
check(#s.workspaces.order == 8, "every checkout and Add as project action has a navigable row")
check(detail_calls() == 5, "unassigned checkout details are fetched on first load")
check(peak_workspace == 3, "detail requests use the existing three-process cap")
check(string.find(checkout_rows["checkout:unassigned:/tmp/ws-test/repo:child"].props.label, string.rep("\u{00a0}", 4), 1, true) == 1, "fork lineage is retained")
check(checkout_rows["checkout:unassigned:/tmp/ws-test/other:orphan"] ~= nil, "missing parents do not hide checkouts")
check(string.find(checkout_rows["checkout:unassigned:/tmp/ws-test/repo:parent"].props.detail, "1 source changes · 2 commits not brought back", 1, true) ~= nil, "unassigned rows show source and commit counts")
workspaces.refresh(s); flush()
check(detail_calls() == 5, "unchanged unassigned details stay cached")
checkouts[1].touched = "2026-11-12T01:00:00Z"
workspaces.refresh(s); flush()
check(detail_calls() == 6, "touched changes re-request unassigned details")
pid += 1
workspaces.refresh(s); flush()
check(detail_calls() == 11, "daemon pid changes re-request all details")
workspace_action(s, "refresh"); flush()
check(detail_calls() == 16, "explicit refresh recomputes all details")
projects = {{id = "git-subfolder", name = "App", folders = {"/tmp/ws-test/repo/app"}}}
workspaces.library_changed(s); flush(); view(s)
check(s.workspaces.rows["checkout:git-subfolder:/tmp/ws-test/repo/app:parent"] ~= nil, "Git-subfolder association uses the capture root")
check(s.workspaces.rows["checkout:unassigned:/tmp/ws-test/repo:parent"] == nil, "assigned checkouts are not duplicated as unassigned")
check(s.workspaces.rows["checkout:unassigned:/tmp/ws-test/other:other"] ~= nil, "unrelated sources remain unassigned")
-- Unknown membership never advertises Add as project, regardless of poll ordering.
workspaces.library_changed(s)
nodes = view(s)
check(has_text(nodes, "Loading Mine projects and workspace membership…") and not has_text(nodes, "Not in a project"), "snapshot loading withholds unassigned classification")
settle() -- Mine snapshot; roots are still pending.
nodes = view(s)
check(s.workspaces.membership == "loading" and not has_text(nodes, "Add as project"), "root lookup must also finish before Add as project appears")
flush()
snapshot_error = "Mine snapshot unavailable"
workspaces.library_changed(s); flush(); nodes = view(s)
check(s.workspaces.membership == "error" and not has_text(nodes, "Not in a project") and not has_text(nodes, "Add as project"), "snapshot failure is not an empty Mine library")
check(string.find(s.workspaces.membership_error, "Mine snapshot unavailable", 1, true) ~= nil, "snapshot failure is surfaced")
snapshot_error = nil; roots_error = "Root lookup unavailable"
workspaces.library_changed(s); flush()
check(s.workspaces.membership == "error" and not has_text(view(s), "Add as project"), "root request failure also withholds unassigned actions")
roots_error = nil
projects = {{id = "missing-source", name = "Missing source", folders = {"/tmp/ws-test/missing"}}}
root_failures["/tmp/ws-test/missing"] = "No such file or directory"
workspaces.library_changed(s); flush(); nodes = view(s)
check(s.workspaces.rows["checkout:missing-source:/tmp/ws-test/missing:unknown"] ~= nil, "exact stored source association survives failed canonicalization")
check(s.workspaces.rows["checkout:unassigned:/tmp/ws-test/missing:unknown"] == nil, "missing curated sources never move to Not in a project")
local folder = nil
for _, n in nodes do if n.props.key == "synthetic:missing-source:/tmp/ws-test/missing:new" then folder = n end end
check(folder and string.find(folder.props.detail, "Source unavailable: No such file or directory", 1, true), "the folder surfaces its resolution error")
root_failures["/tmp/ws-test/missing"] = nil
workspaces.refresh(s); flush()
check(s.workspaces.roots["/tmp/ws-test/missing"].source == "/tmp/ws-test/missing", "polling re-resolves a source when it becomes available")
projects = {{id = "git-subfolder", name = "App", folders = {"/tmp/ws-test/repo/app"}}}
workspaces.library_changed(s); flush()
s.workspaces.locations["/mount/other/sub"] = "/mount/other/sub"
s.workspaces.agents = {{pane = 99, cwd = "/mount/other/sub", state = "waiting_input", live = true, title = "mock"}}
check(has_text(view(s), "\u{00a0}\u{00a0}\u{00a0}\u{00a0}Waiting for you"), "agents also render under unassigned checkouts")
for state, label in {working = "Working", idle = "Idle", done = "Done", failed = "Failed"} do
    s.workspaces.agents[1].state = state
    check(has_text(view(s), string.rep("\u{00a0}", 4) .. label), "agent state labels are English-only")
end
s.workspaces.agents[1].state = "waiting_input"
workspace_action(s, "preview", "other")
check(s.workspaces.dialog == nil and s.workspaces.error ~= nil, "unassigned apply keeps the live-agent guard")
workspace_action(s, "remove", "other")
check(s.workspaces.dialog == nil, "unassigned removal keeps the live-agent guard")
s.workspaces.locations = {}
s.workspaces.agents[1].cwd = "/mount/other/missing-subfolder"
check(has_text(view(s), string.rep("\u{00a0}", 4) .. "Waiting for you"), "unresolved checkout descendants remain grouped")
workspace_action(s, "remove", "other")
check(s.workspaces.dialog == nil, "deleted agent subfolders still block removal")
s.workspaces.agents[1].cwd = "/mount/other-sibling/missing"
check(not has_text(view(s), string.rep("\u{00a0}", 4) .. "Waiting for you"), "unrelated unresolved cwds are not grouped")
s.workspaces.agents = {}
workspace_action(s, "changes", "other"); flush()
check(has_text(all_nodes({workspaces.layer(s)}), "Source changes"), "source-change dialog labels are English-only")
workspace_action(s, "cancel")
workspace_action(s, "preview", "other"); flush()
local preview = all_nodes({workspaces.layer(s)})
for _, label in {"Applied + merged", "Conflict markers", "Left untouched"} do
    check(has_text(preview, label), "apply preview group labels are English-only")
end
workspace_action(s, "cancel")
-- A routine panel timer/library reload retains both drafts and confirmations.
workspace_action(s, "branch", "other")
local draft = s.workspaces.dialog
tf.set(draft.field, "keep-this-draft")
for _, timer in table.clone(timers) do if timer.delay == 120000 then timer.cb(cx) end end
flush()
check(s.workspaces.dialog == draft and draft.field.text == "keep-this-draft", "120-second library refresh preserves an open draft")
workspace_action(s, "cancel")
workspace_action(s, "remove", "other"); flush()
local confirmation = s.workspaces.dialog
workspaces.library_changed(s); flush()
check(s.workspaces.dialog == confirmation and confirmation.info.token == "detail-token", "routine reload preserves removal confirmation and its token")
confirmation.info.commits.head = nil
confirmation.info.commits.refs = {"refs/heads/retained-work"}
workspace_action(s, "cancel")
-- Feed the same real dialog builder an unborn-head response.
workspace_action(s, "remove", "other")
for _, job in pending do if job.payload and job.payload.op == "workspaces" and job.payload.request.action == "details" then
    local cb = job.cb
    job.cb = function(data, err) data.commits = confirmation.info.commits; cb(data, err) end
end end
flush()
local recovery = false
for _, n in all_nodes({workspaces.layer(s)}) do
    if n.props.text and string.find(n.props.text, "refs/heads/retained-work", 1, true) and string.find(n.props.text, "then use Save branch", 1, true) then recovery = true end
end
check(recovery, "unborn-head warning names retained refs and explains how to save their commits")
check(s.workspaces.dialog.buttons[#s.workspaces.dialog.buttons].id == "remove" and not s.workspaces.dialog.buttons[#s.workspaces.dialog.buttons].disabled, "explicit warned Remove remains available for unborn HEAD")
workspaces.leave(s)
check(s.workspaces.dialog == nil, "real view leave still closes dialogs")
workspaces.enter(s); flush()
-- The helper is deliberately asynchronous. Agents can start after its snapshot,
-- and each mutation stage must read and check native agents again at handoff.
agent_panel = s
local function count_action(verb)
    local count = 0
    for _, p in calls do if p.op == "workspaces" and p.request.action == verb then count += 1 end end
    return count
end
local function confirm(verb)
    s.workspaces.agents = {}
    workspace_action(s, verb, "other"); flush()
    workspace_action(s, "dialog", if verb == "preview" then "apply" else "remove")
    flush()
end
local apply_before = count_action("apply")
agent_snapshots = {{}, {{cwd = "/mount/other/sub", state = "working", live = true}}}
confirm("preview")
check(count_action("apply") == apply_before and not s.workspaces.busy, "agent starting during slow authorization blocks Apply at final handoff")
workspace_action(s, "cancel")
local remove_before = count_action("remove")
agent_snapshots = {{}, {{cwd = "/mount/other/missing-subfolder", state = "idle", live = true}}}
confirm("remove")
check(count_action("remove") == remove_before, "new live agent in deleted subfolder blocks Remove at final handoff")
workspace_action(s, "cancel")
agent_snapshots = {{}, {{cwd = "/unknown-alias/sub", state = "working", live = true}}}
confirm("preview")
check(count_action("apply") == apply_before, "a new unresolved alias is not trusted after final snapshot")
workspace_action(s, "cancel")
agent_snapshots = {{{cwd = "/unrelated/missing", state = "working", live = true}}, {{cwd = "/unrelated/missing", state = "working", live = true}}}
confirm("preview")
check(count_action("apply") == apply_before + 1, "known unrelated unresolved agent does not disable Apply")
local applied = nil
for _, p in calls do if p.op == "workspaces" and p.request.action == "apply" then applied = p.request end end
check(applied and #applied.agents == 1 and applied.agents[1].cwd == "/unrelated/missing", "mutation binds the final native snapshot")
workspace_action(s, "cancel")
s.workspaces.agents = {}
workspace_action(s, "remove", "other"); flush()
workspace_action(s, "dialog", "save-remove")
tf.set(s.workspaces.dialog.field, "saved-work")
agent_snapshots = {{}, {}, {}, {{cwd = "/mount/other/sub", state = "idle", live = true}}}
local save_before = count_action("save-before-remove")
workspace_action(s, "dialog", "submit"); flush()
check(count_action("save-before-remove") == save_before + 1, "save-then-remove completes its save stage")
check(count_action("remove") == remove_before and not s.workspaces.busy, "save-then-remove checks a second final snapshot before removal")
workspace_action(s, "cancel")
s.workspaces.agents = {}
projects = {}
workspaces.library_changed(s); flush()
view(s)
s.workspaces.selected = nil
local wanted = "synthetic:unassigned:/tmp/ws-test/repo:add"
for _ = 1, #s.workspaces.order do
    workspaces.key(s, {name = "down"})
    if s.workspaces.selected == wanted then break end
end
check(s.workspaces.selected == wanted and s.workspaces.rows[wanted].kind == "add-project", "arrow navigation reaches Add as project")
local selected_list = nil
for _, n in view(s) do if n.kind == "list" then selected_list = n end end
check(selected_list and selected_list.props.selected == "main.workspace." .. wanted, "Add as project renders visible list focus")
workspaces.key(s, {name = "enter"})
check(s.editor.name.text == "repo" and s.editor.folders[1] == "/tmp/ws-test/repo" and #s.editor.folders == 1, "Add as project prefills name and source")
check(s.editor.id == nil and s.editor.origin_mode == "workspaces", "prefill creates a new Workspaces-origin draft")
tf.set(s.editor.name, "Edited project")
action(s, "editor", "save"); flush(); nodes = view(s)
check(s.mode == "workspaces" and s.workspaces.active and not s.editor, "saving from Add as project stays in Workspaces")
check(projects[1].name == "Edited project", "the prefilled name remains editable")
check(s.sel == "project:1" and s.expanded["project:1"] and s.workspaces.selected == "project:1", "the saved project is expanded and selected")
check(s.workspaces.rows["checkout:1:/tmp/ws-test/repo:parent"] ~= nil and s.workspaces.rows["checkout:unassigned:/tmp/ws-test/repo:parent"] == nil, "saving refreshes and moves checkouts under their project")
check(s.workspaces.rows["checkout:unassigned:/tmp/ws-test/other:other"] ~= nil, "saving leaves unrelated checkouts unassigned")
action(s, "create"); flush()
check(s.editor.origin_mode == "workspaces" and s.editor.folders[1] == "/tmp/ws-test/header", "header Add project uses the existing folder prefill")
action(s, "editor", "save"); flush()
check(s.mode == "workspaces" and s.workspaces.selected == "project:2", "header Add project save stays in Workspaces and selects the project")
action(s, "create"); flush(); action(s, "editor", "cancel")
check(s.mode == "workspaces" and not s.editor, "header cancel returns to Workspaces")
workspace_action(s, "add-project", "/tmp/ws-test/other"); action(s, "editor", "cancel")
check(s.mode == "workspaces" and not s.editor, "Add as project cancel returns to Workspaces")
workspace_action(s, "fork", "other")
tf.set(s.workspaces.dialog.field, "new-child")
workspace_action(s, "dialog", "submit"); flush()
check(toasts[#toasts].title == "Workspace created" and toasts[#toasts].detail == "new-child", "Fork success names the new child, never the parent")
-- The workspace menu and keyboard both use Projects' launch flow, never a fixed harness.
settings.default_harness = "codex"
local before_harnesses = #calls
workspace_action(s, "menu", "other")
check(has_text(all_nodes({workspaces.layer(s)}), "New chat"), "workspace menu uses harness-neutral New chat text")
workspace_action(s, "dialog", "open"); flush()
local plan, surveys = nil, 0
for index = before_harnesses + 1, #calls do
    if calls[index].op == "launch-plan" then plan = calls[index] end
    if calls[index].op == "harnesses" then surveys += 1 end
end
check(plan and plan.harness == "codex" and plan.cwd == "/mount/other" and not plan.project_id and surveys == 0, "installed default launches directly in the checkout")
check(launched[#launched].cwd == "/mount/other" and launched[#launched].pane == 1 and not launched[#launched].resuming, "workspace launch reaches the normal origin-targeted fresh-chat boundary")
settings.default_harness = nil
workspace_action(s, "open", "other")
check(s.picker and s.picker.kind == "harness" and s.picker.subtitle_backup == "Start in other", "no default opens the standard picker and retains the checkout subtitle while checking tools")
flush()
check(s.picker.subtitle == "Start in other", "ready workspace picker shows the checkout subtitle")
action(s, "choose"); flush()
check(launched[#launched].cwd == "/mount/other" and launched[#launched].command == "codex --fixture", "picker choice launches in the checkout")
settings.default_harness = "omp"; launch_error = "OMP CLI is not installed"
workspace_action(s, "open", "other"); flush()
check(s.picker and s.picker.kind == "harness" and s.default_unavailable ~= nil, "unavailable default falls back through Projects' standard picker")
launch_error = nil; action(s, "close")
view(s)
s.workspaces.selected = "checkout:unassigned:/tmp/ws-test/other:other"
settings.default_harness = "codex"
panel.key(s, {name = "n", ctrl = true, shift = true}, cx)
check(s.picker and s.picker.subtitle_backup == "Start in other", "workspace New chat with shortcut uses the selected checkout")
flush()
check(s.picker.subtitle == "Start in other", "keyboard picker restores the checkout subtitle after tool discovery")
action(s, "close")
s.workspaces.selected = nil
for _, key in {{name = "n", ctrl = true}, {name = "n", meta = true}} do
    check(not workspaces.key(s, key), "New chat without a checkout falls through to the panel")
    local before_launch = #launched
    panel.key(s, key, cx); flush()
    check(#launched == before_launch + 1 and launched[#launched].cwd == s.ctx_cwd, "panel New chat still works without a selected workspace")
end
workspaces.leave(s)
local p = start("projects")
action(p, "create"); flush(); action(p, "editor", "save"); flush()
check(p.mode == "projects" and not p.editor and not p.workspaces.active, "Projects-origin saving retains Projects-mode behavior")
action(p, "create"); flush(); action(p, "editor", "cancel")
check(p.mode == "projects" and not p.editor, "Projects-origin cancellation remains unchanged")
print("Workspace frontend: " .. tostring(assertions) .. " assertions passed (no model or network)")
'''
    modules = []
    for name in ("textfield", "fmt", "input", "ui", "picker", "project-editor", "resume", "chat-launch", "workspaces", "panel"):
        path = root / ("blocks/panel.luau" if name == "panel" else f"lib/{name}.luau")
        modules.append(f'modules["{name}"] = (function()\n{path.read_text()}\nend)()')
    fixture = fixture.replace("__MODULES__", "\n".join(modules))
    with tempfile.TemporaryDirectory(prefix="workspaces-") as directory:
        file = Path(directory) / "workspaces.luau"
        file.write_text(fixture)
        return subprocess.run([runtime, str(file)], check=False).returncode


if __name__ == "__main__":
    raise SystemExit(main())
