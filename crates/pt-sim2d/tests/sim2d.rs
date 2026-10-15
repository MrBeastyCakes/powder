//! GPU tests for the 2D simulation. They run on any Vulkan device, including
//! the software rasterizer (lavapipe) in CI.

use pt_core::gpu::Gpu;
use pt_core::MaterialTable;
use pt_sim2d::{make_cell, Sim2d, Stroke, Tool};

fn gpu() -> Gpu {
    Gpu::headless().expect("a Vulkan or Metal device (install mesa-vulkan-drivers for a software one)")
}

fn count(cells: &[u32], mat: u32) -> usize {
    cells.iter().filter(|&&c| c & 0xff == mat).count()
}

struct World {
    w: u32,
    h: u32,
    cells: Vec<u32>,
    temps: Vec<f32>,
}

impl World {
    fn new(w: u32, h: u32) -> World {
        World {
            w,
            h,
            cells: vec![0; (w * h) as usize],
            temps: vec![20.0; (w * h) as usize],
        }
    }
    fn fill(&mut self, x0: u32, y0: u32, x1: u32, y1: u32, mat: u32, temp: f32) {
        for y in y0..y1 {
            for x in x0..x1 {
                let i = (y * self.w + x) as usize;
                self.cells[i] = make_cell(mat, 0);
                self.temps[i] = temp;
            }
        }
    }
    fn sim(&self, gpu: &Gpu, table: &MaterialTable, seed: u32) -> Sim2d {
        let sim = Sim2d::new(gpu, table.clone(), self.w, self.h, seed);
        sim.upload(gpu, &self.cells, &self.temps);
        sim
    }
}

#[test]
fn sand_falls_and_is_conserved() {
    let gpu = gpu();
    let t = MaterialTable::core();
    let sand = t.expect_id("sand");
    let mut w = World::new(64, 64);
    w.fill(20, 0, 44, 10, sand, 20.0);
    let mut sim = w.sim(&gpu, &t, 1);
    sim.run(&gpu, 300);
    let cells = sim.read_cells(&gpu);
    assert_eq!(count(&cells, sand), 240);
    let top_half = &cells[..(64 * 32) as usize];
    assert_eq!(count(top_half, sand), 0, "all sand should have fallen to the bottom half");
    // It piles: the bottom row is wider than the original 24 cells.
    let bottom = &cells[(64 * 63) as usize..];
    assert!(count(bottom, sand) > 24, "sand should spread into a pile");
}

#[test]
fn materials_are_conserved_when_nothing_reacts() {
    let gpu = gpu();
    let t = MaterialTable::core();
    let ids = ["sand", "water", "oil", "stone"].map(|n| t.expect_id(n));
    let mut w = World::new(96, 80);
    for (k, &id) in ids.iter().enumerate() {
        w.fill(k as u32 * 24, 0, k as u32 * 24 + 20, 30, id, 20.0);
    }
    let before: Vec<usize> = ids.iter().map(|&id| count(&w.cells, id)).collect();
    let mut sim = w.sim(&gpu, &t, 7);
    sim.run(&gpu, 400);
    let cells = sim.read_cells(&gpu);
    let after: Vec<usize> = ids.iter().map(|&id| count(&cells, id)).collect();
    assert_eq!(before, after);
}

#[test]
fn liquids_layer_by_density() {
    let gpu = gpu();
    let t = MaterialTable::core();
    let (water, oil) = (t.expect_id("water"), t.expect_id("oil"));
    // Oil starts under the water and should float up through it.
    let mut w = World::new(32, 64);
    w.fill(0, 34, 32, 64, oil, 20.0);
    w.fill(0, 4, 32, 34, water, 20.0);
    let mut sim = w.sim(&gpu, &t, 3);
    sim.run(&gpu, 1500);
    let cells = sim.read_cells(&gpu);
    let mean_row = |mat: u32| {
        let rows: Vec<usize> = cells
            .iter()
            .enumerate()
            .filter(|(_, &c)| c & 0xff == mat)
            .map(|(i, _)| i / 32)
            .collect();
        rows.iter().sum::<usize>() as f32 / rows.len() as f32
    };
    assert!(
        mean_row(oil) < mean_row(water),
        "oil ({}) should sit above water ({})",
        mean_row(oil),
        mean_row(water)
    );
}

