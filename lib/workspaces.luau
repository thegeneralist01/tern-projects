--!strict
-- Host-only management facade and tab view. All OVNFS/Git argv live in the Rust adapter.
local cli = require("./cli")
local fmt = require("./fmt")
local tf = require("./textfield")
local input = require("./input")
local c = require("./ui")
local ui = tern.ui
local M = {}
local AVAILABILITY_KEY = "workspace-availability:v1"
local function search_environment(): string
	local values = {}
	for _, key in {"PATH", "PATHEXT", "HOME", "USERPROFILE", "LOCALAPPDATA"} do
		table.insert(values, tern.getenv(key) or "")
	end
	return table.concat(values, "\n")
end
local running = 0
local queue: {any} = {}
local pump: () -> ()
pump = function()
	while running < 3 and #queue > 0 do
		local job = table.remove(queue, 1)
		if job.current() then
			running += 1
			local function finish(data: any, err: string?)
				running -= 1
				if job.current() then job.cb(data, err) end
				pump()
			end
			local function start(err: string?)
				if err then finish(nil, err) else cli.request({op = "workspaces", request = job.payload}, finish) end
			end
			if job.prepare then job.prepare(start) else start(nil) end
		end
	end
end
local function request(payload: any, cb: (any, string?) -> (), current: () -> boolean, prepare: any?)
	table.insert(queue, {payload = payload, cb = cb, current = current, prepare = prepare})
	pump()
