//! Port of `game/online/match_manifest.lua`.
//!
//! Content-derived session manifest for the online match flow: every
//! identity is derived from shipped content and `sim.tuning`, so two peers
//! running the same build compute the same manifest byte for byte.
//! `template` is what the lobby proposes; `resolve` is what the match flow
//! reads back out of the frozen manifest.
//!
//! ## Content is a parameter, not an import
//!
//! The Lua original reaches directly into `data.arenas`, `data.loadouts`,
//! `data.players`, `data.teams`, and `game.build_info`. This agent's brief
//! places `crates/gc-netcode` on `gc-core`/`gc-sim` only ("Add no crate
//! dependencies" — no `gc-data`, which every one of those tables lives in),
//! and lists this file's `gc-sim` needs as exactly `combat_policy`,
//! `fixed_clock`, `input_frame`, `r#match`, `tuning` — `gc-data` deliberately
//! absent. So content this module cannot look up itself, it takes as an
//! explicit parameter instead: [`TeamContent`], [`BuildIdentity`]. This is
//! the same shape README rule 6.7 prescribes for TypeScript ("TypeScript
//! never imports content tables — it receives them") for the identical
//! reason — a package with no content-table dependency has to receive
//! content rather than look it up — applied here to a Rust crate under the
//! same constraint rather than a language rule.
//!
//! One consequence: [`ownership_for_teams`] (this module's local
//! `canonical_ownership`) does not call `gc_sim::r#match::ownership_for_teams`,
//! because that function's signature takes `&gc_data::teams::TeamData` /
//! `Option<&IndexMap<&str, gc_data::players::PlayerData>>` and this module
//! never holds those types. The algorithm it runs is mechanical and
//! content-free — walk each side's outfield roster order (roster minus its
//! first, keeper, entry) onto the eight canonical slots in
//! `input_frame::slot(1..=8)` order — so it is reproduced locally rather than
//! left unported; see `canonical_ownership`'s doc comment.
//!
//! No dedicated Lua spec exists for this module
//! (`spec/game/online_match_manifest_spec.lua` is absent), so there is no
//! spec to port here; its ids and shapes are exercised indirectly wherever a
//! caller builds a manifest.

use gc_core::fnv1a64;
use gc_sim::{combat_policy, fixed_clock, input_frame, r#match as sim_match, tuning};

use crate::protocol::{self, MatchMode, Value};

/// This build's home team content id (`data.teams["nebula"]`).
pub const HOME_TEAM_ID: &str = "nebula";
/// This build's away team content id (`data.teams["orion"]`).
pub const AWAY_TEAM_ID: &str = "orion";
/// This build's arena content id (`data.arenas["helios_crown"]`).
pub const ARENA_ID: &str = "helios_crown";
/// The default session id a freshly proposed manifest uses.
pub const DEFAULT_SESSION_ID: &str = "session_goliseo";
/// The default seed a freshly proposed manifest uses.
pub const DEFAULT_SEED: i64 = 20003;
/// 7,200 ticks is 120 seconds at `fixed_clock::TICK_RATE` — `sim.match`'s
/// default, the OMP-1 determinism fixture, and `protocol_fixture` all agree
/// on this one match length.
pub const DEFAULT_DURATION_TICKS: i64 = 7200;
/// There is no goal limit: an online match is decided on score at full time,
/// exactly like an offline one. `sim_match::NO_GOAL_LIMIT` and
/// `protocol::MAX_GOALS` agree this is 99, unreachable inside 7,200 ticks.
pub const DEFAULT_MAX_GOALS: i64 = sim_match::NO_GOAL_LIMIT;
/// The combat disposition is still provisional: #114 has not accepted a
/// default, so the manifest says so rather than pretending.
pub const COMBAT_STATUS: &str = "provisional_114";
/// This build's accepted combat interaction rules id.
pub const COMBAT_RULES_ID: &str = "combat_interaction.accepted_2026_07_23";
/// #112's shipped deterministic gameplay combat policy id
/// (`combat_policy::MANIFEST_POLICY_ID`).
pub const GAMEPLAY_AI_POLICY_ID: &str = combat_policy::MANIFEST_POLICY_ID;
/// This build's match configuration id.
pub const MATCH_CONFIG_ID: &str = "match_config.direct_host.v1";
/// This build's fixture id.
pub const FIXTURE_ID: &str = "fixture.content_derived.v1";

/// A closed roster position — the same four `data.players` supports.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlayerPosition {
    /// Goalkeeper. Slotless and AI-only in every online mode.
    Keeper,
    /// Defender.
    Defender,
    /// Midfielder.
    Midfielder,
    /// Forward.
    Forward,
}

