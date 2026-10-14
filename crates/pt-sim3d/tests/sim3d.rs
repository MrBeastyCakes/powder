//! GPU tests for the 3D simulation.

use glam::Vec3;
use pt_core::gpu::Gpu;
use pt_core::MaterialTable;
use pt_sim3d::{Sim3d, Stroke3, Target, Tool};

fn gpu() -> Gpu {
    Gpu::headless().expect("a Vulkan or Metal device (install mesa-vulkan-drivers for a software one)")
}

fn count(cells: &[u32], mat: u32) -> usize {
    cells.iter().filter(|&&c| c & 0xff == mat).count()
}

fn world(dims: [u32; 3]) -> (Vec<u32>, Vec<f32>) {
    let n = dims.iter().map(|&d| d as usize).product();
    (vec![0; n], vec![20.0; n])
}

fn fill(dims: [u32; 3], cells: &mut [u32], temps: &mut [f32], lo: [u32; 3], hi: [u32; 3], mat: u32, temp: f32) {
    for z in lo[2]..hi[2] {
        for y in lo[1]..hi[1] {
            for x in lo[0]..hi[0] {
                let i = ((z * dims[1] + y) * dims[0] + x) as usize;
                cells[i] = mat;
                temps[i] = temp;
            }
        }
    }
}

#[test]
fn sand_falls_and_piles_in_3d() {
    let gpu = gpu();
    let t = MaterialTable::core();
    let sand = t.expect_id("sand");
    let dims = [32, 32, 32];
    let (mut cells, mut temps) = world(dims);
    fill(dims, &mut cells, &mut temps, [12, 24, 12], [20, 32, 20], sand, 20.0);
    let mut sim = Sim3d::new(&gpu, t.clone(), dims, 1);
    sim.upload(&gpu, &cells, &temps);
    sim.run(&gpu, 200);
    let out = sim.read_cells(&gpu);
    assert_eq!(count(&out, sand), 512);
    let upper: usize = (16..32u32)
        .flat_map(|y| (0..32u32).flat_map(move |z| (0..32u32).map(move |x| (x, y, z))))
        .filter(|&(x, y, z)| out[sim.index(x, y, z)] & 0xff == sand)
        .count();
    assert_eq!(upper, 0, "all sand should have fallen into the lower half");
    let floor: usize = (0..32u32)
        .flat_map(|z| (0..32u32).map(move |x| (x, z)))
        .filter(|&(x, z)| out[sim.index(x, 0, z)] & 0xff == sand)
        .count();
    assert!(floor > 64, "the pile should spread wider than its 8 x 8 start, floor has {floor}");
}

#[test]
fn materials_are_conserved_in_3d() {
    let gpu = gpu();
    let t = MaterialTable::core();
    let ids = ["sand", "water", "oil", "stone"].map(|n| t.expect_id(n));
    let dims = [40, 32, 24];
    let (mut cells, mut temps) = world(dims);
    for (k, &id) in ids.iter().enumerate() {
        let x = k as u32 * 10;
        fill(dims, &mut cells, &mut temps, [x, 16, 4], [x + 8, 30, 20], id, 20.0);
    }
    let before: Vec<usize> = ids.iter().map(|&id| count(&cells, id)).collect();
    let mut sim = Sim3d::new(&gpu, t.clone(), dims, 3);
    sim.upload(&gpu, &cells, &temps);
    sim.run(&gpu, 300);
    let out = sim.read_cells(&gpu);
    let after: Vec<usize> = ids.iter().map(|&id| count(&out, id)).collect();
    assert_eq!(before, after);
}

#[test]
fn same_seed_same_world_3d() {
    let gpu = gpu();
    let t = MaterialTable::core();
    let dims = [48, 40, 48];
    let scene = pt_sim3d::scenes::demo(&t, dims, 2);
    let run = || {
        let mut sim = Sim3d::new(&gpu, t.clone(), dims, 9);
        sim.upload(&gpu, &scene.cells, &scene.temps);
        sim.run(&gpu, 120);
        (sim.read_cells(&gpu), sim.read_therm(&gpu))
    };
    let a = run();
    let b = run();
    assert!(a.0 == b.0, "voxels differ between identical runs");
    assert!(a.1 == b.1, "temperatures differ between identical runs");
}

#[test]
fn ice_melts_in_warm_water_3d() {
    let gpu = gpu();
    let t = MaterialTable::core();
    let (ice, water) = (t.expect_id("ice"), t.expect_id("water"));
    let dims = [24, 24, 24];
    let (mut cells, mut temps) = world(dims);
    fill(dims, &mut cells, &mut temps, [0, 0, 0], [24, 24, 24], water, 60.0);
    fill(dims, &mut cells, &mut temps, [8, 8, 8], [16, 16, 16], ice, -5.0);
    let mut sim = Sim3d::new(&gpu, t.clone(), dims, 4);
    sim.upload(&gpu, &cells, &temps);
    sim.run(&gpu, 400);
    let out = sim.read_cells(&gpu);
    let therm = sim.read_therm(&gpu);
    assert!(count(&out, ice) < 512, "some ice should melt");
    assert!(therm[0][0] < 60.0, "the water should cool");
}

#[test]
fn brush_paints_on_surfaces_and_planes() {
    let gpu = gpu();
    let t = MaterialTable::core();
    let (wall, sand) = (t.expect_id("wall"), t.expect_id("sand"));
    let dims = [32, 32, 32];
    let (mut cells, mut temps) = world(dims);
    // A wall slab across the floor, 4 voxels thick.
    fill(dims, &mut cells, &mut temps, [0, 0, 0], [32, 4, 32], wall, 20.0);
    let mut sim = Sim3d::new(&gpu, t.clone(), dims, 5);
    sim.upload(&gpu, &cells, &temps);

    // Straight down from above the middle: lands on top of the slab.
    let down = Stroke3 {
        origin: Vec3::new(16.5, 100.0, 16.5),
        dir: Vec3::NEG_Y,
        radius: 2.0,
        tool: Tool::Replace(sand),
        target: Target::Surface,
    };
    sim.stroke(down);
    sim.run(&gpu, 0);
    let hit = sim.read_pick(&gpu).expect("the ray should hit the slab");
    assert!((hit - Vec3::new(16.5, 4.5, 16.5)).length() < 0.01, "hit at {hit}");
    assert!(count(&sim.read_cells(&gpu), sand) > 0);

    sim.plane_y = 20.0;
    sim.stroke(Stroke3 {
        target: Target::Plane,
        ..down
    });
    sim.run(&gpu, 0);
    let hit = sim.read_pick(&gpu).expect("the ray should cross the plane");
    assert!((hit.y - 20.5).abs() < 0.01, "plane hit at {hit}");
}
