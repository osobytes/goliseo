//! Port of `sim/slot_input.lua`.
//!
//! Fixed-slot input production. Producers materialize complete effective
//! `InputFrame`s before `sim::match` consumes them; the simulation never
//! knows whether a row began as a recording, a neutral fill, or a
//! deterministic bot.

use crate::combat_intent::CombatDecisionReason;
use crate::combat_intent::{self, CombatIntentState};
use crate::combat_observation;
use crate::combat_policy::{self, CombatUnavailableReason, PolicyAction};
use crate::combat_snapshot::CombatMatchState;
use crate::input_frame::{self, EdgeAction, HeldAction, InputFrame, InputSample};
use crate::match_snapshot::{MatchInput, MatchState};
use gc_core::rng;
use gc_core::vec2::Vec2;
use indexmap::IndexMap;

/// Where one canonical slot's effective input comes from this tick.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MatchSlotSourceKind {
    /// Consume the corresponding sample from the supplied `InputFrame`.
    Frame,
    /// Materialize a deterministic bot fill.
    Bot,
    /// Always the neutral sample.
    Neutral,
}

/// One canonical slot's source declaration.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MatchSlotSource {
    /// Which kind of source this slot uses.
    pub kind: MatchSlotSourceKind,
    /// Canonical Park-Miller seed; required only for [`MatchSlotSourceKind::Bot`].
    pub seed: Option<f64>,
}

/// One declared bot fill's retained state.
#[derive(Clone, Debug, PartialEq)]
pub struct SlotBotState {
    /// This slot's dedicated movement/action RNG stream state.
    pub rng: u32,
    /// Producer-side combat intent; never enters the tape.
    pub intent: CombatIntentState,
    /// Last combat decision this fill made, for diagnostics.
    pub decision: Option<SlotBotDecision>,
}

/// One AI combat decision, reported OUT of the producer rather than into
/// the simulation.
///
/// In slot mode every outfielder is slot-covered, so the `combat_commit_*`
/// `MatchEvent` that `sim::match` emits for gameplay-AI outfielders is a
/// structural no-op online — and slot fills are the dominant online
/// population. Without this, a bot fill's combat commits would be
/// diagnostically invisible in the primary online mode.
///
/// It is a RETURN VALUE, not a `MatchEvent`, and deliberately so. In slot
/// mode the decision happens in the producer, before the boundary;
/// appending it to `state.events` would put producer state into a hashed
/// simulation field, so two peers would diverge on whether their caller
/// opted into the diagnostic. The materialized tape stays the only replay
/// authority.
#[derive(Clone, Debug, PartialEq)]
pub struct SlotBotDecision {
    /// Canonical input slot.
    pub slot: i64,
    /// Canonical player index.
    pub player_index: i64,
    /// The decision's action.
    pub action: PolicyAction,
    /// The decision's stable reason.
    pub reason: CombatDecisionReason,
    /// The decision's target, when it committed.
    pub target_player: Option<i64>,
    /// Why the decision was unavailable, when applicable.
    pub unavailable_reason: Option<CombatUnavailableReason>,
    /// The `combat_sim_observation/v1` digest the decision was made on.
    pub digest: String,
}

/// One producer's complete retained state across every canonical slot.
#[derive(Clone, Debug, PartialEq)]
pub struct SlotInputProducerState {
    /// Canonical `InputFrame` slot order.
    pub sources: [MatchSlotSource; 8],
    /// Retained state for every [`MatchSlotSourceKind::Bot`] slot, keyed by
    /// canonical slot index (one-based).
    pub bots: IndexMap<i64, SlotBotState>,
}

/// The all-neutral `MatchInput`: no movement, nothing held, no edges.
/// `aerial_strike`/`aerial_acrobatic` are explicit `Some(false)`, not
/// `None` — this is a fully materialized "nothing is happening" row, not an
/// absent/hand-built input, mirroring `sim/slot_input.lua`'s
/// `neutral_match_input`, which writes `aerial_strike = false` literally.
#[must_use]
pub fn neutral_match_input() -> MatchInput {
    MatchInput {
        aerial_strike: Some(false),
        aerial_acrobatic: Some(false),
        ..MatchInput::default()
    }
}

