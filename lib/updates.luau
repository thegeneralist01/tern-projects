--!strict
-- Host-only update checks. Installation always requires a fresh, version-bound UI approval.
local updates = {}

export type State = {
	status: "idle" | "checking" | "current" | "available" | "installing" | "updated" | "error",
	current_version: string,
	latest_version: string?,
	checked_at: number?,
	automatic: boolean,
	install_blocked: boolean,
	message: string?,
	command: string,
}

type Pending = { version: string, started_at: number }
type Cache = {
	schema: number,
	automatic: boolean,
	attempted_at: number?,
	checked_at: number?,
	latest_version: string?,
	failure: string?,
	pending: Pending?,
}

local COMMAND = "tern plugin install github.com/thegeneralist01/tern-projects --force"
local ENDPOINT = "https://raw.githubusercontent.com/thegeneralist01/tern-projects/main/plugin.toml"
local KEY = "plugin-updates:v1"
local DAY = 24 * 60 * 60
local INSTALL_SECONDS = 120
local MAX_METADATA = 65536
local state: State = { status = "idle", current_version = "unknown", automatic = true, install_blocked = false, command = COMMAND }
local cache: Cache = { schema = 1, automatic = true }
local started = false
local storage_ready = false
local active: ("check" | "install")? = nil
local deletions = 0
local timer: TimerHandle? = nil
local timer_generation = 0
local subscribers: { [number]: () -> () } = {}
local next_subscriber = 0

local function version_parts(value: string): { number }?
	local a, b, c = value:match("^(%d+)%.(%d+)%.(%d+)$")
	if not a or not b or not c then return nil end
	local parts = {}
	for _, part in { a, b, c } do
		if #part > 1 and part:sub(1, 1) == "0" then return nil end
		local n = tonumber(part)
		if not n or n > 9007199254740991 then return nil end
		table.insert(parts, n)
	end
	return parts
end

local function newer(a: string, b: string): boolean
	local left, right = version_parts(a), version_parts(b)
	if not left or not right then return false end
	for i = 1, 3 do
		if left[i] ~= right[i] then return left[i] > right[i] end
	end
	return false
end

local function trim(value: string): string
	return (value:gsub("^%s+", ""):gsub("%s+$", ""))
end

-- Read only top-level identity. Tern's installer validates the complete manifest;
-- unrelated metadata must not make a legitimate version check stop working.
local function manifest_version(text: string): (string?, string?)
	if #text > MAX_METADATA or text:find("%z") then return nil, "Manifest is oversized or contains invalid bytes." end
	local identity: { [string]: string } = {}
	for raw in (text .. "\n"):gmatch("([^\n]*)\n") do
		local line = trim(raw)
		if line:sub(1, 1) == "[" then break end
		local key, value = line:match("^([%w_-]+)%s*=%s*(.+)$")
		if key ~= "schema" and key ~= "id" and key ~= "version" then continue end
		if identity[key] ~= nil then return nil, "Manifest has duplicate identity fields." end
		if key == "schema" then
			local rest = value:match("^1(.*)$")
			if rest == nil or (trim(rest) ~= "" and trim(rest):sub(1, 1) ~= "#") then
				return nil, "Unsupported plugin manifest schema."
			end
			identity.schema = "1"
		else
			local item, rest = value:match('^"([^"\\]*)"(.*)$')
			if item == nil then item, rest = value:match("^'([^']*)'(.*)$") end
			if item == nil or (trim(rest) ~= "" and trim(rest):sub(1, 1) ~= "#") then
				return nil, "Manifest identity must use simple quoted strings."
			end
			identity[key] = item
		end
	end
	if identity.schema ~= "1" or identity.id ~= "tern-projects" then return nil, "Manifest is not schema 1 for tern-projects." end
	if not identity.version or not version_parts(identity.version) then return nil, "Manifest version must be stable major.minor.patch." end
	return identity.version, nil
end

local function installed_version(): (string?, string?)
	local ok, text = pcall(tern.fs.read, tern.plugin.dir .. "/plugin.toml", MAX_METADATA)
	if not ok then return nil, "Cannot read installed plugin manifest: " .. tostring(text) end
	return manifest_version(text)
end

local function notify()
	local callbacks = {}
	for _, callback in subscribers do table.insert(callbacks, callback) end
	for _, callback in callbacks do
		local ok, err = pcall(callback)
		if not ok then tern.log.error("Plugin update subscriber failed: " .. tostring(err)) end
	end
end

local function cancel_timer()
	timer_generation += 1
	if timer then timer:cancel(); timer = nil end
end

local function manual(message: string): string
	return message .. " Run manually: " .. COMMAND .. ". Linked development plugins are not replaced; update their checkout instead."
end

local function save(): (boolean, string?)
	local ok, err = pcall(tern.kv.set, KEY, cache)
	return ok, if ok then nil else tostring(err)
end

local function timestamp(value: any): boolean
	return type(value) == "number" and value >= 0 and value < math.huge and value == math.floor(value)
end

