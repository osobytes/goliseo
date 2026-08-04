-- One proof-of-concept presentation per proof theme, per
-- docs/design/prototype_theme_roster.md.
--
-- All three ride the SAME Rig_Medium skeleton and the SAME animation clips.
-- Everything that differs between them lives in this file: shape language,
-- material, palette and loadout. That is the roster's central claim, and having
-- it be literally one table per theme is the cheapest way to test it.
--
-- Colour slots may be a literal {r,g,b} or one of the strings "team" / "trim".
-- Those resolve against the active team at build time, which is how the roster
-- rule "Theme color is secondary to team ownership" is enforced structurally
-- rather than by hoping an artist remembers it -- the largest readable surfaces
-- (torso, shield face, helmet crest) are wired to the team, not the theme.

local themes = {}

-- Both sides of a match. Team colour owns the big surfaces; trim is the
-- readable secondary used for crests, seams and edge accents.
themes.TEAMS = {
    {
        key = "crimson",
        label = "Crimson",
        main = { 0.70, 0.15, 0.16 },
        trim = { 0.94, 0.78, 0.32 },
    },
    { key = "azure", label = "Azure", main = { 0.14, 0.34, 0.74 }, trim = { 0.72, 0.88, 1.00 } },
}

-- FIGURE STYLE is a second, independent axis from THEME. Theme decides what the
-- character is (knight / ranger / action figure); figure decides how stylised
-- the body language is. Any theme can wear any figure style.
--
-- IMPORTANT scope note: `head_scale` changes the head MESH only. The skeleton
-- is byte-identical across all three -- same bone names, same bone lengths, and
-- the same clips drive all of them. Whether a 1.85x head mesh still counts as
-- "one validated Rig_Medium contract" or as "unrestricted proportions" is a
-- call for #93; the code deliberately keeps the two separable so the decision
-- can go either way without a rewrite.
themes.FIGURES = {
    {
        key = "natural",
        label = "Natural",
        blurb = "Stylised humanoid. Head sized to the rig.",
        head_scale = 1.00,
        head_h = 1.95,
        head_shape = "round",
        head_drop = 0.0,
        neck = true,
        stud = false,
        hands = "mitten",
        limbs = "tapered",
        torso = "bean",
        face = "expressive",
        helm_scale = 1.00,
    },
    {
        key = "minifig",
        label = "Minifig",
        blurb = "Lego-like: cylinder head, printed face, C-clamp hands, tube limbs.",
        head_scale = 1.30,
        head_h = 1.30,
        head_shape = "cylinder",
        head_drop = -0.045,
        neck = false,
        stud = true,
        hands = "clamp",
        limbs = "tube",
        torso = "trapezoid",
        face = "minifig",
        helm_scale = 1.34,
    },
    {
        key = "vinyl",
        label = "Vinyl",
        blurb = "Funko-like: oversized rounded head, big blank eyes, small body.",
        head_scale = 1.85,
        head_h = 1.62,
        head_shape = "boxround",
        head_drop = -0.055,
        neck = false,
        stud = false,
        hands = "mitten",
        limbs = "tapered",
        torso = "bean",
        face = "vinyl",
        helm_scale = 1.62,
    },
}

themes.LIST = {
    -- -----------------------------------------------------------------------
    {
        key = "medieval",
        label = "Medieval Fantasy",
        presentation = "Rook Emberguard",
        blurb = "Broad armoured humanoid. Plates, leather, heraldic blocks, forged edges.",
        -- Shape channel: hard bevels, stacked plate layers, no seams or joints.
        shape = {
            bevel = 0.32,
            plate_layers = 3,
            seams = false,
            joint_balls = false,
            heraldic_block = true,
            pteruges = 0,
        },
        color = {
            skin = { 0.74, 0.55, 0.40 },
            cloth = "team", -- surcoat: the dominant ownership surface
            plate = { 0.66, 0.69, 0.74 },
            plate_dark = { 0.44, 0.47, 0.53 },
            accent = { 0.78, 0.56, 0.22 }, -- warm bronze trim
            strap = { 0.30, 0.20, 0.13 },
            crest = "trim",
            joint = { 0.34, 0.30, 0.28 },
        },
        head = "great_helm",
        loadout = { right = "tournament_sword", left = "heater_shield" },
    },

    -- -----------------------------------------------------------------------
    {
        key = "scifi",
        label = "Galactic Sci-Fi",
        presentation = "Nova Quell",
        blurb = "Clean athletic space-ranger. Panels, luminous seams, compact tech.",
        -- Shape channel: smooth panels, one plate layer, emissive seams.
        shape = {
            bevel = 0.62,
            plate_layers = 1,
            seams = true,
            joint_balls = false,
            heraldic_block = false,
            pteruges = 0,
        },
        color = {
            skin = { 0.72, 0.54, 0.42 },
            limbs = { 0.22, 0.24, 0.30 }, -- sealed undersuit, not bare arms
            cloth = { 0.17, 0.18, 0.22 },
            plate = "team", -- chest and shoulder panels: the ownership surface
            plate_dark = { 0.30, 0.33, 0.39 },
            accent = { 0.86, 0.88, 0.92 }, -- clean white secondary
            strap = { 0.20, 0.22, 0.26 },
            crest = "trim",
            seam = "trim", -- emissive
            joint = { 0.26, 0.28, 0.33 },
        },
        head = "visor_helm",
        loadout = { right = "vector_blade", hip = "pulse_blaster" },
    },

    -- -----------------------------------------------------------------------
    {
        key = "toybox",
        label = "Toybox",
        presentation = "Moxie Modular",
        blurb = "Heroic action figure. Chunky pieces, visible joints, molded edges.",
        -- Shape channel: soft molded edges and exposed ball joints -- the
        -- roster's Toybox read, achieved without touching the proportions.
        shape = {
            bevel = 0.92,
            plate_layers = 2,
            seams = false,
            joint_balls = true,
            heraldic_block = false,
            pteruges = 0,
        },
        color = {
            skin = { 0.96, 0.78, 0.60 }, -- moulded plastic flesh tone
            limbs = { 0.93, 0.90, 0.84 }, -- cream plastic body
            cloth = { 0.88, 0.85, 0.78 },
            plate = "team", -- the armour carries ownership
            plate_dark = { 0.86, 0.82, 0.75 },
            accent = "trim",
            strap = { 0.28, 0.28, 0.32 },
            crest = "trim",
            joint = { 0.24, 0.24, 0.28 }, -- dark exposed ball joints
        },
        head = "figure_helm",
        loadout = { right = "foam_champion", left = "spring_glove" },
    },
}