impl PlayerPosition {
    /// The exact Lua wire string, matching `protocol`'s `POSITIONS` set.
    #[must_use]
    pub fn wire_str(self) -> &'static str {
        match self {
            PlayerPosition::Keeper => "keeper",
            PlayerPosition::Defender => "defender",
            PlayerPosition::Midfielder => "midfielder",
            PlayerPosition::Forward => "forward",
        }
    }
}

/// One roster player's content-derived identity fields — the subset of
/// `data.players`/`data.loadouts` that `content_id`/`template` read. See the
/// module doc comment for why this is a parameter rather than a `data`
/// lookup.
#[derive(Clone, Debug, PartialEq)]
pub struct RosterPlayerContent {
    /// `PlayerData.id`.
    pub player_id: String,
    /// `PlayerData.position`.
    pub position: PlayerPosition,
    /// `PlayerData.number`.
    pub number: i64,
    /// `PlayerData.presentation_id`.
    pub presentation_id: String,
    /// `PlayerData.loadout_id`; keepers never carry one.
    pub loadout_id: Option<String>,
    /// The resolved `loadouts[loadout_id].family_id`; `Some` exactly when
    /// `loadout_id` is `Some`.
    pub loadout_family_id: Option<String>,
}

/// One team's content-derived identity: `data.teams[id]` plus its resolved
/// roster. `roster` is authored keeper-first, four outfielders after — the
/// same order `data/teams.lua` ships and `team_manifest`/`canonical_ownership`
/// both rely on.
#[derive(Clone, Debug, PartialEq)]
pub struct TeamContent {
    /// `TeamData.id`.
    pub id: String,
    /// `TeamData.name`.
    pub name: String,
    /// `TeamData.formation`.
    pub formation: String,
    /// Exactly [`input_frame::FIXTURE_TEAM_SIZE`] players, keeper first.
    pub roster: Vec<RosterPlayerContent>,
}

/// The parts of `build_info` a manifest's `build_id` digests. See the module
/// doc comment: `game/build_info.lua` is a `game/` root file
/// (TypeScript-owned per `v2/README.md` §2's table), so this module receives
/// it rather than importing it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BuildIdentity {
    /// `build_info.name`.
    pub name: String,
    /// `build_info.version`.
    pub version: String,
    /// `build_info.channel`.
    pub channel: String,
}

/// `OnlineMatchContent`: a frozen manifest resolved back into playable
/// content. `ownership` is `(canonical slot wire id, team wire id, player id)`
/// in OMP-1 order — see the module doc comment for why this is a plain `Vec`
/// rather than `gc_sim::input_frame::InputOwnership` (that type's
/// constructors need `gc_data::players::PlayerData`, which this module does
/// not have).
#[derive(Clone, Debug, PartialEq)]
pub struct OnlineMatchContent {
    /// The home side's content.
    pub home: TeamContent,
    /// The away side's content.
    pub away: TeamContent,
    /// The resolved arena id.
    pub arena_id: String,
    /// Canonical slot ownership, OMP-1 order.
    pub ownership: Vec<(String, String, String)>,
}

fn digest(value: &str) -> String {
    let hash = fnv1a64::hash(value.as_bytes());
    hash.chars().take(16).collect()
}

