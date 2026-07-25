local Vec2 = require("core.vec2")
local player_pose = require("game.presentation.player_pose")
local player_renderer = require("game.render.player_renderer")

---@class KeeperPoseSnapshotScenario
---@field id "central_dive_catch"|"stretch_dive_parry"
---@field pose PlayerPoseId
---@field dive number
---@field direction Vec2
---@field holding boolean

---@class KeeperPoseSnapshotsModule
local snapshots = {}

local WIDTH = 360
local HEIGHT = 240
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
    local canvas = love.graphics.newCanvas(WIDTH, HEIGHT)
    love.graphics.setCanvas(canvas)
    love.graphics.clear(0.025, 0.045, 0.09, 1)

    love.graphics.setColor(0.06, 0.2, 0.24, 1)
    love.graphics.rectangle("fill", 0, 145, WIDTH, HEIGHT - 145)
    love.graphics.setColor(0.35, 0.72, 1, 0.8)
    love.graphics.setLineWidth(3)
    love.graphics.line(48, 184, 48, 84, WIDTH - 48, 84, WIDTH - 48, 184)
    love.graphics.setColor(0.35, 0.72, 1, 0.25)
    love.graphics.line(48, 116, WIDTH - 48, 116)

    player_renderer.draw(180, 194, 28, { 0.2, 0.72, 1 }, nil, {
        facing = Vec2.new(1, 0),
        is_keeper = true,
        controlled = true,
        dive = scenario.dive,
        dive_dir = scenario.direction,
        holding = scenario.holding,
        grab = scenario.holding and 0.8 or 0,
        species_shape = "round",
        species_color = { 1, 0.72, 0.24 },
        team = "home",
        pose = {
            id = scenario.pose,
            priority = player_pose.PRIORITY[scenario.pose],
            source = "soccer",
        },
    })

    if not scenario.holding then
        love.graphics.setColor(1, 0.95, 0.7, 1)
        love.graphics.circle("fill", 275, 126, 10)
        love.graphics.setColor(1, 0.45, 0.3, 0.8)
        love.graphics.setLineWidth(3)
        love.graphics.line(286, 121, 315, 107)
    end

    love.graphics.setColor(0.92, 0.97, 1, 1)
    love.graphics.print(scenario.id:gsub("_", " "):upper(), 14, 14)
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
