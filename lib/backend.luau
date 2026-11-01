--!strict
-- Build in plugin data, never in the watched/replaced package directory.
local backend = {}
local building = false
local ready: string? = nil
local waiters: { (string?, string?) -> () } = {}
local root = tern.plugin.data .. "/backend"
local executable = root .. "/release/tern-projects" .. (tern.runtime.os == "windows" and ".exe" or "")

local function inputs(): string
	local files = { "indexer/Cargo.toml", "indexer/Cargo.lock" }
	local function visit(dir: string)
		for _, name in tern.fs.list(tern.plugin.dir .. "/" .. dir) do
			local path = dir .. "/" .. name
			if name:sub(-3) == ".rs" then
				table.insert(files, path)
			elseif not name:find("%.") then
				visit(path)
			end
		end
	end
	visit("indexer/src")
	table.sort(files)
	local values = { tern.runtime.os }
	for _, path in files do
		table.insert(values, path)
		table.insert(values, tern.fs.read(tern.plugin.dir .. "/" .. path))
	end
	return tern.json.encode(values)
end

function backend.status(): string?
	return building and "Building Rust backend…" or nil
end

function backend.ensure(cb: (string?, string?) -> ())
	if ready then cb(ready, nil); return end
	if building then table.insert(waiters, cb); return end
	local ok, source = pcall(inputs)
	if not ok then cb(nil, "Cannot read backend sources: " .. tostring(source)); return end
	local cached, stamp = pcall(tern.fs.read, root .. "/inputs.json")
	if cached and stamp == source and tern.fs.exists(executable) then
		ready = executable
		cb(ready, nil)
		return
	end
	building = true
	table.insert(waiters, cb)
	local function finish(path: string?, err: string?)
		building = false
		ready = path
		local pending = waiters
		waiters = {}
		-- One waiter raising must not withhold the build result from the rest.
		for _, done in pending do
			local ok, failure = pcall(done, path, err)
			if not ok then tern.log.error("Project backend waiter failed: " .. tostring(failure)) end
		end
	end
	local home = tern.getenv("HOME") or tern.getenv("USERPROFILE") or ""
	local cargoHome = tern.getenv("CARGO_HOME") or (home .. "/.cargo")
	local cargo = cargoHome .. "/bin/cargo" .. (tern.runtime.os == "windows" and ".exe" or "")
	if not tern.fs.exists(cargo) then cargo = "cargo" end
	local started, err = pcall(function()
		tern.fs.mkdir(root)
		tern.process.run({ cargo, "build", "--release", "--locked", "--manifest-path", tern.plugin.dir .. "/indexer/Cargo.toml", "--target-dir", root }, { timeout_ms = 1200000 }, function(result)
			if result.status ~= 0 or result.timed_out then
				finish(nil, "Backend build failed. Install Rust/Cargo and your platform's native build tools, then Refresh to retry. " .. (result.timed_out and "Build timed out after 20 minutes." or result.stderr))
				return
			end
			if not tern.fs.exists(executable) then finish(nil, "Cargo succeeded but the backend executable is missing: " .. executable); return end
			local saved, saveErr = pcall(tern.fs.write, root .. "/inputs.json", source)
			if not saved then finish(nil, "Cannot save backend build state: " .. tostring(saveErr)); return end
			finish(executable, nil)
		end)
	end)
	if not started then finish(nil, "Cannot start backend build. Install Rust/Cargo and native build tools, then Refresh to retry: " .. tostring(err)) end
end

return backend
