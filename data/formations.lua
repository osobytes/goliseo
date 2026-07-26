-- Formations for small-sided 5v5 (1 keeper + 4 outfield). Content, not logic.
--
-- Anchors are normalized in an "attacking right" frame:
--   x: 0 = own goal line, 1 = opponent goal line (depth)
--   y: 0 = top touchline, 1 = bottom touchline (width)
-- `sim/placement.lua` converts these to absolute pitch coords and mirrors them
-- for the away side. The `outfield` order is the line order (defence -> attack);
-- a team's roster (minus keeper) must be listed in this same order.

---@class Anchor
---@field x number  -- 0..1 depth (own goal -> opponent goal)
---@field y number  -- 0..1 width (top -> bottom)

---@alias FormationRole "def"|"mid"|"wide"|"fwd"

---@class OutfieldAnchor: Anchor
---@field role FormationRole

---@class FormationData
---@field id string
---@field name string
---@field strength string?
---@field risk string?
---@field keeper Anchor
---@field outfield OutfieldAnchor[]  -- exactly 4

local GK = { x = 0.06, y = 0.5 }

---@type table<string, FormationData>
return {
    ["2-1-1"] = {
        id = "2-1-1",
        name = "Balanced",
        strength = "Two defenders protect the middle.",
        risk = "The lone forward can become isolated.",
        keeper = GK,
        outfield = {
            { x = 0.28, y = 0.30, role = "def" },
            { x = 0.28, y = 0.70, role = "def" },
            { x = 0.52, y = 0.50, role = "mid" },
            { x = 0.76, y = 0.50, role = "fwd" },
        },
    },
    ["1-2-1"] = {
        id = "1-2-1",
        name = "Control",
        strength = "Two midfielders create passing angles.",
        risk = "Only one defender guards counterattacks.",
        keeper = GK,
        outfield = {
            { x = 0.26, y = 0.50, role = "def" },
            { x = 0.52, y = 0.30, role = "wide" },
            { x = 0.52, y = 0.70, role = "wide" },
            { x = 0.78, y = 0.50, role = "fwd" },
        },
    },
    ["1-1-2"] = {
        id = "1-1-2",
        name = "Aggressive",
        strength = "Two forwards keep constant goal pressure.",
        risk = "Large spaces open behind the first press.",
        keeper = GK,
        outfield = {
            { x = 0.26, y = 0.50, role = "def" },
            { x = 0.50, y = 0.50, role = "mid" },
            { x = 0.76, y = 0.30, role = "fwd" },
            { x = 0.76, y = 0.70, role = "fwd" },
        },
    },
}
