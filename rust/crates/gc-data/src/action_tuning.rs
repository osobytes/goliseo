//! Committed-action verb identity and per-verb interrupt-list content
//! (#489). Data only (AGENTS.md §8): `gc_sim::action_slot` is the
//! mechanism, this module is the shapes and the values.
//!
//! ## Scope of this PR
//!
//! [`ActionVerb`] is written to be extensible, but only [`ActionVerb::Tackle`]
//! is wired through `gc_sim::action_slot` today. `shot` and `pass` keep
//! their existing `MatchPlayer::windup_timer`/`windup_shot`/`pass_charge`
//! fields; migrating them onto this seam is left to a follow-up. See
//! `gc_sim::action_slot`'s module doc for the full argument.
//!
//! ## Interrupt lists are not yet a tier-3 band set
//!
//! #489 asks for interrupt lists to be authored as tier-3 band-sets,
//! substituted as a whole unit through `gc_sim::tunable_registry`. That
//! registry's [`gc_sim::tunable_registry::Registry::substitute_band_set`] is
//! typed for `f64`-valued edges (AI membership thresholds); an interrupt
//! list is a set of enum members, not a continuum with edges, so reusing
//! that exact mechanism would mean encoding "does this kind interrupt" as a
//! `0.0`/`1.0` edge value -- possible, but a type-punning fit rather than a
//! natural one. [`ActionInterruptList`] instead declares the same
//! *discipline* (an id, an explicit version, and a fixed member list,
//! substituted only as a whole) without joining the registry's config hash.
//! Nothing currently varies an interrupt list at runtime -- there is no
//! substitution call site -- so the divergence-across-peers risk the
//! registry's hashing exists to catch does not yet apply. Folding interrupt
//! lists into the registry proper (so a future substitution is hashed and
//! handshake-checked like a real band set) is left as a follow-up once a
//! second, non-empty list gives the generalization something to prove
//! itself against.

/// A verb that can occupy a player's committed-action slot
/// ([`gc_sim::action_slot::ActionSlot`]). Extensible: today only `Tackle`
/// is wired through the mechanism -- see the module doc.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActionVerb {
    /// A standing poke tackle attempt. Slide tackles remain a separate,
    /// unconverted mechanic in this PR (`MatchPlayer::slide_timer`) -- see
    /// `gc_sim::action_slot`'s module doc.
    Tackle,
}

/// One interrupt kind a verb's executing phase may declare early-break
/// eligibility for, beyond the universal possession-change invariant
/// (`gc_sim::action_slot::clear`), which is not itself a member of any
/// verb's list because it applies unconditionally and is evaluated once, at
/// the possession-change site, never per verb.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActionInterruptKind {
    /// A hard knockdown (e.g. a slide tackle's stun). Not read by any
    /// verb's list in this PR -- declared so the enum is not a
    /// single-verb-shaped placeholder the moment a second list needs it.
    HardKnockdown,
}

/// One verb's declared early-break list, versioned as a unit: a
/// substitution must replace the whole list under a new version, exactly
/// like `gc_data::tunables::BandSet` (see the module doc on why this is a
/// parallel discipline rather than that same type).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ActionInterruptList {
    /// The verb this list governs.
    pub verb: ActionVerb,
    /// Version of this list's membership. A substitution must change it.
    pub version: &'static str,
    /// The interrupt kinds that may break an executing action of this verb
    /// early. Empty means: nothing off-list interrupts it, full stop.
    pub kinds: &'static [ActionInterruptKind],
}

/// Tackle's initial interrupt list, per #489's own design: "tackles break
/// on nothing." The commit is immune to every generic interrupt until
/// execution resolves or the universal possession invariant clears it (and
/// a tackler, by construction, never owns the ball while charging or
/// executing one, so that invariant is inert for this verb today -- see
/// `gc_sim::action_slot`'s module doc).
pub static TACKLE_INTERRUPTS: ActionInterruptList = ActionInterruptList {
    verb: ActionVerb::Tackle,
    version: "v1",
    kinds: &[],
};

/// Whether `kind` is on `list`. A verb whose list is empty (like
/// [`TACKLE_INTERRUPTS`]) returns `false` for every kind, by construction.
#[must_use]
pub fn interrupts(list: &ActionInterruptList, kind: ActionInterruptKind) -> bool {
    list.kinds.contains(&kind)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tackle_interrupt_list_is_empty_and_interrupts_nothing() {
        assert_eq!(TACKLE_INTERRUPTS.verb, ActionVerb::Tackle);
        assert!(TACKLE_INTERRUPTS.kinds.is_empty());
        assert!(!interrupts(
            &TACKLE_INTERRUPTS,
            ActionInterruptKind::HardKnockdown
        ));
    }
}
