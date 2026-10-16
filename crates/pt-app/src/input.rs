//! Timestamped pointer paths. Thermal tools integrate simulated duration;
//! construction preserves every event, including taps between redraws.

use std::collections::VecDeque;

use pt_core::Tool;
use pt_sim3d::Target;

use crate::clock::{TickWindow, TICK_SECONDS};
use crate::Mode;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BrushSample {
    pub position: [f32; 2],
    pub radius: f32,
    pub tool: Tool,
    pub mode: Mode,
    pub target: Target,
}

impl BrushSample {
    fn thermal(self) -> bool {
        matches!(self.tool, Tool::Heat | Tool::Cool)
    }
}

#[derive(Debug, Clone, Copy)]
pub struct BrushSegment {
    pub from: [f32; 2],
    pub sample: BrushSample,
    pub duration_seconds: f32,
}

#[derive(Debug, Clone, Copy)]
enum Change {
    Sample(BrushSample),
    End,
}

#[derive(Default)]
pub struct InputTimeline {
    pending: VecDeque<(f64, Change)>,
    active: Option<BrushSample>,
    accounted_until: f64,
}

impl InputTimeline {
    pub fn sample(&mut self, at: f64, sample: BrushSample) {
        self.pending.push_back((at, Change::Sample(sample)));
    }

    pub fn end(&mut self, at: f64) {
        self.pending.push_back((at, Change::End));
    }

    fn advance_to(&mut self, end: f64, speed: f64, strokes: &mut Vec<BrushSegment>) {
        while self.pending.front().is_some_and(|(at, _)| *at <= end + 1e-12) {
            let (at, change) = self.pending.pop_front().unwrap();
            let dt = (at - self.accounted_until).max(0.0) * speed;
            if let Some(active) = self.active.filter(|s| s.thermal()) {
                if dt > 0.0 {
                    let mut sample = active;
                    if let Change::Sample(next) = change {
                        if next.tool == active.tool && next.mode == active.mode {
                            sample.position = next.position;
                        }
                    }
                    strokes.push(BrushSegment {
                        from: active.position,
                        sample,
                        duration_seconds: dt as f32,
                    });
                }
            }
            match change {
                Change::Sample(sample) => {
                    if !sample.thermal() {
                        let from = self
                            .active
                            .filter(|s| s.mode == sample.mode && s.tool == sample.tool)
                            .map_or(sample.position, |s| s.position);
                        strokes.push(BrushSegment {
                            from,
                            sample,
                            duration_seconds: 0.0,
                        });
                    }
                    self.active = Some(sample);
                }
                Change::End => self.active = None,
            }
            self.accounted_until = self.accounted_until.max(at);
        }
        if let Some(sample) = self.active.filter(|s| s.thermal()) {
            let dt = (end - self.accounted_until).max(0.0) * speed;
            if dt > 0.0 {
                strokes.push(BrushSegment {
                    from: sample.position,
                    sample,
                    duration_seconds: dt as f32,
                });
            }
        }
        self.accounted_until = self.accounted_until.max(end);
    }

    pub fn tick(&mut self, tick: TickWindow) -> Vec<BrushSegment> {
        let mut strokes = Vec::new();
        // Dropped wall time and paused time may update the brush path, but
        // never inject thermal energy into a later accepted tick.
        if tick.start > self.accounted_until {
            self.advance_to(tick.start, 0.0, &mut strokes);
        }
        self.advance_to(tick.end, tick.speed, &mut strokes);
        if let Some(sample) = self.active {
            if !sample.thermal() || tick.speed == 0.0 {
                strokes.push(BrushSegment {
                    from: sample.position,
                    sample,
                    duration_seconds: if sample.thermal() { TICK_SECONDS as f32 } else { 0.0 },
                });
            }
        }
        strokes
    }