-- Resolves a theme colour slot against the active team.
---@param slot number[]|string
---@param team table
---@return number[]
function themes.resolve(slot, team)
    if slot == "team" then
        return team.main
    elseif slot == "trim" then
        return team.trim
    end
    -- Every string form is handled above, so anything left is a literal colour.
    ---@cast slot number[]
    return slot
end

-- ---------------------------------------------------------------------------
-- Palette slots (#337)
--
-- A vertex used to carry its literal {r,g,b,a} baked in at mesh-build time,
-- which meant every colour variant -- every team, every future cosmetic skin
-- -- needed its own copy of every mesh. Instead each vertex now carries a
-- SLOT INDEX (a small integer, one of `themes.SLOTS` below) and the actual
-- colours are resolved once per (theme, team) into a small array that is
-- uploaded to the shader as a uniform. Meshes become colour-agnostic: the
-- same geometry is redrawn for any palette by swapping the uniform, at zero
-- extra meshes and zero extra draw calls.
--
-- This is the same eight names `theme.color` already speaks (skin, cloth,
-- plate, plate_dark, accent, strap, crest, joint), plus the two theme-color
-- keys that are only meshed by some themes (limbs, seam) and the two literal
-- constants face.lua reaches for regardless of theme (ink, white). Twelve
-- slots total -- cheap next to WebGL1's minimum guaranteed uniform budget
-- (128 vertex / 16 fragment uniform *vectors*; this is 12 vec4s plus three
-- 4x4 matrices, nowhere close).
-- ---------------------------------------------------------------------------

---@type string[]  -- canonical order; index-1 IS the shader's u_palette index
themes.SLOTS = {
    "skin",
    "cloth",
    "plate",
    "plate_dark",
    "accent",
    "strap",
    "crest",
    "joint",
    "limbs",
    "seam",
    "ink",
    "white",
}

themes.SLOT_COUNT = #themes.SLOTS

-- Builders reference `SLOT_INDEX.skin`, `SLOT_INDEX.cloth`, etc. exactly the
-- way they used to reference a resolved colour table -- the value baked per
-- vertex is now this fixed index rather than a literal colour, and it does
-- NOT depend on theme or team, so it is computed once at load time.
---@type table<string, integer>  -- 0-based: matches the shader array index directly
themes.SLOT_INDEX = {}
for i, name in ipairs(themes.SLOTS) do
    themes.SLOT_INDEX[name] = i - 1
end

-- Colours that never vary by theme or team, so they are not part of any
-- theme's `color` table. face.lua reaches for these for pupils/brows/mouths
-- (`ink`) and sclera/catchlights (`white`) on every figure style.
local CONSTANT_COLOR = {
    ink = { 0.12, 0.11, 0.13 },
    white = { 0.97, 0.97, 0.98 },
}

-- A theme may leave a slot unset when its own default already reads right --
-- e.g. Medieval Fantasy shows bare skin on the forearms, so `limbs` is simply
-- not authored. This is the exact fallback the pre-slice code applied once
-- per build (`c.limbs = c.limbs or c.skin`), moved here so it resolves once
-- per (theme, team) rather than once per vertex.
local SLOT_FALLBACK = { limbs = "skin" }

-- Resolves every palette slot for one (theme, team) pair into the flat array
-- the shader wants: `out[i]` is the RGBA for `themes.SLOTS[i]`, i.e. shader
-- index `i - 1`. Slots a theme's shape flags never actually mesh (e.g.
-- `seam` on a theme with `shape.seams = false`) are filled with an inert
-- placeholder -- correct because no vertex in that theme's build ever carries
-- that slot index, so the value is never sampled.
---@param theme table
---@param team table
---@return number[][]
function themes.resolvedPalette(theme, team)
    local out = {}
    for i, name in ipairs(themes.SLOTS) do
        local raw = CONSTANT_COLOR[name]
        if not raw then
            local slot = theme.color[name]
            if slot == nil and SLOT_FALLBACK[name] then
                slot = theme.color[SLOT_FALLBACK[name]]
            end
            raw = slot and themes.resolve(slot, team) or { 0, 0, 0 }
        end
        out[i] = { raw[1], raw[2], raw[3], raw[4] or 1 }
    end
    return out
end

---@param key string
---@return table
function themes.byKey(key)
    for _, theme in ipairs(themes.LIST) do
        if theme.key == key then
            return theme
        end
    end
    error("unknown theme: " .. tostring(key))
end

return themes
