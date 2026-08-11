//! Runtime-tunable gameplay knobs, for the in-match tuning panel (F1). The
//! sim reads live values through a [`Tuning`] instance; defaults here ARE the
//! shipped balance, so a fresh match plays identically to the constants they
//! replaced. Pure module: no I/O — (de)serialization is string-based and the
//! game layer decides where bytes go.
//!
//! AGENTS.md §3 forbids stray global mutable state,
//! so the registry is an explicit value, [`Tuning`], that callers thread
//! through instead of a shared mutable global — every other `sim` module
//! reads live values through the instance it's given.

use indexmap::IndexMap;

/// A single tunable gameplay knob.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Knob {
    /// Registry key, also the serialization key.
    pub key: &'static str,
    /// Shown in the tuning panel.
    pub label: &'static str,
    /// Panel tab this knob's category groups into.
    pub cat: &'static str,
    /// Value a fresh match plays with.
    pub default: f64,
    /// Minimum allowed value.
    pub min: f64,
    /// Maximum allowed value.
    pub max: f64,
    /// Nudge step size.
    pub step: f64,
}

// Registry order = display order. Categories group the panel's tabs.
/// Every authored knob, in registry (display) order.
pub static KNOBS: &[Knob] = &[
    // Movement
    Knob {
        key: "MOVE_ACCEL",
        label: "Acceleration",
        cat: "Movement",
        default: 1100.0,
        min: 400.0,
        max: 2400.0,
        step: 100.0,
    },
    Knob {
        key: "START_ACCEL",
        label: "Standing start",
        cat: "Movement",
        default: 450.0,
        min: 150.0,
        max: 1100.0,
        step: 50.0,
    },
    Knob {
        key: "MOVE_DECEL",
        label: "Deceleration",
        cat: "Movement",
        default: 1500.0,
        min: 400.0,
        max: 3000.0,
        step: 100.0,
    },
    Knob {
        key: "SPRINT_MULT",
        label: "Sprint speed x",
        cat: "Movement",
        default: 1.35,
        min: 1.1,
        max: 1.8,
        step: 0.05,
    },
    Knob {
        key: "SPRINT_REFILL",
        label: "Sprint refill /s",
        cat: "Movement",
        default: 0.4,
        min: 0.1,
        max: 1.0,
        step: 0.05,
    },
    Knob {
        key: "JOCKEY_SLOW",
        label: "Jockey speed x",
        cat: "Movement",
        default: 0.75,
        min: 0.5,
        max: 1.0,
        step: 0.05,
    },
    // Dribble (touch-based ball control)
    Knob {
        key: "DRIBBLE_CLOSE",
        label: "Close ctrl x speed",
        cat: "Dribble",
        default: 1.05,
        min: 0.2,
        max: 1.4,
        step: 0.05,
    },
    Knob {
        key: "DRIBBLE_PUSH",
        label: "Touch push xspeed",
        cat: "Dribble",
        default: 1.5,
        min: 1.15,
        max: 2.2,
        step: 0.05,
    },
    Knob {
        key: "DRIBBLE_ERR",
        label: "Touch error (rad)",
        cat: "Dribble",
        default: 0.3,
        min: 0.0,
        max: 0.6,
        step: 0.05,
    },
    Knob {
        key: "DRIBBLE_TOUCH",
        label: "Gather stiffness",
        cat: "Dribble",
        default: 14.0,
        min: 4.0,
        max: 30.0,
        step: 1.0,
    },
    Knob {
        key: "DRIBBLE_CONTROL",
        label: "Control radius",
        cat: "Dribble",
        default: 34.0,
        min: 20.0,
        max: 80.0,
        step: 2.0,
    },
    // Aerial (headers, volleys, finishing crosses)
    Knob {
        key: "AERIAL_ASSIST",
        label: "Strike reach aid",
        cat: "Aerial",
        default: 44.0,
        min: 0.0,
        max: 80.0,
        step: 4.0,
    },
    Knob {
        key: "AERIAL_MAGNET",
        label: "Ball magnet /s",
        cat: "Aerial",
        default: 260.0,
        min: 0.0,
        max: 600.0,
        step: 20.0,
    },
    // Attacking
    Knob {
        key: "CHARGE_RATE",
        label: "Shot charge /s",
        cat: "Attacking",
        default: 1.5,
        min: 0.5,
        max: 3.0,
        step: 0.1,
    },
    Knob {
        key: "PASS_CHARGE_RATE",
        label: "Pass charge /s",
        cat: "Attacking",
        default: 2.4,
        min: 0.5,
        max: 3.0,
        step: 0.1,
    },
    Knob {
        key: "SHOT_WINDUP",
        label: "Shot wind-up s",
        cat: "Attacking",
        default: 0.15,
        min: 0.0,
        max: 0.4,
        step: 0.01,
    },
    Knob {
        key: "PASS_RANGE_MAX",
        label: "Max pass range",
        cat: "Attacking",
        default: 520.0,
        min: 300.0,
        max: 800.0,
        step: 20.0,
    },
    Knob {
        key: "HEADER_SPEED",
        label: "Header pace x",
        cat: "Attacking",
        default: 0.85,
        min: 0.5,
        max: 1.2,
        step: 0.05,
    },
    Knob {
        key: "VOLLEY_SKY_P",
        label: "Volley sky odds",
        cat: "Attacking",
        default: 0.35,
        min: 0.0,
        max: 1.0,
        step: 0.05,
    },
    // Defending
    Knob {
        key: "AI_STEAL_CD",
        label: "AI poke cooldown",
        cat: "Defending",
        default: 1.2,
        min: 0.4,
        max: 2.5,
        step: 0.1,
    },
    Knob {
        key: "STEAL_ATTEMPT",
        label: "AI poke range",
        cat: "Defending",
        default: 40.0,
        min: 28.0,
        max: 60.0,
        step: 2.0,
    },
    Knob {
        key: "WHIFF_STUMBLE",
        label: "Whiff stumble s",
        cat: "Defending",
        default: 0.3,
        min: 0.0,
        max: 0.8,
        step: 0.05,
    },
    Knob {
        key: "CARRIER_SETTLE",
        label: "AI settle touch s",
        cat: "Defending",
        default: 0.35,
        min: 0.0,
        max: 0.8,
        step: 0.05,
    },
    Knob {
        key: "AI_PASS_PRESSURE",
        label: "AI pass-out range",
        cat: "Defending",
        default: 70.0,
        min: 30.0,
        max: 120.0,
        step: 5.0,
    },
    // Keeper
    Knob {
        key: "SAVE_SPEED_REF",
        label: "Save pace ref",
        cat: "Keeper",
        default: 1300.0,
        // min widened 700 -> 400: the balance search's optimum sat on the old
        // fence (docs/design/fun_metrics.md, phase 3).
        min: 400.0,
        max: 2000.0,
        step: 50.0,
    },
    Knob {
        key: "CATCH_EVEN_QUALITY",
        label: "Catch coin-flip q",
        cat: "Keeper",
        default: 0.45,
        min: 0.2,
        max: 0.8,
        step: 0.02,
    },
    Knob {
        key: "KEEPER_RESPECT_DIST",
        label: "Keeper ring",
        cat: "Keeper",
        default: 120.0,
        min: 60.0,
        max: 180.0,
        step: 10.0,
    },
    Knob {
        key: "KEEPER_HOLD_HUMAN",
        label: "Keeper hold limit s",
        cat: "Keeper",
        default: 5.0,
        min: 2.0,
        max: 10.0,
        step: 0.5,
    },
    Knob {
        key: "PUNT_MAX",
        label: "Max punt range",
        cat: "Keeper",
        default: 640.0,
        min: 400.0,
        max: 900.0,
        step: 20.0,
    },
    // AI
    Knob {
        key: "AI_SHOOT_RANGE",
        label: "AI shoot range",
        cat: "AI",
        default: 240.0,
        min: 160.0,
        // max widened 340 -> 480 (half pitch): the balance search's optimum
        // sat on the old fence (docs/design/fun_metrics.md, phase 3).
        max: 480.0,
        step: 10.0,
    },
    Knob {
        key: "AI_HEADER_RANGE",
        label: "AI header range",
        cat: "AI",
        default: 200.0,
        min: 120.0,
        // max widened 300 -> 420: the balance search's optimum sat on the
        // old fence (docs/design/fun_metrics.md, phase 3).
        max: 420.0,
        step: 10.0,
    },
    Knob {
        key: "CROSS_MIN_SPACE",
        label: "Cross space need",
        cat: "AI",
        default: 30.0,
        min: 10.0,
        max: 60.0,
        step: 5.0,
    },
    Knob {
        key: "LOOSE_MAGNET",
        label: "Loose-ball magnet",
        cat: "AI",
        default: 90.0,
        min: 40.0,
        max: 160.0,
        step: 10.0,
    },
    Knob {
        key: "TRIANGLE_DIST",
        label: "Triangle pass range",
        cat: "AI",
        default: 170.0,
        min: 120.0,
        max: 260.0,
        step: 10.0,
    },
    Knob {
        key: "STAND_WAKE",
        label: "Positional calm",
        cat: "AI",
        default: 34.0,
        min: 16.0,
        max: 80.0,
        step: 2.0,
    },
    Knob {
        key: "AI_SPRINT_SPACE",
        label: "Sprint into space",
        cat: "AI",
        default: 70.0,
        min: 60.0,
        max: 240.0,
        step: 10.0,
    },
    Knob {
        key: "AI_JUKE_DIST",
        label: "Juke reaction range",
        cat: "AI",
        default: 44.0,
        min: 30.0,
        max: 90.0,
        step: 2.0,
    },
    Knob {
        key: "AI_JUKE_CD",
        label: "Juke cooldown s",
        cat: "AI",
        default: 2.0,
        min: 0.6,
        max: 5.0,
        step: 0.2,
    },
    // Replay (presentation)
    Knob {
        key: "REPLAY_SLOWMO",
        label: "Replay speed x",
        cat: "Replay",
        default: 0.35,
        min: 0.1,
        max: 1.0,
        step: 0.05,
    },
    Knob {
        key: "REPLAY_SECONDS",
        label: "Replay length s",
        cat: "Replay",
        default: 4.0,
        min: 2.0,
        max: 8.0,
        step: 0.5,
    },
];

