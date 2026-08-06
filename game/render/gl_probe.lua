-- One-shot GL capability probe for whatever runtime the game is currently on (#360).
--
-- WHY THIS EXISTS. `game/render/bloom.lua` already asks the runtime for a
-- `depth24stencil8` attachment, and `bloom.hasDepth()` is already the runtime
-- answer that `player_renderer_3d.available()` gates on. So the probe that
-- decides whether rigged 3D can run in a browser has shipped for a while --
-- what never existed was a way to READ its answer from outside the process.
-- #328 asserted "love.js is WebGL 1 and cannot supply a depth attachment at
-- all" and nobody had run the probe in a browser to check.
--
-- This module is that readout. It decides nothing: it reports what the runtime
-- said, including the exact error text a refusal came with, in a line-oriented
-- format that survives the only channel love.js has out of the process -- the
-- browser console. `scripts/lovejs_depth_probe.py` reads it and decides.
--
-- TWO MODES, AND THE SPLIT IS NOT COSMETIC. `survey()` asks the runtime what it
-- supports and creates nothing. `attempt()` creates exactly ONE canvas and then
-- stops. They are separate because in love.js an unsupported canvas format does
-- not raise a catchable Lua error -- it aborts the whole runtime, `pcall` and
-- all, and every marker that would have followed is lost with it. One creation
-- per process is what makes each format's answer independently observable, and
-- a missing terminator marker is then itself the evidence that the format did
-- not merely fail but killed the process.
--
-- The rig3d shader link is probed too, and on purpose it is INDEPENDENT of the
-- depth outcome: the shader is written to GLSL ES 1.00 against the 128-vector
-- floor WebGL 1 guarantees, which is a falsifiable prediction that can pass
-- while depth fails or fail while depth passes.

local gl_probe = {}

gl_probe.MARKER = "GC_GLPROBE"

-- The GLSL ES 1.00 floor the rig3d shader is written against. Restated here
-- rather than imported from the renderer so the probe reports a number it
-- computed itself: this report is evidence, and evidence that quotes the thing
-- it is checking has checked nothing.
gl_probe.MIN_VERTEX_UNIFORM_VECTORS = 128
gl_probe.MATRIX_UNIFORM_VECTORS = 12

-- Every depth/stencil attachment LÖVE 11.5 can be asked for, most capable
-- first, readable and non-readable separately. Non-readable is the one that
-- matters for the 3D pass (bloom asks for exactly that), but a runtime that
-- refuses only the readable variant is a different diagnosis from one that
-- refuses the format outright, and the two must not be conflated.
gl_probe.DEPTH_ATTEMPTS = {
    { format = "depth24stencil8", readable = false },
    { format = "depth24stencil8", readable = true },
    { format = "depth32fstencil8", readable = false },
    { format = "depth32f", readable = false },
    { format = "depth24", readable = false },
    { format = "depth24", readable = true },
    { format = "depth16", readable = false },
    { format = "depth16", readable = true },
    { format = "stencil8", readable = false },
}

-- Percent-encoding, not quoting. Marker values carry LÖVE error strings, which
-- contain spaces, colons, newlines and -- fatally for a pipe-delimited format --
-- both `|` and `=`. Anything outside an unreserved set is escaped so a field can
-- never split a marker; the controller decodes with `urllib.parse.unquote`.
---@param value any
---@return string
function gl_probe.encode(value)
    local text = tostring(value)
    local encoded = text:gsub("[^A-Za-z0-9%._%-]", function(character)
        return string.format("%%%02X", string.byte(character))
    end)
    return encoded
end

---@param kind string
---@param fields table[]  -- array of { key, value } pairs, in the order to emit
---@return string
function gl_probe.marker(kind, fields)
    local parts = { gl_probe.MARKER, kind }
    for _, pair in ipairs(fields) do
        parts[#parts + 1] = tostring(pair[1]) .. "=" .. gl_probe.encode(pair[2])
    end
    return table.concat(parts, "|")
end

-- A LÖVE capability table (`getSupported`, `getCanvasFormats`, `getSystemLimits`)
-- as ordered marker fields. Sorted so two runs of the same runtime produce
-- byte-identical markers and a diff means something actually changed.
---@param map table<string, any>?
---@return table[]
function gl_probe.fields(map)
    local keys = {}
    for key in pairs(map or {}) do
        keys[#keys + 1] = tostring(key)
    end
    table.sort(keys)
    local out = {}
    for _, key in ipairs(keys) do
        out[#out + 1] = { key, (map or {})[key] }
    end
    return out
end

-- Classifies one canvas-creation attempt.
--
-- Both failure shapes are treated as failure on purpose. `newCanvas` raising is
-- the documented one; a runtime that returned nil instead would otherwise be
-- recorded as a success and hand `bloom` a nil attachment it would then report
-- as present. The third shape -- the runtime aborting mid-call -- cannot be
-- observed from in here at all, which is exactly why the caller runs one
-- attempt per process and treats a missing marker as that third shape.
---@param new_canvas fun(width: number, height: number, settings: table): any
---@param width number
---@param height number
---@param format string
---@param readable boolean
---@return table  -- { format, readable, ok, error }
function gl_probe.attempt(new_canvas, width, height, format, readable)
    local ok, result = pcall(new_canvas, width, height, {
        format = format,
        readable = readable,
    })
    local failure = ""
    if not ok then
        failure = tostring(result)
    elseif result == nil then
        failure = "newCanvas returned nil without raising"
    end
    return {
        format = format,
        readable = readable,
        ok = failure == "",
        error = failure,
    }
end

-- The rig3d vertex-uniform budget, recomputed from the rig rather than quoted.
---@param palette_slots integer
---@param bones integer
---@param rows_per_bone integer
---@return integer
function gl_probe.budget(palette_slots, bones, rows_per_bone)
    return bones * rows_per_bone + palette_slots + gl_probe.MATRIX_UNIFORM_VECTORS
end

---@param emit fun(line: string)
---@param kind string
---@param fields table[]
local function say(emit, kind, fields)
    emit(gl_probe.marker(kind, fields))
end

-- Asks the runtime what it supports, and creates nothing.
--
-- Ordered so the cheap, safe facts are on the wire before anything that could
-- take the process down with it: a truncated survey is still evidence, and
-- where it truncates is evidence too.
---@param emit fun(line: string)?  -- defaults to print
function gl_probe.survey(emit)
    emit = emit or print
    local proportions = require("game.render.rig3d.proportions")
    local renderer = require("game.render.rig3d.renderer")
    local skeleton = require("game.render.rig3d.skeleton")
    local themes = require("game.render.rig3d.themes")

    local width, height = love.graphics.getDimensions()
    local name, version, vendor, device = love.graphics.getRendererInfo()
    local _, _, flags = love.window.getMode()
    -- LÖVE's type definitions omit `depth`/`stencil` from the window flags table
    -- even though conf.lua sets them and getMode returns them at runtime.
    ---@diagnostic disable-next-line: undefined-field
    local window_depth = (flags or {}).depth or 0
    ---@diagnostic disable-next-line: undefined-field
    local window_stencil = (flags or {}).stencil or false

    say(emit, "env", {
        { "love", string.format("%d.%d.%d", love.getVersion()) },
        { "system_os", love.system.getOS() },
        { "gl_name", name },
        { "gl_version", version },
        { "gl_vendor", vendor },
        { "gl_device", device },
        { "window_depth", window_depth },
        { "window_stencil", window_stencil },
        { "width", width },
        { "height", height },
    })
    say(emit, "supported", gl_probe.fields(love.graphics.getSupported()))
    -- What LÖVE itself believes about each canvas format, asked WITHOUT creating
    -- one. On a runtime where an unsupported format aborts the process rather
    -- than raising, this table is the only safe way to enumerate them -- and it
    -- is also the table `bloom` should have been consulting all along.
    say(emit, "canvas_formats", gl_probe.fields(love.graphics.getCanvasFormats()))
    say(emit, "limits", gl_probe.fields(love.graphics.getSystemLimits()))

    local bones = skeleton.boneCount(proportions.RIG_MEDIUM)
    local used = gl_probe.budget(themes.SLOT_COUNT, bones, skeleton.ROWS_PER_BONE)
    local linked, shader_error = pcall(renderer.load, themes.SLOT_COUNT, bones)
    say(emit, "shader", {
        { "linked", linked },
        { "palette_slots", themes.SLOT_COUNT },
        { "bones", bones },
        { "rows_per_bone", skeleton.ROWS_PER_BONE },
        { "budget_used", used },
        { "budget_floor", gl_probe.MIN_VERTEX_UNIFORM_VECTORS },
        { "error", linked and "" or tostring(shader_error) },
    })

    -- Deliberately last before the terminator: this is the step that binds a
    -- render target and therefore the step that can abort. Everything above is
    -- already on the wire by the time it runs.
    local bloom = require("game.render.bloom")
    local player_renderer_3d = require("game.render.player_renderer_3d")
    bloom.draw(function() end)
    say(emit, "bloom", {
        { "enabled", bloom.config.enabled },
        { "has_depth", bloom.hasDepth() },
        { "depth_format", bloom.depthFormat() or "" },
    })
    local rigged_ok, rigged_available = pcall(player_renderer_3d.available)
    say(emit, "rigged", {
        { "available", rigged_ok and rigged_available or false },
        { "error", rigged_ok and "" or tostring(rigged_available) },
    })

    -- Required by the controller. Without a terminator, a probe that died a
    -- third of the way through is indistinguishable from one that found nothing
    -- to report, and a truncated run would read as a complete negative result.
    say(emit, "end", { { "mode", "survey" } })
end

-- Creates exactly ONE canvas and reports what happened. One per process.
---@param format string
---@param readable boolean
---@param emit fun(line: string)?  -- defaults to print
function gl_probe.attemptOnce(format, readable, emit)
    emit = emit or print
    local width, height = love.graphics.getDimensions()
    -- Announced before the attempt, so a runtime that aborts inside newCanvas
    -- still leaves behind which format it was asked for.
    say(emit, "attempting", { { "format", format }, { "readable", readable } })
    local result = gl_probe.attempt(love.graphics.newCanvas, width, height, format, readable)
    say(emit, "attempt", {
        { "format", result.format },
        { "readable", result.readable },
        { "ok", result.ok },
        { "error", result.error },
    })
    say(emit, "end", { { "mode", "attempt" } })
end

-- A bisecting ladder from a trivial shader up to the real rig3d one.
--
-- "The shader does not compile" is not a finding anyone can act on, and the
-- runtime that rejected it (Firefox, #391) reported four mangled undefined
-- identifiers rather than naming a construct. Each rung adds exactly ONE of
-- rig3d's demands, so the first rung that fails names the construct the runtime
-- will not take. The rungs are deliberately in dependency order and each is
-- otherwise minimal. Every rung passes in both browsers today; the ladder earns
-- its keep the next time one does not.
--
-- The uniform array sizes match `renderer.lua` exactly: 12 palette slots, and
-- 26 bones x 3 rows = 78 bone rows.
--
-- Each rung also matches `renderer.lua` in WHERE it declares things, and that is
-- load-bearing rather than cosmetic. A uniform declared outside `#ifdef VERTEX`
-- compiles into both stages, which LÖVE's generated GLSL ES headers give
-- different default float precision; GLSL ES 1.00 requires a cross-stage uniform
-- to match, and Firefox refuses to link when it does not. Desktop GL and Chrome
-- do not enforce it, so a misplaced declaration here would read as a Firefox
-- capability defect rather than as a defect in the ladder. Varyings stay outside
-- the stage blocks: both stages must see them, and they are exempt from that
-- rule. `spec/render/gl_probe_spec.lua` gates both placements.
local LADDER_PIXEL = [[
#ifdef PIXEL
    vec4 effect(vec4 color, Image tex, vec2 tc, vec2 sc) {
        return vec4(v_probe, 1.0);
    }
#endif
]]

gl_probe.SHADER_LADDER = {
    -- Rung zero: the LÖVE entry points and NO user-declared varying. This is the
    -- shape bloom.lua's own threshold and blur shaders have, and it is here
    -- because the first version of this ladder started one rung too high and
    -- concluded "no LÖVE shader compiles in Firefox" from it. That was wrong:
    -- shaders in this shape compile and run there. The boundary the ladder has
    -- to locate sits between this rung and the next one, so both must exist.
    --
    -- It carries its own pixel stage because every other rung's reads v_probe.
    {
        name = "no_varying",
        source = [[
        #ifdef VERTEX
            vec4 position(mat4 transform_projection, vec4 vertex_position) {
                return transform_projection * vertex_position;
            }
        #endif
        #ifdef PIXEL
            extern number probe_threshold;
            vec4 effect(vec4 color, Image tex, vec2 tc, vec2 sc) {
                vec4 c = Texel(tex, tc);
                number b = max(c.r, max(c.g, c.b));
                return vec4(c.rgb * smoothstep(probe_threshold, probe_threshold + 0.15, b), 1.0);
            }
        #endif
        ]],
        pixel = false,
    },
    -- The same shader plus ONE user-declared varying, and nothing else.
    {
        name = "baseline",
        source = [[
            varying vec3 v_probe;
        #ifdef VERTEX
            vec4 position(mat4 transform_projection, vec4 vertex_position) {
                v_probe = vec3(0.5);
                return transform_projection * vertex_position;
            }
        #endif
        ]],
    },
    -- One custom vertex attribute. rig3d needs four; if one already fails, the
    -- count is not the problem.
    {
        name = "one_custom_attribute",
        source = [[
            varying vec3 v_probe;
        #ifdef VERTEX
            attribute float VertexBoneIndex;
            vec4 position(mat4 transform_projection, vec4 vertex_position) {
                v_probe = vec3(VertexBoneIndex);
                return transform_projection * vertex_position;
            }
        #endif
        ]],
    },
    -- All four of rig3d's custom attributes, same names and types.
    {
        name = "four_custom_attributes",
        source = [[
            varying vec3 v_probe;
        #ifdef VERTEX
            attribute vec3 VertexNormal;
            attribute float VertexPaletteSlot;
            attribute float VertexBoneIndex;
            attribute float VertexMaterial;
            vec4 position(mat4 transform_projection, vec4 vertex_position) {
                v_probe = VertexNormal
                    + vec3(VertexPaletteSlot + VertexBoneIndex + VertexMaterial);
                return transform_projection * vertex_position;
            }
        #endif
        ]],
    },
    -- A uniform array read at a compile-time constant index. GLSL ES 1.00
    -- guarantees this one.
    {
        name = "uniform_array_constant_index",
        source = [[
            varying vec3 v_probe;
        #ifdef VERTEX
            uniform vec4 u_palette[12];
            vec4 position(mat4 transform_projection, vec4 vertex_position) {
                v_probe = u_palette[3].rgb;
                return transform_projection * vertex_position;
            }
        #endif
        ]],
    },
    -- The same array read at an index computed from an attribute. This is the
    -- construct rig3d's palette lookup depends on, and the one GLSL ES 1.00
    -- Appendix A makes optional rather than mandatory.
    {
        name = "uniform_array_dynamic_index",
        source = [[
            varying vec3 v_probe;
        #ifdef VERTEX
            uniform vec4 u_palette[12];
            attribute float VertexPaletteSlot;
            vec4 position(mat4 transform_projection, vec4 vertex_position) {
                v_probe = u_palette[int(VertexPaletteSlot + 0.5)].rgb;
                return transform_projection * vertex_position;
            }
        #endif
        ]],
    },
    -- rig3d's bone lookup: a 78-element uniform array read at three dynamic
    -- indices derived from one attribute.
    {
        name = "bone_array_dynamic_index",
        source = [[
            varying vec3 v_probe;
        #ifdef VERTEX
            uniform vec4 u_bones[78];
            attribute float VertexBoneIndex;
            vec4 position(mat4 transform_projection, vec4 vertex_position) {
                int b = int(VertexBoneIndex + 0.5) * 3;
                v_probe = vec3(dot(u_bones[b], vertex_position),
                               dot(u_bones[b + 1], vertex_position),
                               dot(u_bones[b + 2], vertex_position));
                return transform_projection * vertex_position;
            }
        #endif
        ]],
    },
}

-- Compiles exactly ONE rung and reports what happened. One per process, for the
-- same reason as `attemptOnce`: a shader the runtime cannot take does not
-- always come back as a catchable Lua error.
---@param step string  -- a name from SHADER_LADDER, or "rig3d" for the real one
---@param emit fun(line: string)?  -- defaults to print
function gl_probe.shaderStep(step, emit)
    emit = emit or print
    say(emit, "compiling", { { "step", step } })
    local ok, err
    if step == "rig3d" then
        local proportions = require("game.render.rig3d.proportions")
        local renderer = require("game.render.rig3d.renderer")
        local skeleton = require("game.render.rig3d.skeleton")
        local themes = require("game.render.rig3d.themes")
        ok, err =
            pcall(renderer.load, themes.SLOT_COUNT, skeleton.boneCount(proportions.RIG_MEDIUM))
    else
        local source
        for _, rung in ipairs(gl_probe.SHADER_LADDER) do
            if rung.name == step then
                -- `pixel = false` means the rung brought its own pixel stage,
                -- because the shared one reads a varying the rung must not have.
                source = rung.source
                if rung.pixel ~= false then
                    source = source .. LADDER_PIXEL
                end
            end
        end
        if not source then
            say(emit, "shader_step", {
                { "step", step },
                { "ok", false },
                { "error", "unknown ladder step" },
            })
            say(emit, "end", { { "mode", "shader" } })
            return
        end
        ok, err = pcall(love.graphics.newShader, source)
    end
    say(emit, "shader_step", {
        { "step", step },
        { "ok", ok },
        { "error", ok and "" or tostring(err) },
    })
    say(emit, "end", { { "mode", "shader" } })
end

return gl_probe
