//! The free-flight ball integrator: the one place a loose ball's motion is
//! computed.
//!
//! This module exists so that there is exactly **one** implementation of
//! "what happens to a ball nobody is touching over one fixed tick".
//! [`crate::r#match::update_ball`] calls it for the live ball, and
//! [`crate::ball_prediction`] calls the same function in its scratch world.
//! A forward prediction that ran its own quadratic would be a second
//! implementation of the physics, and two implementations disagree the day
//! one of them changes — a keeper diving to where a hand-rolled parabola
//! said the ball would be, while the real step applied drag and a bounce,
//! is the concrete failure that argument is about.
//!
//! Scope, precisely: integration, grass/air drag, roll spin, gravity, the
//! cage ceiling, the touchlines, the back walls and the two net boxes.
//! Everything that needs a *player* — body blocking, keeper saves, aerial
//! contact, possession — stays in `crate::r#match`, because a scratch world
//! containing only the ball and the ground plane cannot answer those and
//! must not pretend to.
//!
//! The step consumes **no randomness**. That is a determinism requirement,
//! not an accident: a prediction that drew from the seeded stream would
//! spend entries the live sim never sees, and every peer that predicted a
//! different number of times would resimulate a different match. The
//! signature is the proof — this function is handed a ball and a pitch, and
//! has no access to `MatchState::rng` at all.

use crate::match_snapshot::{MatchState, PitchSize, Rect};
use gc_core::vec2::Vec2;

pub(crate) const BALL_RADIUS: f64 = 6.0;
pub(crate) const FRICTION: f64 = 1.2;
pub(crate) const NET_DAMP: f64 = 0.3;
pub(crate) const NET_ROLLOUT: f64 = 200.0;
pub(crate) const GRAVITY: f64 = 900.0;
pub(crate) const BOUNCE: f64 = 0.55;
pub(crate) const AIR_FRICTION: f64 = 0.3;
pub(crate) const GROUND_GRAB_HEIGHT: f64 = 14.0;
pub(crate) const LAND_SETTLE_VZ: f64 = 60.0;
pub(crate) const CAGE_CEILING: f64 = 170.0;
pub(crate) const CEIL_BOUNCE: f64 = 0.55;
pub(crate) const SPIN_DECAY: f64 = 1.4;

/// One ball's complete free-flight state: everything [`step`] reads and
/// writes, and nothing else.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BallFlight {
    /// Ground position.
    pub pos: Vec2,
    /// Horizontal velocity (ground plane), px/s.
    pub vel: Vec2,
    /// Height above the pitch (0 = on the ground).
    pub z: f64,
    /// Vertical velocity, px/s (+ = rising).
    pub vz: f64,
    /// Lateral curve applied while rolling.
    pub spin: f64,
}

impl BallFlight {
    /// The live ball's current free-flight state.
    #[must_use]
    pub fn of(state: &MatchState) -> Self {
        BallFlight {
            pos: state.ball,
            vel: state.ball_vel,
            z: state.ball_z,
            vz: state.ball_vz,
            spin: state.ball_spin,
        }
    }

    /// Write this flight state back onto the live ball.
    pub fn write_back(self, state: &mut MatchState) {
        state.ball = self.pos;
        state.ball_vel = self.vel;
        state.ball_z = self.z;
        state.ball_vz = self.vz;
        state.ball_spin = self.spin;
    }
}

/// The static geometry a free ball bounces off: the pitch rectangle and the
/// two goal mouths (with their net boxes behind the line).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BallArena {
    /// Pitch dimensions.
    pub field: PitchSize,
    /// Left goal; away scores here.
    pub goal_home: Rect,
    /// Right goal; home scores here.
    pub goal_away: Rect,
}

impl BallArena {
    /// This match's arena geometry.
    #[must_use]
    pub fn of(state: &MatchState) -> Self {
        BallArena {
            field: state.field,
            goal_home: state.goal_home,
            goal_away: state.goal_away,
        }
    }
}

/// Whether a ball at `ball` is inside `goal`'s mouth vertically.
pub(crate) fn in_mouth(ball: Vec2, goal: Rect) -> bool {
    ball.y >= goal.y && ball.y <= goal.y + goal.h
}

