local t = require("spec.support.runner")
local rng = require("core.rng")
local network_profiles = require("data.network_profiles")
local input_frame = require("sim.input_frame")
local network_conditions = require("sim.network_conditions")

---@param move_x integer?
---@param edges integer?
---@return InputSample
local function sample(move_x, edges)
    return assert(input_frame.new_sample({ move_x = move_x, edges = edges }))
end

---@param options table?
---@return NetworkProfile
local function profile(options)
    options = options or {}
    return {
        base_delay_ticks = options.base_delay_ticks or 0,
        jitter_min_ticks = options.jitter_min_ticks or 0,
        jitter_max_ticks = options.jitter_max_ticks or 0,
        independent_loss_rate = options.independent_loss_rate or 0,
        duplication_rate = options.duplication_rate or 0,
        burst_start_rate = options.burst_start_rate or 0,
        burst_length_ticks = options.burst_length_ticks or 0,
    }
end

---@param deliveries NetworkDelivery[]
---@return string
local function delivery_schedule(deliveries)
    local parts = {}
    for index, delivery in ipairs(deliveries) do
        parts[index] = ("%d:%d@%d"):format(
            delivery.sequence,
            delivery.duplicate_ordinal,
            delivery.arrival_tick
        )
    end
    return table.concat(parts, ",")
end

---@param conditions NetworkConditions
---@return string
local function outcome(conditions)
    local deliveries = network_conditions.poll(conditions, 1000)
    local counters = network_conditions.counters(conditions)
    local delivery_parts = {}
    for delivery_index, delivery in ipairs(deliveries) do
        local record_parts = {}
        for record_index, record in ipairs(network_conditions.records(delivery)) do
            record_parts[record_index] = ("%d/%d/%d/%d/%d/%d"):format(
                record.tick,
                record.sample.move_x,
                record.sample.move_y,
                record.sample.held,
                record.sample.edges,
                record.sample.aim
            )
        end
        delivery_parts[delivery_index] = ("%d:%d:%d:%d:%d:%s"):format(
            delivery.source_slot,
            delivery.send_tick,
            delivery.sequence,
            delivery.duplicate_ordinal,
            delivery.arrival_tick,
            table.concat(record_parts, ",")
        )
    end
    return table.concat({
        table.concat(delivery_parts, ";"),
        counters.sent,
        counters.delivered,
        counters.independent_lost,
        counters.burst_lost,
        counters.duplicated,
        counters.reordered,
        counters.history_recovered,
    }, "|")
end

---@param seed integer
---@return NetworkConditions
local function populated_playable(seed)
    local conditions = network_conditions.new(network_profiles.playable, seed)
    for tick = 0, 39 do
        assert(network_conditions.send(conditions, tick, (tick % 8) + 1, tick, sample(tick % 128)))
    end
    return conditions
end

