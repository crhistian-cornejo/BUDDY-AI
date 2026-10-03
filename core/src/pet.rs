//! The mascot's idle behavior, decided once for both apps: when Buddy breathes, blinks, looks around, waves, walks
//! (and how far, inside the screen), sits down bored or falls asleep. Apps ask for the next plan, wait, play it and
//! ask again; nothing runs between plans, so idle stays near 0 % CPU.
//!
//! Sitting: after [`SIT_AFTER_SECONDS`] without being used (no hover, click or drag, chat closed, not working,
//! talking or walking) Buddy sits down (`sit-down`, 3 frames) and holds the still `sit` frame. While seated, a sparse
//! micro-move every 5–10 s (slow blink, a look aside, a foot swing, a yawn every ~30 s); with reduced motion, only
//! the still frame. Any use stands it up (`stand-up`, 2 frames): the apps do it at once, and the brain also answers
//! `stand-up` if asked while seated but in use.

use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

/// Walking speed in points per second.
pub const WALK_SPEED: f64 = 32.0;
/// Walks shorter than this are not worth it (Buddy turns around or does something else).
const MIN_WALK: f64 = 24.0;
const MAX_WALK: f64 = 180.0;
/// Buddy only walks when the user has not touched keyboard or mouse for this long: never while they work.
pub const WALK_WHEN_IDLE_SECONDS: f64 = 20.0;
/// Without any input for this long the user is away: Buddy sleeps.
pub const SLEEP_AFTER_SECONDS: f64 = 300.0;
/// Buddy sits down, bored, after this long without being used.
pub const SIT_AFTER_SECONDS: f64 = 10.0;
/// Seated with reduced motion there is nothing to animate: the brain is asked again only this often (to fall asleep).
const STILL_SEATED_WAIT_MS: u32 = 30_000;
const WALK_CHANCE: f64 = 0.22;

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
    /// Seconds since Buddy was last used: hovered, clicked or dragged, the chat closed, or Buddy last worked, talked
    /// or walked.
    pub untouched_seconds: f64,
    /// In use right now (the pointer is over Buddy or the chat is open): Buddy does not sit.
    pub engaged: bool,
    /// Buddy is seated now (the last plan rested on `sit`).
    pub sitting: bool,
}

