-- The whole 3D pipeline for the slice: one shader, a depth buffer, and ONE draw
-- call per character.
--
-- Why this exists at all: the shipping game renders 2.5D (see
-- game/render/camera.lua -- a fake-perspective projection with depth-sorted
-- billboards). There is no mesh/depth pipeline in the repo to reuse and no 3D
-- library vendored, so this file is the small real-3D renderer the slice needs.
-- It is ~200 lines and deliberately has no features the slice does not use.

local mat4 = require("core.mat4")
local skeleton = require("game.render.rig3d.skeleton")

local renderer = {}

-- The GLSL ES 1.00 guaranteed floor for `gl_MaxVertexUniformVectors`. Desktop
-- GL reports far more, but the WebGL 1 target is the one that decides whether
-- this shader links at all, and it is allowed to offer exactly this.
local MIN_VERTEX_UNIFORM_VECTORS = 128
-- u_model, u_view, u_proj: three mat4 = 12 vec4.
local MATRIX_UNIFORM_VECTORS = 12

-- Cel/toon shading: a directional key light quantised into three bands, plus a
-- view-dependent rim so silhouettes separate from the background. Enough to
-- make the 3D form read without pretending to be physically based.
--
-- Colour (#337 slice 1): a vertex no longer carries a literal {r,g,b,a}. It
-- carries a small integer palette-slot index (VertexPaletteSlot), and the
-- VERTEX stage resolves that against `u_palette[]` -- a uniform array sent once
-- per draw -- into a varying that the pixel stage shades. The lookup happens in
-- the vertex stage deliberately: GLSL ES 1.00 (WebGL1/love.js) allows dynamic
-- (non-constant) array indexing in vertex shaders but NOT in fragment shaders,
-- so doing this indexing in `effect()` would compile on desktop and silently
-- fail to compile on the web target.
--
-- SKINNING (#337 slice 2): a vertex also carries the index of the ONE bone that
-- drives it (VertexBoneIndex) and the material family that shades it
-- (VertexMaterial). Both are floats for the same WebGL 1 reason as the palette
-- slot -- there are no integer vertex attributes in GLSL ES 1.00 -- and both are
-- read in the vertex stage, again because the bone lookup is a dynamic array
-- index and only the vertex stage is allowed one.
--
--   * The bone index turns a per-part MODEL MATRIX into a per-vertex lookup, so
--     the ~28 parts of a character stop needing 28 draws.
--   * The material turns what used to be `u_metal` / `u_emissive` uniforms into
--     an interpolated varying. Without that, folding parts into one mesh would
--     still need one draw per material group and stop at ~3 per character
--     instead of 1. Branching on a varying in the fragment stage is fine here;
--     only dynamic *array indexing* is forbidden.
--
-- BONE MATRICES ARE THREE ROWS, NOT FOUR. GLSL ES 1.00 guarantees only 128
-- vertex uniform vectors. 26 bones as `mat4` is 104 vec4, plus 12 palette vec4,
-- plus model/view/proj (12) = 128 exactly -- at the floor, i.e. unlinkable on a
-- conforming minimum implementation. Every bone transform here is rotation +
-- translation + uniform scale, so its fourth row is always (0, 0, 0, 1) and
-- carries nothing: dropping it costs 78 vec4 instead of 104 and brings the
-- total to 102. `renderer.load` asserts that budget rather than trusting it.
--
-- The two `%d`s are the palette size and the bone row count, baked into the
-- source at load time (GLSL array sizes must be compile-time constants) rather
-- than hardcoded, so neither can drift from themes.SLOT_COUNT /
-- skeleton.boneCount.
local SHADER_SOURCE = [[
    varying vec3 v_normal;
    varying vec3 v_world;
    varying vec4 v_slot_color;
    varying float v_material;

    uniform mat4 u_model;
    uniform mat4 u_view;
    uniform mat4 u_proj;
    uniform vec4 u_palette[%d];

#ifdef VERTEX
    // Row-major, three rows per bone: rows 3k, 3k+1, 3k+2 are the bone's
    // world transform, its implicit fourth row being (0, 0, 0, 1).
    uniform vec4 u_bones[%d];

    attribute vec3 VertexNormal;
    attribute float VertexPaletteSlot;
    attribute float VertexBoneIndex;
    attribute float VertexMaterial;

    vec4 position(mat4 transform_projection, vec4 vertex_position) {
        // +0.5 before truncating: indices are written as exact small integers
        // (0.0, 1.0, ...), so this is just safe rounding against float noise.
        int b = int(VertexBoneIndex + 0.5) * 3;
        vec4 r0 = u_bones[b];
        vec4 r1 = u_bones[b + 1];
        vec4 r2 = u_bones[b + 2];

        // vertex_position.w is 1.0, so a row dot product is the full affine
        // transform including translation.
        vec4 posed = vec4(dot(r0, vertex_position),
                          dot(r1, vertex_position),
                          dot(r2, vertex_position), 1.0);

        // LÖVE's own transform is ignored: this is a real 3D pipeline, so we
        // build clip space ourselves from our own matrices. u_model stays a
        // separate uniform (the character's yaw on the pitch) rather than being
        // folded into every bone on the CPU: that would be 26 extra 4x4
        // multiplies per character per frame in Lua to save one 4x4 multiply
        // per vertex on the GPU.
        vec4 world = u_model * posed;
        v_world = world.xyz;

        // Every bone and model transform is rotation + translation + uniform
        // scale, so the upper-left 3x3 is a valid normal matrix -- no
        // inverse-transpose.
        vec3 bone_normal = vec3(dot(r0.xyz, VertexNormal),
                                dot(r1.xyz, VertexNormal),
                                dot(r2.xyz, VertexNormal));
        v_normal = mat3(u_model) * bone_normal;

        v_slot_color = u_palette[int(VertexPaletteSlot + 0.5)];
        // Every vertex of a triangle carries the same material, so this varying
        // is constant across the triangle and never interpolates to a value
        // between two families.
        v_material = VertexMaterial;
        return u_proj * u_view * world;
    }
#endif

#ifdef PIXEL
    uniform vec3 u_light_dir;   // direction the light travels
    uniform vec3 u_cam_pos;
    uniform float u_unlit;      // 1.0 = emit resolved colour flat (shadow, gizmos)

    vec4 effect(vec4 color, Image tex, vec2 tc, vec2 sc) {
        if (u_unlit > 0.5) {
            return v_slot_color;
        }

        // Emissive surfaces ignore the lighting model entirely and are pushed
        // past white at the centre, so they read as light sources rather than
        // as brightly painted metal. (meshbuilder.MATERIAL.emissive == 2)
        if (v_material > 1.5) {
            vec3 n_e = normalize(v_normal);
            float facing = abs(dot(n_e, normalize(u_cam_pos - v_world)));
            return vec4(v_slot_color.rgb * (1.25 + 0.55 * facing), v_slot_color.a);
        }
        // Past the emissive branch only plain (0) and metal (1) remain.
        // (meshbuilder.MATERIAL.metal == 1)
        float u_metal = v_material > 0.5 ? 1.0 : 0.0;

        vec3 n = normalize(v_normal);
        // Back faces are kept (cull mode "none") so a generator that winds a
        // face the wrong way still shades correctly instead of vanishing.
        if (!gl_FrontFacing) {
            n = -n;
        }

        float ndl = dot(n, normalize(-u_light_dir));

        // Three flat bands instead of a smooth ramp: the "toon" part.
        float band = 0.42;
        if (ndl > 0.55) {
            band = 1.0;
        } else if (ndl > 0.12) {
            band = 0.72;
        }

        // Cool bounce light from below so shadowed sides do not go dead.
        float bounce = max(-ndl, 0.0) * 0.18;

        vec3 view_dir = normalize(u_cam_pos - v_world);
        float rim = pow(1.0 - max(dot(n, view_dir), 0.0), 3.0);
        rim = smoothstep(0.35, 0.95, rim);

        vec3 lit = v_slot_color.rgb * (band + bounce)
                 + vec3(0.42, 0.52, 0.70) * rim * mix(0.55, 1.05, u_metal);

        // Metal gets one hard specular band. Skin and cloth get none, and that
        // difference is most of what separates "kit" from "body" at a glance.
        if (u_metal > 0.5) {
            vec3 half_dir = normalize(normalize(-u_light_dir) + view_dir);
            float spec = pow(max(dot(n, half_dir), 0.0), 24.0);
            lit += vec3(1.0, 0.97, 0.88) * smoothstep(0.20, 0.42, spec) * 0.60;
        }

        return vec4(lit, v_slot_color.a);
    }
#endif
]]

local shader
-- How far away the shader is told the camera is. Only affects rim/specular
-- direction, never the projection.
local SHADING_EYE_DISTANCE = 24
local light_dir = { -0.42, -0.78, -0.46 }

---@param palette_size integer  -- must match the length of every palette passed to beginPass
---@param bone_count integer    -- see skeleton.boneCount
function renderer.load(palette_size, bone_count)
    assert(
        type(palette_size) == "number" and palette_size > 0,
        "renderer.load needs a positive palette_size (see themes.SLOT_COUNT)"
    )
    assert(
        type(bone_count) == "number" and bone_count > 0,
        "renderer.load needs a positive bone_count (see skeleton.boneCount)"
    )
    local bone_rows = bone_count * skeleton.ROWS_PER_BONE
    -- Checked here rather than left to a link failure on someone's phone: on
    -- desktop GL this shader links with room to spare, so a rig that outgrows
    -- the WebGL 1 floor would otherwise only surface on the target we cannot
    -- debug. See the SHADER_SOURCE header for the arithmetic.
    local used = bone_rows + palette_size + MATRIX_UNIFORM_VECTORS
    assert(
        used <= MIN_VERTEX_UNIFORM_VECTORS,
        string.format(
            "rig3d vertex uniform budget blown: %d bones x %d rows + %d palette + %d matrix "
                .. "= %d vec4 > the %d GLSL ES 1.00 guarantees",
            bone_count,
            skeleton.ROWS_PER_BONE,
            palette_size,
            MATRIX_UNIFORM_VECTORS,
            used,
            MIN_VERTEX_UNIFORM_VECTORS
        )
    )
    shader = love.graphics.newShader(string.format(SHADER_SOURCE, palette_size, bone_rows))
end

-- Begins a 3D pass INSIDE an existing 2D frame: sets depth and shader state but
-- does not clear colour, because the match has already drawn the pitch beneath.
-- Depth alone is cleared, so each character self-occludes without inheriting the
-- previous one's depth -- inter-character ordering comes from the match's own
-- back-to-front draw order.
--
-- WHY THERE IS NO BACK-TO-FRONT PART SORTING (#337). The issue lists it, and it
-- is deliberately NOT implemented: it is in direct tension with the merge above.
-- Parts that live in one static mesh drawn in one call cannot be reordered per
-- frame by reordering draws, because there are no longer any draws to reorder.
--
-- Sorting only ever bought one thing: self-occlusion WITHOUT a depth buffer, for
-- the WebGL 1 / love.js target that cannot offer DEPTH24_STENCIL8. #330 has
-- since dropped LÖVE rendering for the browser entirely -- Babylon takes the
-- pitch -- so that beneficiary no longer exists, and native LÖVE has a depth
-- buffer for free. Keeping the depth buffer and skipping the sort is strictly
-- better here.
--
-- If a depth-less target ever matters again, the mechanism that restores it
-- WITHOUT giving back the single draw call is: keep this one static vertex
-- buffer, and per frame call `Mesh:setVertexMap` with the parts' index ranges
-- concatenated in back-to-front order (`meshbuilder.merge` already emits parts
-- contiguously, so each part is one [first, last] range and the sort key is its
-- bone's view-space depth). That costs one index-buffer upload per character per
-- frame and still submits exactly one draw call.
--
-- `palette` is sent once here rather than per part (#337 slice 1): every
-- character drawn until the next beginPass shares one set of colours, so this is
-- one uniform upload per PLAYER -- swapping an entire cosmetic variant costs
-- exactly this one send, zero extra meshes and zero extra draw calls.
---@param camera table
---@param palette number[][]  -- RGBA per slot, see themes.resolvedPalette
function renderer.beginPass(camera, palette)
    love.graphics.setShader(shader)
    love.graphics.clear(false, false, true)
    love.graphics.setDepthMode("less", true)
    love.graphics.setMeshCullMode("none")
    love.graphics.setColor(1, 1, 1, 1)
    shader:send("u_view", camera.view)
    shader:send("u_proj", camera.proj)
    shader:send("u_cam_pos", camera.eye)
    shader:send("u_light_dir", light_dir)
    shader:send("u_unlit", 0)
    shader:send("u_palette", unpack(palette))
end

-- Restores 2D state. Must be paired with beginPass.
function renderer.endPass()
    love.graphics.setShader()
    love.graphics.setDepthMode()
    love.graphics.setMeshCullMode("none")
    love.graphics.setColor(1, 1, 1, 1)
end

-- An orthographic projection that puts a character-local point at a given screen
-- position and pixel scale, seen from a fixed elevation.
--
-- This deliberately does NOT reconstruct a true 3D camera for the pitch.
-- game/render/camera.lua stays the authority on where a player is and how big
-- they appear; this only decides how the character is drawn at that spot. That
-- keeps the existing broadcast trapezoid exactly as it is.
---@return table  -- { view, proj, eye }
function renderer.characterCamera(sx, sy, ppm, vw, vh, elevation)
    local dir = { 0, math.sin(elevation), math.cos(elevation) }

    -- LÖVE's canvas framebuffers are Y-inverted relative to the backbuffer, and
    -- this shader bypasses LÖVE's own projection entirely -- so writing "correct"
    -- NDC into a canvas lands the character upside down once it is composited.
    -- The match draws the pitch inside the bloom canvas, so this is the normal
    -- path, not the exception.
    local flip = love.graphics.getCanvas() and -1 or 1
    local ys = flip * 2 * ppm / vh
    local yo = flip * (1 - 2 * sy / vh)

    -- Depth: the character occupies roughly +/-1 m about its own origin, so a
    -- gentle scale keeps it inside the clip range with room to spare.
    -- stylua: ignore
    local proj = {
        2 * ppm / vw, 0,  0,     2 * sx / vw - 1,
        0,            ys, 0,     yo,
        0,            0,  -0.35, 0,
        0,            0,  0,     1,
    }
    -- The rim and specular terms are written for a distant camera. `dir` is a
    -- unit vector, which next to a ~1.8 unit tall figure is close enough that
    -- the view direction swings wildly between the feet and the head and the
    -- shading reads inconsistently across one character. The view matrix keeps
    -- the near eye -- the orthographic depth mapping above is calibrated to it --
    -- and only the position handed to the shader moves out.
    local far = SHADING_EYE_DISTANCE
    return {
        view = mat4.lookAt(dir, { 0, 0, 0 }),
        proj = proj,
        eye = { dir[1] * far, dir[2] * far, dir[3] * far },
    }
end

-- Draws ONE whole character in ONE call: every part, every bone, every material.
--
-- `model` is only the character's placement on the pitch (its yaw). Everything
-- that used to be a per-part model matrix now rides `bone_rows` plus the bone
-- index baked into each vertex, and everything that used to be a per-part
-- material uniform rides the material baked into each vertex.
---@param mesh love.Mesh    -- the merged character mesh, see body.build
---@param model number[]    -- character placement (yaw), NOT a per-part transform
---@param bone_rows number[][]  -- 3 rows per bone in bone order, see skeleton.boneRows
function renderer.draw(mesh, model, bone_rows)
    shader:send("u_model", model)
    shader:send("u_bones", unpack(bone_rows))
    love.graphics.draw(mesh)
end

return renderer