fn team_content_digest_input(team: &TeamContent) -> String {
    let mut parts = vec![team.id.clone(), team.name.clone(), team.formation.clone()];
    for player in &team.roster {
        parts.push(format!(
            "{}/{}/{}/{}/{}/{}",
            player.player_id,
            player.position.wire_str(),
            player.number,
            player.presentation_id,
            player.loadout_id.as_deref().unwrap_or("-"),
            player.loadout_family_id.as_deref().unwrap_or("-"),
        ));
    }
    parts.join(";")
}

/// `match_manifest.content_id`: one digest over exactly the content the
/// simulation reads — both rosters, their loadouts, and the arena. A content
/// edit therefore mints a new `content_id`, which is what a guest compares
/// its build against.
#[must_use]
pub fn content_id(home: &TeamContent, away: &TeamContent, arena_id: &str) -> String {
    let joined = [
        team_content_digest_input(home),
        team_content_digest_input(away),
        arena_id.to_string(),
    ]
    .join("|");
    format!("content.{}", digest(&joined))
}

/// `match_manifest.tuning_id`: tuning is already serialisable for the OMP-1
/// determinism fixture, so the identity is the same string that campaign
/// pins rather than a second one.
#[must_use]
pub fn tuning_id() -> String {
    format!("tuning.{}", digest(&tuning::Tuning::default().serialize()))
}

/// `match_manifest.build_id`: folds `protocol::vocabulary_id()` into the
/// build identity so "same `build_id`" implies "same control vocabulary" —
/// see the Lua original's doc comment for why a vocabulary mismatch has to
/// surface at the manifest check rather than mid-lobby as a generic
/// `protocol_violation`.
#[must_use]
pub fn build_id(identity: &BuildIdentity) -> String {
    let joined = [
        identity.name.as_str(),
        identity.version.as_str(),
        identity.channel.as_str(),
        protocol::vocabulary_id().as_str(),
    ]
    .join("/");
    format!("build.{}", digest(&joined))
}

/// # Panics
///
/// Panics if a non-keeper carries no `loadout_id`/`loadout_family_id` —
/// mirrors the Lua original's `assert(player.loadout_id, "an online
/// outfielder needs a fixed loadout: " .. id)`: authored content, not
/// external input, so a broken invariant here is a programmer error
/// (AGENTS.md §7).
fn roster_player_value(player: &RosterPlayerContent) -> Value {
    if player.position == PlayerPosition::Keeper {
        Value::record(vec![
            ("player_id", Value::str(player.player_id.clone())),
            ("position", Value::str(player.position.wire_str())),
        ])
    } else {
        let loadout_id = player.loadout_id.clone().unwrap_or_else(|| {
            panic!(
                "an online outfielder needs a fixed loadout: {}",
                player.player_id
            )
        });
        let family_id = player.loadout_family_id.clone().unwrap_or_else(|| {
            panic!(
                "an online outfielder's loadout needs a family id: {}",
                player.player_id
            )
        });
        Value::record(vec![
            ("player_id", Value::str(player.player_id.clone())),
            ("position", Value::str(player.position.wire_str())),
            ("loadout_id", Value::str(loadout_id)),
            ("family_id", Value::str(family_id)),
        ])
    }
}

fn team_manifest_value(team: &TeamContent, side: input_frame::Team) -> Value {
    let roster: Vec<Value> = team.roster.iter().map(roster_player_value).collect();
    Value::record(vec![
        ("team", Value::str(protocol::team_wire_str(side))),
        ("team_id", Value::str(team.id.clone())),
        ("roster", Value::array(roster)),
    ])
}