/// Dequantize a canonical `InputSample` into a `MatchInput`.
///
/// # Panics
///
/// Panics if `sample` is not a legal, already-validated sample.
#[must_use]
pub fn to_match_input(sample: &InputSample) -> MatchInput {
    let move_x = input_frame::dequantize_axis(sample.move_x).expect("sample is validated");
    let move_y = input_frame::dequantize_axis(sample.move_y).expect("sample is validated");
    let held =
        |action: HeldAction| input_frame::is_held(sample, action).expect("sample is validated");
    let edge =
        |action: EdgeAction| input_frame::has_edge(sample, action).expect("sample is validated");
    MatchInput {
        r#move: Vec2::new(move_x, move_y),
        shoot: edge(EdgeAction::Shoot),
        shoot_held: held(HeldAction::Shoot),
        pass: edge(EdgeAction::Pass),
        pass_held: held(HeldAction::Pass),
        switch: edge(EdgeAction::Switch),
        dash: edge(EdgeAction::Dash),
        dodge: edge(EdgeAction::Dodge),
        lob: held(HeldAction::Lob),
        sprint: held(HeldAction::Sprint),
        jockey: held(HeldAction::Jockey),
        // A dequantized sample always yields a definite bit, never "unset" —
        // mirrors the Lua original's `== true` coercion (`nil`/`false` on
        // the sample both become the real boolean `false` here).
        aerial_strike: Some(held(HeldAction::AerialStrike)),
        aerial_acrobatic: Some(held(HeldAction::AerialAcrobatic)),
        equipment_held: held(HeldAction::Equipment),
        equipment_pressed: edge(EdgeAction::EquipmentPressed),
        equipment_released: edge(EdgeAction::EquipmentReleased),
    }
}

