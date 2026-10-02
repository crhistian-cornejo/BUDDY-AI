//! The mascot's idle behavior, decided once for both apps: when Buddy breathes, blinks, looks around, waves, walks
//! (and how far, inside the screen) or falls asleep. Apps ask for the next plan, wait, play it and ask again; nothing
//! runs between plans, so idle stays near 0 % CPU.

use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

/// Walking speed in points per second.
pub const WALK_SPEED: f64 = 32.0;
/// Walks shorter than this are not worth it (Buddy turns around or does something else).
const MIN_WALK: f64 = 24.0;
const MAX_WALK: f64 = 180.0;
/// Without any input for this long the user is away: Buddy sleeps.
pub const SLEEP_AFTER_SECONDS: f64 = 300.0;

/// Where the mascot is and what it may do. Horizontal values are the window's left edge, in points.
#[derive(Debug, Clone, Copy, PartialEq, serde::Deserialize)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct PetContext {
    pub x: f64,
    /// The left edge may go from `min_x` to `max_x` (the screen's usable width minus the mascot's).
    pub min_x: f64,
    pub max_x: f64,
    pub reduce_motion: bool,
    /// The user lets Buddy walk around (a setting).
    pub wander: bool,
    /// Seconds since the last keyboard or mouse input on the system.
    pub idle_seconds: f64,
}

/// What to do next: wait `wait_ms`, then play `state` for `duration_ms` while moving `dx` points (walks only).
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct PetPlan {
    pub wait_ms: u32,
    pub state: String,
    pub duration_ms: u32,
    pub dx: f64,
}

/// A rectangle in points (origin at the bottom-left on Mac, top-left on Windows: only sizes and ranges matter here).
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct PetRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// Keeps `window` fully inside `area` (a screen's usable frame): no part of Buddy hides past an edge or a corner.
#[cfg_attr(feature = "ffi", uniffi::export)]
pub fn clamp_to_area(window: PetRect, area: PetRect) -> PetRect {
    let max_x = (area.x + area.width - window.width).max(area.x);
    let max_y = (area.y + area.height - window.height).max(area.y);
    PetRect { x: window.x.clamp(area.x, max_x), y: window.y.clamp(area.y, max_y), ..window }
}

#[cfg_attr(feature = "ffi", derive(uniffi::Object))]
pub struct PetBrain {
    rng: Mutex<u64>,
}

impl Default for PetBrain {
    fn default() -> Self {
        let seed = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(1);
        Self::with_seed(seed)
    }
}

impl PetBrain {
    pub fn with_seed(seed: u64) -> Self {
        Self { rng: Mutex::new(seed.max(1)) }
    }

    /// xorshift64*: deterministic for tests, plenty for picking idle moves.
    fn next_f64(&self) -> f64 {
        let mut s = self.rng.lock().unwrap_or_else(|p| p.into_inner());
        *s ^= *s >> 12;
        *s ^= *s << 25;
        *s ^= *s >> 27;
        (s.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 11) as f64 / (1u64 << 53) as f64
    }

    fn between(&self, lo: f64, hi: f64) -> f64 {
        lo + (hi - lo) * self.next_f64()
    }
}

#[cfg_attr(feature = "ffi", uniffi::export)]
impl PetBrain {
    #[cfg_attr(feature = "ffi", uniffi::constructor)]
    pub fn new() -> std::sync::Arc<Self> {
        std::sync::Arc::new(Self::default())
    }

    pub fn next(&self, ctx: PetContext) -> PetPlan {
        let wait_ms = self.between(4_000.0, 9_000.0) as u32;
        let plan = |state: &str, duration_ms: u32| PetPlan { wait_ms, state: state.into(), duration_ms, dx: 0.0 };

        if ctx.idle_seconds >= SLEEP_AFTER_SECONDS {
            return PetPlan { wait_ms: 0, ..plan("sleep", 10_000) };
        }
        if ctx.reduce_motion {
            // Still frames only: an occasional blink is a single frame swap, nothing slides or bounces.
            return PetPlan { wait_ms: wait_ms + 4_000, ..plan("blink", 150) };
        }
        let roll = self.next_f64();
        if ctx.wander && roll < 0.22 {
            if let Some(walk) = self.walk(&ctx) {
                return PetPlan { wait_ms, ..walk };
            }
        }
        match roll {
            r if r < 0.50 => plan("idle", 2_000),
            r if r < 0.75 => plan("blink", 150),
            r if r < 0.93 => plan("look", 2_000),
            _ => plan("wave", 1_000),
        }
    }
}

