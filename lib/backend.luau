--!strict
-- Build in plugin data, never in the watched/replaced package directory.
local backend = {}
local building = false
local ready: string? = nil
local waiters: { (string?, string?) -> () } = {}
local root = tern.plugin.data .. "/backend"
local executable = root .. "/release/tern-projects" .. (tern.runtime.os == "windows" and ".exe" or "")

local function diagnostics(output: string): string
	local errors = {}
	for line in output:gmatch("[^\r\n]+") do
		if line:find("error:", 1, true) or line:match("^error%[") then
			table.insert(errors, (line:gsub("^%s+", "")))
		end
	end
	return if #errors > 0 then table.concat(errors, "\n") else output
end

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
	local finished = false
	local function finish(path: string?, err: string?)
		if finished then return end
		finished = true
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
	local ext = if tern.runtime.os == "windows" then ".exe" else ""
	local cargo = cargoHome .. "/bin/cargo" .. ext
	-- Rust isn't always under CARGO_HOME (e.g. Homebrew or distro packages),
	-- and a bare "cargo" falls back to the plugin's PATH, which may lack it.
	for _, candidate in { "/opt/homebrew/bin/cargo", "/usr/local/bin/cargo", "/usr/bin/cargo" } do
		if not tern.fs.exists(cargo) and tern.fs.exists(candidate) then cargo = candidate end
	end
	if not tern.fs.exists(cargo) then cargo = "cargo" .. ext end
	-- `tern.process.run` replaces the environment wholesale; cargo needs a
	-- PATH to locate rustc and build tools, plus the directory cargo itself
	-- lives in (e.g. a Homebrew install outside the plugin's PATH).
	local pathSep = if tern.runtime.os == "windows" then ";" else ":"
	local cargoDir = cargo:match("^(.+)[/\\][^/\\]*$") or ""
	-- Keep the plugin's PATH and system tools for compilers/linkers.
	local envPath = (tern.getenv("PATH") or "") .. pathSep .. "/usr/bin" .. pathSep .. "/bin"
	if cargoDir ~= "" then envPath = cargoDir .. pathSep .. envPath end
	local function compile(env: { [string]: string }?)
		local started, err = pcall(tern.process.run, { cargo, "build", "--release", "--locked", "--manifest-path", tern.plugin.dir .. "/indexer/Cargo.toml", "--target-dir", root }, { timeout_ms = 1200000, env = env }, function(result)
			if finished then return end
			if result.status ~= 0 or result.timed_out then
				tern.log.error("Cargo build output:\n" .. result.stdout .. result.stderr)
				local detail = if result.timed_out then "Build timed out after 20 minutes." else diagnostics(if result.stderr ~= "" then result.stderr else result.stdout)
				if detail == "" then detail = "Cargo exited with status " .. tostring(result.status) .. "." end
				finish(nil, "Backend build failed. " .. detail .. "\nRefresh to retry. Full compiler output is in Tern's log.")
				return
			end
			if not tern.fs.exists(executable) then finish(nil, "Cargo succeeded but the backend executable is missing: " .. executable); return end
			local saved, saveErr = pcall(tern.fs.write, root .. "/inputs.json", source)
			if not saved then finish(nil, "Cannot save backend build state: " .. tostring(saveErr)); return end
			finish(executable, nil)
		end)
		if not started then finish(nil, "Cannot start Cargo. Check your Rust installation, then Refresh to retry: " .. tostring(err)) end
	end
	local started, err = pcall(function()
		tern.fs.mkdir(root)
		if tern.runtime.os ~= "macos" then compile(nil); return end
		-- Keep the SDK and compiler in the selected Apple toolchain, not a mix
		-- of PATH's cc/linker and a newer Command Line Tools SDK chosen by rustc.
		tern.process.run({ "/usr/bin/xcrun", "--sdk", "macosx", "--show-sdk-path" }, { timeout_ms = 10000 }, function(result)
			if finished then return end
			local sdk = (result.stdout:gsub("^%s+", ""):gsub("%s+$", ""))
			if result.status ~= 0 or result.timed_out or sdk == "" then
				local detail = if result.timed_out then "xcrun timed out after 10 seconds." else result.stderr
				finish(nil, "Cannot select a macOS SDK. Install or select Apple developer tools, then Refresh to retry. " .. detail)
				return
			end
			compile({
				SDKROOT = sdk,
				CC = "/usr/bin/clang",
				PATH = envPath,
				CARGO_TARGET_AARCH64_APPLE_DARWIN_LINKER = "/usr/bin/clang",
				CARGO_TARGET_X86_64_APPLE_DARWIN_LINKER = "/usr/bin/clang",
			})
		end)
	end)
	if not started then finish(nil, "Cannot prepare backend build: " .. tostring(err)) end
end

return backend