#[test]
fn same_seed_same_world() {
    let gpu = gpu();
    let t = MaterialTable::core();
    let scene = pt_sim2d::scenes::demo(&t, 120, 90, 5);
    let run = || {
        let mut sim = Sim2d::new(&gpu, t.clone(), 120, 90, 11);
        sim.upload(&gpu, &scene.cells, &scene.temps);
        sim.run(&gpu, 200);
        (sim.read_cells(&gpu), sim.read_therm(&gpu))
    };
    let (a_cells, a_therm) = run();
    let (b_cells, b_therm) = run();
    assert!(a_cells == b_cells, "cells differ between identical runs");
    assert!(a_therm == b_therm, "temperatures differ between identical runs");
}

/// Energy relative to ice at 0 degrees, for a world of only ice and water.
fn ice_water_energy(t: &MaterialTable, cells: &[u32], therm: &[[f32; 2]]) -> f64 {
    let (ice, water) = (t.expect_id("ice"), t.expect_id("water"));
    let gi = t.gpu[ice as usize];
    let gw = t.gpu[water as usize];
    cells
        .iter()
        .zip(therm)
        .map(|(&c, th)| {
            let (temp, latent) = (th[0] as f64, th[1] as f64);
            match c & 0xff {
                m if m == ice => gi.heat_capacity as f64 * temp + latent,
                m if m == water => gi.up_latent as f64 + gw.heat_capacity as f64 * temp + latent,
                m => panic!("unexpected material {m}"),
            }
        })
        .sum()
}

#[test]
fn melting_ice_cools_water_and_conserves_energy() {
    let gpu = gpu();
    let t = MaterialTable::core();
    let (ice, water) = (t.expect_id("ice"), t.expect_id("water"));
    let mut w = World::new(48, 48);
    w.fill(0, 0, 48, 48, water, 40.0);
    w.fill(16, 16, 32, 32, ice, -10.0);
    let energy_before = {
        let therm: Vec<[f32; 2]> = w.temps.iter().map(|&t| [t, 0.0]).collect();
        ice_water_energy(&t, &w.cells, &therm)
    };
    let mut sim = w.sim(&gpu, &t, 2);
    // Ice is a solid and water flows, so pin the water by giving the world
    // no air: nothing can move, only heat flows.
    sim.run(&gpu, 600);
    let cells = sim.read_cells(&gpu);
    let therm = sim.read_therm(&gpu);
    let energy_after = ice_water_energy(&t, &cells, &therm);
    let drift = ((energy_after - energy_before) / energy_before).abs();
    assert!(
        drift < 1e-3,
        "energy drifted by {:.4}% ({energy_before} -> {energy_after})",
        drift * 100.0
    );
    assert!(count(&cells, ice) < 256, "some ice should have melted");
    let corner_temp = therm[0][0];
    assert!(corner_temp < 40.0, "the water should have cooled, corner is {corner_temp}");
}

#[test]
fn water_boils_and_steam_rises() {
    let gpu = gpu();
    let t = MaterialTable::core();
    let (water, steam) = (t.expect_id("water"), t.expect_id("steam"));
    let mut w = World::new(32, 48);
    w.fill(0, 40, 32, 48, water, 98.0);
    let mut sim = w.sim(&gpu, &t, 4);
    for _ in 0..60 {
        sim.stroke(Stroke {
            from: [0.0, 46.0],
            to: [32.0, 46.0],
            radius: 3.0,
            tool: Tool::Heat,
        });
        sim.run(&gpu, 2);
    }
    sim.run(&gpu, 60);
    let cells = sim.read_cells(&gpu);
    assert!(count(&cells, steam) > 0, "heating should make steam");
    let top = &cells[..(32 * 24) as usize];
    assert!(count(top, steam) > 0, "steam should rise");
    assert!(count(&cells, water) < 256);
}

#[test]
fn lava_cools_into_stone_in_water() {
    let gpu = gpu();
    let t = MaterialTable::core();
    let (water, lava, stone) = (t.expect_id("water"), t.expect_id("lava"), t.expect_id("stone"));
    let mut w = World::new(40, 40);
    w.fill(0, 10, 40, 40, water, 20.0);
    w.fill(16, 20, 24, 28, lava, 1050.0);
    let mut sim = w.sim(&gpu, &t, 9);
    sim.run(&gpu, 800);
    let cells = sim.read_cells(&gpu);
    assert!(count(&cells, stone) > 0, "lava should solidify into stone");
}

#[test]
fn fire_spreads_through_wood() {
    let gpu = gpu();
    let t = MaterialTable::core();
    let (wood, fire) = (t.expect_id("wood"), t.expect_id("fire"));
    // A plank lit from below at one end: the flames collect under it.
    let mut w = World::new(64, 32);
    w.fill(0, 12, 64, 16, wood, 20.0);
    w.fill(0, 26, 8, 32, fire, 1000.0);
    let before = count(&w.cells, wood);
    let mut sim = w.sim(&gpu, &t, 5);
    sim.run(&gpu, 900);
    let cells = sim.read_cells(&gpu);
    assert!(
        count(&cells, wood) < before - 30,
        "wood should burn ({} of {before} left)",
        count(&cells, wood)
    );
}

