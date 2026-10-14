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
