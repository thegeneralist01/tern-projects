#!/usr/bin/env python3
"""Exercise the actual window agent bridge without a terminal, model or configuration."""
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
    source = (Path(__file__).resolve().parent.parent / "lib/workspace-agents.luau").read_text()
    source = source.replace('local fmt = require("./fmt")', 'local fmt = {unpct = function(s) return s end}', 1)
    fixture = r'''
local store, hooks = {}, {}
local link_handler, reads, scans, timers = nil, 0, 0, 0
local result_read_failure = false
local function copy(value)
    if type(value) ~= "table" then return value end
    local out = {}
    for k, v in value do out[k] = copy(v) end
    return out
end
local assertions = 0
local function check(value, message)
    assert(value, message)
    assertions += 1
end
tern = {
    kv = {get = function(k) reads += 1; if result_read_failure and k == "workspace-results:v1" then error("fixture KV read failure") end; return copy(store[k]) end, set = function(k, v) store[k] = copy(v) end},
    on = function(k, fn) hooks[k] = fn end,
    timer = function() timers += 1 end,
    route = {link = function(fn) link_handler = fn end},
    json = {encode = function() return "{}" end},
}
local pane = {pane = 1, kind = "agent", cwd = "/mount/test", title = "mock", exited = 0, tab = 10}
local active, focused = {}, 2
local pane_present = true
local cx = {
    hosts = {current = function() return {kind = "local"} end},
    agents = {list = function() return active end},
    session = {
        panes = function()
            scans += 1
            local out = {{pane = 2, kind = "terminal", cwd = "/outside", tab = 20}, {pane = 3, kind = "terminal", tab = 10}, {pane = 4, block = "tern-projects.panel", tab = 20}}
            if pane_present then table.insert(out, pane) end
            return out
        end,
        focused = function() return focused end,
        event = function() end,
    },
}
local function load_bridge()
    return (function()
__SOURCE__
    end)()
end
local function reload_bridge()
    local value = load_bridge()
    value.start()
    link_handler({url = "tern-projects://workspace?nonce=fixture", pane = 4}, cx)
    return value
end
local bridge = load_bridge()
bridge.start()
check(reads == 0 and scans == 0 and timers == 0 and next(hooks) == nil, "unused bridge has no KV reads, pane scans, timers or native subscriptions")
check(link_handler({url = "https://example.invalid", pane = 4}, cx) == nil and scans == 0, "unrelated routes do not enumerate panes")
link_handler({url = "tern-projects://workspace?nonce=fixture", pane = 4}, cx)
check(next(hooks) == nil, "missing capability cannot activate tracking through a link")
store["workspace-availability:v1"] = {available = true}
link_handler({url = "tern-projects://workspace?nonce=fixture", pane = 4}, cx)
check(hooks.agent_state ~= nil and hooks.agent_finished ~= nil and timers == 0, "a detected Workspaces view lazily subscribes without a startup timer")
check(bridge.snapshot(cx)[1].state == "idle", "unknown finished result must be idle")
check(not bridge.snapshot(cx)[1].live, "exited pane is not a live agent")
check(#bridge.snapshot(cx) == 1, "ordinary terminal panes are not agents")
hooks.agent_finished({pane = 1, result = "done"}, cx)
check(bridge.snapshot(cx)[1].state == "done", "only finish events record completion")
bridge = reload_bridge()
check(bridge.snapshot(cx)[1].state == "done", "finish result survives module reload")
focused = 1
hooks.focus({pane = 1, tab = 10}, cx)
focused = 2
bridge = reload_bridge()
check(bridge.snapshot(cx)[1].state == "idle", "focus acknowledgement is persisted across reload")
pane.exited = 7
hooks.agent_finished({pane = 1, result = "exited"}, cx)
check(bridge.snapshot(cx)[1].state == "failed", "nonzero native exit is failed")
hooks.agent_finished({pane = 1, result = "closed"}, cx)
check(bridge.snapshot(cx)[1].state == "failed", "closing a failed pane does not overwrite its result")
for _, state in {"working", "waiting_input"} do
    pane.exited = nil
    active = {{pane = 1, cwd = pane.cwd, state = state}}
    hooks.agent_state({pane = 1, new = state}, cx)
    check(bridge.snapshot(cx)[1].state == state, "restart uses native state")
    check(store["workspace-results:v1"]["1"] == nil, "restart persists removal of the old finish result")
    active[1].state = "idle"
    bridge = reload_bridge()
    check(bridge.snapshot(cx)[1].state == "idle", "restart clearing survives reload without a native-state mask")
    active = {}
    pane.exited = 0
    hooks.agent_finished({pane = 1, result = "done"}, cx)
end
focused = 3 -- The agent's tab is shown, but its other split has focus.
for _, result in {"done", "failed"} do
    pane.exited = if result == "failed" then 7 else 0
    hooks.agent_finished({pane = 1, result = result}, cx)
    check(store["workspace-results:v1"]["1"].state == "idle", "finish in an already shown tab is acknowledged immediately")
    focused = 2
    bridge = reload_bridge()
    check(bridge.snapshot(cx)[1].state == "idle", "already-shown acknowledgement survives reload")
    focused = 3
end
store["workspace-results:v1"]["1"].state = "done"
check(bridge.snapshot(cx)[1].state == "idle", "publishing also acknowledges an already shown result")
focused = 2
bridge = reload_bridge()
check(bridge.snapshot(cx)[1].state == "idle", "publish acknowledgement is persisted across reload")
focused = 3
pane_present = false -- Successful commands may auto-close before agent_finished.
hooks.agent_finished({pane = 1, result = "done"}, cx)
check(store["workspace-results:v1"]["1"].state == "idle", "auto-closed result remembers its already-shown tab")
focused = 2
bridge = reload_bridge()
check(bridge.snapshot(cx)[1].state == "idle", "auto-close acknowledgement survives reload")
pane_present = true; pane.exited = nil
for _, harness in {"codex", "claude", "pi", "opencode", "cursor", "custom-agent"} do
    active = {{pane = 1, cwd = pane.cwd, state = "working", harness = harness}}
    hooks.agent_state({pane = 1, new = "working"}, cx)
    check(bridge.snapshot(cx)[1].state == "working" and bridge.snapshot(cx)[1].live, "native agent tracking is independent of harness: " .. harness)
end
result_read_failure = true
check(pcall(function() return bridge.snapshot(cx) end), "a failed result KV read is treated as unknown agent results")
check(pcall(function() hooks.focus({pane = 2, tab = 20}, cx) end), "a failed result KV read cannot raise from native focus hooks")
result_read_failure = false
link_handler({url = "tern-projects://workspace?disable=1", pane = 4}, cx)
local idle_reads, idle_scans = reads, scans
hooks.agent_state({pane = 1, new = "waiting_input"}, cx)
hooks.agent_finished({pane = 1, result = "done"}, cx)
hooks.focus({pane = 1, tab = 10}, cx)
hooks.pane_closed({pane = 1}, cx)
check(reads == idle_reads and scans == idle_scans, "removal makes all previously installed event handlers inert")
print("Workspace agent bridge: " .. tostring(assertions) .. " assertions passed (no model or network)")
'''.replace("__SOURCE__", source)
    with tempfile.TemporaryDirectory(prefix="workspace-agents-") as directory:
        file = Path(directory) / "agent-bridge.luau"
        file.write_text(fixture)
        return subprocess.run([runtime, str(file)], check=False).returncode


if __name__ == "__main__":
    raise SystemExit(main())