#[test]
fn paint_and_erase() {
    let gpu = gpu();
    let t = MaterialTable::core();
    let wall = t.expect_id("wall");
    let mut sim = Sim2d::new(&gpu, t.clone(), 64, 64, 1);
    sim.stroke(Stroke {
        from: [10.0, 10.0],
        to: [50.0, 10.0],
        radius: 2.0,
        tool: Tool::Paint(wall),
    });
    sim.run(&gpu, 0);
    let n = count(&sim.read_cells(&gpu), wall);
    assert!(n > 120 && n < 220, "a 40 x 4 capsule of wall, got {n}");
    sim.stroke(Stroke {
        from: [0.0, 10.0],
        to: [64.0, 10.0],
        radius: 3.0,
        tool: Tool::Erase,
    });
    sim.run(&gpu, 0);
    assert_eq!(count(&sim.read_cells(&gpu), wall), 0);
    assert_eq!(sim.inspect(&gpu, 30, 10).map(|c| c.0), Some(0));
}

#[test]
fn sand_drops_through_the_sim_area_in_two_seconds() {
    // The Phase 1 gate: on a Pixel's 1,440-cell-tall sim area, dropped sand
    // reaches the floor within 2 seconds, which is 120 ticks at 60 a second.
    let gpu = gpu();
    let t = MaterialTable::core();
    let sand = t.expect_id("sand");
    let (wd, ht) = (128, 1440);
    let mut w = World::new(wd, ht);
    w.fill(56, 0, 72, 4, sand, 20.0);
    let mut sim = w.sim(&gpu, &t, 3);
    sim.run(&gpu, 120);
    let cells = sim.read_cells(&gpu);
    let floor = &cells[((ht - 32) * wd) as usize..];
    let landed = count(floor, sand);
    assert!(landed >= 64 * 9 / 10, "only {landed} of 64 grains reached the floor in 120 ticks");
}

#[test]
fn settled_chunks_sleep_and_paint_wakes_them() {
    let gpu = gpu();
    let t = MaterialTable::core();
    let sand = t.expect_id("sand");
    let wall = t.expect_id("wall");
    let mut w = World::new(256, 256);
    w.fill(0, 248, 256, 256, wall, 20.0);
    w.fill(32, 100, 224, 140, sand, 20.0);
    let mut sim = w.sim(&gpu, &t, 5);
    assert!(sim.read_awake(&gpu).iter().all(|&a| a), "a new world starts awake");

    sim.run(&gpu, 600);
    let awake = sim.read_awake(&gpu);
    assert!(awake.iter().all(|&a| !a), "settled sand should sleep: {awake:?}");

    // Sand painted into the sleeping world wakes its chunks and falls.
    sim.stroke(Stroke {
        from: [128.0, 10.0],
        to: [128.0, 10.0],
        radius: 4.0,
        tool: Tool::Paint(sand),
    });
    sim.run(&gpu, 2);
    assert!(sim.read_awake(&gpu).iter().any(|&a| a), "painting should wake the world");
    let before = count(&sim.read_cells(&gpu)[..(256 * 20) as usize], sand);
    assert!(before > 0);
    sim.run(&gpu, 120);
    let after = count(&sim.read_cells(&gpu)[..(256 * 20) as usize], sand);
    assert_eq!(after, 0, "painted sand should fall out of the top rows");
}

#[test]
fn melting_wakes_a_sleeping_chunk() {
    // Ice resting on a floor sleeps; when hot lava melts it, the water must
    // still flow even though nothing moved in that chunk before.
    let gpu = gpu();
    let t = MaterialTable::core();
    let ice = t.expect_id("ice");
    let water = t.expect_id("water");
    let wall = t.expect_id("wall");
    let mut w = World::new(128, 64);
    w.fill(0, 60, 128, 64, wall, 20.0);
    w.fill(60, 40, 68, 60, ice, 200.0);
    let mut sim = w.sim(&gpu, &t, 9);
    sim.run(&gpu, 600);
    let cells = sim.read_cells(&gpu);
    assert!(count(&cells, ice) == 0, "the hot ice should have melted");
    // Melted water spreads into a thin layer instead of standing as a column.
    let column_top = &cells[(45 * 128) as usize..(46 * 128) as usize];
    assert_eq!(count(column_top, water), 0, "melted water should flow out, not stand still");
}

