-- Every physical input the game reads, declared exactly once.
--
-- Two consumers sit on top of this table and must never grow a private keymap:
--   * `game.input.actions` turns key and button EDGES into ActionEvents, which
--     menus and the global toggles consume.
--   * `game.screens.match` POLLS held state every render frame and derives the
--     charge holds and release edges that a MatchInput needs.
-- `docs/controls.md` and the help screen both render `bindings.reference`, so a
-- binding cannot drift from what the game tells the player it is.
--
-- This commit moves the existing bindings here unchanged; it is a refactor and
-- nothing a player presses has moved. The ergonomics of the layout itself are
-- issue #311's second half.

---@alias ControlId
---|"move_up"
---|"move_down"
---|"move_left"
---|"move_right"
---|"action" -- Contextual: shoot / punt while carrying, tackle / jockey without.
---|"play" -- Contextual: pass / throw while carrying, switch player without.
---|"sprint"
---|"modifier" -- Loft: chip a shot, lob a pass, request a bicycle kick.
---|"juke"
---|"equipment"
---|"confirm"
---|"back"
---|"pause"
---|"toggle_fullscreen"
---|"toggle_mute"

---@class ControlBinding
---@field id ControlId
---@field action ActionName -- Edge action this control dispatches to screens.
---@field keys string[] -- LÖVE KeyConstant names.
---@field buttons string[] -- LÖVE GamepadButton names, edge-capable.
---@field axes string[] -- LÖVE GamepadAxis names read as a held trigger, hold-only.

---@class BindingsModule
local bindings = {}

-- A trigger is analog. Past this pull it counts as held, matching the deadzone
-- the movement stick already uses.
bindings.TRIGGER_THRESHOLD = 0.5

---@type ControlBinding[]
local CONTROLS = {
    { id = "move_up", action = "up", keys = { "up", "w" }, buttons = { "dpup" }, axes = {} },
    {
        id = "move_down",
        action = "down",
        keys = { "down", "s" },
        buttons = { "dpdown" },
        axes = {},
    },
    {
        id = "move_left",
        action = "left",
        keys = { "left", "a" },
        buttons = { "dpleft" },
        axes = {},
    },
    {
        id = "move_right",
        action = "right",
        keys = { "right", "d" },
        buttons = { "dpright" },
        axes = {},
    },
    -- Space and gamepad A read as `confirm` in a menu and as ACTION in a match.
    -- One physical control, two contexts -- that is deliberate, not an overload.
    { id = "action", action = "confirm", keys = { "space" }, buttons = { "a" }, axes = {} },
    { id = "play", action = "pass_switch", keys = { "k" }, buttons = { "x" }, axes = {} },
    {
        id = "sprint",
        action = "sprint",
        keys = { "lshift", "rshift" },
        buttons = { "leftshoulder" },
        axes = {},
    },
    { id = "modifier", action = "lob", keys = { "l" }, buttons = { "y" }, axes = {} },
    { id = "juke", action = "juke", keys = { "c" }, buttons = { "leftstick" }, axes = {} },
    -- Equipment shares gamepad B with Back, which `actions.from_gamepad` still
    -- has to disambiguate by context. Issue #311 gives it its own button.
    { id = "equipment", action = "equipment", keys = { "j" }, buttons = { "b" }, axes = {} },
    { id = "confirm", action = "confirm", keys = { "return", "kpenter" }, buttons = {}, axes = {} },
    { id = "back", action = "back", keys = { "escape" }, buttons = { "b" }, axes = {} },
    { id = "pause", action = "pause", keys = { "p" }, buttons = { "start" }, axes = {} },
    {
        id = "toggle_fullscreen",
        action = "toggle_fullscreen",
        keys = { "f11" },
        buttons = {},
        axes = {},
    },
    { id = "toggle_mute", action = "toggle_mute", keys = { "m" }, buttons = {}, axes = {} },
}

bindings.CONTROLS = CONTROLS

---@type table<ControlId, ControlBinding>
local BY_ID = {}
for _, control in ipairs(CONTROLS) do
    BY_ID[control.id] = control
end
bindings.BY_ID = BY_ID

---@param id ControlId
---@return ControlBinding
local function control(id)
    return assert(BY_ID[id], "unknown control: " .. tostring(id))
end
bindings.control = control

---@return table<string, ActionName>
function bindings.key_map()
    local map = {}
    for _, entry in ipairs(CONTROLS) do
        for _, key in ipairs(entry.keys) do
            map[key] = entry.action
        end
    end
    return map
end

---@type table<string, ControlId>
local KEY_CONTROLS = {}
for _, entry in ipairs(CONTROLS) do
    for _, key in ipairs(entry.keys) do
        KEY_CONTROLS[key] = entry.id
    end
end

-- Which role a raw key belongs to. The match screen uses this on the direct-mount
-- path, where it sees unnormalized key events, so that path cannot drift from the
-- polled one.
---@param key string
---@return ControlId?
function bindings.control_for_key(key)
    return KEY_CONTROLS[key]
end

---@return table<string, ActionName>
function bindings.gamepad_map()
    local map = {}
    for _, entry in ipairs(CONTROLS) do
        for _, button in ipairs(entry.buttons) do
            map[button] = entry.action
        end
    end
    return map
end

---@param id ControlId
---@return boolean
local function keyboard_down(id)
    local keys = control(id).keys
    if #keys == 0 or not love.keyboard then
        return false
    end
    return love.keyboard.isDown(unpack(keys)) == true
