#!/usr/bin/env python3
"""Exercise the window bridge that keeps Projects panels on their terminal's folder."""
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
    source = (Path(__file__).resolve().parent.parent / "lib/follow-cwd.luau").read_text()
    fixture = r'''
local assertions = 0
local function check(value, message)
    assert(value, message)
    assertions += 1
end
local hooks, routes = {}, {}
tern = {
    on = function(name, fn) hooks[name] = fn end,
    route = {link = function(fn) table.insert(routes, fn) end},
}
local PANEL = "tern-projects.panel"
local panes = {
    {pane = 100, kind = "plugin", block = PANEL, tab = 1, cwd = "/start"},
    {pane = 1, kind = "terminal", tab = 1, cwd = "/a"},
    {pane = 2, kind = "agent", tab = 1, cwd = "/b"},
    {pane = 4, kind = "file", tab = 1, cwd = "/file"},
    {pane = 200, kind = "plugin", block = PANEL, tab = 2, cwd = "/other"},
    {pane = 3, kind = "terminal", tab = 2, cwd = "/c"},
}
local sent, focused = {}, 0
local cx = {session = {
    panes = function() return panes end,
    focused = function() return focused end,
    event = function(_, pane, ev) table.insert(sent, {pane = pane, act = ev.act, value = ev.value}) end,
}}
local function find(id) for _, p in panes do if p.pane == id then return p end end end
local function take()
    local out = sent
    sent = {}
    return out
end
local function only(list, pane, value)
    return #list == 1 and list[1].pane == pane and list[1].act == "follow-cwd" and list[1].value == value
end
local function focus(id) focused = id; hooks.focus({pane = id, tab = find(id).tab}, cx) end
local function refresh(pane) return routes[1]({url = "tern-projects://follow?", pane = pane}, cx) end
local function load()
    hooks, routes, sent = {}, {}, {}
    local follow = (function()
__SOURCE__
    end)()
    follow.start()
    return follow
end

local follow = load()
follow.track(find(1))
focused = 100
hooks.cwd({pane = 1, path = "/a/sub"}, cx)
check(only(take(), 100, "/a/sub"), "a cd in the opening terminal moves its panel")
hooks.cwd({pane = 2, path = "/b/sub"}, cx)
check(#take() == 0, "another terminal's cd is ignored")
focus(2)
check(only(take(), 100, "/b"), "focusing another terminal in the tab follows it")
focus(2)
check(only(take(), 100, "/b"), "refocusing a terminal sends its folder again for a retargeted panel")
focus(100); focus(4)
check(#take() == 0, "focusing the panel or a non-terminal pane sends nothing")
hooks.cwd({pane = 1, path = "/a/again"}, cx)
check(#take() == 0, "the previously followed terminal no longer moves the panel")
focus(3)
check(only(take(), 200, "/c"), "a focus only moves the panel in its own tab")
focused = 100
find(2).cwd = "/b/later"
check(refresh(100).handled and only(take(), 100, "/b/later"), "Refresh sends the followed terminal's current folder")
find(2).cwd = ""
refresh(100)
check(#take() == 0, "an unknown folder is not sent")
find(2).cwd = "/b"
check(routes[1]({url = "tern-projects://resume?cwd=x", pane = 100}, cx) == nil, "other links are left to their routes")
refresh(1)
check(#take() == 0, "only a Projects panel can ask for its folder")
-- A focused terminal moved into the panel's tab is followed without a new focus event, and stays
-- followed once focus moves to the panel.
find(3).tab = 1; focused = 3
hooks.cwd({pane = 3, path = "/c/moved"}, cx)
check(only(take(), 100, "/c/moved"), "the focused terminal moved into the tab is followed")
find(3).cwd = "/c/moved"; focused = 100
refresh(100)
check(only(take(), 100, "/c/moved"), "the moved terminal stays followed after focusing the panel")
find(3).tab = 2; find(3).cwd = "/c"
-- Parked panes have no tab: a parked panel follows nothing, not even a parked terminal.
table.insert(panes, {pane = 300, kind = "plugin", block = PANEL, cwd = "/parked"})
table.insert(panes, {pane = 6, kind = "terminal", cwd = "/parked-shell"})
focused = 6
hooks.cwd({pane = 6, path = "/parked-shell/sub"}, cx)
refresh(300)
check(#take() == 0, "a parked panel follows no parked terminal")
table.remove(panes); table.remove(panes)

-- After a plugin reload nothing is remembered: only an unambiguous terminal is followed.
follow = load()
focused = 100
refresh(100)
check(#take() == 0, "two terminals in the tab leave the panel where it is")
refresh(200)
check(only(take(), 200, "/c"), "the tab's only terminal is followed")
hooks.cwd({pane = 3, path = "/c/sub"}, cx)
check(only(take(), 200, "/c/sub"), "the only terminal's cd moves the panel")
-- A terminal focused before its tab had a panel is the one a new panel follows.
table.insert(panes, {pane = 7, kind = "terminal", tab = 3, cwd = "/e"})
table.insert(panes, {pane = 8, kind = "terminal", tab = 3, cwd = "/f"})
focus(8); focus(4)
check(#take() == 0, "a tab without a panel is sent nothing")
table.insert(panes, {pane = 400, kind = "plugin", block = PANEL, tab = 3, cwd = "/start"})
focused = 400
refresh(400)
check(only(take(), 400, "/f"), "a terminal focused before the panel opened is followed")
print("Follow cwd: " .. tostring(assertions) .. " assertions passed (no terminal or window)")
'''.replace("__SOURCE__", source)
    with tempfile.TemporaryDirectory(prefix="follow-cwd-") as directory:
        file = Path(directory) / "follow-cwd.luau"
        file.write_text(fixture)
        return subprocess.run([runtime, str(file)], check=False).returncode


if __name__ == "__main__":
    raise SystemExit(main())
