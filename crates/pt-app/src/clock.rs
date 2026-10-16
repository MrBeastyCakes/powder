//! Render-independent legacy clock. A reference tick is 1/60 simulated second.
//! Overload discards wall-time debt, never enlarges a physical timestep.

pub const REFERENCE_HZ: f64 = 60.0;
pub const TICK_SECONDS: f64 = 1.0 / REFERENCE_HZ;
/// Eight legacy ticks use 72 of the 2D backend's 128 parameter slots.
pub const MAX_TICKS_PER_FRAME: usize = 8;

#[derive(Debug, Clone, Copy)]
pub struct TickWindow {
    pub start: f64,
    pub end: f64,
    pub speed: f64,
}

#[derive(Default)]
pub struct FramePlan {
    pub ticks: Vec<TickWindow>,
    pub dropped_seconds: f64,
}

#[derive(Default)]
pub struct SimulationClock {
    last_wall: Option<f64>,
    cursor: f64,
    speed: f64,
    paused: bool,
    pub simulated_seconds: f64,
}

impl SimulationClock {
    /// Resume, replace a world, or discard the time spent in a benchmark.
    pub fn reset(&mut self, now: f64) {
        self.last_wall = Some(now);
        self.cursor = now;
    }

    pub fn frame(&mut self, now: f64, speed: f64, paused: bool, single_step: bool) -> FramePlan {
        let speed = if speed.is_finite() { speed.clamp(0.25, 2.0) } else { 1.0 };
        let mut plan = FramePlan::default();
        let changed = self.paused != paused || self.speed != speed;
        if self.last_wall.is_none() || changed {
            self.reset(now);
        }
        self.last_wall = Some(now);
        self.speed = speed;
        self.paused = paused;
        if paused {
            self.cursor = now;
            if single_step {
                // A manual step advances exactly one reference interval,
                // regardless of the selected playback speed.
                plan.ticks.push(TickWindow {
                    start: now,
                    end: now,
                    speed: 0.0,
                });
                self.simulated_seconds += TICK_SECONDS;
            }
            return plan;
        }
        let wall_step = TICK_SECONDS / speed;
        let available = ((now - self.cursor + wall_step * 1e-9) / wall_step).floor().max(0.0) as usize;
        let count = available.min(MAX_TICKS_PER_FRAME);
        if available > count {
            let skipped = (available - count) as f64 * wall_step;
            self.cursor += skipped;
            plan.dropped_seconds = skipped * speed;
        }
        for _ in 0..count {
            let start = self.cursor;
            self.cursor += wall_step;
            plan.ticks.push(TickWindow {
                start,
                end: self.cursor,
                speed,
            });
        }
        self.simulated_seconds += count as f64 * TICK_SECONDS;
        plan
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(fps: usize, speed: f64) -> (usize, f64) {
        let mut clock = SimulationClock::default();
        clock.frame(0.0, speed, false, false);
        let ticks = (1..=fps)
            .map(|f| clock.frame(f as f64 / fps as f64, speed, false, false).ticks.len())
            .sum();
        (ticks, clock.simulated_seconds)
    }

    #[test]
    fn rendering_and_speed_do_not_change_tick_size() {
        for fps in [30, 60, 90, 120, 144] {
            for speed in [0.25, 0.5, 1.0, 1.5, 2.0] {
                let (ticks, time) = run(fps, speed);
                assert_eq!(ticks, (60.0 * speed) as usize);
                assert!((time - speed).abs() < 1e-12);
            }
        }
    }

    #[test]
    fn pause_step_resume_and_overload_have_no_catch_up() {
        let mut clock = SimulationClock::default();
        clock.frame(0.0, 2.0, false, false);
        assert!(clock.frame(1.0, 2.0, true, false).ticks.is_empty());
        assert_eq!(clock.frame(10.0, 2.0, true, true).ticks.len(), 1);
        assert_eq!(clock.simulated_seconds, TICK_SECONDS);
        assert!(clock.frame(100.0, 2.0, false, false).ticks.is_empty());
        let plan = clock.frame(101.0, 2.0, false, false);
        assert_eq!(plan.ticks.len(), MAX_TICKS_PER_FRAME);
        assert!(plan.dropped_seconds > 1.8);
        assert!(clock.frame(101.0, 2.0, false, false).ticks.is_empty());
        clock.reset(1000.0);
        assert!(clock.frame(1000.0, 2.0, false, false).ticks.is_empty());
    }
}