/// Quantize a `MatchInput` into a canonical `InputSample`.
///
/// # Panics
///
/// Panics if `input.move` is out of the finite quantizable range.
#[must_use]
pub fn to_sample(input: &MatchInput) -> InputSample {
    let (move_x, move_y) =
        input_frame::quantize_move(input.r#move.x, input.r#move.y).expect("move axes are finite");
    let mut held = 0;
    let mut edges = 0;
    let mut held_bit = |enabled: bool, action: HeldAction| {
        if enabled {
            held += action.bit();
        }
    };
    held_bit(input.shoot_held, HeldAction::Shoot);
    held_bit(input.pass_held, HeldAction::Pass);
    held_bit(input.sprint, HeldAction::Sprint);
    held_bit(input.jockey, HeldAction::Jockey);
    held_bit(input.lob, HeldAction::Lob);
    // Mirrors the Lua original's `input.aerial_strike == true`: both `None`
    // (unset) and `Some(false)` (explicit no) quantize to the same "not
    // held" bit — the nil-vs-false distinction only matters to
    // `aerial::strike_requested`'s fallback, not to the wire.
    held_bit(input.aerial_strike == Some(true), HeldAction::AerialStrike);
    held_bit(
        input.aerial_acrobatic == Some(true),
        HeldAction::AerialAcrobatic,
    );
    held_bit(input.equipment_held, HeldAction::Equipment);
    let mut edge_bit = |enabled: bool, action: EdgeAction| {
        if enabled {
            edges += action.bit();
        }
    };
    edge_bit(input.shoot, EdgeAction::Shoot);
    edge_bit(input.pass, EdgeAction::Pass);
    edge_bit(input.switch, EdgeAction::Switch);
    edge_bit(input.dash, EdgeAction::Dash);
    edge_bit(input.dodge, EdgeAction::Dodge);
    edge_bit(input.equipment_pressed, EdgeAction::EquipmentPressed);
    edge_bit(input.equipment_released, EdgeAction::EquipmentReleased);
    input_frame::new_sample(input_frame::InputSampleOptions {
        move_x: Some(move_x),
        move_y: Some(move_y),
        held: Some(held),
        edges: Some(edges),
    })
    .expect("held/edge masks built from action bits are always canonical")
}

/// Build a fresh producer for a full set of eight canonical slot sources.
///
/// # Panics
///
/// Panics if a `Bot` source's seed is missing or not finite, or a non-`Bot`
/// source carries a seed.
#[must_use]
pub fn new_producer(sources: [MatchSlotSource; 8]) -> SlotInputProducerState {
    let mut bots = IndexMap::new();
    for (zero_index, source) in sources.iter().enumerate() {
        match source.kind {
            MatchSlotSourceKind::Bot => {
                let seed = source.seed.expect("bot slot source needs a seed");
                assert!(seed.is_finite(), "bot slot source needs a finite seed");
                bots.insert(
                    zero_index as i64 + 1,
                    SlotBotState {
                        rng: rng::seed(seed),
                        intent: combat_intent::new_state(),
                        decision: None,
                    },
                );
            }
            _ => assert!(
                source.seed.is_none(),
                "only bot slot sources may have a seed"
            ),
        }
    }
    let mut copied = sources;
    for source in &mut copied {
        if let MatchSlotSourceKind::Bot = source.kind {
            source.seed = Some(f64::from(rng::seed(source.seed.expect("checked above"))));
        }
    }
    SlotInputProducerState {
        sources: copied,
        bots,
    }
}

/// The declared fixed-slot bot fill runs the SAME `gameplay_ai/combat/v1`
/// policy, the same observation schema, the same option ordering, and the
/// same reason vocabulary as the gameplay match AI. Its retained intent
/// lives here, on the producer, not in `MatchState`: a materialized tape is
/// the replay authority, and inserting producer policy state into the
/// simulation would make a replay without this producer diverge.
///
/// `sim::bot` is a different population and deliberately does not get
/// this. It is the human-proxy evidence driver, and it may only gain combat
/// capability behind its own explicit policy id.
fn bot_combat_signals(
    state: &MatchState,
    combat_state: Option<&CombatMatchState>,
    player_idx: i64,
    bot_state: &mut SlotBotState,
    slot: i64,
) -> (
    Option<combat_intent::CombatIntentSignals>,
    Option<SlotBotDecision>,
) {
    let Some(combat_state) = combat_state else {
        return (None, None);
    };
    let Some(runtime) = combat_state.players.get(player_idx as usize - 1) else {
        return (None, None);
    };
    let player = &state.players[player_idx as usize - 1];
    if player.is_keeper {
        return (None, None);
    }
    if runtime.forced_ticks > 0 && bot_state.intent.stage != combat_intent::CombatIntentStage::Idle
    {
        bot_state.intent = combat_intent::reset(&bot_state.intent);
    }
    let (signals, next_intent) = combat_intent::materialize(&bot_state.intent);
    bot_state.intent = next_intent;
    if let Some(signals) = signals {
        return (Some(signals), None);
    }
    if runtime.family_id.is_none()
        || runtime.phase != crate::combat_feasibility::CombatActionPhase::Ready
        || runtime.forced_ticks > 0
        || runtime.cooldown_ticks > 0
        || state.kickoff_hold > 0.0
        || state.finished
        || !combat_intent::should_decide(combat_state.tick, player_idx, player.scan_rate)
    {
        return (None, None);
    }
    let observation = combat_observation::build(
        state,
        Some(combat_state),
        player_idx,
        combat_policy::POLICY_ID,
        None,
    );
    let temperature = if player.composure >= combat_policy::SHARP_COMPOSURE {
        0.0
    } else {
        combat_policy::BASE_TEMPERATURE * (1.0 - player.composure)
    };
    let decision = combat_policy::decide(
        &observation,
        combat_intent::decision_seed(combat_state.tick, player_idx),
        Some(temperature),
    );
    let record = SlotBotDecision {
        slot,
        player_index: player_idx,
        action: decision.action,
        reason: decision.reason,
        target_player: decision.target_player,
        unavailable_reason: decision.unavailable_reason,
        digest: decision.digest.clone(),
    };
    bot_state.decision = Some(record.clone());
    if decision.action != PolicyAction::Commit {
        let decline_action = match decision.action {
            PolicyAction::Unavailable => Some(combat_intent::DeclineAction::Unavailable),
            _ => Some(combat_intent::DeclineAction::Decline),
        };
        bot_state.intent = combat_intent::decline(&bot_state.intent, decline_action);
        return (None, Some(record));
    }
    assert!(
        !decision.context_violation,
        "representative_policy_context_violation: gameplay_ai/combat/v1 committed without a purpose context"
    );
    bot_state.intent = combat_intent::commit(
        &bot_state.intent,
        decision.reason,
        decision
            .target_player
            .expect("a commit always names a target"),
        combat_policy::hold_ticks(&observation, &decision),
    );
    (Some(combat_intent::commit_signals()), Some(record))
}

fn bot_input(
    state: &MatchState,
    combat_state: Option<&CombatMatchState>,
    player_idx: i64,
    bot_state: &mut SlotBotState,
    slot: i64,
) -> (MatchInput, Option<SlotBotDecision>) {
    let player = &state.players[player_idx as usize - 1];
    let target;
    let mut dash = false;
    let mut shoot = false;
    let sprint;
    let roll;
    (bot_state.rng, roll) = rng::roll(bot_state.rng);

    if state.owner == Some(player_idx) {
        target = Vec2::new(
            if player.team == crate::match_snapshot::Team::Home {
                state.field.w
            } else {
                0.0
            },
            state.field.h / 2.0,
        );
        let goal_distance = player.pos.dist(target);
        shoot = goal_distance < 180.0 && roll < 0.12;
        sprint = !shoot;
    } else if state.owner.is_none() {
        target = state.ball;
        sprint = player.pos.dist(target) > 100.0;
    } else if state.players[state.owner.expect("checked above") as usize - 1].team != player.team {
        target = state.ball;
        let distance = player.pos.dist(target);
        dash = distance < 34.0 && roll < 0.20;
        sprint = distance > 100.0;
    } else {
        target = player.anchor;
        sprint = false;
    }

    let delta = target.sub(player.pos);
    let r#move = if delta.length() > 1.0 {
        delta.normalized()
    } else {
        Vec2::new(0.0, 0.0)
    };
    let (equipment, decision) =
        bot_combat_signals(state, combat_state, player_idx, bot_state, slot);
    (
        MatchInput {
            r#move,
            shoot,
            shoot_held: false,
            pass: false,
            pass_held: false,
            switch: false,
            dash,
            dodge: false,
            lob: false,
            sprint,
            jockey: false,
            // Deliberate, always-definite intent (never "unset") — mirrors
            // `sim/slot_input.lua`'s bot fill, which writes a real boolean
            // here, not `nil`.
            aerial_strike: Some(state.ball_z > 18.0 && player.pos.dist(state.ball) < 72.0),
            aerial_acrobatic: Some(false),
            equipment_held: equipment.is_some_and(|s| s.equipment_held),
            equipment_pressed: equipment.is_some_and(|s| s.equipment_pressed),
            equipment_released: equipment.is_some_and(|s| s.equipment_released),
        },
        decision,
    )
}

