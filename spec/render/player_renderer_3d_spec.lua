local t = require("spec.support.runner")
local player_renderer_3d = require("game.render.player_renderer_3d")

-- #340, criterion 3: "requesting the rigged renderer and silently getting the
-- procedural one is an error, not a warning."
--
-- These run HEADLESS, which is what makes them worth having: headless is
-- precisely the runtime where `love.graphics.newShader` does not exist, the
-- rigged build fails, and the renderer disables itself. That used to be
-- indistinguishable from success -- one printed line, a full green suite, and
-- 1981 passing tests none of which had executed the rigged renderer. So the
-- environment that caused the blind spot is the one that now proves it is shut.
--
-- The two halves are equally load-bearing:
--
--   * With nobody asking, the fallback must stay silent and recoverable. The
--     rigged path is off by default and a runtime that cannot host it must
--     still play the game -- AGENTS.md #7's "return nil, err" case, expressed
--     here as `available()` returning false.
--   * With `required` set, the same failure must raise. The caller demanded
--     this path; handing them another one is a broken invariant, and #7 says
--     invariants fail loud.
--
-- The tier-4 gate (`scripts/check_rig3d_player_draw.sh`) covers the same
-- semantics with a real GL context. This file covers them in the tier that runs
-- on every commit.

---@param fn fun()
---@return boolean raised
---@return string message
local function raises(fn)
    local ok, err = pcall(fn)
    return not ok, tostring(err)
end

t.describe("player_renderer_3d request semantics", function()
    t.it("defaults to the recoverable fallback", function()
        t.eq(player_renderer_3d.required, false, "the rigged path must be opt-in")
    end)

    t.it("reports unavailable without raising when nobody asked for it", function()
        player_renderer_3d.required = false
        local raised, message = raises(function()
            t.is_true(
                player_renderer_3d.available() == false,
                "headless has no shader compiler, so the rigged renderer cannot be available"
            )
        end)
        t.is_true(not raised, "the default fallback must not raise: " .. message)
    end)

    t.it("raises instead of falling back when the rigged path was required", function()
        player_renderer_3d.required = true
        local raised, message = raises(function()
            player_renderer_3d.available()
        end)
        player_renderer_3d.required = false
        t.is_true(raised, "available() must not answer false to a caller that required rigged")
        t.is_true(
            message:find("rigged 3D players were required", 1, true) ~= nil,
            "the diagnostic must say the rigged path was required, got: " .. message
        )
    end)

    t.it("refuses from draw() too, instead of silently returning", function()
        -- pitch.lua guards with available(), but a caller need not, and draw()
        -- opens with `if not build() then return end` -- a silent no-op for
        -- someone who demanded the rigged path. This covers that entry point.
        --
        -- BE PRECISE ABOUT WHICH PATH THIS REACHES. The failure is already
        -- latched by the test above, so the raise comes from `build()`'s
        -- "a previous build or draw already disabled it" short-circuit, NOT
        -- from the draw-specific `refuse("a draw failed: ...")`. Reaching that
        -- one needs a build that SUCCEEDS and a draw that then fails, which is
        -- unreachable headless -- there is no shader compiler, so build() can
        -- never succeed here -- and faking one would only test the fake. It is
        -- covered by nothing today; see the gap list at the top of
        -- spec/support/rig3d_player_draw.lua.
        player_renderer_3d.required = true
        local raised, message = raises(function()
            player_renderer_3d.draw(0, 0, 1, { 1, 1, 1 }, nil, { team = "home" })
        end)
        player_renderer_3d.required = false
        t.is_true(raised, "draw() must not silently return for a caller that required rigged")
        t.is_true(
            message:find("rigged 3D players were required", 1, true) ~= nil,
            "the diagnostic must say the rigged path was required, got: " .. message
        )
    end)

    t.it("returns to a silent fallback once the requirement is lifted", function()
        -- The failure latches, so this is the case where a gate ran, demanded
        -- rigged, failed, and the process carried on: the game must not inherit
        -- the gate's strictness and start crashing on a legitimate fallback.
        player_renderer_3d.required = false
        local raised, message = raises(function()
            t.is_true(
                player_renderer_3d.available() == false,
                "a latched failure still reports unavailable"
            )
        end)
        t.is_true(not raised, "lifting the requirement must restore the fallback: " .. message)
    end)
end)