/// The canonical slot ownership two rosters imply: each side's outfield
/// roster order (roster minus its keeper, entry 1) maps onto the four
/// canonical slots for that side, in `input_frame::slot` order.
///
/// This mirrors `gc_sim::r#match::ownership_for_teams`'s algorithm exactly
/// (walk `team.roster`, skip the keeper, assign remaining ids to
/// `slot.outfield_index` in order) without calling it — see the module doc
/// comment for why: that function needs `gc_data` types this module does not
/// depend on, but the algorithm itself reads no content beyond the roster
/// order already in `TeamContent`.
///
/// # Panics
///
/// Panics if either roster is not exactly
/// [`input_frame::FIXTURE_TEAM_SIZE`] players with the keeper first —
/// mirrors `gc_sim::r#match::ownership_for_teams`'s own panics, a programmer
/// error in the supplied content rather than external input.
#[must_use]
pub fn canonical_ownership(
    home: &TeamContent,
    away: &TeamContent,
) -> Vec<(String, String, String)> {
    let outfield = |team: &TeamContent, label: &str| -> Vec<String> {
        assert!(
            team.roster.len() as i64 == input_frame::FIXTURE_TEAM_SIZE,
            "{label} roster must have exactly {} players",
            input_frame::FIXTURE_TEAM_SIZE
        );
        assert!(
            team.roster[0].position == PlayerPosition::Keeper,
            "{label} roster must place its keeper first"
        );
        team.roster[1..]
            .iter()
            .map(|p| p.player_id.clone())
            .collect()
    };
    let home_outfield = outfield(home, "home");
    let away_outfield = outfield(away, "away");
    assert!(
        home_outfield.len() as i64 == input_frame::HOME_SLOT_COUNT
            && away_outfield.len() as i64 == input_frame::AWAY_SLOT_COUNT,
        "each side must have four outfielders"
    );
    (1..=input_frame::SLOT_COUNT)
        .map(|index| {
            let slot = input_frame::slot(index).unwrap();
            let ids = if slot.team == input_frame::Team::Home {
                &home_outfield
            } else {
                &away_outfield
            };
            let player_id = ids[(slot.outfield_index - 1) as usize].clone();
            (
                protocol::slot_wire_id(slot.id).to_string(),
                protocol::team_wire_str(slot.team).to_string(),
                player_id,
            )
        })
        .collect()
}

/// `match_manifest.template`: the canonical manifest this build proposes.
/// `mode` only changes how the eight slots are shared; the content, the
/// slots themselves, and every identity are identical across modes.
#[must_use]
pub fn template(
    home: &TeamContent,
    away: &TeamContent,
    arena_id: &str,
    mode: Option<MatchMode>,
    identity: &BuildIdentity,
) -> Value {
    let ownership = canonical_ownership(home, away);
    let slots: Vec<Value> = ownership
        .iter()
        .map(|(slot, team, player_id)| {
            Value::record(vec![
                ("slot", Value::str(slot.clone())),
                ("team", Value::str(team.clone())),
                ("player_id", Value::str(player_id.clone())),
            ])
        })
        .collect();
    let built_id = build_id(identity);
    Value::record(vec![
        ("version", Value::int(protocol::MANIFEST_VERSION)),
        ("session_id", Value::str(DEFAULT_SESSION_ID)),
        ("protocol_version", Value::int(protocol::VERSION)),
        (
            "input_version",
            Value::int(protocol::CURRENT_VERSIONS.input),
        ),
        (
            "snapshot_version",
            Value::int(protocol::CURRENT_VERSIONS.snapshot),
        ),
        ("tape_version", Value::int(protocol::CURRENT_VERSIONS.tape)),
        (
            "combat_schema_version",
            Value::int(protocol::CURRENT_VERSIONS.combat),
        ),
        ("build_id", Value::str(built_id.clone())),
        ("source_id", Value::str(built_id)),
        ("content_id", Value::str(content_id(home, away, arena_id))),
        ("tuning_id", Value::str(tuning_id())),
        ("match_config_id", Value::str(MATCH_CONFIG_ID)),
        ("fixture_id", Value::str(FIXTURE_ID)),
        ("arena_id", Value::str(arena_id)),
        ("combat_rules_id", Value::str(COMBAT_RULES_ID)),
        ("gameplay_ai_policy_id", Value::str(GAMEPLAY_AI_POLICY_ID)),
        ("combat_status", Value::str(COMBAT_STATUS)),
        ("seed", Value::int(DEFAULT_SEED)),
        ("tick_rate", Value::int(fixed_clock::TICK_RATE as i64)),
        ("duration_ticks", Value::int(DEFAULT_DURATION_TICKS)),
        ("max_goals", Value::int(DEFAULT_MAX_GOALS)),
        (
            "match_mode",
            Value::str(mode.unwrap_or(MatchMode::FourVFour).wire_str()),
        ),
        (
            "teams",
            Value::array(vec![
                team_manifest_value(home, input_frame::Team::Home),
                team_manifest_value(away, input_frame::Team::Away),
            ]),
        ),
        ("slots", Value::array(slots)),
    ])
}