end
local function act(verb: string, key: string?): string return "ws=" .. verb .. ":" .. fmt.pct(key or "") end
local function under(path: string, root: string): boolean
	if tern.runtime.os == "windows" then path = path:gsub("\\", "/"); root = root:gsub("\\", "/") end
	return path == root or string.sub(path, 1, #root + 1) == root .. "/"
end
local function target(w: any, key: string): any
	for _, checkout in w.checkouts do if checkout.name == key then return checkout end end
	return nil
end
local function agent_inside(agent: any, location: any, checkout: any): boolean
	if type(location) == "string" then return type(checkout.location) == "string" and under(location, checkout.location) end
	local cwd = agent.cwd
	return type(cwd) == "string" and ((type(checkout.path) == "string" and under(cwd, checkout.path)) or (type(checkout.location) == "string" and under(cwd, checkout.location)))
end
local function grouped_agents(w: any): any
	local out = {}
	for _, agent in w.agents do
		local best, length = nil, -1
		local location = w.locations[agent.cwd or ""]
		for _, checkout in w.checkouts do
			local size = #(if type(checkout.location) == "string" then checkout.location else checkout.path)
			if agent_inside(agent, location, checkout) and size > length then best = checkout; length = size end
		end
		if best then
			out[best.name] = out[best.name] or {}
			table.insert(out[best.name], agent)
		end
	end
	return out
end
local function blocked(w: any, checkout: any, removing: boolean): string?
	for _, agent in grouped_agents(w)[checkout.name] or {} do
		if (removing and agent.live) or agent.state == "working" or agent.state == "waiting_input" then
			return if removing then "An agent is still live here. Close its tab before removing this workspace." else "An agent is working or waiting for you here. Finish or stop it before applying."
		end
	end
	return nil
end
local function payload(checkout: any, verb: string): any return {action = verb, name = checkout.name, identity = checkout.identity} end
local function note_paths(title: string, paths: {any}): {any}
	local kids: {any} = {c.label(title, tostring(#paths))}
	if #paths == 0 then table.insert(kids, c.note("None")) end
	for i, item in paths do
		local text = if type(item) == "string" then item else tostring(item.path) .. (if item.reason then " · " .. item.reason else "")
		table.insert(kids, ui.node("text", {key = title .. ":" .. tostring(i), text = text, lines = 2, role = "tern-projects.workspace-path"}))
	end
	return kids
end
local function append(out: {any}, nodes: {any}) for _, node in nodes do table.insert(out, node) end end
local function result_body(data: any): {any}
	local merged, conflicts, untouched = {}, {}, {}
	for _, item in data.applied or {} do table.insert(merged, item.path) end
	for _, path in data.merged or {} do table.insert(merged, path .. " · three-way merge") end
	for _, item in data.conflicts or {} do table.insert(if item.markers then conflicts else untouched, item) end
	local body = {c.note("Target: " .. tostring(data.source or "")), c.note("Source-only edits are retained. Conflicts with markers need resolving in the target folder.")}
	append(body, note_paths("Applied + merged", merged))
	append(body, note_paths("Conflict markers", conflicts))
	append(body, note_paths("Left untouched", untouched))
	return body
end
local function info_body(info: any): {any}
	local body = {}
	append(body, note_paths("Source changes", info.changes or {}))
	if info.commits and type(info.commits) == "table" then
		append(body, note_paths(tostring(info.commits.count) .. " commits not brought back", info.commits.titles or {}))
		if info.commits.count > 25 then table.insert(body, c.note("Showing the newest 25 commit titles.")) end
		if info.commits.count > 0 and type(info.commits.head) ~= "string" then
			local refs = info.commits.refs or {}
			table.insert(body, c.note("These commits are on retained branches or refs" .. (if #refs > 0 then ": " .. table.concat(refs, ", ") else "") .. ", not the checked-out branch, which has no commits yet. Switch this workspace to a branch containing these commits (or create a branch at a retained ref), then use Save branch. Save branch, then remove cannot save an unborn HEAD.", "warning"))
		end
	end
	if type(info.error) == "string" then table.insert(body, c.banner("git-error", "error", "warn", info.error)) end
	return body
end
local function dialog(w: any, kind: string, title: string, body: {any}, buttons: {any}, extra: any?): any
	local d = extra or {}
	d.kind = kind; d.title = title; d.body = body; d.buttons = buttons; d.focus = 1
	w.dialog = d
	return d
end
local function close(w: any)
	local cancel = w.authorization
	w.dialog = nil; w.nonce = nil; w.authorization = nil; w.flow += 1
	if cancel then cancel(nil, "Workspace authorization cancelled; retry from Workspaces.") end
end
local function failure(s: any, err: string)
	local w = s.workspaces
	if w.dialog and w.dialog.kind == "menu" then close(w) end
	w.error = fmt.error(err)
	if w.dialog then w.dialog.error = w.error end
	s.cx:toast("error", "Workspace operation blocked", w.error)
	s.cx:render()
end
function M.init(s: any, live: (any) -> boolean, launch: (any, string, boolean?) -> (), add_project: (any, string) -> (), on_missing: (any) -> ())
	local ok, cached = pcall(tern.kv.get, AVAILABILITY_KEY)
	local available = false
	if ok and type(cached) == "table" and cached.available == true and cached.environment == search_environment() and type(cached.path) == "string" then
		local exists, present = pcall(tern.fs.exists, cached.path)
		available = exists and present == true
	end
	s.workspaces = {available = available, on_missing = on_missing, detection = 0, state = "loading", membership = "loading", library_seq = 0, projects = {}, roots = {}, checkouts = {}, cache = {}, agents = {}, locations = {}, order = {}, rows = {}, seq = 0, flow = 0, live = live, launch = launch, add_project = add_project, active = false, busy = false}
	s.workspaces.cached_available = ok and type(cached) == "table" and cached.available == true
end
local function alive(s: any): boolean return s.workspaces.live(s) end
local function availability(s: any, available: boolean, path: string?)
	local w = s.workspaces
	local was_available = w.available or w.cached_available
	w.cached_available = nil
	w.available = available
	-- This is a host capability cache, not pane view state or an OVNFS checkout list.
	pcall(tern.kv.set, AVAILABILITY_KEY, {available = available, path = path, environment = search_environment()})
	if not available then
		if w.active then M.leave(s) end
		if was_available then s.cx:open("tern-projects://workspace?disable=1") end
		if s.mode == "workspaces" then w.on_missing(s) end
	end
end
function M.detect(s: any, done: ((boolean) -> ())?)
	local w = s.workspaces
	w.detection += 1
	local detection = w.detection
	local current = function() return alive(s) and w.detection == detection end
	cli.request({op = "workspaces", request = {action = "detect"}}, function(data, err)
		if not current() or err or not data then return end
		availability(s, data.available == true, data.path)
		if done then done(w.available) end
		s.cx:render()
	end, current)
end
local function agent_request(s: any)
	if alive(s) then s.cx:open("tern-projects://workspace?nonce=" .. fmt.pct(s.workspaces.nonce or "")) end
end
local function details(s: any, checkout: any, cb: (any) -> (), verb: string?)
	local w, flow = s.workspaces, s.workspaces.flow
	request(payload(checkout, verb or "details"), function(data, err)
		if err then failure(s, err) else cb(data); s.cx:render() end
	end, function() return alive(s) and w.active and w.flow == flow end)
end
local function load_roots(s: any, folders: {string}, initial: boolean)
	local w = s.workspaces
	local seq, library_seq = w.seq, w.library_seq
	w.root_polling = true
	request({action = "roots", paths = (tern.json :: any).array(folders)}, function(roots, err)
		w.root_polling = false
		if initial then
			w.membership, w.membership_error = if err then "error" else "ready", err
			if not err then w.roots = roots end
		else
			for _, folder in folders do w.roots[folder] = if err then {error = err} else roots[folder] end
		end
		s.cx:render()
	end, function() return alive(s) and w.active and w.seq == seq and w.library_seq == library_seq end)
end
local function load_library(s: any)
	local w = s.workspaces
	w.library_seq += 1; w.membership = "loading"; w.membership_error = nil; w.root_polling = false
	local seq, library_seq = w.seq, w.library_seq
	cli.request({op = "snapshot", view = "mine", per = 0, recent = 0}, function(data: any, err: string?)
		if not alive(s) or not w.active or w.seq ~= seq or w.library_seq ~= library_seq then return end
		if err then w.membership = "error"; w.membership_error = err; s.cx:render(); return end
		s.settings = data.settings or s.settings
		w.projects = data.projects or {}
		local folders = {}
		for _, project in w.projects do for _, folder in project.folders or {} do table.insert(folders, folder) end end
		load_roots(s, folders, true)
		s.cx:render()
	end)
end
function M.refresh(s: any)
	local w = s.workspaces
	if not alive(s) or not w.available or not w.active or w.polling then return end
	if not w.visible then return end
	agent_request(s)
	if w.membership == "ready" and not w.root_polling then
		local retry = {}
		for _, project in w.projects do for _, folder in project.folders or {} do
			if w.roots[folder] and w.roots[folder].error then table.insert(retry, folder) end
		end end
		if #retry > 0 then load_roots(s, retry, false) end
	end
	w.polling = true
	local seq = w.seq
	local current = function() return alive(s) and w.active and w.seq == seq end
	request({action = "poll"}, function(data, err)
		w.polling = false
		if err then w.state = "error"; w.status_error = err; s.cx:render(); return end
		w.state, w.status_error, w.external_busy = data.state, data.error, data.busy
		if data.state == "missing" then availability(s, false, nil); s.cx:render(); return end
		local old_pid = w.pid
		w.pid = data.pid
		w.checkouts = if data.state == "running" then data.checkouts or {} else {}
		for _, checkout in w.checkouts do
			local key = checkout.identity .. ":" .. checkout.touched .. ":" .. tostring(w.pid)
			local cached = w.cache[checkout.name]
			if not cached or cached.key ~= key or old_pid ~= w.pid then
				w.cache[checkout.name] = {key = key, loading = true}
				request(payload(checkout, "details"), function(info, detail_error)
					if w.cache[checkout.name] and w.cache[checkout.name].key == key then
						w.cache[checkout.name] = {key = key, info = info, error = detail_error}
						s.cx:render()
					end
				end, current)
			end
		end
		s.cx:render()
	end, current)
end
function M.enter(s: any)
	local w = s.workspaces
	if not w.available or w.active then return end
	w.active = true; w.seq += 1; w.polling = false
	w.visible = nil; w.agents_unknown = nil
	local seq = w.seq
	load_library(s)
	agent_request(s) -- One visibility handshake, not a periodic request while hidden.
	tern.timer(3000, function()
		if not alive(s) or not w.active or w.seq ~= seq or w.visible ~= nil then return end
		-- Non-local hosts cannot supply window agents. Listing is still useful;
		-- mutations continue to require a fresh, successful native authorization.
		w.visible = true; w.agents_unknown = true; w.agents = {}; w.locations = {}
		M.refresh(s)
	end)
	M.refresh(s)
	local function tick()
		tern.timer(2000, function()
			if not alive(s) or not w.active or w.seq ~= seq then return end
			M.refresh(s); tick()
		end)
	end
	tick()
end
function M.leave(s: any)
	local w = s.workspaces
	w.active = false; w.seq += 1; w.polling = false; close(w)
	-- Cancelled callbacks cannot leave a permanent "loading" entry behind.
	for name, cached in w.cache do if cached.loading then w.cache[name] = nil end end
end
function M.library_changed(s: any)
	local w = s.workspaces
	if not w.active then M.enter(s); return end
	-- Library reloads (including the panel's timer) are not view-leave transitions.
	load_library(s)
	M.refresh(s)
end
local function authorize(s: any, req: any, checkout: any, done: (string?) -> ())
	local w = s.workspaces
	if not alive(s) then done("Projects panel closed before workspace authorization"); return end
	local flow = w.flow
	local function snapshot(cb: (any, string?) -> ())
		w.snapshot_seq = (w.snapshot_seq or 0) + 1
		w.nonce = tostring(tern.now()) .. ":" .. tostring(s.cx.pane) .. ":" .. tostring(w.snapshot_seq)
		local nonce = w.nonce
		w.authorization = function(agents: any, err: string?)
			w.authorization = nil; w.nonce = nil
			cb(agents, err)
		end
		agent_request(s)
		tern.timer(2000, function()
			if w.nonce == nonce and w.authorization then w.authorization(nil, "Cannot read live Tern agents. Focus this local Projects panel and retry.") end
		end)
	end
	local function bind(agents: any): any
		local rows = (tern.json :: any).array({})
		for _, agent in agents do table.insert(rows, {cwd = agent.cwd, state = agent.state, live = agent.live}) end
		return rows
	end
	snapshot(function(agents, err)
		if err then done(err); return end
		local auth = payload(checkout, "authorize")
		auth.removing = req.action == "remove" or req.action == "save-before-remove"
		auth.agents = bind(agents)
		-- Resolve paths and re-list OVNFS before the final native snapshot, not after it.
		cli.request({op = "workspaces", request = auth}, function(prepared: any, auth_error: string?)
			if auth_error then done(auth_error); return end
			if not alive(s) or not w.active or w.flow ~= flow then done("Projects panel left or cancelled before workspace authorization"); return end
			snapshot(function(fresh, snapshot_error)
				if snapshot_error then done(snapshot_error); return end
				local locations = prepared.agent_locations
				for _, agent in fresh do
					local location = locations[agent.cwd or ""]
					local inside = agent_inside(agent, location, prepared)
					if inside and ((auth.removing and agent.live) or agent.state == "working" or agent.state == "waiting_input") then
						done(if auth.removing then "An agent is still live here. Close its tab before removing this workspace." else "An agent is working or waiting for you here. Finish or stop it before applying."); return
					end
					-- A new cwd could be a symlink into this checkout. Never resolve it
					-- slowly after this final snapshot or assume it is unrelated.
					if type(agent.cwd) == "string" and locations[agent.cwd] == nil then done("Agent locations changed during authorization; review and retry."); return end
				end
				req.agents = bind(fresh)
				done(nil) -- Synchronously sends the mutation in the already reserved slot.
			end)
		end)
	end)
end
local function operate(s: any, req: any, checkout: any?, cb: (any) -> ())
	local w = s.workspaces
	if w.busy or w.external_busy then failure(s, "A workspace operation is still running. Refresh before trying again."); return end
	w.busy = true; w.error = nil
	local enqueue: (any) -> ()
	enqueue = function(stage: any)
		-- Mutations outlive their view. Keep their callback even when the dialog/tab closes.
		request(stage, function(data, err)
			if not err and stage.action == "save-before-remove" then
				local removal = payload(checkout, "remove")
				removal.expected = data.token
				enqueue(removal) -- A separate queue admission and fresh agent/listing authorization.
				return
			end
			w.busy = false; w.cache = {}
			if not alive(s) then return end
			if err then failure(s, err) else w.error = nil; cb(data) end
			M.refresh(s); s.cx:render()
		end, function() return true end, if checkout then function(done) authorize(s, stage, checkout, done) end else nil)
	end
	enqueue(req)
end
local function buttons(): {any} return {{id = "cancel", label = "Cancel"}} end
local function form(s: any, kind: string, checkout: any?, source: string?)
	local w = s.workspaces
	close(w)
	local titles = {new = "New workspace", fork = "Fork workspace", branch = "Save commits as a new branch", ["save-remove"] = "Save commits as branch, then remove"}
	local d = dialog(w, kind, titles[kind], {c.note(if source then "Capture / apply target: " .. source else "Workspace: " .. checkout.name)}, {{id = "cancel", label = "Cancel"}, {id = "submit", label = if kind == "save-remove" then "Save branch & remove" else "Create", variant = if kind == "save-remove" then "danger" else "primary"}}, {checkout = checkout, source = source, field = tf.new(""), all = false})
	if kind == "branch" or kind == "save-remove" then table.insert(d.body, c.note("Commits only; working-tree source changes are not applied. Choose a branch that does not exist in the source repository."))
	else table.insert(d.body, c.note("1–64 lowercase letters, numbers, dots, underscores or hyphens; begin with a letter or number. Names are unique across OVNFS.")) end
	d.focus = 0 -- The text field precedes the buttons in the tab order.
	s.cx:frame({{"focus", "layer.workspace.form.name"}})
end
local function show(s: any, checkout: any, kind: string)
	local w = s.workspaces
	close(w)
	local d = dialog(w, "loading", "Checking " .. checkout.name .. "…", {c.note("Reading OVNFS and Git…")}, buttons())
	details(s, checkout, function(data)
		if w.dialog ~= d then return end
		if kind == "preview" then
			local body = result_body(data)
			table.insert(body, c.note("This preview does not write files. Applying writes clean changes and conflict markers into the target folder."))
			if data.details.commits and type(data.details.commits) == "table" and data.details.commits.count > 0 then table.insert(body, c.note(tostring(data.details.commits.count) .. " commits are not carried by Apply. Save them as a new branch separately.", "warning")) end
			dialog(w, "apply", "Preview apply: " .. checkout.name, body, {{id = "cancel", label = "Cancel"}, {id = "apply", label = "Apply to source", variant = "primary"}}, {checkout = checkout, info = data})
		elseif kind == "remove" then
			local body = {c.note("Remove " .. checkout.path .. "? The source folder is not deleted."), c.note("Ignored files, caches (including node_modules), and files over 100 MB are NOT counted in source changes. They are deleted too. Unreturned commits are lost unless saved as a branch.", "warning")}
			append(body, info_body(data))
			local choices = {{id = "cancel", label = "Cancel"}}
			if data.commits and type(data.commits) == "table" and data.commits.count > 0 then table.insert(choices, {id = "save-remove", label = "Save branch, then remove", variant = "primary"}) end
			table.insert(choices, {id = "remove", label = "Remove", variant = "danger", disabled = type(data.error) == "string"})
			dialog(w, "remove", "Remove workspace: " .. checkout.name, body, choices, {checkout = checkout, info = data})
		else dialog(w, "changes", "Changes: " .. checkout.name, info_body(data), {{id = "cancel", label = "Close"}}) end
	end, if kind == "preview" then "preview" else "details")
end
local function submit(s: any, id: string)
	local w, d = s.workspaces, s.workspaces.dialog
	if not d then return end
	if id == "cancel" then if not w.busy then close(w) else w.dialog = nil end; return end
	if w.busy then return end
	if id == "save-remove" then
		local checkout, info = d.checkout, d.info
		form(s, "save-remove", checkout, nil); w.dialog.info = info
		return
	end
	local checkout, req = d.checkout, nil
	if id == "submit" then
		local text = d.field.text
		if d.kind == "new" then req = {action = "new", source = d.source, name = text}
		elseif d.kind == "fork" then req = payload(checkout, "fork"); req.branch = text
		else req = payload(checkout, if d.kind == "save-remove" then "save-before-remove" else d.kind); req.branch = text; if d.info then req.expected = d.info.token end end
	elseif id == "apply" or id == "remove" then req = payload(checkout, id); req.expected = d.info.token end
	if not req then return end
	operate(s, req, checkout, function(data)
		if req.action == "apply" then dialog(w, "result", "Apply result: " .. checkout.name, result_body(data), {{id = "cancel", label = "Close"}})
		else close(w); s.cx:toast("success", if req.action == "remove" or req.action == "save-before-remove" then "Workspace removed" elseif req.action == "branch" then "Commits saved as branch" else "Workspace created", if req.action == "fork" then req.branch else req.name) end
	end)
end
local function choose(s: any, verb: string, key: string)
	local w = s.workspaces
	if verb == "refresh" then
		M.detect(s, function(available)
			if not available or not w.active then return end
			w.error = nil; w.cache = {}; load_library(s); M.refresh(s)
		end)
		return
	end
	if verb == "cancel" then close(w); return end
	if verb == "start" then operate(s, {action = "start"}, nil, function() close(w) end); return end
	if verb == "add-project" then w.add_project(s, key); return end
	if verb == "new" then
		close(w)
		local d = dialog(w, "loading", "Checking capture root…", {c.note("Project folder: " .. key)}, buttons())
		local flow = w.flow
		request({action = "roots", paths = {key}}, function(roots, err)
			if w.dialog ~= d then return end
			local root = roots and roots[key]
			if err or not root or root.error then failure(s, err or (root and root.error) or "Cannot resolve source folder"); return end
			w.roots[key] = root
			form(s, "new", nil, root.source)
			table.insert(w.dialog.body, 1, c.note("Project folder: " .. key))
			s.cx:render()
		end, function() return alive(s) and w.active and w.flow == flow end)
		return
	end
	if verb == "select" then w.selected = key; return end
	if verb == "agent" then s.cx:open("tern-projects://workspace?focus=" .. key); return end
	local checkout = target(w, key)
	if not checkout then return end
	if verb == "menu" then
		local choices = {}
		for _, item in {{"open", "New chat", "play"}, {"changes", "Show source changes / commits", "search"}, {"fork", "Fork workspace", "branch"}, {"preview", "Preview apply to source", "arrow-up"}, {"branch", "Save commits as branch", "branch"}, {"remove", "Remove workspace", "trash"}} do
			if item[1] ~= "branch" or checkout.git then table.insert(choices, {id = item[1], label = item[2], icon = item[3]}) end
		end
		close(w); dialog(w, "menu", checkout.name, {}, choices, {checkout = checkout}); return
	end
	if verb == "open" then close(w); w.launch(s, checkout.path)
	elseif verb == "fork" or verb == "branch" then form(s, verb, checkout, nil)
	elseif verb == "changes" or verb == "preview" or verb == "remove" then
		local reason = if verb ~= "changes" then blocked(w, checkout, verb == "remove") else nil
		if reason then failure(s, reason); return end
		show(s, checkout, verb)
	end
end
function M.event(s: any, ev: any): boolean
	local w = s.workspaces
	if not w.available then return false end
	if ev.ev == "action" and ev.act == "workspace-agents" then
		local ok, data = pcall(tern.json.decode, ev.value or "")
		if ok and type(data) == "table" then
			local became_visible = data.visible == true and not w.visible
			w.visible = data.visible == true
			w.agents = data.agents or {}
			w.agents_unknown = nil
			if w.nonce and data.nonce == w.nonce and w.authorization then
				w.authorization(w.agents, nil)
			end
			if w.active and w.visible then
				local paths = {}
				for _, agent in w.agents do if type(agent.cwd) == "string" then table.insert(paths, agent.cwd) end end
				local seq = w.seq
				request({action = "resolve", paths = (tern.json :: any).array(paths)}, function(locations, err)
					if not err then w.locations = locations; s.cx:render() end
				end, function() return alive(s) and w.active and w.visible and w.seq == seq end)
			end
			if became_visible and w.active then M.refresh(s) end
		end
		return true
	end
	if not w.active then return false end
	if w.dialog and w.dialog.field and (ev.ev == "edit" or ev.ev == "focus") and ev.id == "layer.workspace.form.name" then
		w.dialog.focus = 0
		if ev.ev == "edit" then input.edit(w.dialog.field, ev); w.dialog.all = false end
		return true
	end
	if ev.ev == "action" and ev.act == "ws" then
		local verb, key = string.match(ev.value or "", "^([^:]+):(.*)$")
		if not verb then return true end
		key = fmt.unpct(key)
		if verb == "dialog" then
			local d = w.dialog
			if d and d.kind == "menu" then choose(s, key, d.checkout.name) else submit(s, key) end
		else choose(s, verb, key) end
		return true
	end
	if (ev.ev == "select" or ev.ev == "activate") and ev.id == "main.workspace" then
		local key = string.match(ev.item or "", "^main%.workspace%.(.+)$") or ev.item
		w.selected = key
		if ev.ev == "activate" and w.rows[key] then choose(s, w.rows[key].kind, w.rows[key].key) end
		return true
	end
	return false
end
function M.key(s: any, k: any): boolean
	local w, d = s.workspaces, s.workspaces.dialog
	if d then
		if k.name == "escape" then submit(s, "cancel")
		elseif w.busy then return true
		elseif k.name == "enter" then
			local b = d.buttons[if d.field and d.focus == 0 then #d.buttons else d.focus]
			if b and not b.disabled then if d.kind == "menu" then choose(s, b.id, d.checkout.name) else submit(s, b.id) end end
		elseif k.name == "tab" or (not d.field and (k.name == "up" or k.name == "down" or k.name == "left" or k.name == "right")) then
			local step = if k.shift or k.name == "up" or k.name == "left" then -1 else 1
			if d.field then
				d.focus = (d.focus + step) % (#d.buttons + 1)
				s.cx:frame({{"focus", if d.focus == 0 then "layer.workspace.form.name" else (tern.json :: any).null}})
			else d.focus = (d.focus - 1 + step) % #d.buttons + 1 end
		elseif d.field and d.focus == 0 then local _, all = input.key(d.field, d.all, k, tern.runtime.os); d.all = all end
		return true
	end
	if k.name == "tab" or k.name == "/" then return false end
	if k.name == "r" and k.ctrl then choose(s, "refresh", ""); return true end
	local idx = table.find(w.order, w.selected) or 0
	if #w.order > 0 and (k.name == "down" or k.name == "up") then w.selected = w.order[math.clamp(idx + (if k.name == "down" then 1 else -1), 1, #w.order)]; return true end
	local row = w.rows[w.selected or ""]
	if k.name == "n" and (k.meta or (k.ctrl and tern.runtime.os ~= "macos")) then
		local checkout = if row and row.kind == "menu" then target(w, row.key) else nil
		if checkout then w.launch(s, checkout.path, k.shift == true) end
		return checkout ~= nil
	end
	if row and k.name == "enter" then choose(s, row.kind, row.key); return true end
	if row and k.name == "f10" and k.shift and row.kind == "menu" then choose(s, "menu", row.key); return true end
	return false
end
function M.view(s: any): {any}
	local w, rows, order, keys = s.workspaces, {}, {}, {}
	local function row(key: string, label: string, detail: string?, kind: string, value: string, icon: string, depth: number)
		keys[key] = {kind = kind, key = value}; table.insert(order, key)
		table.insert(rows, ui.node("item", {key = key, label = string.rep("\u{00a0}\u{00a0}", depth) .. label, detail = detail, icon = icon, role = "tern-projects.workspace-row", title = label .. (if detail then "\n" .. detail else ""), actions = {click = act(kind, value)}}))
	end
	local nodes: {any} = {ui.el("div", {key = "wsbar", class = "tp-bar"}, {c.note("OVNFS · " .. w.state), c.grow(), c.button("Add project", "create", {size = "sm", icon = "plus"}), c.icon_button("redo", act("refresh"), "Refresh workspaces", {size = "sm"})})}
	if w.state ~= "running" then
		local text = if w.state == "loading" then "Checking OVNFS…" elseif w.state == "stopped" then "OVNFS is not running" else "OVNFS error"
		table.insert(nodes, c.banner("ws-status", if w.state == "error" then "error" else "info", nil, text .. (if w.state ~= "stopped" and w.status_error then ": " .. fmt.error(w.status_error) else ""), if w.state == "stopped" then {c.button("Start", act("start"), {size = "sm", variant = "primary"})} else nil))
	end
	if w.agents_unknown then table.insert(nodes, c.banner("ws-agents", "info", nil, "Agent status is unavailable. Applying or removing still requires a live Tern agent check.")) end
	if w.external_busy or w.busy then table.insert(nodes, c.banner("ws-pending", "info", nil, "A workspace operation is in progress. It continues across plugin reloads; no operation will be replayed.", nil, true)) end
	if w.error then table.insert(nodes, c.banner("ws-error", "error", "warn", w.error)) end
	if w.membership == "loading" then table.insert(nodes, c.banner("ws-membership", "info", nil, "Loading Mine projects and workspace membership…"))
	elseif w.membership == "error" then table.insert(nodes, c.banner("ws-membership", "error", "warn", "Cannot load workspace project membership: " .. fmt.error(w.membership_error))) end
	local agents = grouped_agents(w)
	local function checkout_rows(prefix: string, checkouts: {any})
		local seen = {}
		local function checkout_row(checkout: any, depth: number)
			if seen[checkout.name] then return end
			seen[checkout.name] = true
			local cached = w.cache[checkout.name]
			local info = cached and cached.info
			local count = if info then tostring(#(info.changes or {})) else "…"
			local description = count .. " source changes"
			if info and info.commits and type(info.commits) == "table" then description ..= " · " .. tostring(info.commits.count) .. " commits not brought back" end
			if cached and (cached.error or (info and type(info.error) == "string")) then description ..= " · details unavailable" end
			description ..= " · " .. string.sub(checkout.touched, 1, 19):gsub("T", " ")
			row("checkout:" .. prefix .. checkout.name, checkout.name, description, "menu", checkout.name, if depth > 1 then "branch" else "folder-open", depth)
			for _, agent in agents[checkout.name] or {} do
				local labels = {working = "Working", waiting_input = "Waiting for you", idle = "Idle", done = "Done", failed = "Failed", exited = "Idle"}
				row("agent:" .. prefix .. tostring(agent.pane), labels[agent.state] or labels.idle, agent.title, "agent", tostring(agent.pane), if agent.state == "working" then "play" elseif agent.state == "waiting_input" then "bell" elseif agent.state == "failed" then "warn" else "circle", depth + 1)
			end
			for _, child in checkouts do if child.parent_name == checkout.name then checkout_row(child, depth + 1) end end
		end
		for _, checkout in checkouts do if not checkout.parent_name then checkout_row(checkout, 1) end end
		for _, checkout in checkouts do checkout_row(checkout, 1) end
	end
	local assigned = {}
	for _, project in w.projects do
		local project_key = "project:" .. tostring(project.id)
		-- Workspace projects are always expanded; keep their heading selectable after creation.
		row(project_key, project.name, nil, "select", project_key, project.icon or "folder", 0)
		for _, folder in project.folders or {} do
			local prefix = tostring(project.id) .. ":" .. folder .. ":"
			local resolved = w.roots[folder]
			local root = resolved and resolved.source
			local detail = if resolved and resolved.error then "Source unavailable: " .. fmt.error(resolved.error) else "New workspace"
			row("synthetic:" .. prefix .. "new", fmt.path(folder), detail, "new", folder, "folder-plus", 0)
			local checkouts = {}
			for _, checkout in w.checkouts do
				if checkout.source == folder or (root and checkout.source_location == root) then
					assigned[checkout.name] = true
					table.insert(checkouts, checkout)
				end
			end
			checkout_rows(prefix, checkouts)
		end
	end
	local sources, unassigned = {}, {}
	for _, checkout in w.checkouts do
		if w.membership == "ready" and not assigned[checkout.name] then
			local source = if type(checkout.source_location) == "string" then checkout.source_location else checkout.source
			if not sources[source] then
				sources[source] = {source = checkout.source, checkouts = {}}
				table.insert(unassigned, source)
			end
			table.insert(sources[source].checkouts, checkout)
		end
	end
	table.sort(unassigned)
	if #unassigned > 0 then
		table.insert(rows, c.header("synthetic:unassigned", "Not in a project"))
		for _, source in unassigned do
			local group = sources[source]
			local prefix = "unassigned:" .. source .. ":"
			table.insert(rows, ui.el("div", {key = "synthetic:" .. prefix .. "source", class = "tp-ws-source"}, {
				ui.node("text", {text = fmt.path(group.source), title = group.source, lines = 3, role = "tern-projects.workspace-path"}),
			}))
			row("synthetic:" .. prefix .. "add", "Add as project", group.source, "add-project", group.source, "plus", 0)
			checkout_rows(prefix, group.checkouts)
		end
	end
	if w.membership == "ready" and #w.projects == 0 and #w.checkouts == 0 then table.insert(rows, c.empty("ws-empty", "folder-plus", "Add a project to see its workspaces", "Uses the same curated Mine projects and folders as Projects. Non-Git folders work too.")) end
	w.rows, w.order = keys, order
	table.insert(nodes, ui.node("list", {key = "workspace", selected = if w.selected and keys[w.selected] then "main.workspace." .. w.selected else nil, max = 1, role = "tern-projects.rows"}, rows))
	return nodes
end
function M.layer(s: any): any
	local w, d = s.workspaces, s.workspaces.dialog
	if not d then return nil end
	local body = table.clone(d.body)
	if d.field then
		table.insert(body, c.label(if d.kind == "branch" or d.kind == "save-remove" then "New branch name" else "Workspace name"))
		table.insert(body, ui.el("div", {key = "form"}, {ui.node("input", {key = "name", text = d.field.text, cursor = tf.utf16(d.field), anchor = if d.all then 0 else nil, placeholder = if d.kind == "branch" or d.kind == "save-remove" then "saved-work" else "workspace-name", aria = "Workspace or branch name"})}))
	end
	if w.busy then table.insert(body, c.note("Working… The operation continues if this panel closes or the plugin reloads.")) end
	if d.error then table.insert(body, c.banner("error", "error", "warn", d.error)) end
	local choices = {}
	for i, b in d.buttons do table.insert(choices, c.button(b.label, act("dialog", b.id), {key = b.id, variant = b.variant, icon = b.icon, focused = d.focus == i, disabled = b.disabled or (w.busy and b.id ~= "cancel")})) end
	if d.kind == "menu" then
		return ui.node("overlay", {key = "workspace", modal = true, size = "sm", head = d.title, role = "tern-projects.dialog"}, {ui.el("div", {class = "tp-ws-menu"}, choices), c.footer({c.button("Cancel", act("dialog", "cancel"), {variant = "ghost"})})})
	end
	return ui.node("overlay", {key = "workspace", modal = true, size = if d.kind == "remove" then "md" else "sm", head = d.title, role = "tern-projects.dialog"}, {ui.el("div", {class = "tp-form tp-ws-form"}, body), c.footer(choices)})
end
return M
