---@class GameSettings
---@field master_volume number
---@field sfx_volume number
---@field crowd_volume number
---@field muted boolean
---@field fullscreen boolean
---@field screen_shake boolean
---@field bloom boolean

---@class SettingsStorage
---@field read fun(): string?
---@field write fun(contents: string): boolean, string?

---@class SettingsModule
local settings = {}

local PATH = "settings.txt"

---@return GameSettings
function settings.defaults()
    return {
        master_volume = 1,
        sfx_volume = 0.8,
        crowd_volume = 0.55,
        muted = false,
        fullscreen = false,
        screen_shake = true,
        bloom = true,
    }
end

---@param value number
---@return number
local function clamp01(value)
    return math.max(0, math.min(1, value))
end

---@param input table?
---@return GameSettings
function settings.validate(input)
    input = input or {}
    local defaults = settings.defaults()
    local function number_or(key)
        local value = input[key]
        return type(value) == "number" and clamp01(value) or defaults[key]
    end
    local function boolean_or(key)
        local value = input[key]
        if type(value) == "boolean" then
            return value
        end
        return defaults[key]
    end
    return {
        master_volume = number_or("master_volume"),
        sfx_volume = number_or("sfx_volume"),
        crowd_volume = number_or("crowd_volume"),
        muted = boolean_or("muted"),
        fullscreen = boolean_or("fullscreen"),
        screen_shake = boolean_or("screen_shake"),
        bloom = boolean_or("bloom"),
    }
end

---@param value boolean
---@return string
local function bool_string(value)
    return value and "true" or "false"
end

---@param value GameSettings
---@return string
function settings.serialize(value)
    local clean = settings.validate(value)
    return table.concat({
        "version=1",
        ("master_volume=%.2f"):format(clean.master_volume),
        ("sfx_volume=%.2f"):format(clean.sfx_volume),
        ("crowd_volume=%.2f"):format(clean.crowd_volume),
        "muted=" .. bool_string(clean.muted),
        "fullscreen=" .. bool_string(clean.fullscreen),
        "screen_shake=" .. bool_string(clean.screen_shake),
        "bloom=" .. bool_string(clean.bloom),
        "",
    }, "\n")
end

---@param contents string
---@return GameSettings
function settings.parse(contents)
    local raw = {}
    for key, value in contents:gmatch("([%w_]+)=([^\r\n]+)") do
        if value == "true" then
            raw[key] = true
        elseif value == "false" then
            raw[key] = false
        else
            raw[key] = tonumber(value)
        end
    end
    return settings.validate(raw)
end

---@return SettingsStorage?
local function love_storage()
    if not love or not love.filesystem then
        return nil
    end
    return {
        read = function()
            if not love.filesystem.getInfo(PATH) then
                return nil
            end
            return love.filesystem.read(PATH)
        end,
        write = function(contents)
            return love.filesystem.write(PATH, contents)
        end,
    }
end

---@param storage SettingsStorage?
---@return GameSettings
function settings.load(storage)
    storage = storage or love_storage()
    if not storage then
        return settings.defaults()
    end
    local contents = storage.read()
    return contents and settings.parse(contents) or settings.defaults()
end

---@param value GameSettings
---@param storage SettingsStorage?
---@return boolean, string?
function settings.save(value, storage)
    storage = storage or love_storage()
    if not storage then
        return false, "settings storage is unavailable"
    end
    return storage.write(settings.serialize(value))
end

return settings