local function load(): (boolean, string?)
	local ok, value = pcall(tern.kv.get, KEY)
	if not ok then return false, "Cannot read update preferences: " .. tostring(value) end
	if value == nil then return true, nil end
	if type(value) ~= "table" or value.schema ~= 1 or type(value.automatic) ~= "boolean"
		or (value.attempted_at ~= nil and not timestamp(value.attempted_at))
		or (value.checked_at ~= nil and not timestamp(value.checked_at))
		or (value.latest_version ~= nil and (type(value.latest_version) ~= "string" or not version_parts(value.latest_version)))
		or (value.failure ~= nil and type(value.failure) ~= "string") then
		return false, "Saved update preferences are malformed."
	end
	if value.pending ~= nil and (type(value.pending) ~= "table" or type(value.pending.version) ~= "string"
		or not version_parts(value.pending.version) or not timestamp(value.pending.started_at)) then
		return false, "Saved pending installation is malformed; no installation will be started."
	end
	cache = {
		schema = 1, automatic = value.automatic, attempted_at = value.attempted_at,
		checked_at = value.checked_at, latest_version = value.latest_version, failure = value.failure,
		pending = if value.pending then { version = value.pending.version, started_at = value.pending.started_at } else nil,
	}
	return true, nil
end

local function cached_state()
	state.automatic = cache.automatic
	state.checked_at = cache.checked_at
	state.latest_version = cache.latest_version
	state.message = cache.failure
	if cache.failure then state.status = "error"
	elseif cache.latest_version then state.status = if newer(cache.latest_version, state.current_version) then "available" else "current"
	else state.status = "idle" end
end

local schedule: () -> ()
local check: () -> ()

schedule = function()
	cancel_timer()
	if not started or not storage_ready or not state.automatic or active then return end
	local now = os.time()
	local age = if cache.attempted_at then now - cache.attempted_at else DAY
	-- A rollback/future timestamp is due now, never a multi-year suppression.
	local delay = if age < 0 then 0 else math.max(0, DAY - age)
	local generation = timer_generation
	timer = tern.timer(delay * 1000, function()
		if generation ~= timer_generation then return end
		timer = nil
		if state.automatic and not active then check() end
	end)
end

local function finish_install(target: string, failure: string?)
	local installed, read_error = installed_version()
	cache.pending = nil
	if installed then state.current_version = installed end
	if not failure and (not installed or newer(target, installed)) then
		failure = "Installation could not be confirmed from disk. " .. (read_error or ("Expected at least " .. target .. ", found " .. tostring(installed) .. "."))
	end
	cache.failure = if failure then manual(failure) else nil
	if not failure then cache.latest_version = installed end
	local saved, save_error = save()
	active = nil
	state.latest_version = cache.latest_version
	state.status = if failure or not saved then "error" else "updated"
	state.message = if not saved then manual("Cannot save installation recovery state: " .. tostring(save_error))
		elseif failure then cache.failure else "Updated to " .. tostring(installed) .. "."
	schedule()
	notify()
end

local function recover_install(pending: Pending)
	active = "install"
	cancel_timer()
	state.status = "installing"
	state.latest_version = pending.version
	state.message = "Confirming the previously approved installation from disk…"
	local installed = installed_version()
	if installed and not newer(pending.version, installed) then finish_install(pending.version, nil); return end
	local age = os.time() - pending.started_at
	if age < 0 or age >= INSTALL_SECONDS then
		finish_install(pending.version, "The previously approved installation was interrupted or could not be confirmed.")
		return
	end
	local generation = timer_generation
	timer = tern.timer((INSTALL_SECONDS - age) * 1000, function()
		if generation ~= timer_generation or active ~= "install" then return end
		timer = nil
		local current = installed_version()
		finish_install(pending.version, if current and not newer(pending.version, current) then nil else "The previously approved installation was interrupted or could not be confirmed.")
	end)
end

function updates.start()
	if started then return end
	started = true
	local loaded, load_error = load()
	storage_ready = loaded
	cached_state()
	local installed, read_error = installed_version()
	if installed then state.current_version = installed end
	if not loaded then state.status = "error"; state.message = manual(tostring(load_error)); notify(); return end
	if cache.pending then recover_install(cache.pending); notify(); return end
	if not installed then
		state.status = "error"
		state.message = manual(tostring(read_error))
	else
		cached_state()
	end
	schedule()
	notify()
end

function updates.get(): State
	return {
		status = state.status, current_version = state.current_version, latest_version = state.latest_version,
		checked_at = state.checked_at, automatic = state.automatic, message = state.message, command = COMMAND,
		install_blocked = deletions > 0,
	}
end

function updates.subscribe(on_change: () -> ()): () -> ()
	next_subscriber += 1
	local id = next_subscriber
	subscribers[id] = on_change
	return function() subscribers[id] = nil end
end

