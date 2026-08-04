-- Faces.
--
-- The v1 head was a blank blob with two dark spheres pushed into it, which is
-- why all three themes read as robots regardless of what the theme actually
-- was. A face is cheap -- eyes with a real sclera, a brow, and a mouth are
-- perhaps 60 triangles -- and it does more for "this is a character" than any
-- amount of armour detail.
--
-- Three treatments, matching the three figure styles:
--   expressive  sclera + pupil + angled brows + a mouth. Reads as a person.
--   minifig     flat dot eyes, straight brows, a printed smile. Reads as a toy
--               with a PRINTED face, which is the Lego language exactly.
--   vinyl       oversized solid eyes, no mouth, no brows. Reads as Funko: the
--               blankness is the whole point, so resist adding a mouth.
--
-- Everything is authored in head-bone space and pushed onto the front of the
-- skull by the caller's `front` function, so it works on any head shape.

local mat4 = require("core.mat4")
local shapes = require("game.render.rig3d.shapes")

local face = {}

-- A mouth: a row of small blocks following a parabola. Rotating each block to
-- follow the tangent looked worse than leaving them axis-aligned at this scale,
-- so they are not rotated -- the curve does the work.
local function mouth(mb, cy, front, width, curve, thickness, color)
    local steps = 9
    for i = 0, steps - 1 do
        local t = (i + 0.5) / steps
        local x = (t - 0.5) * width
        -- Zero at the corners, deepest in the middle: a smile.
        local dip = curve * (1 - (2 * t - 1) ^ 2)
        shapes.box(
            mb,
            mat4.translation(x, cy - dip, front),
            width / steps * 1.20,
            thickness,
            thickness * 0.9,
            color
        )
    end
end

-- Draws a face onto `mb`.
---@param mb table
---@param c table                        -- themes.SLOT_INDEX: slot name -> palette slot index
---@param hr number                      -- head half-width
---@param style string                   -- "expressive" | "minifig" | "vinyl"
---@param front fun(y: number): number   -- front-surface Z at a given height
---@param eye_y number                   -- world-space Y of the eye line
function face.build(mb, c, hr, style, front, eye_y)
    -- `ink` and `sclera` are constant slots (see themes.lua CONSTANT_COLOR):
    -- no theme has ever overridden them, so they are always just `c.ink` /
    -- `c.sclera` now rather than a literal with a theme-override fallback.
    local ink = c.ink
    local sclera = c.sclera

    if style == "vinyl" then
        -- Two big solid eyes and nothing else. Funko's read is the blankness.
        local ew, eh = hr * 0.30, hr * 0.40
        for _, side in ipairs({ -1, 1 }) do
            local z = front(eye_y)
            shapes.extrude(
                mb,
                mat4.translation(side * hr * 0.40, eye_y, z),
                shapes.circleProfile(12),
                {
                    { y = -eh * 0.5, w = ew * 0.62, d = ew * 0.62 },
                    { y = -eh * 0.28, w = ew, d = ew },
                    { y = eh * 0.28, w = ew, d = ew },
                    { y = eh * 0.5, w = ew * 0.62, d = ew * 0.62 },
                },
                ink
            )
            -- A single catchlight keeps it from going dead.
            shapes.sphere(
                mb,
                mat4.translation(
                    side * hr * 0.40 - side * ew * 0.30,
                    eye_y + eh * 0.22,
                    z + ew * 0.34
                ),
                ew * 0.24,
                4,
                8,
                sclera
            )
        end
        return
    end

    if style == "minifig" then
        -- Printed face: flat dots, straight brows, a wide simple smile.
        for _, side in ipairs({ -1, 1 }) do
            local z = front(eye_y)
            shapes.sphere(mb, mat4.translation(side * hr * 0.34, eye_y, z), hr * 0.115, 5, 10, ink)
            shapes.box(
                mb,
                mat4.translation(side * hr * 0.34, eye_y + hr * 0.26, z * 0.99),
                hr * 0.34,
                hr * 0.075,
                hr * 0.06,
                ink
            )
        end
        mouth(
            mb,
            eye_y - hr * 0.44,
            front(eye_y - hr * 0.44),
            hr * 0.78,
            hr * 0.16,
            hr * 0.085,
            ink
        )
        return
    end

    -- Expressive: a real eye -- white sclera with a pupil sitting proud of it --
    -- plus angled brows, which is where nearly all of the personality lives.
    for _, side in ipairs({ -1, 1 }) do
        local z = front(eye_y)
        shapes.sphere(mb, mat4.translation(side * hr * 0.36, eye_y, z), hr * 0.165, 5, 10, sclera)
        shapes.sphere(
            mb,
            mat4.translation(side * hr * 0.36 + side * hr * 0.02, eye_y, z + hr * 0.10),
            hr * 0.085,
            5,
            10,
            ink
        )
        -- Brow, tilted down toward the nose: determined rather than surprised.
        shapes.box(
            mb,
            mat4.multiply(
                mat4.translation(side * hr * 0.36, eye_y + hr * 0.30, z * 0.98),
                mat4.rotationZ(math.rad(side * 11))
            ),
            hr * 0.40,
            hr * 0.085,
            hr * 0.07,
            ink
        )
    end
    -- Nose bridge and a small mouth.
    shapes.box(
        mb,
        mat4.translation(0, eye_y - hr * 0.12, front(eye_y) * 1.02),
        hr * 0.14,
        hr * 0.30,
        hr * 0.12,
        c.skin
    )
    mouth(mb, eye_y - hr * 0.48, front(eye_y - hr * 0.48), hr * 0.52, hr * 0.11, hr * 0.07, ink)
end

return face