end
bindings.keyboard_down = keyboard_down

---@param id ControlId
---@param joystick love.Joystick?
---@return boolean
local function gamepad_down(id, joystick)
    if not joystick then
        return false
    end
    local entry = control(id)
    for _, button in ipairs(entry.buttons) do
        if joystick:isGamepadDown(button) then
            return true
        end
    end
    for _, axis in ipairs(entry.axes) do
        if joystick:getGamepadAxis(axis) >= bindings.TRIGGER_THRESHOLD then
            return true
        end
    end
    return false
end
bindings.gamepad_down = gamepad_down

-- The one question the match asks of a control: is a human holding it right now,
-- on either device?
---@param id ControlId
---@param joystick love.Joystick?
---@return boolean
function bindings.is_down(id, joystick)
    return keyboard_down(id) or gamepad_down(id, joystick)
end

---@type table<string, string>
local KEY_LABELS = {
    up = "↑",
    down = "↓",
    left = "←",
    right = "→",
    lshift = "Shift",
    rshift = "Shift",
    ["return"] = "Enter",
    kpenter = "KP Enter",
    escape = "Esc",
    space = "Space",
}

---@type table<string, string>
local BUTTON_LABELS = {
    a = "A",
    b = "B",
    x = "X",
    y = "Y",
    start = "Start",
    back = "Back",
    leftshoulder = "LB",
    rightshoulder = "RB",
    leftstick = "L3",
    rightstick = "R3",
    dpup = "D-pad ↑",
    dpdown = "D-pad ↓",
    dpleft = "D-pad ←",
    dpright = "D-pad →",
    triggerleft = "LT",
    triggerright = "RT",
}

---@param name string
---@param labels table<string, string>
---@return string
local function labelled(name, labels)
    return labels[name] or name:upper()
end

---@param id ControlId
---@return string
function bindings.key_label(id)
    local seen, parts = {}, {}
    for _, key in ipairs(control(id).keys) do
        local text = labelled(key, KEY_LABELS)
        if not seen[text] then
            seen[text] = true
            parts[#parts + 1] = text
        end
    end
    return table.concat(parts, " / ")
end

---@param id ControlId
---@return string
function bindings.gamepad_label(id)
    local entry = control(id)
    local parts = {}
    for _, button in ipairs(entry.buttons) do
        parts[#parts + 1] = labelled(button, BUTTON_LABELS)
    end
    for _, axis in ipairs(entry.axes) do
        parts[#parts + 1] = labelled(axis, BUTTON_LABELS)
    end
    return table.concat(parts, " / ")
end

---@alias ControlSection "match"|"system"

---@class ControlReferenceRow
---@field label string -- Player-facing name of the role.
---@field note string? -- What the role does, when the name alone is not enough.
---@field section ControlSection
---@field footnote boolean -- Carries the combat-prototype asterisk.
---@field keyboard string
---@field gamepad string

---@class ControlReferenceSpec
---@field label string
---@field note string?
---@field section ControlSection
---@field footnote boolean?
---@field controls ControlId[]
---@field keyboard string? -- Overrides the derived text where grouping reads better.
---@field gamepad string?

-- Display grouping only. Every row still resolves its text through the bindings
-- above, so a rebind cannot leave the printed reference stale.
---@type ControlReferenceSpec[]
local REFERENCE = {
    {
        label = "Move",
        section = "match",
        controls = { "move_up", "move_down", "move_left", "move_right" },
        keyboard = "WASD or Arrows",
        gamepad = "Left Stick or D-pad",
    },
    { label = "ACTION", note = "shoot / tackle", section = "match", controls = { "action" } },
    { label = "PLAY", note = "pass / switch", section = "match", controls = { "play" } },
    { label = "Sprint", note = "hold", section = "match", controls = { "sprint" } },
    {
        label = "Modifier",
        note = "loft / chip, hold",
        section = "match",
        controls = { "modifier" },
    },
    { label = "Juke", section = "match", controls = { "juke" } },
    {
        label = "Equipment",
        note = "hold / tap",
        section = "match",
        footnote = true,
        controls = { "equipment" },
    },
    { label = "Pause", section = "match", controls = { "pause", "back" } },
    { label = "Toggle mute", section = "system", controls = { "toggle_mute" } },
    { label = "Toggle fullscreen", section = "system", controls = { "toggle_fullscreen" } },
}

---@param ids ControlId[]
---@param label_of fun(id: ControlId): string
---@return string
local function joined(ids, label_of)
    local parts = {}
    for _, id in ipairs(ids) do
        local text = label_of(id)
        if text ~= "" then
            parts[#parts + 1] = text
        end
    end
    return table.concat(parts, " / ")
end

-- The rendered control reference, in the order a player should read it. Pass a
-- section to take only the match rows (the help card) or only the system ones.
---@param section ControlSection?
---@return ControlReferenceRow[]
function bindings.reference(section)
    local rows = {}
    for _, spec in ipairs(REFERENCE) do
        if section == nil or spec.section == section then
            rows[#rows + 1] = {
                label = spec.label,
                note = spec.note,
                section = spec.section,
                footnote = spec.footnote == true,
                keyboard = spec.keyboard or joined(spec.controls, bindings.key_label),
                gamepad = spec.gamepad or joined(spec.controls, bindings.gamepad_label),
            }
        end
    end
    return rows
end

return bindings
