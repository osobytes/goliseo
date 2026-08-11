//! Port of `render/player_pose.lua`.
//!
//! Selects the pose id shown for a player. This lives on the determinism-free
//! side of the boundary content-wise (it never changes what the simulation
//! computes), but it is Rust rather than TypeScript so two clients cannot
//! disagree about which pose is displayed — see `v2/README.md`'s note on
//! `player_pose.lua` in the module-owning agent's brief.
//!
//! One authority owns overlap precedence: [`PlayerPoseId::priority`]. Outfield
//! presentation work extends this ordered contract instead of adding
//! renderer-local condition chains.

use gc_sim::aerial::AerialStyle;
use gc_sim::combat_feasibility::CombatActionPhase;
use gc_sim::combat_snapshot::CombatForcedState;
use gc_sim::keeper::{KeeperBehaviorState, SaveStyle};
use gc_sim::r#match::SPRINT_ENGAGE_FRAC;
use gc_sim::match_snapshot::MatchPlayer;
use gc_sim::offball_runs;
use gc_sim::outfield_decision;

/// The closed set of pose ids a renderer can be asked to show.
///
/// Ordered here exactly as `render/player_pose.lua`'s `---@alias
/// PlayerPoseId` lists them; that ordering carries no meaning of its own
/// (priority and tie-break both come from dedicated tables/methods below), it
/// is preserved purely so a reviewer can diff the two files line by line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlayerPoseId {
    /// Keeper gathering the ball into the hands.
    KeeperGrab,
    /// Keeper releasing a throw.
    KeeperThrow,
    /// Keeper releasing a punt.
    KeeperPunt,
    /// Keeper tipping a shot away.
    KeeperTip,
    /// Keeper diving with a spread save.
    KeeperSpread,
    /// Keeper diving with a central save.
    KeeperCentral,
    /// Keeper diving with a full-stretch save.
    KeeperStretch,
    /// Keeper diving with no more specific save style recorded.
    KeeperDive,
    /// Keeper pushing back up after a dive.
    KeeperGetUp,
    /// Keeper set for an anticipated shot.
    KeeperSet,
    /// Keeper's low-ready stance near the ball.
    KeeperReadyLow,
    /// Keeper shuffling along the goal line.
    KeeperShuffle,
    /// Keeper's tall neutral ready stance.
    KeeperReadyTall,
    /// An acrobatic bicycle-kick aerial contact.
    AerialBicycle,
    /// Any other aerial contact (header, volley, control).
    AerialAction,
    /// Forced knockback from a combat hit.
    CombatKnockback,
    /// Forced stagger from a combat hit.
    CombatStagger,
    /// Actively guarding in combat.
    CombatGuard,
    /// A contact-capable combat action.
    CombatActive,
    /// Committed but not yet active combat action.
    CombatWindup,
    /// Charging a held-release combat action.
    CombatAim,
    /// Post-action combat recovery.
    CombatRecovery,
    /// Winding up a soccer shot or punt.
    SoccerWindup,
    /// An active slide tackle.
    Slide,
    /// An active standing tackle.
    Tackle,
    /// Knocked off balance.
    Stumble,
    /// Follow-through after releasing a kick.
    KickFollow,
    /// First-touch settle window.
    Settle,
    /// Telegraphing the opening beat of a granted off-ball run.
    RunTelegraph,
    /// Shepherding stance, AI contain or human jockey.
    Contain,
    /// Sprint tank spent.
    Fatigue,
    /// Ordinary locomotion; the fallback every player always has.
    Locomotion,
}

