--!strict
-- `sources` is supplied by test_chat_launch.py; load production modules unchanged.
local function fixture()
	local f: any = {now = 100, pending = {}, claims = {}, links = {}, tabs = 0}
	local updates = {tern_executable = function() return "tern" end}
	local fake: any = {
		ui = {}, runtime = {os = "macos"},
		json = {null = {}, decode = function(value) return value end},
		pane = {list = function() return {{pane = 42}} end},
		kv = {get = function(key) return f.claims[key] end, set = function(key, value) f.claims[key] = value end},
		now = function() return f.now * 1000 end,
		timer = function() return {cancel = function() end} end,
		on = function() end, command = function() end,
		log = {error = function(message) error(message) end},
		process = {run = function(argv, opts, cb) table.insert(f.pending, {argv = argv, cb = cb}) end},
		route = {link = function(handler) f.route = handler end},
	}
	local cache = {}
	local function load(name)
		if cache[name] then return cache[name] end
		local source = sources[name]
		local chunk = assert(loadstring(source, name))
		local environment = setmetatable({tern = fake, os = {time = function() return f.now end}, require = function(path)
			if path == "./lib/resume" or path == "./resume" then return load("lib/resume") end
			if path == "../lib/chat-launch" then return load("lib/chat-launch") end
			if path == "../lib/updates" or path == "./updates" then return updates end
			if path == "./lib/fmt" or path == "../lib/fmt" then return {pct = function(v) return v end, unpct = function(v) return v end} end
			-- The followed terminal folder stays inert here; scripts/test_follow_cwd.py covers it.
			if path == "./lib/follow-cwd" then return {start = function() end, track = function() end} end
			return {}
		end}, {__index = getfenv()})
		setfenv(chunk, environment)
		cache[name] = chunk()
		return cache[name]
	end
	local cx: any = {
		pane = 42, render = function() end, toast = function() end,
		open = function(_, url) table.insert(f.links, url) end,
		session = {panes = function() return {{pane = 42, block = "tern-projects.panel"}} end, focused = function() return 42 end},
		hosts = {current = function() return {kind = "local"} end},
		settings = {get = function() return {auto_acp = false} end},
		layout = {new_tab = function() f.tabs += 1; return 70 end},
	}
	local panel = load("blocks/panel")
	load("window")
	f.state = {cx = cx, alive = true, confirm = {kind = "launch", buttons = {}}, pending_plan = {cwd = "/project", command = "codex"}}
	f.start = function() panel.event(f.state, {ev = "action", act = "confirm", value = "launch-accept"}, cx) end
	f.reply = function(status, value)
		local request = table.remove(f.pending, 1)
		assert(request, "Expected a pending CLI request")
		request.cb({status = status, stdout = value, stderr = "", timed_out = status == -1})
	end
	f.receive = function()
		assert(#f.links == 1, "Expected one fallback link")
		f.route({url = f.links[1], pane = 42}, cx)
	end
	return f
end

local delayed = fixture()
delayed.start()
assert(next(delayed.claims) == nil, "Do not start fallback authorization while the CLI is still running")
delayed.now = 110
delayed.reply(-1, "")
local _, claim = next(delayed.claims)
assert(claim and claim.created == 110, "Fallback authorization must start after the full CLI timeout")
delayed.now = 119
delayed.receive()
assert(delayed.tabs == 1, "The fallback must still create a tab nine seconds after CLI failure")
delayed.receive()
assert(delayed.tabs == 1, "Consumed fallback authorization must reject replay")

local expired = fixture()
expired.start()
expired.now = 110
expired.reply(-1, "")
expired.now = 120
expired.receive()
assert(expired.tabs == 0, "Fallback authorization must expire ten seconds after CLI failure")

local successful = fixture()
successful.start()
successful.reply(0, {sessions = {{id = 7, tabs = {{blocks = {{id = 42}}}}}}})
successful.reply(0, {block = 70})
assert(next(successful.claims) == nil and #successful.links == 0, "Successful CLI launches must not authorize or send a fallback link")

print("Chat launch regressions passed: delayed fallback, expiry, replay, CLI success")