/// A live registry of tuning knob values.
///
/// An owned value rather than a module-level singleton (see the module
/// doc).
#[derive(Clone, Debug, PartialEq)]
pub struct Tuning {
    values: IndexMap<&'static str, f64>,
}

fn knob_by_key(key: &str) -> Option<&'static Knob> {
    KNOBS.iter().find(|k| k.key == key)
}

impl Default for Tuning {
    fn default() -> Self {
        Self::new()
    }
}

impl Tuning {
    /// A fresh registry at every knob's default value.
    #[must_use]
    pub fn new() -> Self {
        let values = KNOBS.iter().map(|k| (k.key, k.default)).collect();
        Tuning { values }
    }

    /// The current value of a knob. Panics on an unknown key: every caller
    /// reads a key it authored, so an unknown key is a programmer error.
    #[must_use]
    pub fn value(&self, key: &str) -> f64 {
        *self
            .values
            .get(key)
            .unwrap_or_else(|| panic!("unknown tuning key: {key}"))
    }

    /// Distinct categories, in registry order.
    #[must_use]
    pub fn categories(&self) -> Vec<&'static str> {
        let mut seen: Vec<&'static str> = Vec::new();
        for k in KNOBS {
            if !seen.contains(&k.cat) {
                seen.push(k.cat);
            }
        }
        seen
    }

    /// Every knob in one category, in registry order.
    #[must_use]
    pub fn in_category(&self, cat: &str) -> Vec<&'static Knob> {
        KNOBS.iter().filter(|k| k.cat == cat).collect()
    }

    /// Set a knob, clamped to its range. Unknown keys are ignored.
    pub fn set(&mut self, key: &str, v: f64) {
        if let Some(k) = knob_by_key(key) {
            let clamped = v.max(k.min).min(k.max);
            self.values.insert(k.key, clamped);
        }
    }

    /// Nudge a knob by `dirs` steps (negative = down).
    pub fn nudge(&mut self, key: &str, dirs: f64) {
        if let Some(k) = knob_by_key(key) {
            let current = self.value(key);
            self.set(key, current + k.step * dirs);
        }
    }

    /// Reset one knob, or everything when `key` is `None`.
    pub fn reset(&mut self, key: Option<&str>) {
        if let Some(key) = key {
            if let Some(k) = knob_by_key(key) {
                self.values.insert(k.key, k.default);
            }
            return;
        }
        for k in KNOBS {
            self.values.insert(k.key, k.default);
        }
    }

    /// Whether a knob currently sits at its default value.
    #[must_use]
    pub fn is_default(&self, key: &str) -> bool {
        match knob_by_key(key) {
            Some(k) => self.value(key) == k.default,
            None => false,
        }
    }

    /// One `KEY=value` line per NON-default knob (a fresh registry means an
    /// empty string).
    #[must_use]
    pub fn serialize(&self) -> String {
        let mut lines = Vec::new();
        for k in KNOBS {
            let v = self.value(k.key);
            if v != k.default {
                lines.push(format!("{}={}", k.key, format_g6(v)));
            }
        }
        lines.join("\n")
    }

    /// Apply a serialized blob on top of defaults. Malformed lines are
    /// skipped.
    pub fn deserialize(&mut self, blob: &str) {
        self.reset(None);
        for line in blob.split(['\r', '\n']) {
            if line.is_empty() {
                continue;
            }
            if let Some((key, value)) = parse_knob_line(line)
                && let Ok(v) = value.parse::<f64>()
            {
                self.set(key, v);
            }
        }
    }
}