t.describe("OMP-2 deterministic network conditions", function()
    t.it("pins the named laboratory profiles", function()
        local clean = network_profiles.clean
        t.eq(clean.base_delay_ticks, 0)
        t.eq(clean.jitter_min_ticks, 0)
        t.eq(clean.jitter_max_ticks, 0)
        t.eq(clean.independent_loss_rate, 0)
        t.eq(clean.duplication_rate, 0)
        t.eq(clean.burst_start_rate, 0)
        t.eq(clean.burst_length_ticks, 0)

        local parity = network_profiles.omp0_parity
        t.eq(parity.base_delay_ticks, 3)
        t.eq(parity.jitter_min_ticks, 0)
        t.eq(parity.jitter_max_ticks, 0)
        t.eq(parity.independent_loss_rate, 0.01)
        t.eq(parity.duplication_rate, 0)
        t.eq(parity.burst_start_rate, 0)
        t.eq(parity.burst_length_ticks, 0)

        local playable = network_profiles.playable
        t.eq(playable.base_delay_ticks, 3)
        t.eq(playable.jitter_min_ticks, -2)
        t.eq(playable.jitter_max_ticks, 2)
        t.eq(playable.independent_loss_rate, 0.01)
        t.eq(playable.duplication_rate, 0.0025)
        t.eq(playable.burst_start_rate, 0.0025)
        t.eq(playable.burst_length_ticks, 3)

        local stress = network_profiles.stress
        t.eq(stress.base_delay_ticks, 6)
        t.eq(stress.jitter_min_ticks, -3)
        t.eq(stress.jitter_max_ticks, 3)
        t.eq(stress.independent_loss_rate, 0.03)
        t.eq(stress.duplication_rate, 0.01)
        t.eq(stress.burst_start_rate, 0.01)
        t.eq(stress.burst_length_ticks, 3)
    end)

    t.it("preserves exact samples and send order under the clean profile", function()
        local conditions = network_conditions.new(network_profiles.clean, 91)
        local supplied = sample(14, input_frame.EDGE_BITS.dash)
        assert(network_conditions.send(conditions, 0, 1, 0, supplied))
        supplied.move_x = -90
        assert(network_conditions.send(conditions, 1, 1, 1, sample(22)))

        local deliveries = network_conditions.poll(conditions, 1)
        t.eq(delivery_schedule(deliveries), "1:0@0,2:0@1")
        t.eq(deliveries[1].current.sample.move_x, 14)
        t.eq(deliveries[1].current.sample.edges, input_frame.EDGE_BITS.dash)
        t.eq(#deliveries[1].history, 0)
        t.eq(#deliveries[2].history, 1)
        deliveries[2].history[1].sample.move_x = -1
        local records = network_conditions.records(deliveries[2])
        t.eq(records[1].tick, 0)
        t.eq(records[1].sample.move_x, -1, "records copies the caller-owned delivery")
        records[1].sample.move_x = 99
        t.eq(deliveries[2].history[1].sample.move_x, -1)
    end)

    t.it("uses literal fixed latency and clamps negative delivery time", function()
        local delayed = network_conditions.new(profile({ base_delay_ticks = 3 }), 7)
        local receipt = assert(network_conditions.send(delayed, 7, 1, 20, sample(20)))
        t.eq(receipt.arrival_tick, 10)
        t.eq(#network_conditions.poll(delayed, 9), 0)
        t.eq(delivery_schedule(network_conditions.poll(delayed, 10)), "1:0@10")

        local clamped =
            network_conditions.new(profile({ jitter_min_ticks = -3, jitter_max_ticks = -3 }), 7)
        local clamped_receipt = assert(network_conditions.send(clamped, 12, 1, 20, sample(20)))
        t.eq(clamped_receipt.arrival_tick, 12)
    end)

    t.it("bounds transport arrival before mutating send or drain state", function()
        local maximum = network_conditions.MAX_TRANSPORT_TICK
        t.eq(maximum, 2147483647)

        local rejected = network_conditions.new(profile({ base_delay_ticks = 3 }), 7)
        local receipt, message, code =
            network_conditions.send(rejected, maximum - 2, 1, 0, sample(1))
        t.eq(receipt, nil)
        t.eq(code, "malformed")
        t.is_true(assert(message):match("transport tick limit") ~= nil)
        t.eq(network_conditions.counters(rejected).sent, 0)
        t.eq(network_conditions.diagnostics(rejected).retained_authoritative_records, 0)

        local boundary = assert(network_conditions.send(rejected, maximum - 3, 1, 0, sample(1)))
        t.eq(boundary.sequence, 1, "rejected send consumes neither sequence nor RNG state")
        t.eq(boundary.arrival_tick, maximum)
        t.eq(delivery_schedule(network_conditions.poll(rejected, maximum)), "1:0@2147483647")
        t.is_true(
            not pcall(network_conditions.poll, rejected, maximum + 1),
            "poll rejects transport ticks above the exact bound"
        )

        local draining = network_conditions.new(profile({ base_delay_ticks = 3 }), 9)
        assert(network_conditions.send(draining, maximum - 6, 1, 4, sample(4)))
        network_conditions.poll(draining, maximum - 3)
        local before = network_conditions.counters(draining)
        local result, drain_message, drain_code = network_conditions.drain(
            draining,
            maximum - 2,
            1,
            { { source_slot = 1, input_tick = 4 } }
        )
        t.eq(result, nil)
        t.eq(drain_code, "malformed")
        t.is_true(assert(drain_message):match("transport tick limit") ~= nil)
        t.eq(network_conditions.counters(draining).sent, before.sent)
        t.eq(network_conditions.pending(draining), 0)
    end)

    t.it("packs sample extrema into distinct collision-free diagnostic keys", function()
        local minimum = assert(input_frame.new_sample({
            move_x = -127,
            move_y = -127,
            held = 0,
            edges = 0,
        }))
        local before_rollover = assert(input_frame.new_sample({
            move_x = -127,
            move_y = 127,
            held = 127,
            edges = 127,
        }))
        local after_rollover = assert(input_frame.new_sample({
            move_x = -126,
            move_y = -127,
            held = 0,
            edges = 0,
        }))
        local maximum = assert(input_frame.new_sample({
            move_x = 127,
            move_y = 127,
            held = 127,
            edges = 127,
        }))
        -- Every sample here leaves `aim` at AIM_NONE, so the aim radix contributes
        -- a constant 255 to each key; the whole point is that no two distinct
        -- samples share a key, and the extrema still bracket the declared bound.
        local none = input_frame.AIM_NONE
        t.eq(assert(network_conditions.sample_key(minimum)), none)
        t.eq(
            assert(network_conditions.sample_key(before_rollover)),
            ((254 * 256 + 127) * 128 + 127) * 256 + none
        )
        t.eq(assert(network_conditions.sample_key(after_rollover)), 255 * 256 * 128 * 256 + none)
        t.eq(
            assert(network_conditions.sample_key(maximum)),
            (((254 * 255 + 254) * 256 + 127) * 128 + 127) * 256 + none
        )
        t.is_true(
            assert(network_conditions.sample_key(maximum)) < 2 ^ 53,
            "the mixed-radix key must stay exactly representable"
        )
        local aimed = assert(input_frame.new_sample({ aim = 0 }))
        local aimless = assert(input_frame.new_sample({}))
        t.is_true(
            assert(network_conditions.sample_key(aimed))
                ~= assert(network_conditions.sample_key(aimless)),
            "two samples differing only in aim must not share a key"
        )
    end)

    t.it("hits both jitter bounds and reorders only by natural arrival", function()
        local conditions = network_conditions.new(
            profile({ base_delay_ticks = 2, jitter_min_ticks = -2, jitter_max_ticks = 2 }),
            102223
        )
        local upper = assert(network_conditions.send(conditions, 0, 1, 0, sample(1)))
        local lower = assert(network_conditions.send(conditions, 1, 1, 1, sample(2)))
        t.eq(upper.arrival_tick, 4, "first jitter roll selects +2")
        t.eq(lower.arrival_tick, 1, "second jitter roll selects -2")

        t.eq(delivery_schedule(network_conditions.poll(conditions, 1)), "2:0@1")
        t.eq(delivery_schedule(network_conditions.poll(conditions, 4)), "1:0@4")
        t.eq(network_conditions.counters(conditions).reordered, 1)
        t.eq(
            network_conditions.counters(conditions).history_recovered,
            1,
            "the reordered original does not recount history recovered by the later sequence"
        )
    end)

    t.it("follows the literal independent-loss schedule for seed 85", function()
        local conditions = network_conditions.new(profile({ independent_loss_rate = 0.5 }), 85)
        local dropped = {}
        for tick = 0, 5 do
            local receipt = assert(network_conditions.send(conditions, tick, 1, tick, sample(tick)))
            dropped[#dropped + 1] = receipt.dropped and "1" or "0"
        end
        t.eq(table.concat(dropped), "101001")
        t.eq(delivery_schedule(network_conditions.poll(conditions, 5)), "2:0@1,4:0@3,5:0@4")
        local counters = network_conditions.counters(conditions)
        t.eq(counters.independent_lost, 3)
        t.eq(counters.burst_lost, 0)
    end)

    t.it("duplicates envelopes with stable identity and equal-arrival ordering", function()
        local conditions = network_conditions.new(profile({ duplication_rate = 0.5 }), 592)
        for tick = 0, 4 do
            assert(network_conditions.send(conditions, 0, 1, tick, sample(tick)))
        end
        local deliveries = network_conditions.poll(conditions, 0)
        t.eq(delivery_schedule(deliveries), "1:0@0,1:1@0,2:0@0,3:0@0,3:1@0,4:0@0,5:0@0")
        t.eq(deliveries[1].sequence, deliveries[2].sequence)
        t.eq(deliveries[1].current.tick, deliveries[2].current.tick)
        t.eq(network_conditions.counters(conditions).duplicated, 2)
        t.eq(network_conditions.counters(conditions).history_recovered, 0)
    end)

    t.it("applies a literal three-tick burst per source slot", function()
        local conditions =
            network_conditions.new(profile({ burst_start_rate = 0.5, burst_length_ticks = 3 }), 58)
        local receipts = {}
        receipts[1] = assert(network_conditions.send(conditions, 0, 1, 0, sample(0)))
        receipts[2] = assert(network_conditions.send(conditions, 1, 1, 1, sample(1)))
        receipts[3] = assert(network_conditions.send(conditions, 2, 1, 2, sample(2)))
        local other_slot = assert(network_conditions.send(conditions, 2, 2, 0, sample(20)))
        receipts[4] = assert(network_conditions.send(conditions, 3, 1, 3, sample(3)))
        receipts[5] = assert(network_conditions.send(conditions, 4, 1, 4, sample(4)))

        t.eq(receipts[1].drop_reason, nil)
        t.eq(receipts[2].drop_reason, "burst_loss")
        t.eq(receipts[3].drop_reason, "burst_loss")
        t.eq(receipts[4].drop_reason, "burst_loss")
        t.eq(receipts[5].drop_reason, nil)
        t.eq(other_slot.drop_reason, nil, "slot two is not inside slot one's burst")
        t.eq(delivery_schedule(network_conditions.poll(conditions, 4)), "1:0@0,4:0@2,6:0@4")
        t.eq(network_conditions.counters(conditions).burst_lost, 3)
    end)

    t.it("retains exactly six earlier unique rows and recovers loss from history", function()
        local retained = network_conditions.new(network_profiles.clean, 3)
        for tick = 0, 7 do
            assert(network_conditions.send(retained, tick, 1, tick, sample(tick)))
        end
        local deliveries = network_conditions.poll(retained, 7)
        local last = deliveries[#deliveries]
        t.eq(#last.history, 6)
        for index = 1, 6 do
            t.eq(last.history[index].tick, index)
        end
        t.eq(last.current.tick, 7)

        local recovered = network_conditions.new(profile({ independent_loss_rate = 0.5 }), 85)
        t.is_true(assert(network_conditions.send(recovered, 0, 1, 0, sample(40))).dropped)
        t.is_true(not assert(network_conditions.send(recovered, 1, 1, 1, sample(41))).dropped)
        local recovered_delivery = network_conditions.poll(recovered, 1)
        local records = network_conditions.records(recovered_delivery[1])
        t.eq(records[1].tick, 0)
        t.eq(records[2].tick, 1)
        t.eq(network_conditions.counters(recovered).history_recovered, 1)

        local oldest = network_conditions.new(profile({ independent_loss_rate = 0.5 }), 290)
        for tick = 0, 6 do
            assert(network_conditions.send(oldest, tick, 1, tick, sample(60 + tick)))
        end
        local oldest_delivery = network_conditions.poll(oldest, 6)
        t.eq(#oldest_delivery, 1)
        local oldest_records = network_conditions.records(oldest_delivery[1])
        t.eq(#oldest_records, 7)
        t.eq(oldest_records[1].tick, 0, "the oldest of six redundant rows is recovered")
        t.eq(oldest_records[7].tick, 6)
        t.eq(network_conditions.counters(oldest).history_recovered, 6)
    end)

    t.it("rejects conflicting history and resends without adding an input row", function()
        local conditions = network_conditions.new(network_profiles.clean, 11)
        assert(network_conditions.send(conditions, 0, 1, 0, sample(10)))
        assert(network_conditions.send(conditions, 1, 1, 1, sample(11)))

        local rejected, message, code = network_conditions.send(conditions, 1, 1, 1, sample(12))
        t.eq(rejected, nil)
        t.eq(code, "conflicting_authoritative")
        t.is_true(assert(message):match("tick 1 slot 1") ~= nil)
        t.eq(network_conditions.counters(conditions).sent, 2)

        local receipt = assert(network_conditions.resend(conditions, 2, 1, 1))
        t.eq(receipt.authoritative_duplicate, true)
        local deliveries = network_conditions.poll(conditions, 2)
        local resend = deliveries[#deliveries]
        t.eq(#resend.history, 1)
        t.eq(resend.history[1].tick, 0)
        t.eq(resend.current.tick, 1)
    end)

    t.it("drains a lost final sample through resends without a match tick", function()
        local conditions = network_conditions.new(profile({ independent_loss_rate = 0.5 }), 85)
        local original = assert(network_conditions.send(conditions, 0, 1, 99, sample(99)))
        t.eq(original.drop_reason, "independent_loss")

        local result = assert(network_conditions.drain(conditions, 1, 5, {
            { source_slot = 1, input_tick = 99 },
        }))
        t.is_true(result.complete)
        t.eq(result.final_tick, 1)
        t.eq(result.recovered, 1)
        t.eq(result.pending, 0)
        t.eq(delivery_schedule(result.deliveries), "2:0@1")
        t.eq(result.deliveries[1].current.tick, 99, "transport advanced, input did not")
    end)

    t.it("is byte-equivalent for one seed and isolated from match RNG", function()
        local match_state = rng.seed(999)
        local control_state = rng.seed(999)
        local first = outcome(populated_playable(431))
        local second = outcome(populated_playable(431))
        local different = outcome(populated_playable(432))
        t.eq(first, second)
        t.is_true(first ~= different)

        local match_roll, control_roll
        match_state, match_roll = rng.roll(match_state)
        control_state, control_roll = rng.roll(control_state)
        t.eq(match_state, control_state)
        t.eq(match_roll, control_roll)
    end)

    t.it("stays within broad deterministic playable-profile bounds", function()
        local conditions = network_conditions.new(network_profiles.playable, 4242)
        local count = 10000
        for tick = 0, count - 1 do
            assert(
                network_conditions.send(
                    conditions,
                    tick,
                    (tick % input_frame.SLOT_COUNT) + 1,
                    tick,
                    sample(tick % 128)
                )
            )
        end
        network_conditions.poll(conditions, count + 10)
        local counters = network_conditions.counters(conditions)
        t.eq(counters.sent, count)
        t.eq(network_conditions.pending(conditions), 0)
        t.is_true(counters.independent_lost >= 50 and counters.independent_lost <= 160)
        t.is_true(counters.burst_lost >= 20 and counters.burst_lost <= 180)
        t.is_true(counters.duplicated >= 5 and counters.duplicated <= 60)
        t.is_true(counters.reordered > 0)
        t.is_true(counters.history_recovered > 0)
    end)

    t.it("bounds retained diagnostics across a full seven-remote fixture", function()
        local conditions = network_conditions.new(network_profiles.clean, 1201)
        local last_tick = 7200
        local remote_slots = 7
        for tick = 0, last_tick do
            for source_slot = 1, remote_slots do
                assert(
                    network_conditions.send(
                        conditions,
                        tick,
                        source_slot,
                        tick,
                        sample((tick + source_slot) % 128)
                    )
                )
            end
            network_conditions.poll(conditions, tick)
            if tick % 600 == 0 then
                local diagnostics = network_conditions.diagnostics(conditions)
                t.is_true(
                    diagnostics.retained_authoritative_records
                        <= remote_slots * network_conditions.RETAINED_RECORDS
                )
                t.is_true(
                    diagnostics.delivered_ledger_entries
                        <= diagnostics.retained_authoritative_records
                            + diagnostics.pending_record_references
                )
            end
        end

        local diagnostics = network_conditions.diagnostics(conditions)
        t.eq(diagnostics.retained_authoritative_records, 49)
        t.eq(diagnostics.delivered_ledger_entries, 49)
        t.eq(diagnostics.pending_envelopes, 0)
        t.eq(diagnostics.pending_record_references, 0)
        t.eq(diagnostics.peak_retained_authoritative_records, 49)
        t.eq(diagnostics.peak_delivered_ledger_entries, 49)
        t.eq(diagnostics.peak_pending_envelopes, remote_slots)
        t.is_true(diagnostics.peak_pending_record_references >= remote_slots)
        t.eq(network_conditions.counters(conditions).sent, 50407)
    end)
end)
