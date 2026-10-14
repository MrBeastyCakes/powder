//! Starting voxel worlds for playtests and benchmarks.

use pt_core::MaterialTable;

pub struct Scene {
    pub cells: Vec<u32>,
    pub temps: Vec<f32>,
}

struct Builder<'a> {
    dims: [u32; 3],
    table: &'a MaterialTable,
    scene: Scene,
    rng: u64,
}

impl<'a> Builder<'a> {
    fn new(table: &'a MaterialTable, dims: [u32; 3], seed: u64) -> Self {
        let n = dims.iter().map(|&d| d as usize).product();
        Builder {
            dims,
            table,
            scene: Scene {
                cells: vec![0; n],
                temps: vec![20.0; n],
            },
            rng: seed | 1,
        }
    }

    fn rand(&mut self) -> u32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;
        (self.rng >> 32) as u32
    }

    fn set(&mut self, x: u32, y: u32, z: u32, name: &str) {
        let [nx, ny, nz] = self.dims;
        if x >= nx || y >= ny || z >= nz {
            return;
        }
        let id = self.table.expect_id(name);
        let i = ((z * ny + y) * nx + x) as usize;
        let shade = self.rand() >> 24;
        self.scene.cells[i] = id | shade << 8;
        self.scene.temps[i] = self.table.def(id).spawn_temp;
    }

    /// Fills a box given as fractions of the world size.
    fn fill(&mut self, from: [f32; 3], to: [f32; 3], name: &str, density: f32) {
        let lo: Vec<u32> = (0..3).map(|a| (from[a] * self.dims[a] as f32) as u32).collect();
        let hi: Vec<u32> = (0..3).map(|a| (to[a] * self.dims[a] as f32).ceil() as u32).collect();
        for z in lo[2]..hi[2] {
            for y in lo[1]..hi[1] {
                for x in lo[0]..hi[0] {
                    if (self.rand() as f32 / u32::MAX as f32) < density {
                        self.set(x, y, z, name);
                    }
                }
            }
        }
    }
}

/// A sand mound, a walled pool of water with floating ice, a wooden tower
/// with an oil tank, and a lava blob about to fall.
pub fn demo(table: &MaterialTable, dims: [u32; 3], seed: u64) -> Scene {
    let mut b = Builder::new(table, dims, seed);
    b.fill([0.0, 0.0, 0.0], [1.0, 0.03, 1.0], "wall", 1.0);
    b.fill([0.08, 0.03, 0.08], [0.40, 0.35, 0.40], "sand", 0.8);
    // Pool walls.
    b.fill([0.50, 0.03, 0.50], [0.95, 0.20, 0.52], "wall", 1.0);
    b.fill([0.50, 0.03, 0.93], [0.95, 0.20, 0.95], "wall", 1.0);
    b.fill([0.50, 0.03, 0.50], [0.52, 0.20, 0.95], "wall", 1.0);
    b.fill([0.93, 0.03, 0.50], [0.95, 0.20, 0.95], "wall", 1.0);
    b.fill([0.52, 0.03, 0.52], [0.93, 0.16, 0.93], "water", 1.0);
    b.fill([0.65, 0.16, 0.65], [0.78, 0.24, 0.78], "ice", 1.0);
    b.fill([0.12, 0.03, 0.60], [0.20, 0.45, 0.68], "wood", 1.0);
    b.fill([0.10, 0.45, 0.58], [0.30, 0.47, 0.80], "wood", 1.0);
    b.fill([0.12, 0.47, 0.60], [0.28, 0.55, 0.78], "oil", 1.0);
    b.fill([0.30, 0.55, 0.15], [0.45, 0.65, 0.30], "lava", 1.0);
    b.scene
}

/// Every voxel in the top half busy: a random mix of falling powders and
/// liquids, plus lava, ice and burning wood on the floor.
pub fn bench(table: &MaterialTable, dims: [u32; 3], seed: u64) -> Scene {
    let mut b = Builder::new(table, dims, seed);
    let mix = ["sand", "water", "oil", "stone", "sand", "water"];
    let [nx, ny, nz] = dims;
    for z in 0..nz {
        for y in ny / 2..ny {
            for x in 0..nx {
                if b.rand() % 10 < 6 {
                    let name = mix[b.rand() as usize % mix.len()];
                    b.set(x, y, z, name);
                }
            }
        }
    }
    b.fill([0.0, 0.0, 0.0], [0.3, 0.1, 1.0], "lava", 1.0);
    b.fill([0.35, 0.0, 0.0], [0.65, 0.1, 1.0], "ice", 1.0);
    b.fill([0.7, 0.0, 0.0], [1.0, 0.1, 1.0], "wood", 1.0);
    b.fill([0.7, 0.1, 0.0], [1.0, 0.13, 1.0], "fire", 0.5);
    b.scene
}
