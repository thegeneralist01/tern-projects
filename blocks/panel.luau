--!strict
-- Durable library metadata comes from the backend, never pane persistence.
local ui = tern.ui
local cli = require("../lib/cli")
local fmt = require("../lib/fmt")
local tf = require("../lib/textfield")
local input = require("../lib/input")
local editor = require("../lib/project-editor")
local picker = require("../lib/picker")
local c = require("../lib/ui")

local PAGE = 150 -- rendered list rows; Tern virtualizes (and unwires row actions) past 200
local SEARCH_PAGE = 50
local NAME_FIELD = "layer.editor.form.name"
local SEARCH_FIELD = "main.searchbox.query"
local NULL = (tern.json :: any).null

local function pane_alive(id: number): boolean
	for _, p in tern.pane.list() do if p.pane == id then return true end end
	return false
end
local function live(s: any): boolean return s.alive and pane_alive(s.cx.pane) end
local function toast(s: any, level: any, text: string, sub: string?) s.cx:toast(level, text, sub) end
local function project_key(s: any, p: any): string return if s.view == "mine" then "project:" .. tostring(p.id) else "discovered:" .. tostring(p.root) end
local function project_name(p: any): string return p.name or p.label or fmt.basename(p.root or "") end
local function navigable(row: any): boolean return row.kind ~= "header" and row.kind ~= "note" and row.kind ~= "card" end
local function scope_label(s: any): string
	local path = fmt.path(s.search_cwd or s.ctx_cwd)
	local scoped = if s.scope == "all" then "All chats" else "In " .. path
	return if s.project_id then (s.project_label or "Project") .. (if s.scope ~= "all" then " · " .. scoped else "") else scoped
end
-- Shorten only at folder boundaries; native truncation handles a single long basename.
local function display_path(path: string, budget: number): string
	local shown = fmt.path(path)
	if #shown <= budget then return shown end
	local rest = string.gsub(shown, "^[/\\]+", "")
	while #rest + 2 > budget do
		local next = string.match(rest, "^[^/\\]+[/\\](.+)$")
		if not next then break end
		rest = next
	end
	return "…/" .. rest
end

-- Rows -----------------------------------------------------------------------------------