/// What to do next: wait `wait_ms`, play `intro` once (when not empty), then play `state` for `duration_ms` while
/// moving `dx` points (walks only), and hold the still frame of `rest` (`idle` or `sit`) until the next plan.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct PetPlan {
    pub wait_ms: u32,
    pub state: String,
    pub duration_ms: u32,
    pub dx: f64,
    /// A short transition played once before `state`: `sit-down` or `stand-up`.
    pub intro: String,
    pub rest: String,
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
        let plan = |state: &str, duration_ms: u32| PetPlan {
            wait_ms,
            state: state.into(),
            duration_ms,
            dx: 0.0,
            intro: String::new(),
            rest: "idle".into(),
        };
        let untouched = if ctx.engaged { 0.0 } else { ctx.untouched_seconds.max(0.0) };

        if ctx.idle_seconds >= SLEEP_AFTER_SECONDS {
            // Dozes off seated: sits down first unless it already is (or motion is reduced).
            let intro = if ctx.sitting || ctx.reduce_motion { "" } else { "sit-down" };
            return PetPlan { wait_ms: 0, intro: intro.into(), rest: "sit".into(), ..plan("sleep", 10_000) };
        }
        if ctx.sitting {
            if untouched < SIT_AFTER_SECONDS {
                // In use again (the app usually stands Buddy up itself; this keeps both in step).
                let state = if ctx.reduce_motion { "idle" } else { "stand-up" };
                return PetPlan { wait_ms: 0, ..plan(state, 0) };
            }
            return self.seated(&ctx);
        }

        let wait_ms = if ctx.reduce_motion { wait_ms + 4_000 } else { wait_ms };
        if !ctx.engaged {
            let until_sit_ms = ((SIT_AFTER_SECONDS - untouched).max(0.0) * 1000.0) as u32;
            if until_sit_ms <= wait_ms {
                // Reduced motion: straight to the still seated frame, no transition.
                let (state, duration_ms) = if ctx.reduce_motion { ("sit", 0) } else { ("sit-down", 375) };
                return PetPlan { wait_ms: until_sit_ms, rest: "sit".into(), ..plan(state, duration_ms) };
            }
        }
        if ctx.reduce_motion {
            // Still frames only: an occasional blink is a single frame swap, nothing slides or bounces.
            return PetPlan { wait_ms, ..plan("blink", 150) };
        }
        let roll = self.next_f64();
        if ctx.wander && ctx.idle_seconds >= WALK_WHEN_IDLE_SECONDS && roll < WALK_CHANCE
            && let Some(walk) = self.walk(&ctx)
        {
            return PetPlan { wait_ms, ..walk };
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
    /// Seated and bored: a sparse micro-move every 5–10 s that ends back on `sit` (a yawn about every 30 s), or,
    /// now and then, it gets up for a walk. With reduced motion, only the still frame.
    fn seated(&self, ctx: &PetContext) -> PetPlan {
        let rest = |wait_ms: u32, state: &str, duration_ms: u32| PetPlan {
            wait_ms,
            state: state.into(),
            duration_ms,
            dx: 0.0,
            intro: String::new(),
            rest: "sit".into(),
        };
        if ctx.reduce_motion {
            return rest(STILL_SEATED_WAIT_MS, "sit", 0);
        }
        let wait_ms = self.between(5_000.0, 10_000.0) as u32;
        if ctx.wander && ctx.idle_seconds >= WALK_WHEN_IDLE_SECONDS && self.next_f64() < WALK_CHANCE
            && let Some(walk) = self.walk(ctx)
        {
            return PetPlan { wait_ms, intro: "stand-up".into(), ..walk };
        }
        match self.next_f64() {
            r if r < 0.40 => rest(wait_ms, "sit-blink", 500),
            r if r < 0.55 => rest(wait_ms, "sit-look", 2_000),
            r if r < 0.75 => rest(wait_ms, "sit-swing", 1_167),
            _ => rest(wait_ms, "sit-yawn", 1_250),
        }
    }

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
            intro: String::new(),
            rest: "idle".into(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(x: f64) -> PetContext {
        // Used a moment ago and standing: the classic idle life.
        PetContext {
            x,
            min_x: 0.0,
            max_x: 1000.0,
            reduce_motion: false,
            wander: true,
            idle_seconds: 60.0,
            untouched_seconds: 0.0,
            engaged: false,
            sitting: false,
        }
    }

    fn seated(c: PetContext) -> PetContext {
        PetContext { sitting: true, untouched_seconds: 60.0, ..c }
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
    fn never_walks_while_the_user_is_working() {
        let brain = PetBrain::with_seed(13);
        assert!(walks(&brain, PetContext { idle_seconds: 2.0, ..ctx(500.0) }, 300).is_empty());
    }

    #[test]
    fn reduce_motion_only_blinks_and_away_means_sleep() {
        let brain = PetBrain::with_seed(5);
        for _ in 0..100 {
            assert_eq!(brain.next(PetContext { reduce_motion: true, engaged: true, ..ctx(500.0) }).state, "blink");
        }
        let away = brain.next(PetContext { idle_seconds: 600.0, ..ctx(500.0) });
        assert_eq!((away.state.as_str(), away.intro.as_str(), away.rest.as_str()), ("sleep", "sit-down", "sit"));
        let away_seated = brain.next(PetContext { idle_seconds: 600.0, ..seated(ctx(500.0)) });
        assert_eq!((away_seated.intro.as_str(), away_seated.rest.as_str()), ("", "sit"));
    }

    #[test]
    fn sits_down_after_ten_quiet_seconds_and_not_before() {
        let brain = PetBrain::with_seed(17);
        for tenths in 0..200 {
            let untouched = tenths as f64 / 10.0;
            let plan = brain.next(PetContext { untouched_seconds: untouched, ..ctx(500.0) });
            let until_sit = ((SIT_AFTER_SECONDS - untouched).max(0.0) * 1000.0) as u32;
            if plan.state == "sit-down" {
                assert_eq!(plan.wait_ms, until_sit, "sits exactly when the 10 s are up");
                assert_eq!(plan.rest, "sit");
                assert!(plan.intro.is_empty());
            } else {
                assert!(plan.wait_ms < until_sit, "a {} plan would run past the sit time", plan.state);
                assert_eq!(plan.rest, "idle");
            }
        }
        assert_eq!(brain.next(PetContext { untouched_seconds: 12.0, ..ctx(500.0) }).wait_ms, 0);
    }

    #[test]
    fn never_sits_while_in_use() {
        let brain = PetBrain::with_seed(19);
        for _ in 0..300 {
            let plan = brain.next(PetContext { engaged: true, untouched_seconds: 120.0, ..ctx(500.0) });
            assert_eq!(plan.rest, "idle");
            assert!(!plan.state.starts_with("sit"));
        }
    }

    #[test]
    fn seated_buddy_is_bored_sparsely_and_yawns_every_half_minute_or_so() {
        let brain = PetBrain::with_seed(23);
        let (mut total_ms, mut yawns) = (0u64, 0u32);
        for _ in 0..4_000 {
            let plan = brain.next(PetContext { idle_seconds: 2.0, ..seated(ctx(500.0)) });
            assert!(["sit-blink", "sit-look", "sit-swing", "sit-yawn"].contains(&plan.state.as_str()), "{}", plan.state);
            assert_eq!(plan.rest, "sit");
            assert!(plan.wait_ms >= 5_000, "micro-moves are a few seconds apart");
            assert!(plan.duration_ms <= 2_000, "and short");
            total_ms += (plan.wait_ms + plan.duration_ms) as u64;
            yawns += (plan.state == "sit-yawn") as u32;
        }
        let every = total_ms as f64 / 1000.0 / yawns as f64;
        assert!((20.0..=40.0).contains(&every), "a yawn every {every:.1} s");
    }

    #[test]
    fn seated_buddy_stands_up_to_walk() {
        let brain = PetBrain::with_seed(29);
        let walks: Vec<_> = (0..400).map(|_| brain.next(seated(ctx(500.0)))).filter(|p| p.dx != 0.0).collect();
        assert!(!walks.is_empty());
        assert!(walks.iter().all(|p| p.intro == "stand-up" && p.rest == "idle"));
        assert!((0..300).all(|_| brain.next(PetContext { wander: false, ..seated(ctx(500.0)) }).dx == 0.0));
    }

    #[test]
    fn used_again_while_seated_means_stand_up() {
        let brain = PetBrain::with_seed(31);
        for c in [PetContext { engaged: true, ..seated(ctx(500.0)) }, PetContext { untouched_seconds: 1.0, ..seated(ctx(500.0)) }] {
            let plan = brain.next(c);
            assert_eq!((plan.state.as_str(), plan.wait_ms, plan.rest.as_str()), ("stand-up", 0, "idle"));
        }
    }

    #[test]
    fn reduce_motion_sits_still_without_transitions() {
        let brain = PetBrain::with_seed(37);
        let down = brain.next(PetContext { reduce_motion: true, untouched_seconds: 30.0, ..ctx(500.0) });
        assert_eq!((down.state.as_str(), down.intro.as_str(), down.rest.as_str()), ("sit", "", "sit"));
        for _ in 0..100 {
            let plan = brain.next(PetContext { reduce_motion: true, ..seated(ctx(500.0)) });
            assert_eq!((plan.state.as_str(), plan.rest.as_str(), plan.dx), ("sit", "sit", 0.0));
            assert!(plan.wait_ms >= STILL_SEATED_WAIT_MS);
        }
        let up = brain.next(PetContext { reduce_motion: true, engaged: true, ..seated(ctx(500.0)) });
        assert_eq!(up.state, "idle");
    }

    #[test]
    fn plans_serialize_in_camel_case_for_the_windows_frontend() {
        let json = serde_json::to_value(PetBrain::with_seed(1).next(ctx(500.0))).unwrap();
        for key in ["waitMs", "state", "durationMs", "dx", "intro", "rest"] {
            assert!(json.get(key).is_some(), "missing {key} in {json}");
        }
    }

    #[test]
    fn every_planned_state_exists_in_the_base_character() {
        let character = crate::pixel::builtin("buddy-base").unwrap();
        let brain = PetBrain::with_seed(9);
        let mut sitting = false;
        for i in 0..2_000 {
            let c = PetContext {
                idle_seconds: if i % 50 == 0 { 900.0 } else { 60.0 },
                untouched_seconds: (i % 40) as f64,
                engaged: i % 97 == 0,
                reduce_motion: i % 13 == 0,
                sitting,
                ..ctx(500.0)
            };
            let plan = brain.next(c);
            for state in [&plan.state, &plan.intro, &plan.rest] {
                assert!(state.is_empty() || character.states.contains_key(state), "missing state {state}");
            }
            assert!(["idle", "sit"].contains(&plan.rest.as_str()));
            sitting = plan.rest == "sit";
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
