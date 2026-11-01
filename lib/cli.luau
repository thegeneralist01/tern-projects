--!strict
-- Helpers for spawning tern-projects and decoding its JSON output.

local backend = require("./backend")
local cli = { status = backend.status, ensure = backend.ensure }

-- Path to the index SQLite DB.
function cli.db(): string
	return tern.plugin.data .. "/index.sqlite"
end

-- Spawn the binary with `args` prepended by `{bin, "--db", db}`.
-- Decodes stdout as JSON and calls cb(data, nil) on success,
-- or cb(nil, errmsg) on non-zero exit, decode failure, timeout, or a spawn that
-- never started. Each request reports exactly once, and a queued request whose
-- caller stopped caring (`should_start`) never starts a process.
local function spawn(args: { string }, timeout_ms: number, stdin: string?, cb: (data: any?, err: string?) -> (), should_start: (() -> boolean)?)
	backend.ensure(function(b, buildErr)
		if not b then cb(nil, buildErr); return end
		if should_start and not should_start() then return end
		local argv: { string } = { b, "--db", cli.db() }
		for _, a in args do
			table.insert(argv, a)
		end
		local finished = false
		local function finish(data: any?, err: string?)
			if finished then return end
			finished = true
			cb(data, err)
		end
		local started, startErr = pcall((tern :: any).process.run, argv, { timeout_ms = timeout_ms, stdin = stdin }, function(result: any, _cx: any?)
			if finished then return end
			if result.timed_out then
				finish(nil, "timed out")
				return
			end
			if result.status ~= 0 then
				local msg = result.stderr ~= "" and result.stderr or result.stdout
				finish(nil, msg ~= "" and msg or ("exit " .. tostring(result.status)))
				return
			end
			local ok, decoded = pcall(tern.json.decode, result.stdout)
			if not ok then
				finish(nil, "JSON decode error: " .. tostring(decoded))
				return
			end
			finish(decoded, nil)
		end)
		if not started then
			-- A synchronous callback that raised already reported; log that instead of reporting twice.
			if finished then tern.log.error("Project request callback failed: " .. tostring(startErr))
			else finish(nil, "Cannot start project backend: " .. tostring(startErr)) end
		end
	end)
end

function cli.run(args: { string }, timeout_ms: number, cb: (data: any?, err: string?) -> ())
	spawn(args, timeout_ms, nil, cb)
end

function cli.request(payload: any, cb: (data: any?, err: string?) -> (), should_start: (() -> boolean)?)
	local ok, encoded = pcall(tern.json.encode, payload)
	if not ok then cb(nil, tostring(encoded)); return end
	local timeouts = {
		harnesses = 60000,
		["launch-plan"] = 60000,
		["delete-preview"] = 120000,
		delete = 180000,
	}
	spawn({ "library" }, timeouts[payload.op] or 15000, encoded, function(data, err)
		if payload.op == "delete" and err == "timed out" then
			err = "Deletion timed out. The original chat may already have been removed; refresh before obtaining a new deletion preview."
		end
		cb(data, err)
	end, should_start)
end

-- Convenience timeout constants (ms).
cli.TIMEOUT_INDEX = 180000
cli.TIMEOUT_QUERY = 15000

return cli
