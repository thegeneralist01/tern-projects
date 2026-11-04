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

local LAUNCH_CLAIM_LIFETIME = 10 -- seconds; must match the panel's one-shot claim lifetime
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

-- The installer reloads the plugin and can close its panel. Deliver from the
-- window instead, acknowledging separately so updater preferences stay host-owned.
local function update_toast(cx: SessionContext, remaining: number)
	local ok, record = pcall(tern.kv.get, "plugin-updates:v1")
	if not ok or type(record) ~= "table" then return end
	if record.pending then
		if remaining > 0 then
			tern.timer(2000, function(next_cx: SessionContext)
				update_toast(next_cx, remaining - 1)
			end)
		end
		return
	end
	if record.failure then return end
	local version = record.completed_version
	if type(version) ~= "string" then return end
	local read, acknowledged = pcall(tern.kv.get, "plugin-update-toast:v1")
	if not read or acknowledged == version then return end
	-- Record delivery before showing to avoid replay on subsequent reloads.
	local saved, err = pcall(tern.kv.set, "plugin-update-toast:v1", version)
	if not saved then
		tern.log.error("Cannot acknowledge plugin update toast: " .. tostring(err))
		return
	end
	cx:toast("success", "Updated!", "Projects " .. version)
end

tern.on("window_start", function(cx: SessionContext)
	migrate_keybinds(cx)
end)

-- Module load also runs on plugin reload; window_start may already have fired.
-- Poll only while an installation is pending, for at most 130 seconds.
tern.timer(500, function(cx: SessionContext)
	update_toast(cx, 65)
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
	local key = ""
	local pending: any = nil
	if route == "launch" then
		local nonce = unpct(string.match(q, "n=([^&]*)") or "")
		if nonce == "" then return { handled = true } end
		key = "launch:" .. nonce
		local ok, claim = pcall(tern.kv.get, key)
		if not ok or type(claim) ~= "table" then return { handled = true } end
		pending = claim
	end
	-- A claim never substitutes for a live Projects panel in this window. In particular,
	-- a link credited to a newly focused pane cannot authorize a different pane's launch.
	if origin == nil or origin.block ~= KIND or (pending and pending.pane ~= link.pane) then
		if pending then cx:toast("error", "Chat not started", "Focus the Projects panel and try again.") end
		return { handled = true }
	end
	if cx.session:focused() ~= link.pane then
		if pending then cx:toast("error", "Chat not started", "Focus the Projects panel and try again.") end
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
	if pending then
		local created = pending.created
		local age = if type(created) == "number" then os.time() - created else LAUNCH_CLAIM_LIFETIME
		if age < 0 or age >= LAUNCH_CLAIM_LIFETIME or pending.cwd ~= cwd or pending.command ~= cmd then
			return { handled = true }
		end
		-- Only the matching, authorized window consumes this nonce. Reject replay even
		-- when tab creation fails, and fail closed if removing the claim fails.
		local consumed = pcall(tern.kv.set, key, nil)
		if not consumed then
			cx:toast("error", "Chat not started", "Cannot consume the chat launch authorization. Try again.")
			return { handled = true }
		end
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
