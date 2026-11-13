--!strict
-- Host-side launches must not inherit the daemon's unrelated terminal/window.
local updates = require("./updates")
local resume = require("./resume")
local launch = {}

function launch.open(pane: number, cwd: string, command: string, resuming: boolean, alive: () -> boolean, fallback: () -> ())
	local bin = updates.tern_executable()
	local function run(argv: {string}, cb: (any) -> ())
		local finished = false
		local started, err = pcall(tern.process.run, argv, {timeout_ms = 10000}, function(result)
			finished = true
			if not alive() then return end
			if result.status ~= 0 then fallback(); return end
			cb(result)
		end)
		if not started then
			if finished then tern.log.error("Chat launch callback failed: " .. tostring(err))
			elseif alive() then fallback() end
		end
	end
	local function create(window: string, session: number, cmd: string)
		if not alive() then return end
		local session_id = string.format("%.0f", session)
		run({bin, "new", "tab", session_id, "--window", window, "--cwd", cwd, "--json", "--", cmd}, function(result)
			local ok, created = pcall(tern.json.decode, result.stdout)
			-- Success may already have created a tab: never fall back and duplicate it.
			if not ok or type(created) ~= "table" or type(created.block) ~= "number" then return end
			pcall(tern.process.run, {bin, "focus", session_id .. "/" .. string.format("%.0f", created.block), "--window", window}, {timeout_ms = 5000}, function() end)
		end)
	end
	local function with_settings(window: string, session: number)
		if not resuming then create(window, session, command); return end
		-- `plugin dir` follows TERN_CONFIG_DIR and the platform's config location.
		-- Read at launch, not plugin load, so changes to ACP preferences take effect.
		run({bin, "plugin", "dir", "--window", window}, function(result)
			local dir = string.gsub(result.stdout, "[\r\n]+$", "")
			local config = string.match(dir, "^(.*)[/\\][^/\\]+$")
			if not config then fallback(); return end
			local ok, cmd = pcall(function()
				local path = config .. "/settings.json"
				local settings = if tern.fs.exists(path) then tern.json.decode(tern.fs.read(path)) else {}
				-- Tern's default auto_acp is false when the preference is absent.
				return resume.command(command, settings.agents or {})
			end)
			if not ok then fallback(); return end
			create(window, session, cmd)
		end)
	end
	local function locate(layout: any, window: string): boolean
		if type(layout) ~= "table" or type(layout.sessions) ~= "table" then return false end
		for _, session in layout.sessions do
			for _, tab in session.tabs do
				for _, block in tab.blocks do
					if block.id == pane then with_settings(window, session.id); return true end
				end
			end
		end
		return false
	end
	run({bin, "inspect", "--window", "", "--json"}, function(result)
		local ok, layout = pcall(tern.json.decode, result.stdout)
		if not ok or type(layout) ~= "table" then fallback(); return end
		if locate(layout, "") then return end
		-- inspect lists all attached clients, including mobile clients whose desktop
		-- window is closed. Never substitute the CLI's default window/session.
		local windows, seen = {}, {[""] = true}
		for _, client in layout.clients or {} do
			local key = client.window
			if type(key) == "string" and not seen[key] then
				seen[key] = true
				table.insert(windows, key)
			end
		end
		local index = 0
		local function next_window()
			index += 1
			local window = windows[index]
			if window == nil then fallback(); return end
			run({bin, "ls", "--window", window, "--json"}, function(found)
				local decoded, sessions = pcall(tern.json.decode, found.stdout)
				if not decoded then fallback(); return end
				if not locate(sessions, window) then next_window() end
			end)
		end
		next_window()
	end)
end

return launch