impl PlayerPoseId {
    /// This pose's overlap precedence: higher wins. Mirrors
    /// `render/player_pose.lua`'s `player_pose.PRIORITY` table exactly,
    /// including the equal `keeper_stretch`/`keeper_dive` values.
    #[must_use]
    pub const fn priority(self) -> i64 {
        match self {
            Self::KeeperGrab => 123,
            Self::KeeperThrow => 122,
            Self::KeeperPunt => 121,
            Self::KeeperTip => 110,
            Self::KeeperSpread => 102,
            Self::KeeperCentral => 101,
            Self::KeeperStretch => 100,
            Self::KeeperDive => 100,
            Self::KeeperGetUp => 79,
            Self::KeeperSet => 78,
            Self::KeeperReadyLow => 15,
            Self::KeeperShuffle => 14,
            Self::KeeperReadyTall => 13,
            Self::AerialBicycle => 95,
            Self::AerialAction => 94,
            Self::CombatKnockback => 90,
            Self::CombatStagger => 89,
            Self::CombatGuard => 84,
            Self::CombatActive => 83,
            Self::CombatWindup => 82,
            Self::CombatAim => 81,
            Self::CombatRecovery => 80,
            Self::SoccerWindup => 70,
            // Outfield ground poses, all below the combat band so a forced
            // state or a committed combat phase keeps presentation
            // precedence. The order reads down from most committed to
            // least: challenges (slide, tackle) outrank the recovery they
            // can cause (stumble), which outranks what the player is doing
            // with the ball (kick_follow, settle), which outranks intent
            // off the ball (run_telegraph, contain) and finally condition
            // (fatigue).
            Self::Slide => 60,
            Self::Tackle => 55,
            Self::Stumble => 50,
            Self::KickFollow => 45,
            Self::Settle => 40,
            Self::RunTelegraph => 35,
            Self::Contain => 30,
            Self::Fatigue => 20,
            Self::Locomotion => 0,
        }
    }

    /// This pose's wire identifier, exactly the Lua alias member's spelling.
    /// The tie-break rule in [`resolve`] compares this string, matching
    /// Lua's `candidate.id < best.id` (both operate on the pose's stable
    /// name — Lua directly as a string, this one through this method).
    #[must_use]
    pub const fn wire_name(self) -> &'static str {
        match self {
            Self::KeeperGrab => "keeper_grab",
            Self::KeeperThrow => "keeper_throw",
            Self::KeeperPunt => "keeper_punt",
            Self::KeeperTip => "keeper_tip",
            Self::KeeperSpread => "keeper_spread",
            Self::KeeperCentral => "keeper_central",
            Self::KeeperStretch => "keeper_stretch",
            Self::KeeperDive => "keeper_dive",
            Self::KeeperGetUp => "keeper_get_up",
            Self::KeeperSet => "keeper_set",
            Self::KeeperReadyLow => "keeper_ready_low",
            Self::KeeperShuffle => "keeper_shuffle",
            Self::KeeperReadyTall => "keeper_ready_tall",
            Self::AerialBicycle => "aerial_bicycle",
            Self::AerialAction => "aerial_action",
            Self::CombatKnockback => "combat_knockback",
            Self::CombatStagger => "combat_stagger",
            Self::CombatGuard => "combat_guard",
            Self::CombatActive => "combat_active",
            Self::CombatWindup => "combat_windup",
            Self::CombatAim => "combat_aim",
            Self::CombatRecovery => "combat_recovery",
            Self::SoccerWindup => "soccer_windup",
            Self::Slide => "slide",
            Self::Tackle => "tackle",
            Self::Stumble => "stumble",
            Self::KickFollow => "kick_follow",
            Self::Settle => "settle",
            Self::RunTelegraph => "run_telegraph",
            Self::Contain => "contain",
            Self::Fatigue => "fatigue",
            Self::Locomotion => "locomotion",
        }
    }
}

/// Which system a selected pose came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlayerPoseSource {
    /// The soccer simulation (kicks, keeper saves, tackles, locomotion, ...).
    Soccer,
    /// The combat simulation.
    Combat,
    /// The locomotion fallback.
    Locomotion,
}

/// One candidate (or the winning) pose, with the precedence it was scored at.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PlayerPoseSelection {
    /// The selected pose.
    pub id: PlayerPoseId,
    /// The priority this candidate was scored at (normally
    /// `id.priority()`; see [`resolve`] for why it is carried explicitly
    /// rather than re-derived).
    pub priority: i64,
    /// Which system this pose came from.
    pub source: PlayerPoseSource,
}

