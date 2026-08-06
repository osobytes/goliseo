local t = require("spec.support.runner")
local gl_probe = require("game.render.gl_probe")

-- The probe's whole job is to get a runtime's answer -- including the exact
-- error text of a refusal -- out of a browser through a pipe-delimited console
-- line. Everything below is that transport, tested without a GL context.

t.describe("gl_probe.encode", function()
    t.it("escapes the delimiters a LÖVE error string would otherwise split a marker on", function()
        local encoded = gl_probe.encode("a|b=c d")
        t.eq(encoded, "a%7Cb%3Dc%20d")
    end)

    t.it("leaves the unreserved set alone so a format name stays readable", function()
        t.eq(gl_probe.encode("depth24stencil8"), "depth24stencil8")
        t.eq(gl_probe.encode("a.b-c_d"), "a.b-c_d")
    end)

    t.it("encodes newlines, which a multi-line shader compile log is full of", function()
        t.eq(gl_probe.encode("x\ny"), "x%0Ay")
    end)

    t.it("stringifies non-strings so a boolean field is never nil", function()
        t.eq(gl_probe.encode(true), "true")
        t.eq(gl_probe.encode(12), "12")
    end)
end)

t.describe("gl_probe.marker", function()
    t.it("emits kind then fields in the order given", function()
        t.eq(
            gl_probe.marker("attempt", { { "format", "depth16" }, { "ok", true } }),
            "GC_GLPROBE|attempt|format=depth16|ok=true"
        )
    end)

    t.it("keeps a value containing a pipe inside its own field", function()
        local line = gl_probe.marker("attempt", { { "error", "bad|worse" }, { "ok", false } })
        -- Five parts, not six: the pipe in the value must not create a field.
        local parts = 0
        for _ in line:gmatch("[^|]+") do
            parts = parts + 1
        end
        t.eq(parts, 4)
        t.is_true(line:find("error=bad%%7Cworse", 1, false) ~= nil)
    end)
end)

t.describe("gl_probe.fields", function()
    t.it("sorts keys so two runs of the same runtime produce identical markers", function()
        local fields = gl_probe.fields({ zebra = true, alpha = false, mid = 1 })
        t.eq(fields[1][1], "alpha")
        t.eq(fields[2][1], "mid")
        t.eq(fields[3][1], "zebra")
    end)

    t.it("tolerates a nil capability table rather than erroring the whole survey", function()
        t.eq(#gl_probe.fields(nil), 0)
    end)
end)

t.describe("gl_probe.attempt", function()
    t.it("records a raising newCanvas as a failure and keeps its message", function()
        local result = gl_probe.attempt(function()
            error("The depth24stencil8 canvas format is not supported")
        end, 4, 4, "depth24stencil8", false)
        t.is_true(not result.ok)
        t.is_true(result.error:find("not supported", 1, true) ~= nil)
    end)

    t.it("treats a nil return as a failure, not a success with no canvas", function()
        local result = gl_probe.attempt(function()
            return nil
        end, 4, 4, "depth16", false)
        t.is_true(not result.ok)
        t.is_true(result.error:find("nil", 1, true) ~= nil)
    end)

    t.it("passes the format and readable flag through to newCanvas", function()
        local seen
        local result = gl_probe.attempt(function(_, _, settings)
            seen = settings
            return {}
        end, 8, 8, "depth16", true)
        t.is_true(result.ok)
        t.eq(seen.format, "depth16")
        t.eq(seen.readable, true)
    end)
end)

t.describe("gl_probe.budget", function()
    t.it("recomputes the rig3d vertex-uniform cost rather than quoting it", function()
        -- 26 bones x 3 rows + 12 palette + 12 matrix = 102, under the 128 floor.
        t.eq(gl_probe.budget(12, 26, 3), 102)
        t.is_true(gl_probe.budget(12, 26, 3) <= gl_probe.MIN_VERTEX_UNIFORM_VECTORS)
    end)

    t.it("shows four-row bones land exactly on the floor, which is why they are three", function()
        -- 26 x 4 = 104, +12 palette +12 matrix = 128. Not over the guarantee,
        -- but with zero headroom -- which is renderer.lua's stated reason for
        -- dropping the always-(0,0,0,1) fourth row. Three rows leave 26 spare.
        t.eq(gl_probe.budget(12, 26, 4), gl_probe.MIN_VERTEX_UNIFORM_VECTORS)
        t.eq(gl_probe.MIN_VERTEX_UNIFORM_VECTORS - gl_probe.budget(12, 26, 3), 26)
    end)
end)

t.describe("gl_probe ladders", function()
    t.it("names every rung uniquely, so a controller can drive them by name", function()
        local seen = {}
        for _, rung in ipairs(gl_probe.SHADER_LADDER) do
            t.is_true(seen[rung.name] == nil)
            seen[rung.name] = true
            t.is_true(rung.source:find("position", 1, true) ~= nil)
        end
        t.is_true(seen.baseline == true)
    end)

    t.it("attempts the packed depth-stencil format bloom asks for first", function()
        t.eq(gl_probe.DEPTH_ATTEMPTS[1].format, "depth24stencil8")
        t.eq(gl_probe.DEPTH_ATTEMPTS[1].readable, false)
    end)

    -- #391, carried over from #395's review: three rungs used to declare their
    -- uniform array outside the stage blocks, and every one of them failed in
    -- Firefox at LINK -- `Uniform 'u_palette' is not linkable between attached
    -- shaders` -- for a reason that has nothing to do with the construct the rung
    -- exists to isolate. A ladder that fails for its own reasons measures itself.
    --
    -- The same rule `spec/render/rig3d_spec.lua` enforces on the real shader,
    -- enforced here on the rungs that imitate it, because the ladder is only
    -- evidence while the two agree.
    t.it("declares every rung's uniforms inside a stage block, and its varyings outside", function()
        local checked_uniforms = 0
        local checked_varyings = 0
        for _, rung in ipairs(gl_probe.SHADER_LADDER) do
            local stack = {}
            for line in (rung.source .. "\n"):gmatch("(.-)\n") do
                local opened = line:match("^%s*#ifdef%s+([%u_]+)")
                if opened then
                    stack[#stack + 1] = opened
                elseif line:match("^%s*#endif") then
                    t.is_true(#stack > 0, "unbalanced #endif in ladder rung " .. rung.name)
                    stack[#stack] = nil
                else
                    local stage = stack[#stack]
                    local uniform = line:match("^%s*uniform%s+[%w_]+%s+([%w_]+)")
                    if uniform then
                        checked_uniforms = checked_uniforms + 1
                        t.is_true(
                            stage == "VERTEX" or stage == "PIXEL",
                            rung.name
                                .. " declares "
                                .. uniform
                                .. " outside every stage block, so it compiles into both"
                        )
                    end
                    local varying = line:match("^%s*varying%s+[%w_]+%s+([%w_]+)")
                    if varying then
                        checked_varyings = checked_varyings + 1
                        t.is_true(
                            stage == nil,
                            rung.name
                                .. " declares "
                                .. varying
                                .. " inside a stage block, so the other stage cannot see it"
                        )
                    end
                end
            end
            t.eq(#stack, 0, "unbalanced #ifdef in ladder rung " .. rung.name)
        end
        -- The rungs that carry the constructs, so a ladder that stopped
        -- declaring anything cannot pass this by having nothing to check.
        t.is_true(checked_uniforms >= 3, "expected the rungs' uniforms, got " .. checked_uniforms)
        t.is_true(checked_varyings >= 6, "expected the rungs' varyings, got " .. checked_varyings)
    end)
end)