/// `match_manifest.resolve`: turns a frozen manifest back into the content
/// the simulation needs. `home`/`away` must already be the content the
/// manifest's `teams[].team_id` name — the module doc comment explains why
/// this module cannot look that up itself. Every field is checked against
/// the manifest's own canonical slot table, so content that disagrees with
/// what the manifest declares fails here rather than producing a quietly
/// different match.
pub fn resolve(
    manifest: &Value,
    home: &TeamContent,
    away: &TeamContent,
    arena_id: &str,
) -> std::result::Result<OnlineMatchContent, String> {
    protocol::validate_manifest(manifest).map_err(|err| err.message)?;
    let manifest_teams = manifest.get("teams").ok_or("manifest has no teams")?;
    let home_team_id = manifest_teams
        .get_index(1)
        .and_then(|t| t.get("team_id"))
        .and_then(Value::as_str);
    let away_team_id = manifest_teams
        .get_index(2)
        .and_then(|t| t.get("team_id"))
        .and_then(Value::as_str);
    if home_team_id != Some(home.id.as_str()) || away_team_id != Some(away.id.as_str()) {
        return Err("the manifest names a team this build does not have".to_string());
    }
    if manifest.get("arena_id").and_then(Value::as_str) != Some(arena_id) {
        return Err("the manifest names an arena this build does not have".to_string());
    }
    for (side_index, team) in [home, away].into_iter().enumerate() {
        let manifest_team = manifest_teams.get_index(side_index as i64 + 1).unwrap();
        let manifest_roster = manifest_team
            .get("roster")
            .ok_or("manifest team has no roster")?;
        for (index, player) in team.roster.iter().enumerate() {
            let declared = manifest_roster
                .get_index(index as i64 + 1)
                .ok_or("manifest roster is short")?;
            if declared.get("player_id").and_then(Value::as_str) != Some(player.player_id.as_str())
            {
                return Err("the manifest names a player this build does not have".to_string());
            }
            if declared.get("position").and_then(Value::as_str) != Some(player.position.wire_str())
            {
                return Err("the manifest disagrees about a player's position".to_string());
            }
            let declared_loadout = declared.get("loadout_id").and_then(Value::as_str);
            if let Some(declared_loadout) = declared_loadout
                && player.loadout_id.as_deref() != Some(declared_loadout)
            {
                return Err("the manifest disagrees about a player's fixed loadout".to_string());
            }
        }
    }
    let ownership = canonical_ownership(home, away);
    let manifest_slots = manifest.get("slots").ok_or("manifest has no slots")?;
    for (index, (slot, _, player_id)) in ownership.iter().enumerate() {
        let declared = manifest_slots
            .get_index(index as i64 + 1)
            .ok_or("manifest slots are short")?;
        if declared.get("player_id").and_then(Value::as_str) != Some(player_id.as_str()) {
            return Err(format!(
                "the manifest disagrees about canonical slot {slot}"
            ));
        }
    }
    Ok(OnlineMatchContent {
        home: home.clone(),
        away: away.clone(),
        arena_id: arena_id.to_string(),
        ownership,
    })
}