local function rebuild(s: any)
	local old_index = table.find(s.order, s.sel)
	local rows, order = {}, {}
	local function add(k: string, kind: string, ref: any, parent: string?)
		rows[k] = {kind = kind, ref = ref, parent = parent}; table.insert(order, k)
	end
	local function card(k: string, ref: any)
		add(k, "card", ref)
		for _, b in ref.buttons or {} do add(b.row, "cardbtn", b, k) end
	end
	if s.mode == "projects" then
		if #s.pinned > 0 then
			add("h:pinned", "header", {label = "Pinned"})
			for _, chat in s.pinned do add("pin:" .. chat.uid, "session", chat) end
		end
		add("h:projects", "header", {label = if s.view == "mine" then "Projects" else "Folders"})
		if s.loaded and #s.projects == 0 then
			if s.view == "mine" then
				card("card:first", {icon = "folder-plus", title = "No projects yet", body = "Group chats from one or more folders, like an app and its API, into a project.", buttons = {
					{row = "create", label = "Create project", act = "create", variant = "primary", icon = "plus"},
					{row = "discover", label = "Browse discovered folders", act = "view=discovered", variant = "ghost"},
				}})
			else
				card("card:nofolders", {icon = "folder", title = "No folders yet", body = "Chats appear here grouped by the folder they started in."})
			end
		end
		for _, p in s.projects do
			local key = project_key(s, p)
			add(key, "project", p)
			if s.expanded[key] then
				local chats = p.sessions or {}
				if #chats == 0 then add(key .. ":start", "start", p, key) end
				local shown = math.min(#chats, if s.more[key] then 25 else 5)
				for i = 1, shown do local chat = chats[i]; add(key .. ":" .. chat.uid, "session", chat, key) end
				if #chats > shown then add(key .. ":more", "more", {count = math.min(#chats, 25) - shown}, key) end
				if s.more[key] and shown > 5 then add(key .. ":fewer", "fewer", p, key) end
				if (p.count or 0) > shown and (s.more[key] or #chats <= shown) then add(key .. ":older", "older", p, key) end
			end
		end
		add("h:recent", "header", {label = "Recents"})
		if s.loaded and #s.recent == 0 then add("recent:none", "note", {label = "No chats yet"}) end
		for _, chat in s.recent do add("recent:" .. chat.uid, "session", chat) end
	elseif s.mode == "search" then
		for _, chat in s.results do add("search:" .. chat.uid, "session", chat) end
		if s.loaded and #s.results == 0 then
			local q = s.query.text
			local buttons = {}
			if s.scope ~= "all" then table.insert(buttons, {row = "widen", label = if q ~= "" then "Search all chats" else "Show all chats", act = "scope=all", variant = "primary", icon = "search"}) end
			if s.project_id then table.insert(buttons, {row = "unfilter", label = "Clear project filter", act = "filter-project=all", variant = "ghost"}) end
			local title = if q ~= "" then "No matches for “" .. q .. "”" else "No chats"
			local body = if s.scope ~= "all" or s.project_id then scope_label(s) .. (if q ~= "" then " · No matches." else " · No chats yet.") else (if q ~= "" then "Try different words." else "Chats appear after indexing.")
			card("card:noresults", {icon = "search", title = title, body = body, buttons = buttons})
		end
	end
	s.rows, s.order = rows, order
	s.page = math.min(s.page, math.max(0, math.ceil(#order / PAGE) - 1))
	if s.sel and not rows[s.sel] then
		s.sel = nil
		local idx = math.max(1, math.min(old_index or 1, #order))
		for i = idx, #order do if navigable(rows[order[i]]) then s.sel = order[i]; break end end
		if not s.sel then for i = idx - 1, 1, -1 do if navigable(rows[order[i]]) then s.sel = order[i]; break end end end
		if s.sel then s.page = math.floor(((table.find(order, s.sel) or 1) - 1) / PAGE) end
	end
end

-- Requests -------------------------------------------------------------------------------

local reload: (any) -> ()
local action: (any, string, string?) -> ()
local end_launch_wait: (any) -> ()
local cancel_flow: (any) -> ()

-- Launch waiting: New chat reports a stage and a deadline instead of going quiet ------
local LAUNCH_TIMEOUT = {["launch-plan"] = 15000, harnesses = 65000}

end_launch_wait = function(s: any)
	for _, timer in s.launch_timers or {} do pcall(function() timer:cancel() end) end
	s.launch_timers = nil
	s.launch_wait_seq = (s.launch_wait_seq or 0) + 1
	s.launch_pending = false
	s.launch_slow = false
	s.launch_phase = nil
	-- The picker's subtitle is the only line a loading sheet shows.
	local p = s.launch_picker
	if p and p.subtitle_backup then p.subtitle = p.subtitle_backup; p.subtitle_backup = nil end
	s.launch_picker = nil
end

cancel_flow = function(s: any)
	s.flow = (s.flow or 0) + 1
	s.picker = nil; s.menu = nil; s.confirm = nil; s.pending_plan = nil
	end_launch_wait(s)
end

-- A picker names its own stage; the panel banner names the launch itself.
local function wait_text(s: any, in_picker: boolean): string
	if s.launch_slow then return "Still preparing. You can cancel and retry." end
	if not in_picker then return "Preparing new chat…" end
	return if s.launch_phase == "harnesses" then "Checking installed tools…" else "Preparing launch…"
end

local function launch_timeout_text(): string
	return if cli.status() then "The backend is still building. Retry when the build finishes." else "Preparing new chat timed out. Retry or choose another tool."
end

local function begin_launch_wait(s: any, phase: string)
	end_launch_wait(s)
	local seq = s.launch_wait_seq
	s.launch_pending = true
	s.launch_slow = false
	s.launch_phase = phase
	-- A new attempt supersedes the banner of the last one.
	s.error = nil; s.error_reload = false
	if s.picker then
		s.launch_picker = s.picker
		s.picker.subtitle_backup = s.picker.subtitle
		s.picker.subtitle = wait_text(s, true)
	end
	local function current(): boolean return live(s) and s.launch_pending and s.launch_wait_seq == seq end
	s.launch_timers = {
		tern.timer(5000, function()
			if not current() then return end
			s.launch_slow = true
			if s.picker then s.picker.subtitle = wait_text(s, true) end
			s.cx:render()
		end),
		tern.timer(LAUNCH_TIMEOUT[phase], function()
			if not current() then return end
			local p = s.picker
			local text = launch_timeout_text()
			cancel_flow(s)
			-- A picker stays open on the failure, so another tool remains reachable.
			if p then p.state = "error"; p.message = text; s.picker = p; picker.settle(p)
			else s.error = text; s.error_reload = false end
			toast(s, "error", "Couldn't prepare new chat", text)
			s.cx:render()
		end),
	}
	s.cx:render()
end

local function request(s: any, payload: any, cb: (any) -> (), channel: string?, on_err: ((string) -> ())?)
	local key = channel or "request"
	s.tokens[key] = (s.tokens[key] or 0) + 1
	local token = s.tokens[key]
	local mode, flow = s.mode, s.flow or 0
	local function current(): boolean
		return live(s) and s.tokens[key] == token and s.mode == mode and (s.flow or 0) == flow
	end
	cli.request(payload, function(data, err)
		if not live(s) or s.tokens[key] ~= token then return end
		if key ~= "mutation" and (s.mode ~= mode or (s.flow or 0) ~= flow) then s.cx:render(); return end
		if err then
			if key == "mutation" then s.busy = false end
			if on_err then on_err(err) else s.error = err; s.error_reload = false end
		else cb(data) end
		s.cx:render()
	end, if key == "launch" or key == "harnesses" then current else nil)
end
local function mutate(s: any, payload: any, done: ((any) -> ())?, on_err: ((string) -> ())?)
	if s.busy then return end
	local conf, p = s.confirm, s.picker
	local verb = if payload.op == "delete" then "Deleting…" elseif payload.op == "project-remove" then "Removing…" elseif payload.assignment then "Moving…" elseif payload.archived == false then "Restoring…" else "Saving…"
	s.busy = true; s.busy_label = verb
	if conf then conf.error = nil end
	if p then p.message = nil; p.busy_label = verb end
	request(s, payload, function(data)
		s.busy = false; s.busy_label = nil; s.error = nil
		if done then done(data) else toast(s, "success", "Changes saved") end
		reload(s)
	end, "mutation", function(err)
		s.busy_label = nil
		if conf and s.confirm == conf then conf.error = fmt.error(err)
		elseif p and s.picker == p then p.message = fmt.error(err)
		else s.error = err; s.error_reload = false end
		toast(s, "error", "Couldn't complete the change", fmt.error(err))
		if on_err then on_err(err) end
	end)
end
reload = function(s: any)
	s.gen += 1
	local gen = s.gen
	local function accept(data: any, err: string?)
		if not live(s) or s.gen ~= gen then return end
		s.loading = false
		if err then s.error = err; s.error_reload = true else
			s.error = nil; s.loaded = true
			if s.mode == "search" then s.results = data.sessions or {}; s.total = data.total or 0
			else s.projects = data.projects or {}; s.pinned = data.pinned or {}; s.recent = data.recent or {}; s.settings = data.settings or s.settings end
			rebuild(s)
			if s.search_edge and s.mode == "search" then
				s.sel = s.order[if s.search_edge == "last" then #s.order else 1]
				s.search_edge = nil
			end
		end
		s.cx:render()
	end
	s.loading = true
	if s.mode == "search" then
		cli.request({op = "snapshot", view = "mine", per = 0, recent = 0}, function(data, err)
			if not live(s) or s.gen ~= gen or err or not data then return end
			s.settings = data.settings or s.settings
			s.project_list = data.projects or {}
			if s.project_id then
				local found = false
				for _, p in s.project_list do if tostring(p.id) == s.project_id then found = true; s.project_label = p.name end end
				if not found then s.project_id = nil; s.project_label = nil; s.search_cwd = nil; s.offset = 0; reload(s); return end
			end
			s.cx:render()
		end)
		local args = {"query", "--scope", s.scope, "--cwd", s.search_cwd or s.ctx_cwd, "--sort", s.sort, "--limit", tostring(SEARCH_PAGE), "--offset", tostring(s.offset), "--q", s.query.text}
		if s.project_id then table.insert(args, "--project-id"); table.insert(args, s.project_id) end
		cli.run(args, cli.TIMEOUT_QUERY, accept)
	else
		cli.request({op = "snapshot", view = s.view, per = 25, recent = 15}, accept)
		if s.mode == "settings" then
			cli.request({op = "archived", limit = 1, offset = 0}, function(data, err)
				if not live(s) or err or not data then return end
				s.archived_total = data.total or 0
				s.cx:render()
			end)
		end
	end
end
local function refresh(s: any)
	reload(s)
	if s.indexing or not live(s) then return end
	s.indexing = true
	cli.run({"index"}, cli.TIMEOUT_INDEX, function(_, err)
		if not live(s) then return end
		s.indexing = false
		if err then s.error = err; s.error_reload = false; s.cx:render() else s.indexed_at = os.time(); reload(s) end
	end)
end
local function resume(s: any, chat: any)
	local r = chat.resume
	if not r then return end
	if r.kind == "spawn" then
		(tern :: any).process.run(r.argv or {}, {timeout_ms = 10000}, function(result: any)
			if not live(s) then return end
			if result.status ~= 0 then toast(s, "error", "Couldn't resume chat", if result.stderr ~= "" then result.stderr else result.stdout) end
		end)
	elseif r.command and r.command ~= "" then
		s.cx:open("tern-projects://resume?cwd=" .. fmt.pct(chat.cwd or "") .. "&cmd=" .. fmt.pct(r.command))
	end
end
local function copy(s: any, text: string, what: string)
	if text == "" then return end
	s.cx:copy(text)
	toast(s, "success", "Copied " .. what)
end

-- Project editor -------------------------------------------------------------------------

local function focus_editor_field(s: any)
	local d = s.editor
	if not d then return end
	if d.browser then return end
	s.cx:frame({{"focus", if d.focus == "name" then NAME_FIELD else NULL}})
end
local function open_editor(s: any, p: any?)
	cancel_flow(s)
	s.editor = editor.new(p)
	local draft = s.editor
	if not p and s.ctx_cwd ~= "" then
		request(s, {op = "folders", path = s.ctx_cwd, cwd = s.ctx_cwd, offset = 0, limit = 1}, function(data)
			if s.editor == draft and #draft.folders == 0 then editor.add_folder(draft, data.path) end
		end, "prefill", function() end)
	end
	s.cx:render(); focus_editor_field(s)
end
-- Lists `path` (typed text when nil). `opts.filter` keeps a partial name filter; otherwise the
-- field is rewritten to the listed folder followed by "/".
local function browse(s: any, path: string?, opts: any?)
	local d = s.editor
	if not d or not d.browser then return end
	local b = d.browser
	local o = opts or {}
	b.gen += 1
	local gen, text = b.gen, path or b.picker.query.text
	b.busy = true
	editor.sync_browser(d)
	cli.request({op = "folders", path = text, cwd = s.ctx_cwd, offset = 0, limit = 100}, function(data, err)
		if not live(s) or s.editor ~= d or d.browser ~= b or b.gen ~= gen then return end
		b.busy = false
		if err then
			b.error = fmt.error(err)
			b.entries = {}; b.total = 0; b.parent = nil; b.listing = nil
		else
			b.error = nil; b.loaded = true
			b.listing = data.path
			b.parent = if type(data.parent) == "string" then data.parent else nil
			b.entries = data.entries or {}; b.total = data.total or 0
			b.picker.selected = nil
			if o.filter then b.filter = o.filter else
				b.filter = ""
				local shown = fmt.path(data.path)
				tf.set(b.picker.query, if string.sub(shown, -1) == "/" then shown else shown .. "/")
			end
		end
		editor.sync_browser(d)
		s.cx:render()
	end)
	s.cx:render()
end
local function browse_more(s: any)
	local d = s.editor
	local b = d and d.browser
	if not b or not b.listing or b.busy then return end
	local gen, offset = b.gen, #b.entries
	b.busy = true
	cli.request({op = "folders", path = b.listing, cwd = s.ctx_cwd, offset = offset, limit = 100}, function(data, err)
		if not live(s) or s.editor ~= d or d.browser ~= b or b.gen ~= gen then return end
		b.busy = false
		if err then b.error = err else
			for _, e in data.entries or {} do table.insert(b.entries, e) end
			b.total = data.total or b.total
		end
		editor.sync_browser(d)
		s.cx:render()
	end)
end
-- The field changed: filter the listed folder, or list the folder the text now names.
local function browser_query(s: any)
	local d = s.editor
	local b = d and d.browser
	if not b then return end
	local dir, base = editor.split(b.picker.query.text)
	b.error = nil
	if dir == nil or editor.is_listing(b, dir) then
		b.filter = base
		b.gen += 1; b.busy = false
		editor.sync_browser(d)
		return
	end
	b.filter = base
	b.gen += 1
	-- The text now names a different folder: no old row may authorize acceptance.
	b.busy = true; b.target = nil
	editor.sync_browser(d)
	local gen = b.gen
	tern.timer(140, function()
		if live(s) and s.editor == d and d.browser == b and b.gen == gen then browse(s, dir, {filter = base}) end
	end)
end
local launch_plan: (any, string, boolean) -> ()
local harness_picker: (any, string) -> ()
local start_launch: (any, boolean) -> ()
local function browser_intent(s: any, intent: string?, value: string?)
	local d = s.editor
	if not d or not d.browser or not intent then return end
	local b = d.browser
	if intent == "close" then
		d.browser = nil
		if d.launch_only then s.editor = nil end
		s.cx:render(); focus_editor_field(s)
	elseif intent == "open" or intent == "complete" then browse(s, value)
	elseif intent == "browse" then browse(s, nil)
	elseif intent == "query" then browser_query(s)
	elseif intent == "more" then browse_more(s)
	elseif intent == "add" then
		local target = b.target
		if not target or b.busy then return end
		if d.launch_only then
			s.launch_target = {cwd = target, label = fmt.basename(fmt.path(target))}
			s.editor = nil
			start_launch(s, d.choose_harness == true)
			return
		end
		if table.find(d.folders, target) then return end
		editor.add_folder(d, target)
		d.browser = nil; d.focus = "add-folder"
		s.cx:render(); focus_editor_field(s)
	end
end
local function editor_intent(s: any, intent: string?, value: string?)
	local d = s.editor
	if not d or not intent then return end
	if intent == "focus" then focus_editor_field(s); return end
	if intent == "cancel" then s.editor = nil; s.tokens.prefill = (s.tokens.prefill or 0) + 1; s.cx:frame({{"focus", NULL}}); return end
	if intent == "browse" then
		-- Start beside the primary folder: related folders usually share its parent.
		local start = if d.folders[1] then (string.match(d.folders[1], "^(.+)/[^/]+/?$") or d.folders[1]) else nil
		editor.open_browser(d, start)
		s.cx:frame({{"focus", NULL}})
		browse(s, start)
		return
	end
	if intent == "save" then
		if s.busy then return end
		d.attempted = true
		if not editor.valid(d) then focus_editor_field(s); return end
		s.busy = true; s.busy_label = "Saving…"; d.error = nil
		cli.request({op = "project-save", id = d.id, name = d.name.text, icon = d.icon, folders = (tern.json :: any).array(d.folders), cwd = s.ctx_cwd}, function(data, err)
			if not live(s) then return end
			s.busy = false; s.busy_label = nil
			if s.editor ~= d then
				if err then toast(s, "error", "Couldn't save project", fmt.error(err)) else toast(s, "success", "Saved " .. d.name.text) end
				reload(s); return
			end
			if err then d.error = fmt.error(err) else
				s.editor = nil; s.view = "mine"; s.mode = "projects"
				local p = data and data.project
				if p then local key = "project:" .. tostring(p.id); s.sel = key; s.expanded[key] = true end
				toast(s, "success", if d.id then "Saved " .. d.name.text else "Created " .. d.name.text)
				s.cx:frame({{"focus", NULL}})
				reload(s)
			end
			s.cx:render()
		end)
		return
	end
	-- Form controls ("icon", "primary", "remove-folder", "add-folder", ...)
	local next = editor.action(d, intent, value)
	if next then editor_intent(s, next, nil) else focus_editor_field(s) end
end

-- Pickers --------------------------------------------------------------------------------

local function open_picker(s: any, spec: any)
	s.menu = nil; s.confirm = nil
	s.picker = picker.new(spec)
	s.cx:frame({{"focus", NULL}})
end
local function project_items(projects: any, current: string?): {any}
	local names: {[string]: number} = {}
	for _, p in projects do names[p.name] = (names[p.name] or 0) + 1 end
	local out = {}
	for _, p in projects do
		local folders = p.folders or {}
		local detail = if names[p.name] > 1 and folders[1] then fmt.path(folders[1]) else fmt.plural(p.count or 0, "chat")
		table.insert(out, {id = tostring(p.id), label = p.name, detail = detail, icon = p.icon or "folder", group = "Projects"})
	end
	return out
end
local function load_projects(s: any, p: any, prefix: {any})
	request(s, {op = "snapshot", view = "mine", per = 0, recent = 0}, function(data)
		if s.picker ~= p then return end
		s.project_list = data.projects or {}
		local items = table.clone(prefix)
		for _, it in project_items(s.project_list, nil) do table.insert(items, it) end
		p.items = items; p.state = "ready"
		picker.settle(p)
	end, "choices", function(err) if s.picker == p then p.state = "error"; p.message = fmt.error(err) end end)
end
local function move_picker(s: any, chat: any)
	local current = if chat.assignment and chat.assignment.mode == "project" then tostring(chat.assignment.project_id) else (chat.assignment and chat.assignment.mode or "automatic")
	local matches = {}
	if current == "automatic" and s.view == "mine" then
		for _, p in s.projects do
			for _, session in p.sessions or {} do if session.uid == chat.uid then table.insert(matches, project_name(p)); break end end
		end
	end
	local prefix = {
		{id = "automatic", label = "Automatic", detail = if #matches > 0 then table.concat(matches, " · ") else "Match by folder", title = if #matches > 0 then "Matches " .. table.concat(matches, ", ") else "Use the chat's original source folder", icon = "folder"},
		{id = "none", label = "No project", detail = "Recents only", icon = "slash-circle"},
	}
	open_picker(s, {kind = "move", title = "Move to project", subtitle = chat.title or chat.uid, icon = "folder-go", noun = "projects", search = true, placeholder = "Filter projects", state = "loading", items = prefix, current = {current}, selected = current, data = {uid = chat.uid},
		actions = {{id = "close", label = "Cancel", keys = {"esc"}}, {id = "choose", label = "Move", keys = {"enter"}, primary = true}}})
	load_projects(s, s.picker, prefix)
end
local function filter_picker(s: any)
	local prefix = {{id = "all", label = "All projects", detail = "No project filter", icon = "layers"}}
	local current = s.project_id or "all"
	open_picker(s, {kind = "filter", title = "Filter by project", subtitle = "Search within a named project", icon = "funnel", noun = "projects", search = true, placeholder = "Filter projects", state = "loading", items = prefix, current = {current}, selected = current,
		actions = {{id = "close", label = "Cancel", keys = {"esc"}}, {id = "choose", label = "Filter", keys = {"enter"}, primary = true}}})
	load_projects(s, s.picker, prefix)
end
local function launch_label(s: any): string?
	local t = s.launch_target
	return t and t.label
end
harness_picker = function(s: any, purpose: string)
	local default = s.settings.default_harness
	local is_default = purpose == "default"
	local where = launch_label(s)
	local seeds = {}
	if is_default then table.insert(seeds, {id = "ask", label = "Ask every time", icon = "help", disabled = true}) end
	for _, id in {"omp", "claude", "codex", "pi", "opencode", "cursor"} do table.insert(seeds, {id = id, label = fmt.agent_label(id), icon = fmt.agent_icon(id), disabled = true}) end
	local actions = {{id = "close", label = "Cancel", keys = {"esc"}}, {id = "choose", label = if is_default then "Use as default" else "Start chat", keys = {"enter"}, primary = true}}
	if not is_default then table.insert(actions, {id = "choose-default", label = "Start & set default", keys = {if tern.runtime.os == "macos" then "cmd+enter" else "ctrl+enter"}, ["end"] = true}) end
	open_picker(s, {
		kind = if is_default then "default-harness" else "harness",
		title = if is_default then "Default harness" else "New chat",
		subtitle = if is_default then "Used when you start a new chat" elseif where then "Start in " .. where else "Choose a harness",
		icon = if is_default then "sliders" else "pen",
		noun = "harnesses",
		state = "loading",
		message = nil,
		items = seeds,
		current = if is_default then {if type(default) == "string" then default else "ask"} else {},
		actions = actions,
	})
	local p = s.picker
	if not is_default and s.default_unavailable then p.subtitle = s.default_unavailable end
	if not is_default then begin_launch_wait(s, "harnesses") end
	request(s, {op = "harnesses"}, function(data)
		if s.picker ~= p then return end
		end_launch_wait(s)
		local items = {}
		if is_default then table.insert(items, {id = "ask", label = "Ask every time", detail = "Choose a harness for each new chat", icon = "help"}) end
		local folders = s.launch_target and s.launch_target.folders or 1
		for _, h in data.harnesses or {} do
			local badges = {}
			if h.id == default then table.insert(badges, {text = "Default", tone = "accent"}) end
			local capability = if h.multi_root then fmt.plural(folders, "folder") else "Primary folder only"
			local reason = if type(h.reason) == "string" and h.reason ~= "" then h.reason else "Not installed"
			local lower_reason = string.lower(reason)
			local short_reason = if string.find(lower_reason, "not found", 1, true) then "Not installed" elseif h.id == "cursor" then "Not Cursor Agent" elseif string.find(lower_reason, "updat", 1, true) then "Update required" else "Unavailable"
			table.insert(items, {
				id = h.id, label = h.label, icon = fmt.agent_icon(h.id), tone = fmt.agent_tone(h.id, s.settings.icon_style),
				detail = if h.installed then (if is_default then nil else capability) else short_reason,
				disabled = if h.installed then nil else reason,
				title = if h.installed then h.path else reason,
				badges = badges,
			})
		end
		p.items = items; p.state = "ready"
		if is_default then
			p.selected = if type(default) == "string" then default else "ask"
		else
			-- A missing or unavailable default must not stay selected; settle picks the first usable tool.
			p.selected = nil
			if type(default) == "string" then
				for _, it in items do if it.id == default and not it.disabled then p.selected = default end end
			end
		end
		picker.settle(p)
	end, "harnesses", function(err)
		if s.picker == p then
			end_launch_wait(s)
			p.state = "error"; p.message = fmt.error(err)
		end
	end)
end
local function load_archived_page(s: any, p: any, offset: number)
	p.state = "loading"
	request(s, {op = "archived", offset = offset, limit = 50}, function(data)
		if s.picker ~= p then return end
		s.archived = data.sessions or {}; s.archived_total = data.total or 0
		p.data.offset = offset
		local items = {}
		for _, chat in s.archived do
			local where = fmt.agent_label(chat.agent) .. " · " .. fmt.basename(chat.cwd or "")
			table.insert(items, {id = chat.uid, label = chat.title or chat.uid, detail = where .. " · " .. fmt.age(chat.updated_ms or 0), title = fmt.agent_label(chat.agent) .. " · " .. fmt.path(chat.cwd or ""), icon = fmt.agent_icon(chat.agent), tone = fmt.agent_tone(chat.agent, s.settings.icon_style)})
		end
		p.items = items; p.total = s.archived_total; p.state = "ready"
		p.subtitle = if s.archived_total == 0 then "Hidden from lists and search" else tostring(offset + 1) .. "–" .. tostring(offset + #items) .. " of " .. tostring(s.archived_total) .. " · Hidden from lists and search"
		for _, a in p.actions do
			if a.id == "archived-prev" then a.disabled = offset == 0
			elseif a.id == "archived-next" then a.disabled = offset + 50 >= s.archived_total end
		end
		picker.settle(p)
	end, "archived", function(err) if s.picker == p then p.state = "error"; p.message = fmt.error(err) end end)
end
local function archived_picker(s: any)
	open_picker(s, {kind = "archived", title = "Archived chats", subtitle = "Hidden from lists and search", icon = "inbox", noun = "archived chats", search = true, placeholder = "Filter this page", state = "loading", items = {}, empty = "No archived chats", data = {offset = 0},
		layout = "cards",
		actions = {{id = "close", label = "Close", keys = {"esc"}}, {id = "archived-prev", label = "Previous", keys = {"ctrl+p"}, disabled = true}, {id = "archived-next", label = "Next", keys = {"ctrl+n"}, disabled = true}, {id = "archived-delete", label = "Delete permanently…", keys = {if tern.runtime.os == "macos" then "cmd+backspace" else "ctrl+backspace"}, danger = true, ["end"] = true}, {id = "archived-restore", label = "Restore", keys = {"enter"}, primary = true}}})
	load_archived_page(s, s.picker, 0)
end

-- Launching ------------------------------------------------------------------------------

local function launch_route(s: any, plan: any)
	if type(plan.cwd) ~= "string" or type(plan.command) ~= "string" or plan.command == "" then s.error = "The launch plan was incomplete."; return end
	s.picker = nil; s.confirm = nil
	-- A window attributes the link to whichever pane is focused when it opens. The nonce lets
	-- the route tell our own launch (which may have lost focus) from text that merely looks like one.
	local nonce = tostring(tern.now()) .. "-" .. tostring(s.cx.pane)
	tern.kv.set("launch", {nonce = nonce, pane = s.cx.pane})
	s.cx:open("tern-projects://launch?n=" .. fmt.pct(nonce) .. "&cwd=" .. fmt.pct(plan.cwd) .. "&cmd=" .. fmt.pct(plan.command))
end
launch_plan = function(s: any, harness: string, from_default: boolean)
	local target = s.launch_target
	if not target then return end
	s.launch_harness = harness
	begin_launch_wait(s, "launch-plan")
	if s.picker then s.picker.state = "loading"; s.picker.message = "Preparing launch…" end
	local payload = {op = "launch-plan", harness = harness, project_id = target.project_id, cwd = target.cwd}
	request(s, payload, function(plan)
		end_launch_wait(s)
		if #(plan.warnings or {}) == 0 then launch_route(s, plan); return end
		s.pending_plan = plan
		s.picker = nil
		-- Backend warnings name each omitted folder; group them instead of repeating sentences.
		local unsupported, missing, notes = {}, {}, {}
		for _, w in plan.warnings do
			local gone, why = string.match(w, "^Additional source folder was not added because it is missing or unreadable: (.-) %((.*)%)$")
			local omitted = string.match(w, "^Additional source folder was not added: (.+)$")
			if gone then table.insert(missing, {path = gone, why = why})
			elseif omitted then table.insert(unsupported, {path = omitted})
			elseif not string.match(w, " will open the primary folder only%.") then table.insert(notes, w) end
		end
		local label = fmt.agent_label(harness)
		local function folder_row(path: string, icon: string, tag: any?, title: string?): any
			local kids: {any} = {ui.node("icon", {name = icon, tone = if icon == "warn" then "warning" else nil}), ui.node("text", {text = display_path(path, math.max(18, (s.cols or 44) - 20)), truncate = "start", grow = 1, title = title or path, role = "tern-projects.path"})}
			if tag then table.insert(kids, tag) end
			return ui.el("div", {class = "tp-folder" .. (if icon == "folder-open" then " primary" else " omitted")}, kids)
		end
		local body: {any} = {}
		if #unsupported > 0 then
			table.insert(body, c.note(label .. " opens one folder per chat, so this chat starts in the primary folder."))
		else
			table.insert(body, c.note("Some source folders can't be opened. The chat starts without them."))
		end
		table.insert(body, ui.el("div", {class = "tp-folders"}, {folder_row(plan.cwd, "folder-open", ui.el("span", {class = "tp-tag", text = "Primary"}))}))
		local left_out = {}
		for _, f in unsupported do table.insert(left_out, folder_row(f.path, "slash-circle")) end
		for _, f in missing do table.insert(left_out, folder_row(f.path, "warn", ui.el("span", {class = "tp-tag warn", text = "Unavailable"}), f.why)) end
		if #left_out > 0 then
			table.insert(body, c.label("Not included"))
			table.insert(body, ui.el("div", {class = "tp-folders"}, left_out))
		end
		for _, n in notes do table.insert(body, c.note(n)) end
		if #unsupported > 0 and #(plan.manual_commands or {}) == 0 then
			table.insert(body, c.note(label .. " has no command to add folders after the chat starts.", "muted"))
		end
		for i, cmd in plan.manual_commands or {} do
			table.insert(body, ui.el("div", {class = "tp-cmd"}, {ui.node("text", {text = cmd, truncate = "end", grow = 1, role = "tern-projects.command"}), c.button("Copy", "confirm=copy-manual-" .. tostring(i), {size = "xs"})}))
		end
		s.confirm = {kind = "launch", title = "Start " .. label .. " in " .. (target.label or "this project") .. (if #unsupported > 0 then "'s primary folder only?" else " without unavailable folders?"), body = body,
			buttons = {{id = "cancel", label = "Cancel"}, {id = "launch-accept", label = "Start in " .. fmt.basename(plan.cwd), variant = "primary"}}, focus = 2, data = plan}
	end, "launch", function(err)
		end_launch_wait(s)
		local message = fmt.error(err)
		if target.project_id and string.find(err, "Primary source folder is missing or unreadable", 1, true) then
			s.picker = nil
			s.confirm = {kind = "missing-primary", title = "Primary folder is unavailable", body = {c.note(message), c.note("Edit the project to choose another primary source folder.")},
				buttons = {{id = "cancel", label = "Cancel"}, {id = "edit-launch-project", label = "Edit project…", variant = "primary"}}, focus = 1, data = target}
		elseif from_default and (string.find(err, " CLI is not installed", 1, true) or string.find(err, " is unavailable:", 1, true)) then
			-- The default tool is gone: say why and hand over to the picker instead of failing quietly.
			s.default_unavailable = fmt.agent_label(harness) .. " isn't available. " .. message
			toast(s, "error", "Couldn't prepare new chat", message)
			harness_picker(s, "launch")
		elseif s.picker then s.picker.state = "ready"; s.picker.message = message; toast(s, "error", "Couldn't prepare new chat", message)
		else s.error = err; s.error_reload = false; toast(s, "error", "Couldn't prepare new chat", message) end
	end)
end
local function launch_target_for(s: any, key: string?): boolean
	local row = s.rows[key or s.sel or ""]
	if row and row.parent then row = s.rows[row.parent] end
	if row and row.kind == "project" then
		local p = row.ref
		s.launch_target = if s.view == "mine" then {project_id = tostring(p.id), label = project_name(p), folders = #(p.folders or {})} else {cwd = p.root, label = project_name(p), folders = 1}
		return true
	end
	if row and row.kind == "session" and type(row.ref.cwd) == "string" and row.ref.cwd ~= "" then
		s.launch_target = {cwd = row.ref.cwd, label = fmt.basename(row.ref.cwd), folders = 1}; return true
	end
	if s.ctx_cwd ~= "" then s.launch_target = {cwd = s.ctx_cwd, label = fmt.basename(fmt.path(s.ctx_cwd)), folders = 1}; return true end
	return false
end
start_launch = function(s: any, choose: boolean)
	local default = s.settings.default_harness
	s.default_unavailable = nil
	-- An installed default needs one launch plan, not the whole tool survey.
	if choose or type(default) ~= "string" then harness_picker(s, "launch"); return end
	launch_plan(s, default, true)
end
local function new_chat(s: any, key: string?, choose: boolean)
	if s.launch_pending then return end
	cancel_flow(s)
	if not launch_target_for(s, key) then
		s.editor = editor.new({name = "", folders = {}})
		s.editor.launch_only = true; s.editor.choose_harness = choose
		editor.open_browser(s.editor, nil)
		s.editor.browser.picker.title = "New chat"
		s.editor.browser.picker.subtitle = "Choose a starting folder"
		browse(s, nil)
		return
	end
	start_launch(s, choose)
end

-- Menus and confirmations -------------------------------------------------------------------

-- One action list per row kind; "-" marks a separator in the keyboard menu (native menus skip it).
local function menu_entries(s: any, key: string): {string}
	local row = s.rows[key]
	if not row then return {} end
	if row.kind == "session" then
		return {"resume", "copy-resume-command", "copy-chat-id", "-", if row.ref.pinned then "unpin" else "pin", "move-to-project…", if row.ref.archived then "restore" else "archive", "-", if s.mode == "search" then "search-options" else "panel-options", "-", "delete-permanently…"}
	elseif row.kind == "project" then
		if s.view == "mine" then return {"new-chat", "new-chat-with…", "-", "edit-project…", "remove-project…", "-", "panel-options"} end
		return {"new-chat", "new-chat-with…", "-", "save-as-project…", "-", "panel-options"}
	end
	return {}
end
local function menu_ids(s: any, key: string): {string}
	local out = {}
	for _, id in menu_entries(s, key) do if id ~= "-" then table.insert(out, id) end end
	return out
end
local MENU_META: {[string]: any} = {
	["resume"] = {label = "Resume", icon = "play", hint = {"enter"}},
	["copy-resume-command"] = {label = "Copy resume command", icon = "copy", hint = {"ctrl", "y"}},
	["copy-chat-id"] = {label = "Copy chat ID", icon = "hash"},
	["pin"] = {label = "Pin", icon = "pin"},
	["unpin"] = {label = "Unpin", icon = "pin"},
	["move-to-project…"] = {label = "Move to project…", icon = "folder-go"},
	["archive"] = {label = "Archive", icon = "inbox"},
	["restore"] = {label = "Restore", icon = "undo"},
	["delete-permanently…"] = {label = "Delete permanently…", icon = "trash", tone = "error"},
	["new-chat"] = {label = "New chat", icon = "pen", hint = {if tern.runtime.os == "macos" then "cmd" else "ctrl", "n"}},
	["search-options"] = {label = "Search options…", icon = "sliders", hint = {"shift", "f10"}},
	["panel-options"] = {label = "Panel options…", icon = "sliders"},
	["new-chat-with…"] = {label = "New chat with…", icon = "sparkle", hint = {"shift", if tern.runtime.os == "macos" then "cmd" else "ctrl", "n"}},
	["edit-project…"] = {label = "Edit project…", icon = "sliders"},
	["remove-project…"] = {label = "Remove project…", icon = "trash", tone = "error"},
	["save-as-project…"] = {label = "Save as project…", icon = "folder-plus"},
	["create"] = {label = "Create project…", icon = "plus"},
	["refresh"] = {label = "Refresh", icon = "redo", hint = {"ctrl", "r"}},
	["scope=workspace"] = {label = "Worktree", icon = "folder"},
	["scope=repository"] = {label = "Repository", icon = "code"},
	["scope=all"] = {label = "All chats", icon = "layers"},
	["filter"] = {label = "Filter by project…", icon = "funnel"},
	["filter-project=all"] = {label = "Clear project filter", icon = "x"},
	["sort=updated"] = {label = "Sort: Newest", icon = "clock"},
	["sort=relevance"] = {label = "Sort: Best match", icon = "sparkle"},
}
local function open_menu(s: any, key: string?)
	key = key or s.sel
	if not key or not s.rows[key] or (s.rows[key].kind ~= "session" and s.rows[key].kind ~= "project") then
		local ids = if s.mode == "search" then {"scope=workspace", "scope=repository", "scope=all", "-", "filter", "filter-project=all", "-", "sort=updated", "sort=relevance"} else {"new-chat", "new-chat-with…", "create", "-", "refresh"}
		s.menu = {ids = ids, selected = ids[1], title = if s.mode == "search" then "Search options" else "Projects"}
		return
	end
	local row = s.rows[key]
	if not row or (row.kind ~= "session" and row.kind ~= "project") then return end
	local ids = menu_entries(s, key)
	s.picker = nil; s.confirm = nil
	s.sel = key
	s.menu = {target = key, ids = ids, selected = ids[1]}
end
local function confirm_remove(s: any, p: any)
	s.remove_id = tostring(p.id)
	s.confirm = {kind = "remove", title = "Remove “" .. project_name(p) .. "”?", body = {
		c.note("The project is removed from your sidebar. Chats and source folders will not be deleted. Chats you moved here go back to matching by folder."),
	}, buttons = {{id = "cancel", label = "Cancel"}, {id = "remove-confirm", label = "Remove project", variant = "danger"}}, focus = 1}
end
local function confirm_delete(s: any, chat: any)
	s.menu = nil; s.picker = nil
	s.delete_preview = nil
	local function header(): any
		return ui.el("div", {class = "tp-subject"}, {
			ui.node("icon", {name = fmt.agent_icon(chat.agent), tone = fmt.agent_tone(chat.agent, s.settings.icon_style)}),
			ui.el("div", {class = "tp-subject-t"}, {
				ui.node("text", {text = chat.title or chat.uid, lines = 2, role = "tern-projects.subject-title"}),
				ui.node("text", {text = fmt.agent_label(chat.agent) .. " · " .. fmt.path(chat.cwd or "") .. " · " .. fmt.age(chat.updated_ms or 0), truncate = "middle", role = "tern-projects.subject-meta"}),
			}),
		})
	end
	s.confirm = {kind = "delete", title = "Delete chat permanently?", body = {header(), ui.el("div", {class = "tp-loading"}, {ui.node("spinner", {style = "ring"}), ui.el("span", {text = "Checking what will be deleted…"})})},
		buttons = {{id = "cancel", label = "Cancel"}, {id = "delete-confirm", label = "Delete permanently", variant = "danger", disabled = true}}, focus = 1}
	local conf = s.confirm
	request(s, {op = "delete-preview", uid = chat.uid}, function(data)
		if s.confirm ~= conf then return end
		s.delete_preview = data
		local body: {any} = {header()}
		local blocked = type(data.blocked_reason) == "string"
		if blocked then
			table.insert(body, c.banner("blocked", "warning", "warn", data.blocked_reason))
		else
			table.insert(body, c.note("This deletes the original chat from " .. fmt.agent_label(data.agent or chat.agent) .. ". It can't be undone. Archive the chat instead if you only want to hide it."))
			local deps = #(data.dependent_ids or {})
			local relation = if chat.agent == "omp" or chat.agent == "claude" then "subagent chat" elseif chat.agent == "opencode" then "forked session" else "related chat"
			if deps > 0 then table.insert(body, c.note("Also deletes " .. fmt.plural(deps, relation) .. ".", "warning")) end
			local targets = data.targets or {}
			if #targets > 0 then
				table.insert(body, c.label("Files and records", tostring(#targets)))
				local lines = {}
				for i, t in targets do
					if i > 4 then table.insert(lines, ui.node("text", {text = "+ " .. tostring(#targets - 4) .. " more", role = "tern-projects.more-line"})); break end
					table.insert(lines, ui.node("text", {text = fmt.path(t), truncate = "start", title = t, role = "tern-projects.path"}))
				end
				table.insert(body, ui.el("div", {class = "tp-targets"}, lines))
			end
		end
		conf.body = body
		conf.buttons = if blocked
			then {{id = "cancel", label = "Cancel"}, {id = "archive-instead", label = "Archive instead", variant = "primary", disabled = chat.archived}}
			else {{id = "cancel", label = "Cancel"}, {id = "delete-confirm", label = "Delete permanently", variant = "danger"}}
		conf.focus = 1
		conf.chat = chat
	end, "delete-preview", function(err)
		if s.confirm ~= conf then return end
		conf.body = {header(), c.banner("preview-error", "error", "warn", fmt.error(err))}
	end)
end

-- Row activation ---------------------------------------------------------------------------

local function find_chat(s: any, key: string): any
	local row = s.rows[key]
	if row and row.kind == "session" then return row.ref end
	for _, chat in s.archived or {} do if chat.uid == key then return chat end end
	return nil
end
local function enter_search(s: any)
	cancel_flow(s)
	s.mode = "search"; s.page = 0; s.offset = 0; s.sel = nil; s.loaded = false
	rebuild(s); reload(s)
	s.cx:frame({{"focus", SEARCH_FIELD}})
end
local function activate(s: any, key: string?)
	local row = s.rows[key or ""]
	if not row then return end
	if row.kind == "cardbtn" then
		local a, v = string.match(row.ref.act, "^([^=]+)=(.*)$")
		action(s, a or row.ref.act, v)
		return
	end
	s.sel = key
	if row.kind == "session" then resume(s, row.ref)
	elseif row.kind == "project" then s.expanded[key] = not s.expanded[key]; rebuild(s)
	elseif row.kind == "more" then s.more[row.parent] = true; rebuild(s)
	elseif row.kind == "fewer" then s.more[row.parent] = nil; rebuild(s)
	elseif row.kind == "start" then new_chat(s, row.parent, false)
	elseif row.kind == "older" then
		local p = row.ref
		s.search_cwd = nil; s.project_label = nil
		if s.view == "mine" then s.scope = "all"; s.project_id = tostring(p.id); s.project_label = p.name
		else s.scope = "repository"; s.search_cwd = p.root; s.project_id = nil end
		enter_search(s)
	end
end
local function change_mode(s: any, mode: string)
	if mode == "search" then
		if s.search_cwd then s.search_cwd = nil end
		enter_search(s)
		return
	end
	cancel_flow(s)
	s.mode = mode; s.page = 0; s.settings_focus = nil
	rebuild(s); reload(s)
	s.cx:frame({{"focus", NULL}})
end
local function search_changed(s: any)
	s.offset = 0; s.page = 0; s.gen += 1
	local gen = s.gen
	tern.timer(150, function() if live(s) and s.gen == gen then reload(s) end end)
end

-- Actions ------------------------------------------------------------------------------------

local SETTINGS = {"default", "icons", "archived", "refresh"}
local function hide_busy(s: any)
	cancel_flow(s)
	s.editor = nil; s.archived_picker = nil
	toast(s, "info", s.busy_label or "Change in progress", "Hidden, not cancelled. You'll be notified when it finishes.")
end
local function picker_action(s: any, act: string?)
	local p = s.picker
	if not p or not act then return end
	if act == "close" then if s.busy then hide_busy(s) else cancel_flow(s) end; return end
	if s.busy then return end
	for _, a in p.actions do if a.id == act and a.disabled then return end end
	if act == "query" then return end
	if act == "clear" then
		if p.query then tf.set(p.query, ""); p.query_all = false; picker.settle(p) end
		return
	end
	local it = picker.selected_item(p)
	if p.kind == "harness" then
		if (act == "choose" or act == "choose-default") and it and not it.disabled and p.state == "ready" then
			if act == "choose-default" then
				mutate(s, {op = "settings-update", default_harness = it.id}, function()
					s.settings.default_harness = it.id
					toast(s, "success", "Default harness set to " .. it.label)
					if s.picker == p then launch_plan(s, it.id, false) end
				end)
			else launch_plan(s, it.id, false) end
		end
	elseif p.kind == "default-harness" then
		if act == "choose" and it and not it.disabled then
			if table.find(p.current, it.id) then cancel_flow(s); return end
			local value = if it.id == "ask" then NULL else it.id
			mutate(s, {op = "settings-update", default_harness = value}, function() s.picker = nil; toast(s, "success", if it.id == "ask" then "New chat will ask for a harness" else "New chat uses " .. it.label) end)
		end
	elseif p.kind == "move" then
		if act == "choose" and it then
			if table.find(p.current, it.id) then cancel_flow(s); return end
			local assignment = if it.id == "automatic" or it.id == "none" then {mode = it.id} else {mode = "project", project_id = it.id}
			local label = it.label
			mutate(s, {op = "chat-update", uid = p.data.uid, assignment = assignment}, function()
				s.picker = nil
				toast(s, "success", if it.id == "automatic" then "Project set to Automatic" elseif it.id == "none" then "Removed from projects" else "Moved to " .. label)
			end)
		end
	elseif p.kind == "filter" then
		if act == "choose" and it then action(s, "filter-project", it.id); s.picker = nil end
	elseif p.kind == "archived" then
		if act == "archived-prev" or act == "archived-next" then
			if p.state == "loading" then return end
			load_archived_page(s, p, math.max(0, p.data.offset + (if act == "archived-next" then 50 else -50))); return
		end
		if not it then return end
		local chat = find_chat(s, it.id)
		if not chat then return end
		if act == "archived-restore" then
			mutate(s, {op = "chat-update", uid = chat.uid, archived = false}, function()
				toast(s, "success", "Restored", chat.title)
				if s.picker == p then load_archived_page(s, p, if p.data.offset >= (p.total or 1) - 1 then math.max(0, p.data.offset - 50) else p.data.offset) end
			end)
		elseif act == "archived-delete" then
			s.archived_picker = p
			confirm_delete(s, chat)
		end
	end
end
local function confirm_action(s: any, id: string)
	local conf = s.confirm
	if not conf then return end
	for _, b in conf.buttons do if b.id == id and b.disabled then return end end
	if id == "cancel" then
		if s.busy then hide_busy(s); return end
		cancel_flow(s)
		if s.archived_picker then s.picker = s.archived_picker; s.archived_picker = nil end
		return
	end
	if s.busy then return end
	if id == "edit-launch-project" then
		local pid = conf.data.project_id
		for _, p in s.projects do if tostring(p.id) == pid then open_editor(s, p); return end end
		request(s, {op = "snapshot", view = "mine", per = 0, recent = 0}, function(data)
			for _, p in data.projects or {} do if tostring(p.id) == pid then open_editor(s, p); return end end
		end, "edit-launch")
		return
	end
	if id == "launch-accept" then if s.pending_plan then launch_route(s, s.pending_plan) end; return end
	local manual = string.match(id, "^copy%-manual%-(%d+)$")
	if manual then local cmd = (conf.data.manual_commands or {})[tonumber(manual)]; if cmd then copy(s, cmd, "command") end; return end
	if id == "remove-confirm" then
		local pid = s.remove_id
		if pid then mutate(s, {op = "project-remove", id = pid}, function()
			s.confirm = nil
			toast(s, "success", "Project removed", "Chats and folders were not changed.")
			if s.project_id == pid then s.project_id = nil; s.project_label = nil; s.search_cwd = nil; s.offset = 0 end
		end) end
	elseif id == "delete-confirm" then
		local p = s.delete_preview
		if p and type(p.blocked_reason) ~= "string" then mutate(s, {op = "delete", uid = p.uid, expected_source = p.source, expected_version = p.version}, function()
			s.confirm = nil; s.delete_preview = nil
			toast(s, "success", "Chat deleted permanently")
			if s.archived_picker then
				local archived = s.archived_picker; s.archived_picker = nil
				s.picker = archived
				load_archived_page(s, archived, if archived.data.offset >= (archived.total or 1) - 1 then math.max(0, archived.data.offset - 50) else archived.data.offset)
			end
		end) end
	elseif id == "archive-instead" then
		local chat = conf.chat
		if chat then mutate(s, {op = "chat-update", uid = chat.uid, archived = true}, function() s.confirm = nil; toast(s, "success", "Archived", "Restore it from Settings → Archived chats.") end) end
	end
end
local function chat_action(s: any, act: string, chat: any)
	if act == "resume" then resume(s, chat)
	elseif act == "copy-resume-command" then copy(s, chat.resume and chat.resume.command or "", "resume command")
	elseif act == "copy-chat-id" then copy(s, chat.id or chat.uid, "chat ID")
	elseif act == "pin" or act == "unpin" then
		mutate(s, {op = "chat-update", uid = chat.uid, pinned = act == "pin"}, function() toast(s, "success", if act == "pin" then "Pinned" else "Unpinned") end)
	elseif act == "archive" or act == "restore" then
		mutate(s, {op = "chat-update", uid = chat.uid, archived = act == "archive"}, function()
			if act == "archive" then toast(s, "success", "Archived", "Restore it from Settings → Archived chats.") else toast(s, "success", "Restored") end
		end)
	elseif act == "move-to-project…" then move_picker(s, chat)
	elseif act == "delete-permanently…" then confirm_delete(s, chat) end
end
local function row_action(s: any, act: string, key: string)
	if act == "search-options" or act == "panel-options" then open_menu(s, ""); return end
	local row = s.rows[key]
	if not row then return end
	s.menu = nil
	if row.kind == "session" then chat_action(s, act, row.ref); return end
	if row.kind ~= "project" then return end
	local p = row.ref
	if act == "new-chat" or act == "new-chat-with…" then new_chat(s, key, act == "new-chat-with…")
	elseif act == "edit-project…" then open_editor(s, p)
	elseif act == "save-as-project…" then open_editor(s, {name = project_name(p), folders = {p.root}})
	elseif act == "remove-project…" then confirm_remove(s, p) end
end
action = function(s: any, act: string, value: string?)
	if act == "dismiss-error" then s.error = nil; return end
	if act == "cancel-launch" then
		if not s.launch_pending then return end
		cancel_flow(s)
		toast(s, "info", "New chat cancelled")
		return
	end
	if s.busy then return end
	if act == "editor" then local a, v = string.match(fmt.unpct(value or ""), "^([^=]+)=(.*)$"); editor_intent(s, a or fmt.unpct(value or ""), v); return end
	if act == "mode" then change_mode(s, value or "projects")
	elseif act == "view" then
		if value == s.view then return end
		-- The other view's groups have a different shape; never render them under the new view.
		cancel_flow(s); s.view = value; s.projects = {}; s.page = 0; s.sel = nil; s.loaded = false; rebuild(s); reload(s)
	elseif act == "create" then open_editor(s, nil)
	elseif act == "refresh" then refresh(s)
	elseif act == "reload" then reload(s)
	elseif act == "search-options" or act == "panel-options" then open_menu(s, "")
	elseif act == "scope" then s.scope = value; s.search_cwd = nil; s.offset = 0; s.page = 0; reload(s)
	elseif act == "sort" then
		if s.query.text == "" then return end
		s.sort = value or (if s.sort == "updated" then "relevance" else "updated"); s.offset = 0; reload(s)
	elseif act == "filter" then filter_picker(s)
	elseif act == "filter-project" then
		s.project_id = if value ~= "all" then value else nil; s.project_label = nil; s.search_cwd = nil
		if s.project_id then
			s.scope = "all"
			for _, p in s.project_list or {} do if tostring(p.id) == s.project_id then s.project_label = p.name end end
		end
		s.offset = 0; reload(s)
	elseif act == "page" then s.page = math.max(0, s.page + (if value == "next" then 1 else -1))
	elseif act == "search-page" then
		if s.loading or (value == "next" and s.offset + SEARCH_PAGE >= s.total) or (value ~= "next" and s.offset == 0) then return end
		s.offset = math.max(0, s.offset + (if value == "next" then SEARCH_PAGE else -SEARCH_PAGE))
		s.sel = nil; s.search_edge = if value == "next" then "first" else "last"; reload(s)
	elseif act == "clear-query" then tf.set(s.query, ""); s.query_all = false; search_changed(s)
	elseif act == "activate" then activate(s, fmt.unpct(value or ""))
	elseif act == "actions" then open_menu(s, if value then fmt.unpct(value) else nil)
	elseif act == "new-chat" or act == "new-chat-with…" then new_chat(s, if value then fmt.unpct(value) else nil, act == "new-chat-with…")
	elseif act == "default-picker" then s.settings_focus = "default"; harness_picker(s, "default")
	elseif act == "icon-style" then
		s.settings_focus = "icons"
		local style = value or (if s.settings.icon_style == "monochrome" then "color" else "monochrome")
		if style ~= s.settings.icon_style then
			local previous = s.settings.icon_style
			s.settings.icon_style = style
			mutate(s, {op = "settings-update", icon_style = style}, nil, function() s.settings.icon_style = previous end)
		end
	elseif act == "archived" then s.settings_focus = "archived"; archived_picker(s)
	elseif act == "resume" or act == "copy-resume-command" or act == "copy-chat-id" then
		-- Native menus name the clicked row; the dock and shortcuts act on the selection.
		local chat = find_chat(s, if value then fmt.unpct(value) else (s.sel or ""))
		if chat then chat_action(s, act, chat) end
	elseif act == "edit-selected" then
		local row = s.rows[s.sel or ""]
		if row and row.kind == "project" then row_action(s, if s.view == "mine" then "edit-project…" else "save-as-project…", s.sel) end
	elseif value and s.rows[fmt.unpct(value)] then
		row_action(s, act, fmt.unpct(value))
	end
end
local function menu_activate(s: any, id: string)
	local m = s.menu
	if not m then return end
	if id == "search-options" or id == "panel-options" then
		open_menu(s, "")
		return
	end
	if m.target then row_action(s, id, m.target); return end
	s.menu = nil
	local act, value = string.match(id, "^([^=]+)=(.*)$")
	action(s, act or id, value)
end

-- Keyboard -------------------------------------------------------------------------------------

local function nav(s: any, dir: number)
	if s.loading and s.mode == "search" then return end
	local idx = table.find(s.order, s.sel) or (if dir < 0 then #s.order + 1 else 0)
	idx += dir
	while idx >= 1 and idx <= #s.order do
		if navigable(s.rows[s.order[idx]]) then s.sel = s.order[idx]; s.page = math.floor((idx - 1) / PAGE); return end
		idx += dir
	end
	if s.mode == "search" and s.sel then action(s, "search-page", if dir > 0 then "next" else "prev") end
end
local function settings_key(s: any, k: Key): boolean
	local name = k.name
	local idx = table.find(SETTINGS, s.settings_focus) or 0
	if name == "up" or name == "down" then
		idx = if idx == 0 then (if name == "up" then #SETTINGS else 1) else math.clamp(idx + (if name == "up" then -1 else 1), 1, #SETTINGS)
		s.settings_focus = SETTINGS[idx]
		return true
	end
	local f = s.settings_focus
	if f == "icons" and (name == "left" or name == "right") then action(s, "icon-style", if name == "left" then "color" else "monochrome"); return true end
	if (name == "enter" or name == "space") and f then
		if f == "default" then action(s, "default-picker", nil)
		elseif f == "icons" then action(s, "icon-style", nil)
		elseif f == "archived" then action(s, "archived", nil)
		elseif f == "refresh" then refresh(s) end
		return true
	end
	return false
end
local function menu_key(s: any, k: Key): boolean
	local m = s.menu
	local items = {}
	for _, id in m.ids do if id ~= "-" then table.insert(items, id) end end
	local idx = table.find(items, m.selected) or 1
	if k.name == "escape" then s.menu = nil
	elseif k.name == "up" or k.name == "down" then m.selected = items[math.clamp(idx + (if k.name == "up" then -1 else 1), 1, #items)]
	elseif k.name == "home" then m.selected = items[1]
	elseif k.name == "end" then m.selected = items[#items]
	elseif k.name == "enter" or k.name == "space" then menu_activate(s, m.selected) end
	return true
end
local function confirm_key(s: any, k: Key): boolean
	local conf = s.confirm
	local n = #conf.buttons
	if k.name == "escape" then confirm_action(s, "cancel")
	elseif k.name == "tab" or k.name == "left" or k.name == "right" then
		local dir = if k.name == "left" or (k.name == "tab" and k.shift) then -1 else 1
		conf.focus = (conf.focus - 1 + dir) % n + 1
	elseif k.name == "enter" or k.name == "space" then
		local b = conf.buttons[conf.focus]
		if b then confirm_action(s, b.id) end
	end
	return true
end

-- Views ------------------------------------------------------------------------------------------

local function harness_icon(s: any, agent: string): any
	return ui.node("icon", {name = fmt.agent_icon(agent), tone = fmt.agent_tone(agent, s.settings.icon_style), title = fmt.agent_label(agent)})
end
local function nav_view(s: any, narrow: boolean): any
	return ui.el("div", {key = "nav", class = "tp-top"}, {
		c.seg({
			{id = "projects", label = "Projects", icon = "folder", title = "Projects (Tab)"},
			{id = "search", label = "Search", icon = "search", title = "Search chats (/)"},
			{id = "settings", label = "Settings", icon = "gear", title = "Settings"},
		}, s.mode, "mode", {class = "tp-nav", icons_only = narrow}),
	})
end
local function launch_hint(s: any): string
	local row = s.rows[s.sel or ""]
	if row and row.parent then row = s.rows[row.parent] end
	if row and row.kind == "project" then return "New chat in " .. project_name(row.ref) end
	if row and row.kind == "session" and row.ref.cwd and row.ref.cwd ~= "" then return "New chat in " .. fmt.basename(row.ref.cwd) end
	if s.ctx_cwd ~= "" then return "New chat in " .. fmt.path(s.ctx_cwd) end
	return "New chat"
end
local function projects_bar(s: any, cols: number): any
	local kids: {any} = {
		c.seg({{id = "mine", label = if cols < 48 then "Mine" else "My projects", title = "Projects you created"}, {id = "discovered", label = if cols < 48 then "Found" else "Discovered", title = "Folders found in your chat history"}}, s.view, "view", {size = "sm", key = "view"}),
		c.grow(),
	}
	if s.indexing then table.insert(kids, ui.el("span", {class = "tp-spin", title = "Indexing chat history…"}, {ui.node("spinner", {style = "ring"})})) end
	local shortcut = if tern.runtime.os == "macos" then "⌘N" else "Ctrl+N"
	if cols >= 48 then
		table.insert(kids, c.button(if cols >= 56 then launch_hint(s) else "New chat", "new-chat", {key = "new-chat", icon = "pen", size = "sm", title = launch_hint(s) .. " (" .. shortcut .. ")", disabled = s.launch_pending}))
	else
		table.insert(kids, c.icon_button("pen", "new-chat", launch_hint(s) .. " (" .. shortcut .. ")", {key = "new-chat", size = "sm", disabled = s.launch_pending}))
	end
	table.insert(kids, c.icon_button("redo", "refresh", if s.indexing then "Indexing chat history…" else "Refresh chat history (Ctrl+R)", {key = "refresh", size = "sm", disabled = s.indexing}))
	return ui.el("div", {key = "bar", class = "tp-bar"}, kids)
end
local function search_bar(s: any): {any}
	local narrow = (s.cols or 44) < 48
	local field = ui.node("input", {
		key = "query",
		text = s.query.text,
		cursor = tf.utf16(s.query),
		anchor = if s.query_all then 0 else nil,
		placeholder = "Search chat titles and messages",
		aria = "Search chats",
		prompt = {ui.span("\u{f002}", "icon")},
		role = "tern-projects.search-field",
	})
	local scopes = c.seg({
		{id = "workspace", label = if narrow then "Tree" else "Worktree", title = "Chats started in " .. fmt.path(s.search_cwd or s.ctx_cwd)},
		{id = "repository", label = if narrow then "Repo" else "Repository", title = "Chats in any worktree of the repository at " .. fmt.path(s.search_cwd or s.ctx_cwd)},
		{id = "all", label = if narrow then "All" else "All chats", title = "Every indexed chat"},
	}, s.scope, "scope", {size = "sm", key = "scopes", class = "fill"})
	local filter_label = if s.project_id then (s.project_label or "Project") else "All projects"
	local filter = c.button(filter_label, "filter", {key = "filter", variant = if s.project_id then nil else "ghost", size = "xs", icon = if s.project_id then "funnel" else "layers", trailing = "chev", pressed = s.project_id ~= nil, title = "Filter by project"})
	local count = if s.loading and not s.loaded then "Searching…" else fmt.plural(s.total, "chat")
	local caption = scope_label(s) .. " · " .. count
	local caption_kids: {any} = {ui.node("text", {text = caption, truncate = "end", grow = 1, title = caption, role = "tern-projects.caption"})}
	if s.loading and s.loaded then table.insert(caption_kids, ui.el("span", {class = "tp-spin"}, {ui.node("spinner", {style = "ring"})})) end
	local controls: {any} = {filter}
	if s.project_id then table.insert(controls, c.icon_button("x", "filter-project=all", "Clear project filter", {variant = "ghost", size = "xs"})) end
	table.insert(controls, c.grow())
	table.insert(controls, c.button(if s.sort == "updated" or s.query.text == "" then "Newest" else "Best match", "sort", {key = "sort", size = "xs", variant = "ghost", trailing = "chev", disabled = s.query.text == "", title = "Search result order"}))
	table.insert(controls, c.icon_button("more", "search-options", "Search options (⇧F10)", {variant = "ghost", size = "xs"}))
	return {
		ui.el("div", {key = "searchbox", class = "tp-searchbox"}, {field}),
		ui.el("div", {key = "scopebar", class = "tp-bar"}, {scopes}),
		ui.el("div", {key = "caption", class = "tp-caption"}, caption_kids),
		ui.el("div", {key = "search-controls", class = "tp-bar"}, controls),
	}
end
local function list_view(s: any): any
	local items = {}
	local rendered: {[string]: boolean} = {}
	local names: {[string]: number} = {}
	for _, p in s.projects do local name = string.lower(project_name(p)); names[name] = (names[name] or 0) + 1 end
	local first, last = s.page * PAGE + 1, math.min(#s.order, (s.page + 1) * PAGE)
	for i = first, last do
		local key = s.order[i]
		local row = s.rows[key]
		local r = row.ref
		if row.kind == "card" or row.kind == "cardbtn" then
			local card_key = if row.kind == "card" then key else row.parent :: string
			if not rendered[card_key] then
				rendered[card_key] = true
				local cref = s.rows[card_key].ref
				local buttons = {}
				for _, b in cref.buttons or {} do
					table.insert(buttons, c.button(b.label, b.act, {key = b.row, variant = b.variant, size = "sm", icon = b.icon, focused = s.sel == b.row}))
				end
				table.insert(items, c.empty(card_key, cref.icon, cref.title, cref.body, buttons))
			end
		elseif row.kind == "header" then
			table.insert(items, c.header(key, r.label, r.value, if key == "h:projects" then c.icon_button("plus", "create", "Create project", {variant = "ghost", size = "xs", key = "create"}) else nil))
		else
			local props: any = {key = key}
			local child = row.parent ~= nil
			if row.kind == "session" then
				props.icon = fmt.agent_icon(r.agent or "")
				props.tone = fmt.agent_tone(r.agent, s.settings.icon_style)
				props.label = r.title or r.uid
				props.value = if r.pinned then {ui.span("\u{f08d} ", "icon"), ui.span(fmt.age(r.updated_ms or 0), "muted")} else fmt.age(r.updated_ms or 0)
				props.title = fmt.agent_label(r.agent or "") .. " · " .. fmt.path(r.cwd or "") .. (if r.pinned then " · Pinned" else "")
				local snippet = s.mode == "search" and s.query.text ~= "" and type(r.snippet) == "string" and r.snippet ~= ""
				if snippet then
					props.detail = fmt.snippet_spans(r.snippet, r.title)
					props.role = "tern-projects.result"
				else
					props.role = if child then "tern-projects.child" else "tern-projects.chat"
				end
				local menu = {}
				for _, id in menu_ids(s, key) do table.insert(menu, id) end
				props.actions = {menu = menu}
			elseif row.kind == "project" then
				local open = s.expanded[key] == true
				props.icon = if s.view == "mine" then (r.icon or "folder") else (if open then "folder-open" else "folder")
				props.label = project_name(r)
				props.value = if (r.count or 0) > 0 then tostring(r.count) else nil
				local collision = (names[string.lower(project_name(r))] or 0) > 1
				if s.view == "mine" then
					local folders = r.folders or {}
					local function slug(name: string): string return string.gsub(string.lower(name), "[%s_%-]+", "") end
					props.detail = if collision and folders[1] then fmt.path(folders[1]) elseif #folders > 1 then fmt.plural(#folders, "folder") elseif folders[1] and slug(fmt.basename(folders[1])) ~= slug(project_name(r)) then fmt.basename(folders[1]) else nil
				elseif collision then
					props.detail = fmt.path(string.match(r.root or "", "^(.*)[/\\][^/\\]+[/\\]?$") or r.root or "")
				end
				props.title = if s.view == "mine" then table.concat(r.folders or {}, "\n") else r.root
				props.role = if open then "tern-projects.project-open" else "tern-projects.project"
				props.actions = {click = "activate=" .. fmt.pct(key), dblclick = "ignore", menu = menu_ids(s, key)}
			elseif row.kind == "more" then
				props.icon = "chev"; props.label = "Show " .. tostring(r.count) .. " more"; props.role = "tern-projects.more"
				props.actions = {click = "activate=" .. fmt.pct(key)}
			elseif row.kind == "fewer" then
				props.icon = "chev"; props.label = "Show fewer"; props.role = "tern-projects.more"
				props.actions = {click = "activate=" .. fmt.pct(key)}
			elseif row.kind == "start" then
				props.icon = "pen"; props.label = "Start a chat in " .. project_name(r); props.role = "tern-projects.more"
				props.actions = {click = "activate=" .. fmt.pct(key)}
			elseif row.kind == "older" then
				props.icon = "search"; props.label = "Search all " .. tostring(r.count or 0) .. " chats in " .. project_name(r); props.role = "tern-projects.more"
				props.actions = {click = "activate=" .. fmt.pct(key)}
			elseif row.kind == "note" then
				props.label = r.label; props.disabled = true; props.role = if child then "tern-projects.child-note" else "tern-projects.note"
			end
			table.insert(items, ui.node("item", props))
		end
	end
	local sel_row = s.rows[s.sel or ""]
	local selected = if sel_row and sel_row.kind ~= "cardbtn" then "main.rows." .. s.sel else nil
	local empty = if not s.loaded then {ui.span(if s.mode == "search" then "Searching…" else "Loading chats…", "muted")} else nil
	return ui.node("list", {key = "rows", role = "tern-projects.rows", max = 1, selected = selected, empty = empty}, items)
end
local function pager(s: any): any?
	if s.mode == "search" then
		if s.total <= SEARCH_PAGE then return nil end
		local from, to = s.offset + 1, math.min(s.total, s.offset + #s.results)
		return ui.el("div", {key = "pager", class = "tp-pager"}, {
			c.icon_button("chev-r", "search-page=prev", "Previous page", {variant = "ghost", size = "xs", class = "flip", disabled = s.loading or s.offset == 0}),
			ui.el("span", {class = "tp-pager-t", text = tostring(from) .. "–" .. tostring(to) .. " of " .. tostring(s.total)}),
			c.icon_button("chev-r", "search-page=next", "Next page", {variant = "ghost", size = "xs", disabled = s.loading or s.offset + SEARCH_PAGE >= s.total}),
		})
	end
	if #s.order <= PAGE then return nil end
	local pages = math.ceil(#s.order / PAGE)
	return ui.el("div", {key = "pager", class = "tp-pager"}, {
		c.icon_button("chev-r", "page=prev", "Previous page", {variant = "ghost", size = "xs", class = "flip", disabled = s.page == 0}),
		ui.el("span", {class = "tp-pager-t", text = "Page " .. tostring(s.page + 1) .. " of " .. tostring(pages)}),
		c.icon_button("chev-r", "page=next", "Next page", {variant = "ghost", size = "xs", disabled = s.page + 1 >= pages}),
	})
end
local function setting_row(key: string, focused: boolean, label: string, desc: string, control: any): any
	return ui.el("div", {key = key, class = "tp-set" .. (if focused then " kfocus" else "")}, {
		ui.el("div", {class = "tp-set-l"}, {ui.el("div", {class = "tp-set-t", text = label}), ui.el("div", {class = "tp-set-d", text = desc})}),
		ui.el("div", {class = "tp-set-c"}, {control}),
	})
end
local function settings_view(s: any): any
	local f = s.settings_focus
	local default = s.settings.default_harness
	local harness = if type(default) == "string" then fmt.agent_label(default) else "Ask every time"
	local index_desc = if s.indexing then "Indexing chat history…" elseif s.indexed_at then "Updated " .. fmt.ago(s.indexed_at * 1000) .. " · every 2 min" else "Updated automatically · every 2 min"
	local archived = s.archived_total
	local mod = if tern.runtime.os == "macos" then "cmd" else "ctrl"
	local shortcuts = {
		{"Open Projects", if tern.runtime.os == "macos" then {"alt", "cmd", "s"} else {"ctrl", "alt", "s"}},
		{"Search everywhere", if tern.runtime.os == "macos" then {"ctrl", "alt", "cmd", "s"} else {"ctrl", "alt", "shift", "s"}},
		{"Search from Projects / Settings", {"/"}},
		{"Cycle Mine / Discovered / Search / Settings", {"tab"}},
		{"Cycle views backwards", {"shift", "tab"}},
		{"Move selection", {"up", "down"}},
		{"Move 10 rows / cross result pages", {"page_up", "page_down"}},
		{"Expand / collapse project", {"left", "right"}},
		{"Resume chat / toggle project", {"enter"}},
		{"Actions / search or panel options", {"shift", "f10"}},
		{"New chat", {mod, "n"}},
		{"New chat with…", {"shift", mod, "n"}},
		{"Copy resume command", {"ctrl", "y"}},
		{"Refresh", {"ctrl", "r"}},
		{"Clear search / selection; close sheet", {"esc"}},
	}
	local keys = {}
	for _, sc in shortcuts do
		table.insert(keys, ui.el("div", {class = "tp-kbd-row"}, {ui.el("span", {text = sc[1]}), ui.node("kbd", {keys = sc[2]})}))
	end
	return ui.el("div", {key = "settings", class = "tp-settings"}, {
		ui.el("div", {class = "tp-sec-t", text = "New chats"}),
		ui.el("div", {class = "tp-card"}, {
			setting_row("default", f == "default", "Default harness", "Used when you start a new chat.",
				c.button(harness, "default-picker", {size = "sm", trailing = "chev", icon = if type(default) == "string" then fmt.agent_icon(default) else nil, focused = f == "default"})),
		}),
		ui.el("div", {class = "tp-sec-t", text = "Appearance"}),
		ui.el("div", {class = "tp-card"}, {
			setting_row("icons", f == "icons", "Harness icons", "Tints each harness glyph.",
				c.seg({{id = "color", label = "Color"}, {id = "monochrome", label = if (s.cols or 44) < 48 then "Neutral" else "Monochrome", title = "Monochrome"}}, s.settings.icon_style or "color", "icon-style", {size = "sm", focused = if f == "icons" then s.settings.icon_style or "color" else nil})),
		}),
		ui.el("div", {class = "tp-sec-t", text = "Chats"}),
		ui.el("div", {class = "tp-card"}, {
			setting_row("archived", f == "archived", "Archived chats", if archived and archived > 0 then fmt.plural(archived, "chat") .. " hidden from lists and search." else "Archived chats are hidden from lists and search.",
				c.button("View", "archived", {size = "sm", focused = f == "archived"})),
			setting_row("refresh", f == "refresh", "Chat index", index_desc,
				c.button(if s.indexing then "Refreshing…" else "Refresh", "refresh", {size = "sm", icon = "redo", disabled = s.indexing, focused = f == "refresh"})),
		}),
		ui.el("div", {class = "tp-sec-t", text = "Keyboard"}),
		ui.el("div", {class = "tp-card tp-keys"}, keys),
	})
end
local function dock_view(s: any): any?
	local row = s.rows[s.sel or ""]
	if not row or (row.kind ~= "session" and row.kind ~= "project") then return nil end
	local r = row.ref
	local kids: {any} = {}
	local narrow = (s.cols or 44) < 48
	if row.kind == "session" then
		local meta = fmt.agent_label(r.agent or "") .. " · " .. fmt.plural(r.messages or 0, "message") .. " · " .. fmt.age(r.updated_ms or 0)
		local heading = {harness_icon(s, r.agent or ""), ui.node("text", {text = r.title or r.uid, lines = if narrow then 1 else 2, grow = 1, role = "tern-projects.dock-title"})}
		if r.pinned then table.insert(heading, ui.el("span", {class = "tp-pin", title = "Pinned", aria = "Pinned"}, {ui.node("icon", {name = "pin"})})) end
		table.insert(kids, ui.el("div", {class = "tp-dock-h"}, heading))
		table.insert(kids, ui.node("text", {text = meta, truncate = "end", role = "tern-projects.dock-meta"}))
		local where = {ui.node("text", {text = display_path(r.cwd or "", math.max(18, (s.cols or 44) - 12)), truncate = "start", grow = 1, title = r.cwd, role = "tern-projects.dock-path"})}
		if type(r.branch) == "string" and r.branch ~= "" then table.insert(where, ui.node("text", {text = "⎇ " .. r.branch, title = r.branch, truncate = "end", role = "tern-projects.dock-branch"})) end
		table.insert(kids, ui.el("div", {class = "tp-dock-where"}, where))
		if type(r.last_assistant) == "string" and r.last_assistant ~= "" then
			table.insert(kids, ui.node("text", {text = r.last_assistant, lines = 2, role = "tern-projects.dock-preview"}))
		end
		table.insert(kids, ui.el("div", {class = "tp-dock-a"}, {
			c.button("Resume", "resume", {variant = "primary", size = "sm", icon = "play", title = "Resume (⏎)"}),
			if narrow then c.icon_button("copy", "copy-resume-command", "Copy resume command (Ctrl+Y)", {size = "sm"}) else c.button("Copy command", "copy-resume-command", {size = "sm", icon = "copy", title = "Copy resume command (Ctrl+Y)"}),
			c.icon_button("more", "actions", "More actions (⇧F10)", {size = "sm"}),
		}))
	else
		local mine = s.view == "mine"
		local folders = if mine then (r.folders or {}) else {r.root}
		table.insert(kids, ui.el("div", {class = "tp-dock-h"}, {ui.node("icon", {name = if mine then (r.icon or "folder") else "folder"}), ui.node("text", {text = project_name(r), lines = 1, role = "tern-projects.dock-title"})}))
		local meta = fmt.plural(r.count or 0, "chat")
		if mine then meta ..= " · " .. fmt.plural(#folders, "folder") end
		if r.updated_ms and (r.count or 0) > 0 then meta ..= " · " .. fmt.age(r.updated_ms) end
		table.insert(kids, ui.node("text", {text = meta, truncate = "end", role = "tern-projects.dock-meta"}))
		if folders[1] then
			local paths = {ui.node("text", {text = display_path(folders[1], math.max(18, (s.cols or 44) - 12)), truncate = "start", grow = 1, title = table.concat(folders, "\n"), role = "tern-projects.dock-path"})}
			if #folders > 1 then table.insert(paths, ui.node("text", {text = "+" .. tostring(#folders - 1) .. " more", role = "tern-projects.dock-more"})) end
			table.insert(kids, ui.el("div", {class = "tp-dock-where"}, paths))
		end
		table.insert(kids, ui.el("div", {class = "tp-dock-a"}, {
			c.button("New chat", "new-chat=" .. fmt.pct(s.sel), {variant = "primary", size = "sm", icon = "pen", title = "New chat in " .. project_name(r), disabled = s.launch_pending}),
			c.button(if mine then "Edit" else "Save as project…", "edit-selected", {size = "sm", icon = if mine then "sliders" else "folder-plus"}),
			c.icon_button("more", "actions", "More actions (⇧F10)", {size = "sm"}),
		}))
	end
	return ui.col({ui.el("div", {key = "inspector", class = "tp-dock"}, kids)})
end
local function confirm_view(s: any): any
	local conf = s.confirm
	local buttons = {}
	for i, b in conf.buttons do
		if i == 2 then table.insert(buttons, c.grow()) end
		table.insert(buttons, c.button(if s.busy then (if b.id == "cancel" then "Hide" else s.busy_label or "Working…") else b.label, "confirm=" .. b.id, {key = b.id, variant = if b.id == "cancel" then "ghost" else b.variant, focused = conf.focus == i, disabled = b.disabled or (s.busy and b.id ~= "cancel")}))
	end
	local body = table.clone(conf.body)
	if s.busy then table.insert(body, ui.el("div", {class = "tp-loading"}, {ui.node("spinner", {style = "ring"}), ui.el("span", {text = (s.busy_label or "Working…") .. " You can hide this dialog; the change will continue."})})) end
	if conf.error then table.insert(body, c.banner("op-error", "error", "warn", conf.error)) end
	return ui.node("overlay", {key = "confirm:" .. conf.kind, modal = true, size = "sm", head = conf.title, role = "tern-projects.dialog"}, {
		ui.el("div", {key = "body", class = "tp-form"}, body),
		c.footer(buttons),
	})
end
local function menu_view(s: any): any
	local m = s.menu
	local kids = {}
	local n = 0
	for _, id in m.ids do
		if id == "-" then
			n += 1
			table.insert(kids, ui.node("rule", {key = "sep" .. tostring(n)}))
		else
			local meta = MENU_META[id] or {label = id}
			local checked = id == "scope=" .. s.scope or id == "sort=" .. s.sort
			local icon = if checked then "check" else meta.icon
			table.insert(kids, ui.node("item", {key = id, label = meta.label, icon = icon, tone = meta.tone, hint = meta.hint, role = if meta.tone == "error" then "tern-projects.menu-danger" else "tern-projects.menu-item", actions = {click = "menu=" .. id}}))
		end
	end
	local anchor: any = "center"
	local idx = table.find(s.order, m.target)
	if idx and math.floor((idx - 1) / PAGE) == s.page then anchor = {node = "main.rows." .. m.target, side = "below"} end
	-- Non-modal like a native context menu: a click elsewhere dismisses it and still lands.
	local modal = anchor == "center"
	return ui.node("overlay", {key = "menu", modal = modal, anchor = anchor, size = "sm", head = m.title, aria = m.title or "Actions", role = "tern-projects.menu"}, {
		ui.node("list", {key = "list", selected = "layer.menu.list." .. m.selected}, kids),
	})
end
local function layer_view(s: any): any?
	if s.editor then
		local d = s.editor
		if d.browser then return ui.col({picker.view(d.browser.picker, s.busy)}) end
		return ui.col({editor.view(d, s.busy)})
	end
	if s.picker then return ui.col({picker.view(s.picker, s.busy)}) end
	if s.confirm then return ui.col({confirm_view(s)}) end
	if s.menu then return ui.col({menu_view(s)}) end
	return nil
end

-- Block --------------------------------------------------------------------------------------------

local panel: BlockDef<any> = {
	init = function(cx, args, saved)
		local s: any = {cx = cx, mode = args and args[1] or "projects", ctx_cwd = args and args[2] or "", view = "mine", projects = {}, pinned = {}, recent = {}, expanded = {}, more = {}, settings = {icon_style = "color"}, query = tf.new(""), query_all = false, scope = "all", sort = "updated", results = {}, total = 0, offset = 0, page = 0, rows = {}, order = {}, gen = 0, tokens = {}, alive = true, busy = false, indexing = false, loaded = false, archived = {}, launch_pending = false, launch_slow = false, launch_wait_seq = 0}
		if saved then
			if saved.view == "mine" or saved.view == "discovered" then s.view = saved.view end
			if saved.scope == "workspace" or saved.scope == "repository" or saved.scope == "all" then s.scope = saved.scope end
			if saved.sort == "updated" or saved.sort == "relevance" then s.sort = saved.sort end
			if type(saved.query) == "string" then tf.set(s.query, saved.query) end
		end
		pcall(tern.fs.write, tern.plugin.data .. "/panel-cols", tostring(cx.cols))
		rebuild(s)
		refresh(s)
		local function tick()
			tern.timer(120000, function() if not live(s) then s.alive = false; return end; refresh(s); tick() end)
		end
		tick()
		return s
	end,
	save = function(s) return {mode = s.mode, view = s.view, query = s.query.text, scope = s.scope, sort = s.sort} end,
	-- Width-dependent layout (row previews, icon-only controls) must follow pane resizes.
	resize = function(s, cols, _rows, _cx) s.cols = cols end,
	title = function(s) return if s.mode == "projects" then "Projects" elseif s.mode == "search" then "Projects · Search" else "Projects · Settings" end,
	view = function(s, cx)
		-- `cx.cols` is a placeholder until the pane reports a resize; only trust reported widths.
		local cols = s.cols or 44
		local children: {any} = {nav_view(s, cols < 48)}
		local building = cli.status()
		if building then
			table.insert(children, c.banner("building", "info", nil, building .. " This runs once after installing or updating and can take a few minutes.", nil, true))
		end
		if s.launch_pending then
			table.insert(children, c.banner("launch-wait", "info", nil, wait_text(s, false), {c.button("Cancel", "cancel-launch", {size = "xs"})}, true))
		end
		if s.error then
			local buttons = {}
			if s.error_reload then table.insert(buttons, c.button("Reload", "reload", {size = "xs"})) end
			table.insert(buttons, c.icon_button("x", "dismiss-error", "Dismiss", {variant = "ghost", size = "xs"}))
			table.insert(children, c.banner("error", "error", "warn", fmt.error(s.error), buttons))
		end
		if s.busy and not s.editor and not s.picker and not s.confirm then table.insert(children, c.banner("busy", "info", nil, (s.busy_label or "Working…") .. " The change is continuing.", nil, true)) end
		if s.mode == "settings" then
			table.insert(children, settings_view(s))
		else
			if s.mode == "projects" then table.insert(children, projects_bar(s, cols))
			else for _, n in search_bar(s) do table.insert(children, n) end end
			table.insert(children, list_view(s))
			local pg = pager(s)
			if pg then table.insert(children, pg) end
		end
		local result: any = {main = ui.col(children)}
		if s.mode ~= "settings" then result.dock = dock_view(s) end
		result.layer = layer_view(s)
		return result
	end,
	key = function(s, k, _cx)
		if s.busy then if k.name == "escape" then hide_busy(s) end; return true end
		if s.editor then
			local d = s.editor
			if d.browser then
				local handled, intent, value = editor.browser_key(d, k)
				browser_intent(s, intent, value)
				return handled
			end
			local handled, intent, value = editor.key(d, k)
			editor_intent(s, intent, value)
			return handled
		end
		if s.picker then
			local _, intent = picker.key(s.picker, k)
			picker_action(s, intent)
			return true
		end
		if s.confirm then return confirm_key(s, k) end
		if s.menu then return menu_key(s, k) end
		local name = k.name
		if name == "tab" then
			local stops = {"mine", "discovered", "search", "settings"}
			local current = if s.mode == "projects" then s.view else s.mode
			local i = table.find(stops, current) or 1
			local next = stops[(i - 1 + (if k.shift then -1 else 1)) % #stops + 1]
			if next == "mine" or next == "discovered" then
				if s.mode ~= "projects" then s.view = next; s.projects = {}; s.sel = nil; s.loaded = false; change_mode(s, "projects")
				else action(s, "view", next) end
			else change_mode(s, next) end
			return true
		end
		if name == "r" and k.ctrl then refresh(s); return true end
		if name == "n" and (k.meta or (k.ctrl and tern.runtime.os ~= "macos")) then new_chat(s, nil, k.shift == true); return true end
		if name == "/" and s.mode ~= "search" then change_mode(s, "search"); return true end
		if name == "escape" and s.mode ~= "search" then
			if s.launch_pending then cancel_flow(s); return true end
			s.sel = nil; s.settings_focus = nil; return true
		end
		if s.mode == "settings" then return settings_key(s, k) end
		if name == "f10" and k.shift then open_menu(s, nil); return true end
		if name == "up" or name == "down" then nav(s, if name == "up" then -1 else 1); return true end
		if name == "page_up" or name == "page_down" then for _ = 1, 10 do nav(s, if name == "page_up" then -1 else 1); if s.loading then break end end; return true end
		if name == "enter" then activate(s, s.sel); return true end
		if name == "y" and k.ctrl then action(s, "copy-resume-command", nil); return true end
		if s.mode == "projects" then
			if name == "left" or name == "right" then
				local row = s.rows[s.sel or ""]
				if row and row.kind == "project" then s.expanded[s.sel] = name == "right"; rebuild(s)
				elseif row and row.parent and name == "left" then s.sel = row.parent end
				return true
			end
		elseif s.mode == "search" then
			local before = s.query.text
			if name == "escape" and s.sel then s.sel = nil; return true end
			if name == "escape" then
				if before == "" then change_mode(s, "projects"); return true end
				tf.set(s.query, ""); s.query_all = false
			else
				local handled, all = input.key(s.query, s.query_all, k, tern.runtime.os)
				s.query_all = all
				if not handled then return false end
			end
			if before ~= s.query.text then search_changed(s) end
			return true
		end
		return false
	end,
	event = function(s, ev, cx)
		local id, name = ev.id or "", ev.ev
		if s.menu and string.sub(id, 1, 11) ~= "layer.menu." and id ~= "layer.menu" then s.menu = nil end
		if name == "focus" or name == "edit" then
			if s.busy then return end
			local field
			if s.editor and not s.editor.browser then
				if id ~= NAME_FIELD then return end
				if name == "focus" then s.editor.focus = "name" end
				if s.editor.focus ~= "name" then return end
				field = s.editor.name
			elseif not s.editor and not s.picker and not s.confirm and not s.menu and s.mode == "search" and id == SEARCH_FIELD then field = s.query end
			if not field then return end
			if name == "focus" then cx:frame({{"focus", id}})
			else
				local before = field.text
				if input.edit(field, ev) then
					if field == s.query then
						s.query_all = false
						if before ~= field.text then search_changed(s) end
					else s.editor.selected_all = false; s.editor.name_touched = true end
				end
			end
		elseif name == "select" or name == "activate" then
			local item = ev.item or ""
			if s.editor and s.editor.browser then
				local p = s.editor.browser.picker
				p.selected = item
				editor.sync_browser(s.editor)
				if name == "activate" then local _, intent, value = editor.browser_key(s.editor, {name = "enter"} :: any); browser_intent(s, intent, value) end
			elseif s.picker then
				s.picker.selected = item
				if name == "activate" then local a = picker.primary(s.picker); if a then picker_action(s, a.id) end end
			elseif not s.editor and not s.confirm and not s.menu and id == "main.rows" then
				local key = string.match(item, "^main%.rows%.(.+)$") or item
				local row = s.rows[key]
				if row and navigable(row) then s.sel = key; if name == "activate" then activate(s, key) end end
			end
		elseif name == "action" then
			local act, value = ev.act or "", ev.value
			if string.sub(act, 1, 5) == "open=" then
				local payload = string.sub(act, 6)
				local mode, cwd = string.match(payload, "^([^:]+):?(.*)")
				if mode then s.editor = nil; cancel_flow(s); if cwd ~= "" then s.ctx_cwd = cwd end; change_mode(s, mode) end
			elseif s.editor then
				if s.editor.browser then
					if act == "close" or act == "add" then browser_intent(s, act, nil)
					elseif act == "go" then local _, intent, v = editor.browser_key(s.editor, {name = "enter"} :: any); browser_intent(s, intent, v)
					elseif act == "clear" then
						tf.set(s.editor.browser.picker.query, "")
						s.editor.browser.picker.query_all = false
						browser_query(s)
					end
				elseif act == "editor" then action(s, act, value) end
			elseif s.picker then picker_action(s, act)
			elseif s.confirm then
				if act == "confirm" and value then confirm_action(s, value) end
			elseif s.menu then
				if act == "menu" and value then menu_activate(s, value) end
			else
				if value == nil then
					local target = string.match(id, "^main%.rows%.(.+)$")
					if target and s.rows[target] and table.find(menu_ids(s, target), act) then value = fmt.pct(target) end
				end
				action(s, act, value)
			end
		end
		cx:render()
	end,
}
return panel