#[test]
fn falls_accelerate() {
    // A grain dropped from rest covers less than half as far in its first 30
    // ticks as in its first 60; at a constant speed it would be exactly half.
    let gpu = gpu();
    let t = MaterialTable::core();
    let sand = t.expect_id("sand");
    let (wd, ht) = (64, 1440);
    let mut w = World::new(wd, ht);
    w.fill(32, 0, 33, 1, sand, 20.0);
    let mut sim = w.sim(&gpu, &t, 4);
    let depth = |sim: &Sim2d| {
        let cells = sim.read_cells(&gpu);
        cells.iter().position(|&c| c & 0xff == sand).map(|i| i as u32 / wd).unwrap()
    };
    sim.run(&gpu, 30);
    let d30 = depth(&sim);
    sim.run(&gpu, 30);
    let d60 = depth(&sim);
    assert!(d30 > 0 && d60 > d30, "the grain should be falling: {d30}, {d60}");
    assert!(
        d30 * 10 < d60 * 4,
        "falls should speed up: {d30} rows after 30 ticks, {d60} after 60"
    );
}

/// Mean of one wind component over a block of air cells.
fn mean_wind(wind: &[[f32; 2]], aw: usize, cols: std::ops::Range<usize>, rows: std::ops::Range<usize>, axis: usize) -> f32 {
    let n = (cols.len() * rows.len()) as f32;
    rows.flat_map(|j| cols.clone().map(move |i| (i, j)))
        .map(|(i, j)| wind[j * aw + i][axis])
        .sum::<f32>()
        / n
}

#[test]
fn hot_air_rises_over_lava() {
    let gpu = gpu();
    let t = MaterialTable::core();
    let mut w = World::new(128, 128);
    // A lava pool in a wall basin, so the lava itself stays put.
    w.fill(40, 108, 88, 128, t.expect_id("wall"), 20.0);
    w.fill(48, 112, 80, 128, t.expect_id("lava"), 1200.0);
    let mut sim = w.sim(&gpu, &t, 3);
    sim.run(&gpu, 240);
    let wind = sim.read_wind(&gpu);
    let [aw, _] = sim.air_dims();
    // Air cells 12..20 sit over the lava; rows 22..27 are just above it.
    let over = mean_wind(&wind, aw as usize, 12..20, 22..27, 1);
    let beside = mean_wind(&wind, aw as usize, 0..8, 22..27, 1);
    assert!(over < -0.03, "air over the lava should rise, got {over}");
    assert!(beside > over, "air beside the lava should not rise as fast: {beside} vs {over}");
}

#[test]
fn water_jets_through_a_hole_in_its_tank() {
    let gpu = gpu();
    let t = MaterialTable::core();
    let wall = t.expect_id("wall");
    let water = t.expect_id("water");
    let mut w = World::new(256, 192);
    w.fill(0, 0, 4, 192, wall, 20.0);
    w.fill(60, 0, 64, 192, wall, 20.0);
    w.fill(4, 20, 60, 192, water, 20.0);
    // A hole halfway down the tank's right wall.
    w.fill(60, 100, 64, 108, 0, 20.0);
    let mut sim = w.sim(&gpu, &t, 3);
    sim.run(&gpu, 30);
    let cells = sim.read_cells(&gpu);
    // Where the water falling outside the tank is, a little below the hole.
    let xs: Vec<u32> = (120..160u32)
        .flat_map(|y| (64..256u32).map(move |x| (x, y)))
        .filter(|&(x, y)| cells[(y * 256 + x) as usize] & 0xff == water)
        .map(|(x, _)| x)
        .collect();
    let mean = xs.iter().sum::<u32>() as f32 / xs.len().max(1) as f32;
    // Poured water runs down the wall (mean x about 74 with the push turned
    // off); a jet lands well away from it.
    assert!(
        xs.len() > 50 && mean > 85.0,
        "the water should jet away from the tank, mean x {mean}"
    );
}

#[test]
fn still_water_leaves_the_air_still() {
    let gpu = gpu();
    let t = MaterialTable::core();
    let mut w = World::new(128, 128);
    w.fill(4, 60, 124, 128, t.expect_id("wall"), 20.0);
    w.fill(8, 64, 120, 124, t.expect_id("water"), 20.0);
    let mut sim = w.sim(&gpu, &t, 3);
    sim.run(&gpu, 240);
    let max = sim.read_wind(&gpu).iter().map(|v| v[0].hypot(v[1])).fold(0.0f32, f32::max);
    assert!(max < 0.1, "a tank of still water should not stir the air, max wind {max}");
}