/// The keeper-specific signals the renderer cannot read off [`MatchPlayer`]
/// alone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeeperPoseContext {
    /// The keeper is within smother range of the ball.
    pub near_ball: bool,
    /// The keeper is shuffling along the goal line.
    pub shuffling: bool,
    /// A tip event was recorded for this player this frame.
    pub tip: bool,
}

/// The outfield signals the renderer cannot read off [`MatchPlayer`] alone:
/// the match clock the run telegraph window is measured against, the
/// team-owned press mode, and the render-owned release follow-through. Every
/// field is supplied by the caller; this module never samples a clock or a
/// global.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OutfieldPoseContext {
    /// Seconds since kickoff (`-MatchState.time_left`); `None` if the caller
    /// supplied no clock, in which case a run never telegraphs.
    pub now: Option<f64>,
    /// The team press state holds this player in contain.
    pub containing: bool,
    /// A confirmed release is still following through.
    pub kick_follow: bool,
}

/// The slice of `@gc/presentation`'s `CombatPlayerPresentation` (TS-owned:
/// `game/presentation/combat.lua`) that pose selection reads. Declared
/// locally rather than imported, because presentation crosses the language
/// boundary into TypeScript and this module only ever reads three of its
/// fields.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CombatPoseSample {
    /// The player's current combat action phase.
    pub phase: CombatActionPhase,
    /// The player's forced (stagger/knockback) disable state, if any.
    pub forced_state: Option<CombatForcedState>,
    /// Ticks remaining in the forced state.
    pub forced_ticks: i64,
}

fn add(candidates: &mut Vec<PlayerPoseSelection>, id: PlayerPoseId, source: PlayerPoseSource) {
    candidates.push(PlayerPoseSelection {
        id,
        priority: id.priority(),
        source,
    });
}

// The opening beat of a run the simulation has already granted. The window
// and its edge handling belong to `gc_sim::offball_runs`; presentation only
// asks.
fn run_telegraphing(player: &MatchPlayer, now: Option<f64>) -> bool {
    let Some(now) = now else {
        return false;
    };
    let decision = &player.outfield_decision;
    if !outfield_decision::is_run_intent(decision.intent) {
        return false;
    }
    let Some(expires_at) = decision.run_expires_at else {
        return false;
    };
    offball_runs::telegraphing(now, expires_at)
}

// The sprint tank is spent when the player is not sprinting and cannot
// engage one: the same hysteresis boundary `gc_sim::r#match` uses, so the
// pose never claims a burst is available when it is not.
fn sprint_spent(player: &MatchPlayer) -> bool {
    !player.sprinting && player.sprint_meter <= SPRINT_ENGAGE_FRAC
}

/// Resolve the highest-priority pose from a non-empty candidate list,
/// breaking a tie by the pose id's [`PlayerPoseId::wire_name`] in ascending
/// order. This is [`select`]'s reduction step, factored out and made public
/// on purpose.
///
/// `render/player_pose.lua`'s own spec for this contract
/// (`spec/game/combat_presentation_spec.lua`'s "chooses overlapping poses
/// from declared priority with a stable tie rule") exercises it by mutating
/// the module-global `PRIORITY` table in place: raise `slide`'s priority
/// above `soccer_windup`'s, observe the winner flip, then set them equal and
/// observe the lexical tie-break. Rust has no equivalent of a mutable global
/// enum-keyed table (README rule 4 rules out the map that would back it, and
/// a `static` interior-mutable cell would smuggle shared mutable state into
/// a pure module for one spec's convenience). Constructing the candidate
/// list directly with the same overridden priorities and calling `resolve`
/// exercises the identical reduction contract; see
/// `tests/combat_presentation_spec.rs`.
///
/// # Panics
///
/// If `candidates` is empty. [`select`] never calls this with an empty list:
/// it always adds a `Locomotion` candidate.
#[must_use]
pub fn resolve(candidates: &[PlayerPoseSelection]) -> PlayerPoseSelection {
    assert!(
        !candidates.is_empty(),
        "pose selection always has at least the locomotion fallback"
    );
    let mut best = candidates[0];
    for &candidate in &candidates[1..] {
        // Equal priorities use pose id order so selection never depends on
        // candidate discovery or condition ordering.
        if candidate.priority > best.priority
            || (candidate.priority == best.priority
                && candidate.id.wire_name() < best.id.wire_name())
        {
            best = candidate;
        }
    }
    best
}

