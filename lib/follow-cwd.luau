--!strict
-- Window-only: each Projects panel follows the folder of the terminal or agent last focused in its tab,
-- like Tern's Files pane. The panel ignores a folder it already shows.
local M = {}
local KIND = "tern-projects.panel"
-- Tab id -> its last focused shell, recorded whether or not the tab has a panel yet.
local last: {[number]: number} = {}
local loaded = false

local function shell(p: any): boolean
	return p ~= nil and p.tab ~= nil and (p.kind == "terminal" or p.kind == "agent")
end
local function send(cx: any, panel: number, cwd: any)
	if type(cwd) == "string" and cwd ~= "" then
		cx.session:event(panel, {ev = "action", id = "main", act = "follow-cwd", value = cwd})
	end
end
-- The focused shell when it is in the panel's tab (it may have moved there without a new focus),
-- else the tab's last focused shell, else the tab's only shell (nothing is remembered after a reload).
-- A parked panel has no tab and follows nothing.
local function source(cx: any, panes: {any}, panel: any): any
	if panel.tab == nil then return nil end
	local focused, id = cx.session:focused(), last[panel.tab]
	local remembered: any, only: any, count = nil, nil, 0
	for _, p in panes do
		if p.tab == panel.tab and shell(p) then
			if p.pane == focused then last[panel.tab] = p.pane; return p end
			if p.pane == id then remembered = p end
			only = p; count += 1
		end
	end
	return remembered or (if count == 1 then only else nil)
end

-- The pane a panel was opened or retargeted from, in case no focus was seen since a reload.
function M.track(from: any)
	if shell(from) then last[from.tab] = from.pane end
end

function M.start()
	if loaded then return end
	loaded = true
	tern.on("focus", function(ev: any, cx: any)
		if not cx then return end
		local panes = cx.session:panes()
		local focused: any = nil
		for _, p in panes do if p.pane == ev.pane then focused = p end end
		if not shell(focused) then return end
		last[focused.tab] = focused.pane
		-- Sent even when unchanged: the panel may have been retargeted from another pane meanwhile.
		for _, p in panes do
			if p.block == KIND and p.tab == focused.tab then send(cx, p.pane, focused.cwd) end
		end
	end)
	tern.on("cwd", function(ev: any, cx: any)
		if not cx then return end
		local panes = cx.session:panes()
		for _, p in panes do
			if p.block == KIND then
				local from = source(cx, panes, p)
				if from and from.pane == ev.pane then send(cx, p.pane, ev.path) end
			end
		end
	end)
	-- Refresh asks again, so a missed change or a reload never leaves the panel on an old folder.
	tern.route.link(function(link: any, cx: any)
		if not string.match(link.url, "^tern%-projects://follow%?") then return nil end
		local panes = cx.session:panes()
		for _, p in panes do
			if p.pane == link.pane and p.block == KIND then
				local from = source(cx, panes, p)
				if from then send(cx, p.pane, from.cwd) end
			end
		end
		return {handled = true}
	end)
end

return M
