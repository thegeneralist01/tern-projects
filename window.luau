--!strict
-- tern-projects window half: commands, toggle logic, and guarded launch routes.
-- iOS has no tern.process; skip setup on that platform.
if tern.runtime.os == "ios" then
	return
end

local fmt = require("./lib/fmt")
local unpct = fmt.unpct
local pct = fmt.pct

local KIND = "tern-projects.panel"
local WIDTH = 44

-- Per-pane state for toggle tracking.
local last_mode: { [number]: string } = {}
local prev_focus: { [number]: number } = {}

-- Look a pane up through the window's pane list: cx.session:pane(id) can return nil for a live pane.
local function find_pane(cx: SessionContext, id: number?): PaneInfo?
	if id == nil then return nil end
	for _, p in cx.session:panes() do
		if p.pane == id then return p end
	end
	return nil
end

-- Toggle or open the projects panel in `mode` for the current tab.
local function toggle(cx: SessionContext, mode: string)
	local f = cx.session:focused()
	local fi = find_pane(cx, f)
	local tab = fi ~= nil and fi.tab or nil
	if tab == nil then
		return
	end

	-- Find an existing panel pane on this tab.
	local panel: number? = nil
	for _, p in cx.session:panes() do
		if p.block == KIND and p.tab == tab then
			panel = p.pane
			break
		end
	end

	if panel ~= nil then
		if f == panel and last_mode[panel] == mode then
			-- Same key again while panel is focused: close it.
			cx.layout:close(panel)
			local pf = prev_focus[panel]
			if pf ~= nil then
				-- Try to refocus the previous pane.
				for _, p in cx.session:panes() do
					if p.pane == pf then
						cx.layout:focus(pf)
						break
					end
				end
			end
			return
		end
		-- Retarget the existing panel to the new mode/cwd.
		local new_cwd = (f ~= panel and fi ~= nil) and (fi.cwd or "") or ""
		cx.session:event(panel, {
			ev = "action",
			id = "main",
			act = "open=" .. mode .. ":" .. new_cwd,
			value = mode .. ":" .. new_cwd,
		})
		cx.layout:focus(panel)
		last_mode[panel] = mode
		if f ~= panel then
			prev_focus[panel] = f :: number
		end
		return
	end

	-- Create a new panel.
	pcall(tern.fs.remove, tern.plugin.data .. "/panel-cols")
	local new_cwd = fi ~= nil and (fi.cwd or "") or ""
	local new_pane = cx.layout:split(f :: number, "left", { block = KIND, args = { mode, new_cwd } })
	if new_pane == nil then
		-- Tern 0.9.5 cannot resolve some live pane ids passed back to split; open beside the focused pane instead.
		new_pane = cx:new_block(KIND, { mode, new_cwd }, "split")
	end
	if new_pane == nil then
		cx:toast("error", "Projects block unavailable", "tern plugin list")
		return
	end
	last_mode[new_pane] = mode
	prev_focus[new_pane] = f :: number

	-- After the block reports its width, resize the split.
	tern.timer(400, function(cx2: SessionContext)
		local ok, data = pcall(tern.fs.read, tern.plugin.data .. "/panel-cols")
		if not ok then
			return
		end
		local c = tonumber(data)
		if c ~= nil and c > WIDTH + 2 then
			cx2.layout:resize(new_pane :: number, "right", -(c - WIDTH))
		end
	end)
end

-- User overrides take precedence over plugin defaults. Migrate only exact old actions.
local function migrate_keybinds(cx: SessionContext)
	local replacements = {
		["plugin.tern-sessions.sessions"] = "plugin.tern-projects.projects",
		["plugin.tern-sessions.search"] = "plugin.tern-projects.search",
	}
	for chord, value in cx.settings:keybinds() do
		if type(value) == "string" then
			local replacement = replacements[value]
			if replacement ~= nil then cx.settings:bind(chord, replacement) end
		else
			local changed = false
			local actions: { string } = {}
			for _, action in value do
				local replacement = replacements[action]
				table.insert(actions, replacement or action)
				if replacement ~= nil then changed = true end
			end
			if changed then cx.settings:bind(chord, actions) end
		end
	end
end

tern.on("window_start", function(cx: SessionContext)
	migrate_keybinds(cx)
end)

tern.command({
	id = "projects",
	title = "Projects",
	icon = "folder",
	keys = { if tern.runtime.os == "macos" then "cmd+alt+s" else "ctrl+alt+s" },
	run = function(cx)
		migrate_keybinds(cx)
		toggle(cx, "projects")
	end,
})

tern.command({
	id = "search",
	title = "Projects: Search",
	icon = "search",
	keys = { if tern.runtime.os == "macos" then "ctrl+alt+cmd+s" else "ctrl+alt+shift+s" },
	run = function(cx)
		migrate_keybinds(cx)
		toggle(cx, "search")
	end,
})

tern.command({
	id = "settings",
	title = "Projects: Settings",
	icon = "settings",
	run = function(cx) toggle(cx, "settings") end,
})

-- Clean up tracking state when a pane closes.
tern.on("pane_closed", function(ev: any, _cx: any)
	last_mode[ev.pane] = nil
	prev_focus[ev.pane] = nil
end)

-- Host-generated commands must execute only on the originating local window.
tern.route.link(function(link: any, cx: SessionContext)
	local route, q = string.match(link.url, "^tern%-projects://([a-z]+)%?(.*)$")
	if route ~= "resume" and route ~= "launch" then return nil end
	if link.pane == nil then return { handled = true } end
	local origin = find_pane(cx, link.pane)
	-- The panel records its own launch: the window credits the focused pane, which is not the
	-- panel once focus moved, so the URL alone cannot prove where it came from.
	local nonce = unpct(string.match(q, "n=([^&]*)") or "")
	local pending = if nonce ~= "" then tern.kv.get("launch") else nil
	local claimed = type(pending) == "table" and pending.nonce == nonce
	if claimed then tern.kv.set("launch", nil) end
	if claimed then
		if pending.pane ~= link.pane or cx.session:focused() ~= link.pane then
			cx:toast("error", "Chat not started", "Focus the Projects panel and try again.")
			return { handled = true }
		end
	elseif origin == nil or origin.block ~= KIND then
		-- A custom URL in unrelated terminal/chat content is not a launch request.
		return { handled = true }
	elseif cx.session:focused() ~= link.pane then
		return { handled = true }
	end
	if cx.hosts:current().kind ~= "local" then
		cx:toast("error", "Cannot launch a local chat on a remote host", "Switch to the local host first.")
		return { handled = true }
	end
	local cwd = unpct(string.match(q, "cwd=([^&]*)") or "")
	local cmd = unpct(string.match(q, "cmd=([^&]*)") or "")
	if cmd == "" or cwd == "" or string.find(cwd, "[%z\r\n]") or string.find(cmd, "[%z\r\n]") then
		cx:toast("error", "Invalid chat launch", "The working directory or command is missing or invalid.")
		return { handled = true }
	end
	local ok, pane = pcall(function()
		return cx.layout:new_tab({ cwd = cwd, command = cmd })
	end)
	if not ok or pane == nil then
		cx:toast("error", "Could not open chat tab", if ok then "Tern did not create a tab." else tostring(pane))
	end
	return { handled = true }
end)

-- Suppress unused warning for pct (used by blocks/panel.luau via lib/fmt.luau).
local _ = pct
