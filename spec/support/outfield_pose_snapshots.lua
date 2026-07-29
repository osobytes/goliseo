local Vec2 = require("core.vec2")
local pitch = require("game.render.pitch")
local release_follow = require("game.render.release_follow")
local outfield_press = require("sim.outfield_press")
local match = require("sim.match")
local teams = require("data.teams")

---@class OutfieldPoseSnapshotsModule
local snapshots = {}

local WIDTH = 960
local HEIGHT = 540
local BASELINE_DIR = "spec/visual/baselines"
local SNAPSHOT_ID = "contain_vs_tackle"

-- The two poses issue #58 requires a human to compare directly: an AI presser
-- holding contain and an outfielder committed to a standing challenge, at the
-- projected radius normal play uses. Everyone else is parked behind the
-- subjects so the comparison is about the two silhouettes.
---@return love.ImageData
local function render()
    local state = match.new({
        home = teams.nebula,
        away = teams.orion,
        field = { w = WIDTH, h = HEIGHT },
    })

    local contain_index, tackle_index = 2, 3
    for index, player in ipairs(state.players) do
        if not player.is_keeper and index ~= contain_index and index ~= tackle_index then
            player.pos = Vec2.new(WIDTH * 0.12, HEIGHT * 0.12 + index * 14)
            player.run_vel = Vec2.new(0, 0)
        end
    end

    local contain = state.players[contain_index]
    contain.pos = Vec2.new(WIDTH * 0.42, HEIGHT * 0.56)
    contain.facing = Vec2.new(1, 0)
    contain.run_vel = Vec2.new(0, 0)
    state.outfield_press.home = outfield_press.contain(contain_index)

    local tackle = state.players[tackle_index]
    tackle.pos = Vec2.new(WIDTH * 0.58, HEIGHT * 0.56)
    tackle.facing = Vec2.new(1, 0)
    tackle.run_vel = Vec2.new(0, 0)
    tackle.tackle_timer = 0.12

    state.owner = nil
    state.ball = Vec2.new(WIDTH * 0.5, HEIGHT * 0.56)
    state.events = {}
    release_follow.reset()

    local canvas = love.graphics.newCanvas(WIDTH, HEIGHT)
    love.graphics.setCanvas(canvas)
    pitch.draw(state, { w = WIDTH, h = HEIGHT }, {
        home_color = teams.nebula.color,
        away_color = teams.orion.color,
    })

    love.graphics.setColor(0.92, 0.97, 1, 1)
    love.graphics.print(SNAPSHOT_ID:gsub("_", " "):upper(), 14, 14)
    love.graphics.print("LIVE PITCH SCALE · 960×540", 14, 30)
    love.graphics.print("LEFT: CONTAIN   RIGHT: TACKLE", 14, 46)
    love.graphics.setCanvas()
    love.graphics.flushBatch()
    return canvas:newImageData()
end

---@param path string
---@return string?
local function read_file(path)
    local file = io.open(path, "rb")
    if not file then
        return nil
    end
    local value = file:read("*a")
    file:close()
    return value
end

---@param path string
---@param value string
local function write_file(path, value)
    local file = assert(io.open(path, "wb"))
    assert(file:write(value))
    assert(file:close())
end

---@param write boolean
---@return boolean ok
---@return string report
function snapshots.run(write)
    local root = love.filesystem.getSource()
    local image = render()
    local encoded = image:encode("png"):getString()
    local relative = BASELINE_DIR .. "/" .. SNAPSHOT_ID .. ".png"
    local path = root .. "/" .. relative
    if write then
        write_file(path, encoded)
        return true, "wrote " .. relative
    end
    local expected = read_file(path)
    if expected == nil then
        return false, "missing " .. relative
    end
    if expected ~= encoded then
        return false, "mismatch " .. relative
    end
    return true, "matched " .. relative
end

return snapshots