/// Select the pose to display for `player` this frame.
///
/// `combat`, `keeper_context` and `outfield_context` are all optional: a
/// caller with no combat presentation for this player, no keeper signals (an
/// outfielder) or no outfield signals (a keeper, or a caller that has not
/// wired the context yet) passes `None`.
#[must_use]
pub fn select(
    player: &MatchPlayer,
    combat: Option<&CombatPoseSample>,
    keeper_context: Option<&KeeperPoseContext>,
    outfield_context: Option<&OutfieldPoseContext>,
) -> PlayerPoseSelection {
    let mut candidates: Vec<PlayerPoseSelection> = Vec::new();

    if player.is_keeper {
        if player.grab_timer > 0.0 {
            add(
                &mut candidates,
                PlayerPoseId::KeeperGrab,
                PlayerPoseSource::Soccer,
            );
        } else if player.throw_timer > 0.0 {
            add(
                &mut candidates,
                PlayerPoseId::KeeperThrow,
                PlayerPoseSource::Soccer,
            );
        } else if player.windup_timer > 0.0 {
            add(
                &mut candidates,
                PlayerPoseId::KeeperPunt,
                PlayerPoseSource::Soccer,
            );
        }
        if keeper_context.is_some_and(|c| c.tip) {
            add(
                &mut candidates,
                PlayerPoseId::KeeperTip,
                PlayerPoseSource::Soccer,
            );
        }
        if player.dive_timer > 0.0 {
            match player.save_style {
                Some(SaveStyle::Spread) => {
                    add(
                        &mut candidates,
                        PlayerPoseId::KeeperSpread,
                        PlayerPoseSource::Soccer,
                    );
                }
                Some(SaveStyle::Central) => {
                    add(
                        &mut candidates,
                        PlayerPoseId::KeeperCentral,
                        PlayerPoseSource::Soccer,
                    );
                }
                Some(SaveStyle::Stretch) => {
                    add(
                        &mut candidates,
                        PlayerPoseId::KeeperStretch,
                        PlayerPoseSource::Soccer,
                    );
                }
                None => {
                    add(
                        &mut candidates,
                        PlayerPoseId::KeeperDive,
                        PlayerPoseSource::Soccer,
                    );
                }
            }
        }
        // Getting up is the post-dive recovery window the simulation owns,
        // not the keeper's "recover" positioning state.
        if player.keeper_get_up_timer > 0.0 {
            add(
                &mut candidates,
                PlayerPoseId::KeeperGetUp,
                PlayerPoseSource::Soccer,
            );
        } else if player.keeper_state == KeeperBehaviorState::Set || player.keeper_set > 0.0 {
            add(
                &mut candidates,
                PlayerPoseId::KeeperSet,
                PlayerPoseSource::Soccer,
            );
        } else if keeper_context.is_some_and(|c| c.near_ball) {
            add(
                &mut candidates,
                PlayerPoseId::KeeperReadyLow,
                PlayerPoseSource::Soccer,
            );
        } else if keeper_context.is_some_and(|c| c.shuffling) {
            add(
                &mut candidates,
                PlayerPoseId::KeeperShuffle,
                PlayerPoseSource::Soccer,
            );
        } else {
            add(
                &mut candidates,
                PlayerPoseId::KeeperReadyTall,
                PlayerPoseSource::Soccer,
            );
        }
    }

    if player.aerial_timer > 0.0 && player.aerial_style == Some(AerialStyle::Bicycle) {
        add(
            &mut candidates,
            PlayerPoseId::AerialBicycle,
            PlayerPoseSource::Soccer,
        );
    } else if player.aerial_timer > 0.0 && player.aerial_style.is_some() {
        add(
            &mut candidates,
            PlayerPoseId::AerialAction,
            PlayerPoseSource::Soccer,
        );
    }

    if let Some(combat) = combat {
        if combat.forced_state == Some(CombatForcedState::Knockback) && combat.forced_ticks > 0 {
            add(
                &mut candidates,
                PlayerPoseId::CombatKnockback,
                PlayerPoseSource::Combat,
            );
        } else if combat.forced_state == Some(CombatForcedState::Stagger) && combat.forced_ticks > 0
        {
            add(
                &mut candidates,
                PlayerPoseId::CombatStagger,
                PlayerPoseSource::Combat,
            );
        }
        match combat.phase {
            CombatActionPhase::Guard => {
                add(
                    &mut candidates,
                    PlayerPoseId::CombatGuard,
                    PlayerPoseSource::Combat,
                );
            }
            CombatActionPhase::Active => {
                add(
                    &mut candidates,
                    PlayerPoseId::CombatActive,
                    PlayerPoseSource::Combat,
                );
            }
            CombatActionPhase::Windup => {
                add(
                    &mut candidates,
                    PlayerPoseId::CombatWindup,
                    PlayerPoseSource::Combat,
                );
            }
            CombatActionPhase::Aim => {
                add(
                    &mut candidates,
                    PlayerPoseId::CombatAim,
                    PlayerPoseSource::Combat,
                );
            }
            CombatActionPhase::Recovery => {
                add(
                    &mut candidates,
                    PlayerPoseId::CombatRecovery,
                    PlayerPoseSource::Combat,
                );
            }
            CombatActionPhase::Ready => {}
        }
    }

    if player.windup_timer > 0.0 {
        add(
            &mut candidates,
            PlayerPoseId::SoccerWindup,
            PlayerPoseSource::Soccer,
        );
    }
    if player.slide_timer > 0.0 {
        add(
            &mut candidates,
            PlayerPoseId::Slide,
            PlayerPoseSource::Soccer,
        );
    }

    // Outfield poses. Keepers keep their own vocabulary end to end, so a
    // keeper punt or a keeper stumble never borrows an outfielder's
    // silhouette.
    if !player.is_keeper {
        if player.tackle_timer > 0.0 {
            add(
                &mut candidates,
                PlayerPoseId::Tackle,
                PlayerPoseSource::Soccer,
            );
        }
        if player.stun_timer > 0.0 {
            add(
                &mut candidates,
                PlayerPoseId::Stumble,
                PlayerPoseSource::Soccer,
            );
        }
        if outfield_context.is_some_and(|c| c.kick_follow) {
            add(
                &mut candidates,
                PlayerPoseId::KickFollow,
                PlayerPoseSource::Soccer,
            );
        }
        if player.settle_timer > 0.0 {
            add(
                &mut candidates,
                PlayerPoseId::Settle,
                PlayerPoseSource::Soccer,
            );
        }
        if run_telegraphing(player, outfield_context.and_then(|c| c.now)) {
            add(
                &mut candidates,
                PlayerPoseId::RunTelegraph,
                PlayerPoseSource::Soccer,
            );
        }
        // One shared silhouette, two independent mechanics: the human
        // jockey stance and the AI presser's contain mode read the same on
        // screen and keep their own reach, speed, and trigger rules in
        // `gc_sim::r#match`.
        if player.jockey_timer > 0.0 || outfield_context.is_some_and(|c| c.containing) {
            add(
                &mut candidates,
                PlayerPoseId::Contain,
                PlayerPoseSource::Soccer,
            );
        }
        if sprint_spent(player) {
            add(
                &mut candidates,
                PlayerPoseId::Fatigue,
                PlayerPoseSource::Soccer,
            );
        }
    }

    add(
        &mut candidates,
        PlayerPoseId::Locomotion,
        PlayerPoseSource::Locomotion,
    );

    resolve(&candidates)
}
