local t = require("spec.support.runner")
local desync_package = require("game.online.desync_package")
local net_diagnostics = require("game.online.net_diagnostics")
local fixture = require("game.online.net_diagnostics_fixture")
local match_driver = require("game.online.match_driver")
local match_driver_fixture = require("game.online.match_driver_fixture")
local input_frame = require("sim.input_frame")
local match_snapshot = require("sim.match_snapshot")
local rollback_session = require("sim.rollback_session")

---@param options InputSampleOptions
---@return InputSample
local function sample(options)
    return assert(input_frame.new_sample(options))
end

---@param value any
---@param out string[]
local function collect_strings(value, out)
    if type(value) == "string" then
        out[#out + 1] = value
    elseif type(value) == "table" then
        for key, child in pairs(value) do
            collect_strings(key, out)
            collect_strings(child, out)
        end
    end
end

---@class DesyncCapture
---@field harness NetDiagnosticsFixtureHarness
---@field package table
---@field agreed MatchDriverCheckpoint
---@field diverged MatchDriverCheckpoint

-- Run a short 2v2 fixture, then build a package that claims the divergence
-- happened at the second published checkpoint. The divergence itself is
-- synthetic -- two fake-star peers in one process do not disagree -- but every
-- other part of the package is real: real wires, real identity, real boundary
-- hashes, real control and runtime ordering.
---@param steps integer?
---@return DesyncCapture
local function capture(steps)
    local harness = fixture.harness("2v2", { hash_interval_ticks = 5 })
    fixture.run(harness, steps or 40, { [1] = sample({ move_x = 55 }) })
    local host = harness.peers[1]
    local checkpoints = match_driver.checkpoints(host.driver)
    assert(#checkpoints >= 2, "the fixture published too few checkpoints to diverge between")
    local agreed = checkpoints[#checkpoints - 1]
    local diverged = checkpoints[#checkpoints]
    local wires = host.tap:wires()
    local built = assert(desync_package.build({
        recorder = host.recorder,
        manifest = harness.session.manifest,
        freeze = harness.session.freeze,
        peer_id = host.peer_id,
        remote_peer_id = harness.session.guest_peer_ids[1],
        agreed_boundary_tick = agreed.tick,
        agreed_boundary_hash = agreed.hash,
        divergence_tick = diverged.tick,
        local_hash = diverged.hash,
        remote_hash = "deadbeefdeadbeef",
        input_wires = wires,
        first_difference = { path = "state.ball.pos.x", expected = 480.5, actual = 480.75 },
    }))
    return { harness = harness, package = built, agreed = agreed, diverged = diverged }
end

t.describe("desync package", function()
    t.it("carries the identity needed to rebuild the exact match", function()
        local captured = capture()
        local package = captured.package
        local manifest = captured.harness.session.manifest
        t.eq(package.package_version, desync_package.VERSION)
        t.eq(package.session.manifest_id, captured.harness.session.freeze.manifest_id)
        t.eq(package.session.fixture_id, manifest.fixture_id)
        t.eq(package.session.build_id, manifest.build_id)
        t.eq(package.session.input_version, manifest.input_version)
        t.eq(package.session.snapshot_version, manifest.snapshot_version)
        t.eq(package.session.tape_version, manifest.tape_version)
        t.eq(package.reproduction.local_peer_id, captured.harness.peers[1].peer_id)
        t.eq(package.reproduction.remote_peer_id, captured.harness.session.guest_peer_ids[1])
    end)

    t.it("names the first differing path and both boundary hashes", function()
        local captured = capture()
        local divergence = captured.package.divergence
        t.eq(divergence.agreed_boundary_tick, captured.agreed.tick)
        t.eq(divergence.agreed_boundary_hash, captured.agreed.hash)
        t.eq(divergence.divergence_tick, captured.diverged.tick)
        t.eq(divergence.local_hash, captured.diverged.hash)
        t.eq(divergence.remote_hash, "deadbeefdeadbeef")
        t.eq(divergence.first_difference_path, "state.ball.pos.x")
        t.is_true(divergence.first_difference_expected ~= divergence.first_difference_actual)
    end)

    t.it("claims fixture-boundary-zero reproducibility only when the wires reach it", function()
        local captured = capture()
        t.eq(captured.package.reproduction.reproducible_from, "fixture_boundary_zero")
        t.eq(
            captured.package.inputs.from_input_tick,
            captured.harness.session.freeze.first_input_tick
        )
        t.eq(captured.package.inputs.retention, "complete")

        -- Drop the opening wires and the claim must weaken rather than persist.
        local host = captured.harness.peers[1]
        local wires = host.tap:wires()
        local trimmed = {}
        for index = 12, #wires do
            trimmed[#trimmed + 1] = wires[index]
        end
        local weaker = assert(desync_package.build({
            recorder = host.recorder,
            manifest = captured.harness.session.manifest,
            freeze = captured.harness.session.freeze,
            peer_id = host.peer_id,
            remote_peer_id = captured.harness.session.guest_peer_ids[1],
            agreed_boundary_tick = captured.agreed.tick,
            agreed_boundary_hash = captured.agreed.hash,
            divergence_tick = captured.diverged.tick,
            local_hash = captured.diverged.hash,
            remote_hash = "deadbeefdeadbeef",
            input_wires = trimmed,
        }))
        t.eq(weaker.reproduction.reproducible_from, "retained_window")
        t.is_true(weaker.inputs.from_input_tick > captured.package.inputs.from_input_tick)
    end)

    t.it("stays bounded and marks truncation of the wire window", function()
        local captured = capture()
        local host = captured.harness.peers[1]
        ---@type string[]
        local many = {}
        local source = host.tap:wires()
        while #many < desync_package.MAX_WIRES + 20 do
            for _, wire in ipairs(source) do
                many[#many + 1] = wire
            end
        end
        local built = assert(desync_package.build({
            recorder = host.recorder,
            manifest = captured.harness.session.manifest,
            freeze = captured.harness.session.freeze,
            peer_id = host.peer_id,
            remote_peer_id = captured.harness.session.guest_peer_ids[1],
            agreed_boundary_tick = captured.agreed.tick,
            agreed_boundary_hash = captured.agreed.hash,
            divergence_tick = captured.diverged.tick,
            local_hash = captured.diverged.hash,
            remote_hash = "deadbeefdeadbeef",
            input_wires = many,
        }))
        t.eq(#built.inputs.wires, desync_package.MAX_WIRES)
        t.eq(built.inputs.retention, "truncated")
        t.is_true(built.reproduction.reproducible_from ~= "fixture_boundary_zero")
        local bytes = assert(desync_package.encode(built))
        t.is_true(#bytes < 400 * 1024, "a bounded capture grew past a size anyone would attach")
    end)

    t.it("refuses a divergence that is not after the agreed boundary", function()
        local captured = capture()
        local host = captured.harness.peers[1]
        local built, err = desync_package.build({
            recorder = host.recorder,
            manifest = captured.harness.session.manifest,
            freeze = captured.harness.session.freeze,
            peer_id = host.peer_id,
            remote_peer_id = captured.harness.session.guest_peer_ids[1],
            agreed_boundary_tick = captured.diverged.tick,
            agreed_boundary_hash = captured.diverged.hash,
            divergence_tick = captured.agreed.tick,
            local_hash = captured.agreed.hash,
            remote_hash = "deadbeefdeadbeef",
            input_wires = host.tap:wires(),
        })
        t.eq(built, nil)
        t.is_true(err ~= nil)
    end)

    t.it("requires the diagnostic export opt-in", function()
        local harness = fixture.harness("2v2", { hash_interval_ticks = 5, export_opt_in = false })
        fixture.run(harness, 20)
        local host = harness.peers[1]
        local checkpoints = match_driver.checkpoints(host.driver)
        local built, err = desync_package.build({
            recorder = host.recorder,
            manifest = harness.session.manifest,
            freeze = harness.session.freeze,
            peer_id = host.peer_id,
            remote_peer_id = harness.session.guest_peer_ids[1],
            agreed_boundary_tick = checkpoints[1].tick,
            agreed_boundary_hash = checkpoints[1].hash,
            divergence_tick = checkpoints[#checkpoints].tick,
            local_hash = checkpoints[#checkpoints].hash,
            remote_hash = "deadbeefdeadbeef",
            input_wires = host.tap:wires(),
        })
        t.eq(built, nil)
        t.is_true(err ~= nil and err:find("opted%-in") ~= nil)
    end)

    t.it("encodes deterministically and carries no signalling material", function()
        local first = capture()
        local second = capture()
        t.eq(
            assert(desync_package.digest(second.package)),
            assert(desync_package.digest(first.package)),
            "two identical captures produced different package digests"
        )

        ---@type string[]
        local parts = {}
        collect_strings(first.package, parts)
        local text = table.concat(parts, "\n")
        for _, forbidden in ipairs({ "ice-", "candidate", "v=0", "sdp", "192.168." }) do
            t.is_true(
                text:find(forbidden, 1, true) == nil,
                forbidden .. " reached a desync package"
            )
        end
    end)

    t.it("summarises without exposing anything sensitive", function()
        local captured = capture()
        local lines = desync_package.summary(captured.package)
        t.is_true(#lines >= 5)
        local joined = table.concat(lines, "\n")
        t.is_true(joined:find("reproducible from", 1, true) ~= nil)
        t.is_true(joined:find(captured.package.session.manifest_id, 1, true) ~= nil)
    end)
end)

t.describe("desync package offline reproduction", function()
    -- The load-bearing claim: a package plus the pinned fixture is enough to
    -- rebuild the match on a machine that never saw the session, with no log at
    -- all. This rebuilds boundary zero from the fixture, feeds the package's
    -- canonical rows into a *fresh* rollback session, steps to the boundary the
    -- package names, and requires the same hash.
    t.it("rebuilds the agreed boundary hash from the package alone", function()
        local captured = capture()
        local package = captured.package
        t.eq(package.reproduction.reproducible_from, "fixture_boundary_zero")

        local manifest = captured.harness.session.manifest
        local rows =
            assert(desync_package.rows(package, manifest, package.reproduction.local_peer_id))
        t.is_true(#rows > 0)

        -- Canonical order is a contract of `rows`, not an accident of arrival.
        for index = 2, #rows do
            local previous, current = rows[index - 1], rows[index]
            t.is_true(
                current.tick > previous.tick
                    or (current.tick == previous.tick and current.slot_index > previous.slot_index),
                "package rows are not in canonical (tick, slot) order"
            )
        end

        -- A reproducer holds no local slots: every row is authority handed to it
        -- by the package, exactly as it would be on a machine that never played.
        ---@type RollbackInputSource[]
        local sources = {}
        for index = 1, input_frame.SLOT_COUNT do
            sources[index] = "remote"
        end
        local session = rollback_session.new(match_driver_fixture.initial_snapshot(), sources)
        t.is_true(assert(rollback_session.add_authoritative_batch(session, rows)).inserted > 0)

        local boundary = package.divergence.agreed_boundary_tick
        while true do
            local diagnostics = rollback_session.diagnostics(session)
            if diagnostics.present_boundary > boundary then
                break
            end
            local output = rollback_session.step(session)
            if output == nil then
                break
            end
        end
        local lookup = rollback_session.snapshot(session, boundary)
        t.is_true(
            lookup.status == "present" or lookup.status == "retained",
            "the reproduction could not reach the boundary the package named"
        )
        t.eq(
            match_snapshot.hash(assert(lookup.snapshot)),
            package.divergence.agreed_boundary_hash,
            "an offline reproduction disagreed with the captured boundary hash"
        )
    end)

    t.it("keeps the export and the package agreeing on identity", function()
        local captured = capture()
        local artifact = assert(net_diagnostics.export(captured.harness.peers[1].recorder))
        for key, value in pairs(artifact.session) do
            t.eq(
                captured.package.session[key],
                value,
                "package identity drifted from the export at " .. key
            )
        end
    end)
end)
