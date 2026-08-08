//! Port of `render/identity.lua`.
//!
//! Resolves a player id to the presentation identity the pitch draws: display
//! name, species flavour, silhouette and palette.
//!
//! This lives in `gc-render` rather than `gc-sim` because nothing here can change
//! what the simulation computes — it is a read-only projection of content onto
//! presentation. It is on the Rust side of the boundary only because it is the
//! producer half of the RenderFrame.
//!
//! Note the split the Lua makes and this preserves: a showcase player carries
//! both a *mechanical* species and an optional *presentation* species. The
//! presentation one wins here, and only here — swapping how a player looks must
//! never move their stats.

use gc_data::players::Position;
use gc_data::species::Shape;

/// Everything the renderer needs to draw one player's identity.
#[derive(Clone, Debug, PartialEq)]
pub struct PlayerPresentationIdentity {
    /// The player's persistent id.
    pub player_id: &'static str,
    /// Display name.
    pub name: &'static str,
    /// Playing position.
    pub position: Position,
    /// The species used for presentation, which may differ from the mechanical one.
    pub species_id: &'static str,
    /// That species' display name.
    pub species_name: &'static str,
    /// Short marketing tagline; empty when the species authors none.
    pub tagline: &'static str,
    /// Visual silhouette family.
    pub shape: Shape,
    /// `{r, g, b}` in 0..1.
    pub palette: [f64; 3],
}

/// The palette used when a species authors none, matching the Lua's literal.
const DEFAULT_PALETTE: [f64; 3] = [0.55, 0.72, 0.92];

/// Resolve a player id to its presentation identity.
///
/// Returns `None` when the player is unknown, has no showcase compatibility row,
/// or names a species that does not exist — the same three `nil` returns the Lua
/// has. Those are lookup misses over content, not broken invariants, so they are
/// absent values rather than panics (AGENTS.md §7).
#[must_use]
pub fn for_player(player_id: &str) -> Option<PlayerPresentationIdentity> {
    let player = gc_data::players::get(player_id)?;
    let showcase = gc_data::showcase_player_compatibility::get(player.id)?;
    // Presentation species wins over the mechanical one wherever authored.
    let species_id = showcase.presentation_species.unwrap_or(showcase.species);
    let presentation = gc_data::species::get(species_id)?;
    Some(PlayerPresentationIdentity {
        player_id: player.id,
        name: player.name,
        position: player.position,
        species_id: presentation.id,
        species_name: presentation.name,
        tagline: presentation.tagline.unwrap_or(""),
        shape: presentation.shape.unwrap_or(Shape::Round),
        palette: presentation.palette.unwrap_or(DEFAULT_PALETTE),
    })
}