impl PetBrain {
    /// A walk that stays inside `[min_x, max_x]`: turns around near an edge, and gives up when there is no room.
    fn walk(&self, ctx: &PetContext) -> Option<PetPlan> {
        let room_right = ctx.max_x - ctx.x;
        let room_left = ctx.x - ctx.min_x;
        let mut right = self.next_f64() < 0.5;
        if (right && room_right < MIN_WALK) || (!right && room_left < MIN_WALK) {
            right = !right;
        }
        let room = if right { room_right } else { room_left };
        if room < MIN_WALK {
            return None;
        }
        let distance = self.between(MIN_WALK, MAX_WALK).min(room);
        Some(PetPlan {
            wait_ms: 0,
            state: if right { "walk-right" } else { "walk-left" }.into(),
            duration_ms: (distance / WALK_SPEED * 1000.0) as u32,
            dx: if right { distance } else { -distance },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(x: f64) -> PetContext {
        PetContext { x, min_x: 0.0, max_x: 1000.0, reduce_motion: false, wander: true, idle_seconds: 0.0 }
    }

    fn walks(brain: &PetBrain, c: PetContext, n: usize) -> Vec<PetPlan> {
        (0..n).map(|_| brain.next(c)).filter(|p| p.state.starts_with("walk")).collect()
    }

    #[test]
    fn walks_never_leave_the_screen() {
        let brain = PetBrain::with_seed(7);
        for x in [0.0, 5.0, 500.0, 990.0, 1000.0] {
            for plan in walks(&brain, ctx(x), 400) {
                let end = x + plan.dx;
                assert!((0.0..=1000.0).contains(&end), "x {x} + {} leaves the screen", plan.dx);
                assert!(plan.dx.abs() >= MIN_WALK);
                assert_eq!(plan.state == "walk-right", plan.dx > 0.0);
                assert!((plan.duration_ms as f64 - plan.dx.abs() / WALK_SPEED * 1000.0).abs() <= 1.0);
            }
        }
    }

    #[test]
    fn at_an_edge_buddy_turns_around() {
        let brain = PetBrain::with_seed(3);
        let at_right = walks(&brain, ctx(1000.0), 400);
        assert!(!at_right.is_empty());
        assert!(at_right.iter().all(|p| p.dx < 0.0));
        let at_left = walks(&brain, ctx(0.0), 400);
        assert!(at_left.iter().all(|p| p.dx > 0.0));
    }

    #[test]
    fn no_room_or_no_wander_means_no_walk() {
        let brain = PetBrain::with_seed(11);
        let narrow = PetContext { min_x: 100.0, max_x: 110.0, ..ctx(105.0) };
        assert!(walks(&brain, narrow, 300).is_empty());
        assert!(walks(&brain, PetContext { wander: false, ..ctx(500.0) }, 300).is_empty());
    }

    #[test]
    fn reduce_motion_only_blinks_and_away_means_sleep() {
        let brain = PetBrain::with_seed(5);
        for _ in 0..100 {
            assert_eq!(brain.next(PetContext { reduce_motion: true, ..ctx(500.0) }).state, "blink");
        }
        assert_eq!(brain.next(PetContext { idle_seconds: 600.0, ..ctx(500.0) }).state, "sleep");
    }

    #[test]
    fn every_planned_state_exists_in_the_base_character() {
        let character = crate::pixel::builtin("buddy-base").unwrap();
        let brain = PetBrain::with_seed(9);
        for i in 0..500 {
            let c = PetContext { idle_seconds: if i % 50 == 0 { 900.0 } else { 0.0 }, ..ctx(500.0) };
            let plan = brain.next(c);
            assert!(character.states.contains_key(&plan.state), "missing state {}", plan.state);
        }
    }

    #[test]
    fn clamps_windows_into_the_area_corners_included() {
        let area = PetRect { x: 0.0, y: 25.0, width: 1440.0, height: 875.0 };
        let w = |x, y| PetRect { x, y, width: 96.0, height: 96.0 };
        assert_eq!(clamp_to_area(w(-50.0, -50.0), area), w(0.0, 25.0));
        assert_eq!(clamp_to_area(w(2000.0, 2000.0), area), w(1344.0, 804.0));
        assert_eq!(clamp_to_area(w(100.0, 100.0), area), w(100.0, 100.0));
        // A window bigger than the area pins to its origin.
        let big = PetRect { x: 30.0, y: 30.0, width: 2000.0, height: 2000.0 };
        assert_eq!(clamp_to_area(big, area), PetRect { x: 0.0, y: 25.0, ..big });
    }
}
