--!strict
-- Resume tabs do not run the interactive shell wrappers that select ACP.
local resume = {}

type AgentSettings = {auto_acp: boolean?, keep_tui: {string}?}

-- Of the indexed harnesses, Tern's automatic ACP wrappers support these two.
-- OMP already renders natively; Pi, OpenCode, and Cursor keep their own paths.
local prefixes = {codex = "codex resume ", claude = "claude --resume "}

function resume.command(command: string, agents: AgentSettings): string
	if agents.auto_acp ~= true then return command end
	for agent, prefix in prefixes do
		if string.sub(command, 1, #prefix) == prefix and not table.find(agents.keep_tui or {}, agent) then
			-- Keep the backend's already-quoted session argument intact. Only the
			-- launch command changes; the panel's copy command does not.
			return "tern acp " .. agent .. " --resume " .. string.sub(command, #prefix + 1)
		end
	end
	return command
end

return resume