    pub fn construction(&mut self, now: f64) -> Vec<BrushSegment> {
        let mut strokes = Vec::new();
        self.advance_to(now, 0.0, &mut strokes);
        strokes
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::SimulationClock;

    fn sample(tool: Tool, x: f32) -> BrushSample {
        BrushSample {
            position: [x, 2.0],
            radius: 1.0,
            tool,
            mode: Mode::TwoD,
            target: Target::Surface,
        }
    }

    type RecordedExposure = (u64, u64, u32, u32, u32);

    fn replay(fps: usize) -> (Vec<RecordedExposure>, f64) {
        let mut timeline = InputTimeline::default();
        // Queue the same timestamped samples before asking for any frames.
        timeline.sample(0.003, sample(Tool::Heat, 1.0));
        timeline.sample(0.041, sample(Tool::Heat, 4.0));
        timeline.sample(0.079, sample(Tool::Heat, 9.0));
        timeline.end(0.113);
        let mut clock = SimulationClock::default();
        clock.frame(0.0, 1.0, false, false);
        let mut output = Vec::new();
        let mut duration = 0.0;
        for frame in 1..=fps {
            for tick in clock.frame(frame as f64 / fps as f64, 1.0, false, false).ticks {
                for stroke in timeline.tick(tick) {
                    duration += stroke.duration_seconds as f64;
                    output.push((
                        tick.start.to_bits(),
                        tick.end.to_bits(),
                        stroke.from[0].to_bits(),
                        stroke.sample.position[0].to_bits(),
                        stroke.duration_seconds.to_bits(),
                    ));
                }
            }
        }
        (output, duration)
    }

    #[test]
    fn timestamped_thermal_path_is_identical_at_every_refresh_rate() {
        let expected = replay(60);
        assert!((expected.1 - 0.110).abs() < 1e-8);
        for fps in [30, 90, 120, 144] {
            assert_eq!(replay(fps), expected);
        }
    }

    #[test]
    fn pause_preserves_multiple_construction_segments_but_applies_no_heat() {
        let mut timeline = InputTimeline::default();
        timeline.sample(0.01, sample(Tool::Paint(2), 1.0));
        timeline.sample(0.02, sample(Tool::Paint(2), 2.0));
        timeline.sample(0.03, sample(Tool::Paint(2), 3.0));
        timeline.end(0.04);
        timeline.sample(0.05, sample(Tool::Heat, 3.0));
        let strokes = timeline.construction(1.0);
        assert_eq!(strokes.len(), 3);
        assert_eq!(strokes[1].from, [1.0, 2.0]);
        assert_eq!(strokes[2].sample.position, [3.0, 2.0]);
        assert!(timeline.construction(2.0).is_empty());
        let strokes = timeline.tick(TickWindow {
            start: 2.0,
            end: 2.0,
            speed: 0.0,
        });
        assert_eq!(strokes.len(), 1);
        assert_eq!(strokes[0].duration_seconds, TICK_SECONDS as f32);
        timeline.end(2.1);
        timeline.construction(2.2);
        assert!(timeline
            .tick(TickWindow {
                start: 2.2,
                end: 2.2,
                speed: 0.0
            })
            .is_empty());
    }

    #[test]
    fn discarded_wall_time_does_not_duplicate_sources() {
        let mut timeline = InputTimeline::default();
        timeline.sample(0.0, sample(Tool::Heat, 1.0));
        let strokes = timeline.tick(TickWindow {
            start: 10.0,
            end: 10.0 + TICK_SECONDS,
            speed: 1.0,
        });
        let energy_time: f32 = strokes.iter().map(|s| s.duration_seconds).sum();
        assert!((energy_time as f64 - TICK_SECONDS).abs() < 1e-8);
        assert!(timeline
            .tick(TickWindow {
                start: 10.0 + TICK_SECONDS,
                end: 10.0 + TICK_SECONDS,
                speed: 1.0
            })
            .is_empty());
    }

    #[test]
    fn stationary_tool_and_radius_changes_apply_at_their_timestamp() {
        let mut timeline = InputTimeline::default();
        timeline.sample(0.0, sample(Tool::Heat, 1.0));
        let mut cool = sample(Tool::Cool, 1.0);
        cool.radius = 3.0;
        timeline.sample(0.025, cool);
        timeline.end(0.050);
        let mut heat_seconds = 0.0;
        let mut cool_seconds = 0.0;
        for tick in 0..3 {
            for stroke in timeline.tick(TickWindow {
                start: tick as f64 * TICK_SECONDS,
                end: (tick + 1) as f64 * TICK_SECONDS,
                speed: 1.0,
            }) {
                if stroke.sample.tool == Tool::Heat {
                    assert_eq!(stroke.sample.radius, 1.0);
                    heat_seconds += stroke.duration_seconds as f64;
                } else {
                    assert_eq!(stroke.sample.tool, Tool::Cool);
                    assert_eq!(stroke.sample.radius, 3.0);
                    cool_seconds += stroke.duration_seconds as f64;
                }
            }
        }
        assert!((heat_seconds - 0.025).abs() < 1e-8);
        assert!((cool_seconds - 0.025).abs() < 1e-8);
    }
}
