--!strict
-- tern-projects host half: start updates independently, then register the panel.

pcall(tern.fs.mkdir, tern.plugin.data)
require("./lib/updates").start()
tern.block.define("panel", require("./blocks/panel"))

require("./lib/cli").ensure(function(_path, err)
	if err then tern.log.error(err) end
end)
