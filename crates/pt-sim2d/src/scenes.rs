//! Starting worlds for playtests and benchmarks, generated on the CPU.

use crate::make_cell;
use pt_core::MaterialTable;

/// A world ready to upload: packed cells and temperatures.
pub struct Scene {
    pub cells: Vec<u32>,
    pub temps: Vec<f32>,
}

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 32) as u32
    }
    fn chance(&mut self, p: f32) -> bool {
        (self.next() as f32 / u32::MAX as f32) < p
    }
}

struct Painter<'a> {
    w: u32,
    h: u32,
    table: &'a MaterialTable,
    scene: Scene,
    rng: Rng,
}

impl<'a> Painter<'a> {
    fn new(table: &'a MaterialTable, w: u32, h: u32, seed: u64) -> Self {
        let n = (w * h) as usize;
        Painter {
            w,
            h,
            table,
            scene: Scene {
                cells: vec![0; n],
                temps: vec![20.0; n],
            },
            rng: Rng(seed | 1),
        }
    }

    fn set(&mut self, x: u32, y: u32, name: &str) {
        if x >= self.w || y >= self.h {
            return;
        }
        let id = self.table.expect_id(name);
        let i = (y * self.w + x) as usize;
        let shade = (self.rng.next() >> 24) as u8;
        self.scene.cells[i] = make_cell(id, shade);
        self.scene.temps[i] = self.table.def(id).spawn_temp;
    }

    /// Fills a rectangle given as fractions of the world size.
    fn rect(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, name: &str, density: f32) {
        let (ax, ay) = ((x0 * self.w as f32) as u32, (y0 * self.h as f32) as u32);
        let (bx, by) = ((x1 * self.w as f32) as u32, (y1 * self.h as f32) as u32);
        for y in ay..by {
            for x in ax..bx {
                if self.rng.chance(density) {
                    self.set(x, y, name);
                }
            }
        }
    }
}

/// A small lab: a sand hill, a water pool with an ice block, a wooden shelf
/// with oil above a lava pocket, and walls to hold it together.
pub fn demo(table: &MaterialTable, w: u32, h: u32, seed: u64) -> Scene {
    let mut p = Painter::new(table, w, h, seed);
    p.rect(0.0, 0.94, 1.0, 1.0, "wall", 1.0);
    p.rect(0.05, 0.60, 0.40, 0.94, "sand", 0.9);
    p.rect(0.48, 0.70, 0.50, 0.94, "wall", 1.0);
    p.rect(0.50, 0.74, 0.95, 0.94, "water", 1.0);
    p.rect(0.62, 0.66, 0.74, 0.76, "ice", 1.0);
    p.rect(0.08, 0.30, 0.45, 0.32, "wood", 1.0);
    p.rect(0.10, 0.24, 0.40, 0.30, "oil", 1.0);
    p.rect(0.55, 0.35, 0.80, 0.37, "stone", 1.0);
    p.rect(0.60, 0.20, 0.75, 0.30, "lava", 1.0);
    p.scene
}

/// A worst case for speed: the whole world is active. The top half is a
/// random mix of falling powders and liquids, with hot lava, burning oil and
/// melting ice so every layer has work to do.
pub fn bench(table: &MaterialTable, w: u32, h: u32, seed: u64) -> Scene {
    let mut p = Painter::new(table, w, h, seed);
    let mix = ["sand", "water", "oil", "stone", "sand", "water"];
    for y in 0..h / 2 {
        for x in 0..w {
            if p.rng.chance(0.6) {
                let name = mix[(p.rng.next() as usize) % mix.len()];
                p.set(x, y, name);
            }
        }
    }
    p.rect(0.0, 0.85, 0.3, 1.0, "lava", 1.0);
    p.rect(0.35, 0.85, 0.65, 1.0, "ice", 1.0);
    p.rect(0.7, 0.85, 1.0, 1.0, "wood", 1.0);
    p.rect(0.7, 0.80, 1.0, 0.85, "fire", 0.5);
    p.scene
}
