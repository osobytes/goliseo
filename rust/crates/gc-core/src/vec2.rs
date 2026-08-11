//! Immutable 2D vector. Pure math, usable by every layer.
//!
//! This type lives in `gc-core` — not only in the TypeScript presentation
//! package — because it is on the determinism path. Fifteen `sim/` modules use
//! it, including `match`, `match_snapshot`, `keeper`, `combat` and `slot_input`,
//! so its arithmetic feeds simulation state and must be bit-reproducible.
//!
//! `length` uses `sqrt`, which IEEE 754 specifies as correctly rounded, so it is
//! exact on every conforming runtime. That is why it can be used directly
//! on the determinism path while the transcendentals had to be routed through
//! [`crate::deterministic_math`].

/// An immutable 2D vector.
///
/// Every operation returns a new value; nothing mutates in place.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Vec2 {
    /// The horizontal component.
    pub x: f64,
    /// The vertical component.
    pub y: f64,
}

// The inherent `add`/`sub`/`scale` methods give an explicit call shape
// (`a.add(b)`) alongside the operator traits below, which give the same
// results for anyone who prefers `a + b`. Clippy objects to inherent methods
// that shadow trait names; keeping both is the deliberate trade, so the lint
// is silenced here rather than in the workspace config.
#[allow(clippy::should_implement_trait)]
impl Vec2 {
    /// Construct a vector from its components.
    ///
    /// Callers that want a zero vector use [`Vec2::default`] or pass `0.0`
    /// explicitly.
    #[must_use]
    pub const fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }

    /// Componentwise sum.
    #[must_use]
    pub fn add(self, o: Self) -> Self {
        Self::new(self.x + o.x, self.y + o.y)
    }

    /// Componentwise difference.
    #[must_use]
    pub fn sub(self, o: Self) -> Self {
        Self::new(self.x - o.x, self.y - o.y)
    }

    /// Multiply both components by a scalar.
    #[must_use]
    pub fn scale(self, s: f64) -> Self {
        Self::new(self.x * s, self.y * s)
    }

    /// Euclidean length.
    ///
    /// The operand order is fixed — `x * x + y * y`, then `sqrt` — because
    /// floating-point addition is not associative and the determinism evidence
    /// depends on bit-exact output.
    #[must_use]
    pub fn length(self) -> f64 {
        (self.x * self.x + self.y * self.y).sqrt()
    }

    /// Unit vector in the same direction, or zero when the length is zero.
    #[must_use]
    pub fn normalized(self) -> Self {
        let len = self.length();
        if len == 0.0 {
            return Self::new(0.0, 0.0);
        }
        Self::new(self.x / len, self.y / len)
    }

    /// Distance to another vector.
    ///
    /// Computed as `self.sub(o).length()` rather than a fused form — the
    /// intermediate rounding is part of the result.
    #[must_use]
    pub fn dist(self, o: Self) -> f64 {
        self.sub(o).length()
    }
}

impl std::ops::Add for Vec2 {
    type Output = Self;
    fn add(self, o: Self) -> Self {
        Vec2::add(self, o)
    }
}

impl std::ops::Sub for Vec2 {
    type Output = Self;
    fn sub(self, o: Self) -> Self {
        Vec2::sub(self, o)
    }
}

impl std::ops::Mul<f64> for Vec2 {
    type Output = Self;
    fn mul(self, s: f64) -> Self {
        self.scale(s)
    }
}

impl std::ops::Neg for Vec2 {
    type Output = Self;
    fn neg(self) -> Self {
        Self::new(-self.x, -self.y)
    }
}
