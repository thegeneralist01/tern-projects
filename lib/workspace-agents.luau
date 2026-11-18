--!strict
-- Window-only bridge. Native Tern agent state, independent of the selected harness.
local fmt = require("./fmt")
local M = {}
local KEY = "workspace-results:v1"
local known: {[string]: any} = {}
local loaded = false
local enabled, routed = false, false

local function results(): any
	local ok, value = pcall(tern.kv.get, KEY)
	if not ok then return {} end
	return if type(value) == "table" then value else {}
end
local function panes(cx: any): any
	local out = {}
	for _, p in cx.session:panes() do out[tostring(p.pane)] = p end
	return out
end
local function acknowledge(marks: any, visible: any, tab: number?): boolean
	local changed = false
	for key, mark in marks do
		local pane = visible[key]
		local result_tab = if pane then pane.tab else mark.tab
		if tab ~= nil and result_tab == tab and mark.state ~= "idle" then
			mark.state = "idle"; changed = true
		end
	end
	return changed
end
function M.snapshot(cx: any): any
	local marks, visible, rows = results(), panes(cx), {}
	local focused = visible[tostring(cx.session:focused())]
	-- A result can arrive while its tab is already shown, without a new focus event.
	if acknowledge(marks, visible, if focused then focused.tab else nil) then tern.kv.set(KEY, marks) end
	for _, agent in cx.agents:list() do
		local key = tostring(agent.pane)
		local row = {pane = agent.pane, cwd = agent.cwd, state = agent.state, live = agent.state ~= "exited", title = "Agent"}
		local p = visible[key]
		if p then row.title = p.title or "Agent"; row.tab = p.tab end
		if agent.state ~= "working" and agent.state ~= "waiting_input" and marks[key] then row.state = marks[key].state end
		known[key] = row
		rows[key] = row
	end
	-- Tern can retain an exited agent pane after removing it from agents:list().
	-- Without a recorded finish event its result is unknown, never inferred from old terminal text.
	for key, pane in visible do
		if pane.kind == "agent" and not rows[key] and not marks[key] then
			rows[key] = {pane = pane.pane, cwd = pane.cwd, state = "idle", live = pane.exited == nil, title = pane.title or "Agent", tab = pane.tab}
			known[key] = rows[key]
		end
	end
	for key, mark in marks do
		if not rows[key] then
			rows[key] = {pane = mark.pane, cwd = mark.cwd, state = mark.state, live = false, title = mark.title or "Agent", tab = mark.tab}
			known[key] = rows[key]
		end
	end
	local list = {}
	for _, row in rows do table.insert(list, row) end
	table.sort(list, function(a, b) return a.pane < b.pane end)
	return list
end
local function publish(cx: any)
	local rows = M.snapshot(cx)
	for _, p in cx.session:panes() do
		if p.block == "tern-projects.panel" then
			local focused = panes(cx)[tostring(cx.session:focused())]
			local payload = tern.json.encode({agents = rows, visible = focused ~= nil and focused.tab == p.tab})
			cx.session:event(p.pane, {ev = "action", id = "main", act = "workspace-agents", value = payload})
		end
	end
end
local function activate()
	enabled = true
	if loaded then return end
	loaded = true
	tern.on("agent_state", function(ev: any, cx: any)
		if not enabled or not cx then return end
		if ev.new == "working" or ev.new == "waiting_input" then
			local marks, key = results(), tostring(ev.pane)
			if marks[key] then marks[key] = nil; tern.kv.set(KEY, marks) end
		end
		publish(cx)
	end)
	tern.on("agent_finished", function(ev: any, cx: any)
		if not enabled or not cx then return end
		local key = tostring(ev.pane)
		local row = known[key]
		if not row then M.snapshot(cx); row = known[key] end
		if row then
			local marks = results()
			local state = if ev.result == "failed" then "failed" elseif ev.result == "done" or ev.result == "exited" then "done" else "idle"
			if ev.result == "closed" and marks[key] then state = marks[key].state end
			-- An exited process is only failed when Tern reports its nonzero status.
			for _, p in cx.session:panes() do if p.pane == ev.pane and type(p.exited) == "number" and p.exited ~= 0 then state = "failed" end end
			marks[key] = {pane = ev.pane, cwd = row.cwd, title = row.title, state = state, tab = row.tab}
			local visible = panes(cx)
			local focused = visible[tostring(cx.session:focused())]
			-- Successful commands can close their pane before this event is delivered.
			acknowledge(marks, visible, if focused then focused.tab else nil)
			tern.kv.set(KEY, marks)
		end
		publish(cx)
	end)
	tern.on("focus", function(ev: any, cx: any)
		if not enabled or not cx then return end
		-- Seeing a tab acknowledges its finished marks, not only its focused split.
		local marks = results()
		if acknowledge(marks, panes(cx), ev.tab) then tern.kv.set(KEY, marks) end
		publish(cx)
	end)
	tern.on("pane_closed", function(_ev: any, cx: any) if enabled and cx then publish(cx) end end)
end
function M.start()
	if routed then return end
	routed = true
	-- Register only a route until a detected Workspaces view asks for agent state.
	-- No startup timer, pane enumeration, KV reads or agent subscriptions when unused.
	tern.route.link(function(link: any, cx: any)
		local q = string.match(link.url, "^tern%-projects://workspace%?(.*)$")
		if not q or not link.pane then return nil end
		if not cx then return {handled = true} end
		local origin = panes(cx)[tostring(link.pane)]
		if not origin or origin.block ~= "tern-projects.panel" or cx.hosts:current().kind ~= "local" then return {handled = true} end
		if q == "disable=1" then enabled = false; return {handled = true} end
		local ok, availability = pcall(tern.kv.get, "workspace-availability:v1")
		if not ok or type(availability) ~= "table" or availability.available ~= true then return {handled = true} end
		activate()
		local focus = tonumber(string.match(q, "focus=(%d+)"))
		if focus then
			for _, row in M.snapshot(cx) do
				if row.pane == focus then
					if panes(cx)[tostring(focus)] then cx.layout:focus(focus)
					else cx:toast("info", "Agent pane is closed", "Its last recorded result is retained in Workspaces.") end
					break
				end
			end
		else
			local nonce = fmt.unpct(string.match(q, "nonce=([^&]*)") or "")
			local focused = panes(cx)[tostring(cx.session:focused())]
			cx.session:event(link.pane, {ev = "action", id = "main", act = "workspace-agents", value = tern.json.encode({agents = M.snapshot(cx), nonce = nonce, visible = focused ~= nil and focused.tab == origin.tab})})
		end
		return {handled = true}
	end)
end
return M
