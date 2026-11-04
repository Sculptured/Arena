//! The arena clock of a live branch: milliseconds that run with real time × speed while agents
//! work, and stand still while the world is paused (saving or restoring).

use std::{
    sync::Mutex,
    time::{Duration, Instant},
};

/// The speed of new branch clocks: env `ARENA_CLOCK_SPEED` (a positive number), default 1.
pub fn speed_from_env() -> f64 {
    std::env::var("ARENA_CLOCK_SPEED")
        .ok()
        .and_then(|v| v.trim().parse::<f64>().ok())
        .filter(|s| s.is_finite() && *s >= 0.0)
        .unwrap_or(1.0)
}

struct State {
    /// Clock ms counted before `running_since`.
    counted: f64,
    /// When the clock last started running; None while paused.
    running_since: Option<Instant>,
    /// Open pauses; the clock runs when there are none.
    pauses: u32,
    /// Set by `freeze`: the clock never runs again.
    frozen: bool,
    speed: f64,
}

impl State {
    fn now_ms(&self) -> f64 {
        let running = self.running_since.map_or(Duration::ZERO, |t| t.elapsed());
        self.counted + running.as_secs_f64() * 1000.0 * self.speed
    }

    /// Folds the running time into `counted` and restarts from now (when running).
    fn settle(&mut self) {
        self.counted = self.now_ms();
        if self.running_since.is_some() {
            self.running_since = Some(Instant::now());
        }
    }
}

pub struct BranchClock {
    state: Mutex<State>,
}

impl BranchClock {
    /// A running clock that starts at `base_ms` and runs at `speed` × real time.
    pub fn new(base_ms: i64, speed: f64) -> BranchClock {
        BranchClock {
            state: Mutex::new(State {
                counted: base_ms as f64,
                running_since: Some(Instant::now()),
                pauses: 0,
                frozen: false,
                speed,
            }),
        }
    }

    /// A running clock from `base_ms` at the speed `ARENA_CLOCK_SPEED` sets.
    pub fn from_env(base_ms: i64) -> BranchClock {
        BranchClock::new(base_ms, speed_from_env())
    }

    pub fn now_ms(&self) -> i64 {
        self.state.lock().unwrap().now_ms() as i64
    }

    pub fn set_speed(&self, speed: f64) {
        let mut s = self.state.lock().unwrap();
        s.settle();
        s.speed = speed;
    }

    /// Stops the clock until the matching `resume` (pauses nest).
    pub fn pause(&self) {
        let mut s = self.state.lock().unwrap();
        s.settle();
        s.pauses += 1;
        s.running_since = None;
    }

    /// Ends one `pause`; the clock runs again once every pause has ended.
    pub fn resume(&self) {
        let mut s = self.state.lock().unwrap();
        s.pauses = s.pauses.saturating_sub(1);
        if s.pauses == 0 && !s.frozen && s.running_since.is_none() {
            s.running_since = Some(Instant::now());
        }
    }

    /// Stops the clock for good (a later `freeze` changes nothing; it never resumes).
    pub fn freeze(&self) {
        let mut s = self.state.lock().unwrap();
        s.settle();
        s.frozen = true;
        s.running_since = None;
    }

    /// Pauses the clock until the returned guard drops; the guard tells how long it was held.
    pub fn paused(&self) -> Paused<'_> {
        self.pause();
        Paused {
            clock: self,
            since: Instant::now(),
        }
    }
}

/// A pause of a `BranchClock`, ended on drop.
pub struct Paused<'a> {
    clock: &'a BranchClock,
    since: Instant,
}

impl Paused<'_> {
    /// Real milliseconds since the pause began.
    pub fn elapsed_ms(&self) -> i64 {
        self.since.elapsed().as_millis() as i64
    }

    /// Real milliseconds since the pause began, with the fraction kept.
    pub fn elapsed_ms_f64(&self) -> f64 {
        crate::overhead::ms_since(self.since)
    }
}

impl Drop for Paused<'_> {
    fn drop(&mut self) {
        self.clock.resume();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sleep(ms: u64) {
        std::thread::sleep(Duration::from_millis(ms));
    }

    #[test]
    fn runs_with_real_time_from_its_base() {
        let c = BranchClock::new(5000, 1.0);
        sleep(100);
        let t = c.now_ms();
        assert!((5090..5400).contains(&t), "{t}");
    }

    #[test]
    fn pauses_nest_and_stop_the_clock() {
        let c = BranchClock::new(0, 1.0);
        c.pause();
        let t = c.now_ms();
        {
            let p = c.paused();
            sleep(80);
            assert!(p.elapsed_ms() >= 80);
        }
        // Still paused: the outer pause has not ended.
        assert_eq!(c.now_ms(), t);
        c.resume();
        sleep(50);
        assert!(c.now_ms() >= t + 45);
        // A resume without a pause does nothing.
        c.resume();
        c.pause();
        let t = c.now_ms();
        sleep(30);
        assert_eq!(c.now_ms(), t);
    }

    #[test]
    fn a_frozen_clock_stays_frozen() {
        let c = BranchClock::new(0, 1.0);
        c.freeze();
        let t = c.now_ms();
        drop(c.paused());
        c.freeze();
        sleep(30);
        assert_eq!(c.now_ms(), t);
    }

    #[test]
    fn speed_scales_real_time() {
        let c = BranchClock::new(0, 4.0);
        sleep(50);
        assert!(c.now_ms() >= 190, "{}", c.now_ms());
        c.set_speed(0.0);
        let t = c.now_ms();
        sleep(30);
        assert_eq!(c.now_ms(), t);
    }
}