/// Parse one `KEY=value` line: the key is alphanumeric/underscore, the value
/// is restricted to digits, `-`, `.`, `e`, and `E`. Returns `None` if the
/// line does not match.
fn parse_knob_line(line: &str) -> Option<(&str, &str)> {
    let eq = line.find('=')?;
    let (key, rest) = (&line[..eq], &line[eq + 1..]);
    if key.is_empty() || !key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return None;
    }
    if rest.is_empty()
        || !rest
            .chars()
            .all(|c| c.is_ascii_digit() || c == '-' || c == '.' || c == 'e' || c == 'E')
    {
        return None;
    }
    Some((key, rest))
}

/// Format `value` the way C's `%.6g` would, for the magnitude range every
/// tuning knob lives in (units to low thousands). Not a general `%g`
/// implementation — it always uses fixed notation, which matches `%g` only
/// while the exponent stays inside roughly `[-4, 6)`, comfortably covering
/// every authored knob bound.
fn format_g6(value: f64) -> String {
    if value == 0.0 {
        return "0".to_string();
    }
    let magnitude = value.abs().log10().floor() as i32;
    let decimals = (5 - magnitude).clamp(0, 17) as usize;
    let mut s = format!("{value:.decimals$}");
    if s.contains('.') {
        while s.ends_with('0') {
            s.pop();
        }
        if s.ends_with('.') {
            s.pop();
        }
    }
    s
}
