-- The whole 3D pipeline for the slice: one shader, a depth buffer, and a
-- model/view/projection matrix per draw call.
--
-- Why this exists at all: the shipping game renders 2.5D (see
-- game/render/camera.lua -- a fake-perspective projection with depth-sorted
-- billboards). There is no mesh/depth pipeline in the repo to reuse and no 3D
-- library vendored, so this file is the small real-3D renderer the slice needs.
-- It is ~120 lines and deliberately has no features the slice does not use.

local mat4 = require("core.mat4")

local renderer = {}

-- Cel/toon shading: a directional key light quantised into three bands, plus a
-- view-dependent rim so silhouettes separate from the background. Enough to
-- make the 3D form read without pretending to be physically based.
local SHADER_SOURCE = [[
    varying vec3 v_normal;
    varying vec3 v_world;

    uniform mat4 u_model;
    uniform mat4 u_view;
    uniform mat4 u_proj;

#ifdef VERTEX
    attribute vec3 VertexNormal;

    vec4 position(mat4 transform_projection, vec4 vertex_position) {
        // LÖVE's own transform is ignored: this is a real 3D pipeline, so we
        // build clip space ourselves from our own matrices.
        vec4 world = u_model * vertex_position;
        v_world = world.xyz;
        // Every model transform is rotation + translation + uniform scale, so
        // the upper-left 3x3 is a valid normal matrix -- no inverse-transpose.
        v_normal = mat3(u_model) * VertexNormal;
        return u_proj * u_view * world;
    }
#endif

#ifdef PIXEL
    uniform vec3 u_light_dir;   // direction the light travels
    uniform vec3 u_cam_pos;
    uniform float u_unlit;      // 1.0 = emit vertex colour flat (shadow, gizmos)
    uniform float u_metal;      // 1.0 = armour: hard specular band + hotter rim
    uniform float u_emissive;   // 1.0 = self-lit: energy blades, panel seams

    vec4 effect(vec4 color, Image tex, vec2 tc, vec2 sc) {
        if (u_unlit > 0.5) {
            return color;
        }

        // Emissive surfaces ignore the lighting model entirely and are pushed
        // past white at the centre, so they read as light sources rather than
        // as brightly painted metal.
        if (u_emissive > 0.5) {
            vec3 n_e = normalize(v_normal);
            float facing = abs(dot(n_e, normalize(u_cam_pos - v_world)));
            return vec4(color.rgb * (1.25 + 0.55 * facing), color.a);
        }

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

        vec3 lit = color.rgb * (band + bounce)
                 + vec3(0.42, 0.52, 0.70) * rim * mix(0.55, 1.05, u_metal);

        // Metal gets one hard specular band. Skin and cloth get none, and that
        // difference is most of what separates "kit" from "body" at a glance.
        if (u_metal > 0.5) {
            vec3 half_dir = normalize(normalize(-u_light_dir) + view_dir);
            float spec = pow(max(dot(n, half_dir), 0.0), 24.0);
            lit += vec3(1.0, 0.97, 0.88) * smoothstep(0.20, 0.42, spec) * 0.60;
        }

        return vec4(lit, color.a);
    }
#endif
]]

local shader
local light_dir = { -0.42, -0.78, -0.46 }

function renderer.load()
    shader = love.graphics.newShader(SHADER_SOURCE)
end

-- Begins a 3D pass INSIDE an existing 2D frame: sets depth and shader state but
-- does not clear colour, because the match has already drawn the pitch beneath.
-- Depth alone is cleared, so each character self-occludes without inheriting the
-- previous one's depth -- inter-character ordering comes from the match's own
-- back-to-front draw order.
---@param camera table
function renderer.beginPass(camera)
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
    shader:send("u_metal", 0)
    shader:send("u_emissive", 0)
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
    return { view = mat4.lookAt(dir, { 0, 0, 0 }), proj = proj, eye = dir }
end

-- Begins a 3D pass: clears colour + depth, enables depth testing, and uploads
-- the camera matrices once for every draw that follows.
---@param camera table  -- { view = mat4, proj = mat4, eye = {x, y, z} }
---@param sky number[]
function renderer.beginFrame(camera, sky)
    love.graphics.clear(sky[1], sky[2], sky[3], 1, true, true)
    love.graphics.setShader(shader)
    love.graphics.setDepthMode("less", true)
    -- "none" keeps back faces: see the gl_FrontFacing note in the shader.
    love.graphics.setMeshCullMode("none")
    love.graphics.setColor(1, 1, 1, 1)

    shader:send("u_view", camera.view)
    shader:send("u_proj", camera.proj)
    shader:send("u_cam_pos", camera.eye)
    shader:send("u_light_dir", light_dir)
    shader:send("u_unlit", 0)
    shader:send("u_metal", 0)
    shader:send("u_emissive", 0)
end

---@param mesh love.Mesh
---@param model number[]
---@param material string|nil  -- "metal" for armour and weapons, nil for the rest
function renderer.draw(mesh, model, material)
    shader:send("u_model", model)
    shader:send("u_metal", material == "metal" and 1 or 0)
    shader:send("u_emissive", material == "emissive" and 1 or 0)
    love.graphics.draw(mesh)
end

-- Draws unlit, without writing depth: the ground shadow and the skeleton
-- gizmos, which must not occlude the figure.
---@param mesh love.Mesh
---@param model number[]
---@param depth_test boolean
function renderer.drawOverlay(mesh, model, depth_test)
    love.graphics.setDepthMode(depth_test and "less" or "always", false)
    shader:send("u_unlit", 1)
    shader:send("u_metal", 0)
    shader:send("u_emissive", 0)
    shader:send("u_model", model)
    love.graphics.draw(mesh)
    shader:send("u_unlit", 0)
    love.graphics.setDepthMode("less", true)
end

-- Restores LÖVE's normal 2D state so the HUD can be drawn.
function renderer.endFrame()
    love.graphics.setShader()
    love.graphics.setDepthMode()
    love.graphics.setMeshCullMode("none")
    love.graphics.setColor(1, 1, 1, 1)
end

-- Builds the view/projection pair for an orbiting camera.
---@param orbit table  -- { yaw, pitch, distance, target = {x, y, z} }
---@return table
function renderer.camera(orbit, width, height)
    local cp = math.cos(orbit.pitch)
    local eye = {
        orbit.target[1] + math.sin(orbit.yaw) * cp * orbit.distance,
        orbit.target[2] + math.sin(orbit.pitch) * orbit.distance,
        orbit.target[3] + math.cos(orbit.yaw) * cp * orbit.distance,
    }
    return {
        eye = eye,
        view = mat4.lookAt(eye, orbit.target),
        proj = mat4.perspective(46, width / height, 0.05, 60),
    }
end

return renderer
