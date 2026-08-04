local Vec2 = require("core.vec2")
local pitch = require("game.render.pitch")
local render_payload = require("spec.support.render_payload")
local match = require("sim.match")
local teams = require("data.teams")

---@class KeeperPoseSnapshotScenario
---@field id "central_dive_catch"|"stretch_dive_parry"
---@field pose PlayerPoseId
---@field dive number
---@field direction Vec2
---@field holding boolean

---@class KeeperPoseSnapshotsModule
local snapshots = {}

local WIDTH = 960
local HEIGHT = 540
local BASELINE_DIR = "spec/visual/baselines"

---@type KeeperPoseSnapshotScenario[]
local SCENARIOS = {
    {
        id = "central_dive_catch",
        pose = "keeper_central",
        dive = 0.72,
        direction = Vec2.new(0, -1),
        holding = true,
    },
    {
        id = "stretch_dive_parry",
        pose = "keeper_stretch",
        dive = 0.28,
        direction = Vec2.new(0, 1),
        holding = false,
    },
}

---@param scenario KeeperPoseSnapshotScenario
---@return love.ImageData
local function render(scenario)
    local state = match.new({
        home = teams.nebula,
        away = teams.orion,
        field = { w = WIDTH, h = HEIGHT },
    })
    local keeper = state.players[1]
    keeper.pos = Vec2.new(24, scenario.holding and 250 or 290)
    keeper.run_vel = Vec2.new(0, 0)
    keeper.dive_timer = scenario.dive * 0.3
    keeper.dive_dir = scenario.direction
    keeper.save_style = scenario.pose == "keeper_central" and "central" or "stretch"
    if scenario.holding then
        state.owner = 1
        keeper.feet_ball = false
        state.ball = keeper.pos
    else
        state.owner = nil
        state.ball = Vec2.new(keeper.pos.x + 45, keeper.pos.y)
    end

    local canvas = love.graphics.newCanvas(WIDTH, HEIGHT)
    love.graphics.setCanvas(canvas)
    pitch.draw(render_payload.frame(state), { w = WIDTH, h = HEIGHT }, {
        home_color = teams.nebula.color,
        away_color = teams.orion.color,
    })

    love.graphics.setColor(0.92, 0.97, 1, 1)
    love.graphics.print(scenario.id:gsub("_", " "):upper(), 14, 14)
    love.graphics.print("LIVE PITCH SCALE · 960×540", 14, 30)
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
    local reports = {}
    local ok = true
    for _, scenario in ipairs(SCENARIOS) do
        local image = render(scenario)
        local encoded = image:encode("png"):getString()
        local relative = BASELINE_DIR .. "/" .. scenario.id .. ".png"
        local path = root .. "/" .. relative
        if write then
            write_file(path, encoded)
            reports[#reports + 1] = "wrote " .. relative
        else
            local expected = read_file(path)
            if expected ~= encoded then
                ok = false
                reports[#reports + 1] = "mismatch " .. relative
            else
                reports[#reports + 1] = "matched " .. relative
            end
        end
    end
    return ok, table.concat(reports, "\n")
end

return snapshots