/// Advance one free ball by `dt`: integrate, decay, curve, bounce off
/// touchlines/back walls. Returns whether the trajectory bounced — the live
/// sim uses that to unset keeper set-position latches, which the scratch
/// world has no use for.
pub fn step(ball: &mut BallFlight, arena: &BallArena, dt: f64) -> bool {
    ball.pos = ball.pos.add(ball.vel.scale(dt));
    // Full grass friction only near the ground; a lofted ball glides
    // (light drag).
    let airborne = ball.z > GROUND_GRAB_HEIGHT;
    let hfric = if airborne { AIR_FRICTION } else { FRICTION };
    ball.vel = ball.vel.scale((1.0 - hfric * dt).max(0.0));
    let mut trajectory_bounced = false;
    // Spin only bends a ball rolling on the grass.
    if !airborne && ball.spin != 0.0 && ball.vel.length() > 1.0 {
        let v = ball.vel;
        let perp = Vec2::new(-v.y, v.x).normalized();
        ball.vel = ball.vel.add(perp.scale(ball.spin * dt));
        ball.spin *= (1.0 - SPIN_DECAY * dt).max(0.0);
    }

    // Vertical: integrate under gravity, land and rebound (keeping
    // horizontal pace).
    ball.z += ball.vz * dt;
    ball.vz -= GRAVITY * dt;
    // Cage ceiling: a skied ball bounces back down into the arena.
    if ball.z >= CAGE_CEILING {
        ball.z = CAGE_CEILING;
        if ball.vz > 0.0 {
            ball.vz = -ball.vz * CEIL_BOUNCE;
            trajectory_bounced = true;
        }
    }
    if ball.z <= 0.0 {
        ball.z = 0.0;
        if ball.vz < 0.0 {
            if -ball.vz <= LAND_SETTLE_VZ {
                ball.vz = 0.0; // settle: stop micro-bouncing
            } else {
                ball.vz = -ball.vz * BOUNCE; // rebound up; vel unchanged
            }
        }
    }

    if ball.pos.y < BALL_RADIUS {
        ball.pos.y = BALL_RADIUS;
        ball.vel.y = -ball.vel.y;
        trajectory_bounced = true;
    } else if ball.pos.y > arena.field.h - BALL_RADIUS {
        ball.pos.y = arena.field.h - BALL_RADIUS;
        ball.vel.y = -ball.vel.y;
        trajectory_bounced = true;
    }
    // X walls: through a goal mouth the ball plays on into the net box
    // behind the line; anywhere else it bounces back in. Inside a net box
    // the side netting clamps y, the back net kills most pace, and a
    // gentle "slope" rolls a dead ball back out toward the line so it
    // can't strand there.
    if ball.pos.x < BALL_RADIUS {
        if in_mouth(ball.pos, arena.goal_home) {
            let g = arena.goal_home;
            if ball.pos.x < g.x + BALL_RADIUS {
                ball.pos.x = g.x + BALL_RADIUS;
                ball.vel = Vec2::new(-ball.vel.x * NET_DAMP, ball.vel.y * NET_DAMP);
            }
            if ball.pos.y < g.y + BALL_RADIUS {
                ball.pos.y = g.y + BALL_RADIUS;
                ball.vel.y = -ball.vel.y * NET_DAMP;
            } else if ball.pos.y > g.y + g.h - BALL_RADIUS {
                ball.pos.y = g.y + g.h - BALL_RADIUS;
                ball.vel.y = -ball.vel.y * NET_DAMP;
            }
            ball.vel.x += NET_ROLLOUT * dt;
        } else {
            ball.pos.x = BALL_RADIUS;
            ball.vel.x = -ball.vel.x;
            trajectory_bounced = true;
        }
    } else if ball.pos.x > arena.field.w - BALL_RADIUS {
        if in_mouth(ball.pos, arena.goal_away) {
            let g = arena.goal_away;
            if ball.pos.x > g.x + g.w - BALL_RADIUS {
                ball.pos.x = g.x + g.w - BALL_RADIUS;
                ball.vel = Vec2::new(-ball.vel.x * NET_DAMP, ball.vel.y * NET_DAMP);
            }
            if ball.pos.y < g.y + BALL_RADIUS {
                ball.pos.y = g.y + BALL_RADIUS;
                ball.vel.y = -ball.vel.y * NET_DAMP;
            } else if ball.pos.y > g.y + g.h - BALL_RADIUS {
                ball.pos.y = g.y + g.h - BALL_RADIUS;
                ball.vel.y = -ball.vel.y * NET_DAMP;
            }
            ball.vel.x -= NET_ROLLOUT * dt;
        } else {
            ball.pos.x = arena.field.w - BALL_RADIUS;
            ball.vel.x = -ball.vel.x;
            trajectory_bounced = true;
        }
    }
    trajectory_bounced
}