/// Produce the complete effective frame consumed by `sim::match`. Bot
/// decisions are quantized through [`to_sample`], so the result can be
/// saved and replayed later without this producer or its RNG state.
///
/// # Panics
///
/// Panics if `state` is not in slot mode, `frame` is invalid, `frame.tick`
/// does not match `state.input_tick`, or the slot mapping is incomplete.
pub fn materialize(
    producer: &mut SlotInputProducerState,
    state: &MatchState,
    frame: &InputFrame,
    combat_state: Option<&CombatMatchState>,
) -> (InputFrame, Vec<SlotBotDecision>) {
    assert!(state.slot_mode, "slot producer requires a slot-mode match");
    input_frame::validate(frame).expect("slot producer requires a valid InputFrame");
    assert_eq!(
        frame.tick, state.input_tick,
        "input frame tick does not match match state"
    );
    let mut slots = [InputSample::default(); 8];
    let mut decisions = Vec::new();
    for index in 1..=input_frame::SLOT_COUNT {
        let player_idx =
            state.slot_players[index as usize - 1].expect("slot mapping is incomplete");
        let source = producer.sources[index as usize - 1];
        match source.kind {
            MatchSlotSourceKind::Frame => {
                slots[index as usize - 1] = frame.slots[index as usize - 1];
            }
            MatchSlotSourceKind::Bot => {
                let bot_state = producer
                    .bots
                    .get_mut(&index)
                    .expect("bot source has retained state");
                let (input, decision) =
                    bot_input(state, combat_state, player_idx, bot_state, index);
                slots[index as usize - 1] = to_sample(&input);
                if let Some(decision) = decision {
                    decisions.push(decision);
                }
            }
            MatchSlotSourceKind::Neutral => {
                slots[index as usize - 1] = input_frame::neutral_sample();
            }
        }
    }
    (
        input_frame::new(frame.tick, Some(slots)).expect("canonical slots always validate"),
        decisions,
    )
}

/// The last combat decision a declared bot fill made on `slot`, or `None`
/// if it has not decided yet. [`materialize`] returns the same records for
/// the tick it just produced; this is for a caller that wants to read one
/// back later.
#[must_use]
pub fn last_decision(producer: &SlotInputProducerState, slot: i64) -> Option<SlotBotDecision> {
    producer.bots.get(&slot).and_then(|b| b.decision.clone())
}