check = function()
	if not started then updates.start() end
	if active then return end
	if not storage_ready then
		local loaded, err = load()
		storage_ready = loaded
		if not loaded then state.status = "error"; state.message = manual(tostring(err)); notify(); return end
		state.automatic = cache.automatic
		if cache.pending then recover_install(cache.pending); notify(); return end
	end
	cancel_timer()
	active = "check"
	cache.attempted_at = os.time()
	local saved, save_error = save()
	local installed, read_error = installed_version()
	local finished = false
	local function finish(latest: string?, failure: string?)
		if finished then return end
		finished = true
		cache.latest_version = latest
		cache.failure = if failure then manual(failure) else nil
		if latest then cache.checked_at = os.time() end
		local persisted, persist_error = save()
		active = nil
		state.checked_at = cache.checked_at
		state.latest_version = latest
		state.status = if failure or not persisted then "error" elseif latest and newer(latest, state.current_version) then "available" else "current"
		state.message = if not persisted then manual("Cannot save update check state: " .. tostring(persist_error)) else cache.failure
		schedule()
		notify()
	end
	if not saved then finish(nil, "Cannot save the update attempt: " .. tostring(save_error)); return end
	if not installed then finish(nil, tostring(read_error)); return end
	state.current_version = installed
	state.status = "checking"
	state.latest_version = nil
	state.message = nil
	notify()
	local ok, err = pcall((tern :: any).fetch, ENDPOINT, { timeout_ms = 10000 }, function(result: any)
		if finished then return end
		if result.timed_out then finish(nil, "Update check timed out after 10 seconds."); return end
		if result.error then finish(nil, "Update check failed: " .. tostring(result.error)); return end
		if result.status ~= 200 then finish(nil, "Update check returned HTTP " .. tostring(result.status) .. (if result.status == 429 then " (rate limited)." else ".")); return end
		if type(result.body) ~= "string" then finish(nil, "Update metadata is not text."); return end
		local latest, metadata_error = manifest_version(result.body)
		finish(latest, metadata_error)
	end)
	if not ok and not finished then finish(nil, "Cannot start update check: " .. tostring(err)) end
end

function updates.check()
	check()
end

function updates.set_automatic(enabled: boolean)
	if not started then updates.start() end
	if not storage_ready then state.status = "error"; state.message = manual("Cannot change unreadable update preferences."); notify(); return end
	if cache.automatic == enabled then return end
	local previous = cache.automatic
	cache.automatic = enabled
	local saved, err = save()
	if not saved then
		cache.automatic = previous
		state.status = "error"
		state.message = manual("Cannot save automatic update preference: " .. tostring(err))
		notify()
		return
	end
	state.automatic = enabled
	-- Do not cancel an already approved installation's recovery deadline.
	if active ~= "install" then schedule() end
	notify()
end

-- Reloading the plugin must not interrupt permanent source cleanup. This shared
-- host guard also rejects a deletion queued until after installation began.
function updates.begin_deletion(): ((() -> ())?, string?)
	if not started then updates.start() end
	if active == "install" then return nil, "A plugin update is in progress. Wait before deleting a chat." end
	deletions += 1
	notify()
	local released = false
	return function()
		if released then return end
		released = true
		deletions -= 1
		notify()
	end, nil
end

function updates.install(expected_version: string)
	if not started then updates.start() end
	if active or state.status ~= "available" or state.latest_version ~= expected_version then return end
	if deletions > 0 then
		state.message = "Wait for permanent chat deletion to finish before installing an update."
		notify()
		return
	end
	if not storage_ready then state.status = "error"; state.message = manual("Cannot persist installation approval."); notify(); return end
	local installed, read_error = installed_version()
	if not installed or installed ~= state.current_version or not newer(expected_version, installed) then
		state.status = "error"
		state.message = manual(read_error or "Installed plugin changed. Check for updates again before approving an installation.")
		notify()
		return
	end
	cache.pending = { version = expected_version, started_at = os.time() }
	cache.failure = nil
	local saved, err = save()
	if not saved then
		cache.pending = nil
		state.status = "error"
		state.message = manual("Cannot persist installation approval; installation was not started: " .. tostring(err))
		notify()
		return
	end
	active = "install"
	cancel_timer()
	state.status = "installing"
	state.message = "Installing the approved update to " .. expected_version .. "…"
	notify()
	local finished = false
	local function finish(failure: string?)
		if finished then return end
		finished = true
		finish_install(expected_version, failure)
	end
	local ok, spawn_error = pcall((tern :: any).process.run,
		{ "tern", "plugin", "install", "github.com/thegeneralist01/tern-projects", "--force" },
		{ timeout_ms = INSTALL_SECONDS * 1000 }, function(result: any)
			if finished then return end
			if result.timed_out then finish("Installation timed out after 120 seconds."); return end
			if result.status ~= 0 then
				local detail = if result.stderr ~= "" then result.stderr else result.stdout
				finish("Installer exited with status " .. tostring(result.status) .. ". " .. tostring(detail):sub(1, 2000))
				return
			end
			finish(nil)
		end)
	if not ok and not finished then finish("Cannot start the Tern installer: " .. tostring(spawn_error)) end
end

return updates
