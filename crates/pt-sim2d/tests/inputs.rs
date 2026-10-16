//! Regression cases for fixed-time input and command-ordered uniform data.

use pt_core::gpu::Gpu;
use pt_core::{MaterialTable, Tool};
use pt_sim2d::{make_cell, Sim2d, Stroke, TickInput, TimedStroke};

fn world(gpu: &Gpu) -> Sim2d {
    let table = MaterialTable::core();
    let wall = table.expect_id("wall");
    let sim = Sim2d::new(gpu, table, 32, 16, 17);
    sim.upload(gpu, &vec![make_cell(wall, 0); 32 * 16], &[20.0; 32 * 16]);
    sim
}

fn exposure(x: f32, tool: Tool, seconds: f32, advance: bool) -> TickInput {
    TickInput {
        strokes: vec![TimedStroke {
            stroke: Stroke {
                from: [x, 8.0],
                to: [x, 8.0],
                radius: 2.0,
                tool,
            },
            duration_seconds: seconds,
        }],
        advance,
    }
}

#[test]
fn queued_construction_preserves_every_stroke_in_order() {
    let gpu = Gpu::headless().unwrap();
    let table = MaterialTable::core();
    let wall = table.expect_id("wall");
    let water = table.expect_id("water");
    let mut sim = Sim2d::new(&gpu, table, 32, 16, 1);
    for (x, tool) in [(6.0, Tool::Replace(wall)), (24.0, Tool::Replace(water))] {
        sim.stroke(Stroke {
            from: [x, 8.0],
            to: [x, 8.0],
            radius: 2.0,
            tool,
        });
    }
    sim.run(&gpu, 0);
    assert_eq!(sim.inspect(&gpu, 6, 8).unwrap().0, wall);
    assert_eq!(sim.inspect(&gpu, 24, 8).unwrap().0, water);
    assert_eq!(sim.tick, 0, "construction must not advance simulated time");
}

#[test]
fn multiple_encodes_keep_distinct_brush_parameters_until_submission() {
    let gpu = Gpu::headless().unwrap();
    let mut sim = world(&gpu);
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    sim.encode_actions(&gpu.queue, &mut encoder, &[exposure(6.0, Tool::Heat, 1.0 / 120.0, false)]);
    sim.encode_actions(&gpu.queue, &mut encoder, &[exposure(24.0, Tool::Cool, 1.0 / 240.0, false)]);
    gpu.queue.submit([encoder.finish()]);
    gpu.wait();
    // Wall heat capacity is one legacy energy unit/degree. No tick means no
    // conductive exchange; each location receives exactly its own exposure.
    assert!((sim.inspect(&gpu, 6, 8).unwrap().1 - 40.0).abs() < 1e-5);
    assert!((sim.inspect(&gpu, 24, 8).unwrap().1 - 10.0).abs() < 1e-5);
    assert_eq!(sim.tick, 0);
}

#[test]
fn thermal_exposure_is_additive_and_zero_exposure_is_inert() {
    let gpu = Gpu::headless().unwrap();
    let mut split = world(&gpu);
    let mut whole = world(&gpu);
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    split.encode_actions(
        &gpu.queue,
        &mut encoder,
        &[
            exposure(6.0, Tool::Heat, 1.0 / 120.0, false),
            exposure(6.0, Tool::Heat, 1.0 / 120.0, false),
            exposure(6.0, Tool::Cool, 0.0, false),
        ],
    );
    whole.encode_actions(&gpu.queue, &mut encoder, &[exposure(6.0, Tool::Heat, 1.0 / 60.0, false)]);
    gpu.queue.submit([encoder.finish()]);
    gpu.wait();
    assert_eq!(split.read_therm(&gpu), whole.read_therm(&gpu));
    assert!((split.inspect(&gpu, 6, 8).unwrap().1 - 60.0).abs() < 1e-5);
}

#[test]
fn interleaved_actions_match_separate_submissions() {
    let gpu = Gpu::headless().unwrap();
    let table = MaterialTable::core();
    // Moving/reacting material exercises the tick-dependent random seed and
    // block offset; a uniform stationary wall would not detect stale ticks.
    let scene = pt_sim2d::scenes::bench(&table, 64, 48, 23);
    let mut batched = Sim2d::new(&gpu, table.clone(), 64, 48, 17);
    let mut separate = Sim2d::new(&gpu, table, 64, 48, 17);
    batched.upload(&gpu, &scene.cells, &scene.temps);
    separate.upload(&gpu, &scene.cells, &scene.temps);
    let actions: Vec<_> = (0..12)
        .map(|i| {
            exposure(
                4.0 + i as f32 * 2.0,
                if i % 2 == 0 { Tool::Heat } else { Tool::Cool },
                1.0 / 600.0,
                true,
            )
        })
        .collect();
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    // Two encodes in one command buffer exercise parameter copies as well as
    // source ordering across a boundary between encoded batches.
    batched.encode_actions(&gpu.queue, &mut encoder, &actions[..5]);
    batched.encode_actions(&gpu.queue, &mut encoder, &actions[5..]);
    gpu.queue.submit([encoder.finish()]);
    for action in &actions {
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        separate.encode_actions(&gpu.queue, &mut encoder, std::slice::from_ref(action));
        gpu.queue.submit([encoder.finish()]);
    }
    gpu.wait();
    assert_eq!(batched.tick, 12);
    assert_eq!(batched.read_cells(&gpu), separate.read_cells(&gpu));
    assert_eq!(batched.read_therm(&gpu), separate.read_therm(&gpu));
}
