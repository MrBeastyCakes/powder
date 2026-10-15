//! The standard Phase 0 benchmark. The same scenes and measurements run from
//! the command line on a desktop GPU and from the app's Benchmark button on a
//! phone, so results across devices compare directly.

use std::time::Instant;

use pt_core::gpu::Gpu;
use pt_core::MaterialTable;
use pt_sim2d::Sim2d;
use pt_sim3d::Sim3d;

/// One measured configuration.
#[derive(Debug, Clone)]
pub struct BenchResult {
    pub label: String,
    pub cells: u64,
    pub ms_per_tick: f64,
}

impl BenchResult {
    /// Ticks per second the simulation alone could sustain.
    pub fn ticks_per_second(&self) -> f64 {
        1000.0 / self.ms_per_tick
    }

    /// Whether the simulation alone fits in a 60 fps frame with room left for
    /// drawing (half the 16.7 ms budget).
    pub fn fits_60fps(&self) -> bool {
        self.ms_per_tick <= 8.3
    }
}

/// Times `ticks` ticks of the 2D bench scene at `width` x `height`.
pub fn bench_2d(gpu: &Gpu, table: &MaterialTable, width: u32, height: u32, ticks: u32) -> BenchResult {
    bench_2d_with(gpu, table, width, height, ticks, Scene2d::Busy, pt_sim2d::DEFAULT_SUBSTEPS)
}

/// The 2D scenes the benchmark can time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scene2d {
    /// The worst case: the whole top half falling, with lava, ice and fire.
    Busy,
    /// The demo lab, after it has had a few seconds to settle.
    Demo,
}

/// Times `ticks` ticks of a 2D scene with `substeps` block movement passes
/// per tick.
pub fn bench_2d_with(gpu: &Gpu, table: &MaterialTable, width: u32, height: u32, ticks: u32, scene: Scene2d, substeps: u32) -> BenchResult {
    let (world, warmup, name) = match scene {
        Scene2d::Busy => (pt_sim2d::scenes::bench(table, width, height, 42), 8, "busy"),
        Scene2d::Demo => (pt_sim2d::scenes::demo(table, width, height, 1), 180, "demo"),
    };
    let mut sim = Sim2d::new(gpu, table.clone(), width, height, 42);
    sim.move_substeps = substeps;
    sim.upload(gpu, &world.cells, &world.temps);
    sim.run(gpu, warmup);
    let start = Instant::now();
    sim.run(gpu, ticks);
    let ms = start.elapsed().as_secs_f64() * 1000.0 / ticks as f64;
    BenchResult {
        label: format!("2D {width} x {height}, {name}, {substeps} passes"),
        cells: sim.cell_count(),
        ms_per_tick: ms,
    }
}

/// The 2D rows the app's Benchmark button runs: the worst case at the
/// default and fewer movement passes, which together show what the passes
/// cost on this GPU, and a typical scene.
pub fn bench_2d_breakdown(gpu: &Gpu, table: &MaterialTable, width: u32, height: u32, ticks: u32) -> Vec<BenchResult> {
    let normal = pt_sim2d::DEFAULT_SUBSTEPS;
    vec![
        bench_2d_with(gpu, table, width, height, ticks, Scene2d::Busy, normal),
        bench_2d_with(gpu, table, width, height, ticks, Scene2d::Busy, normal / 2),
        bench_2d_with(gpu, table, width, height, ticks, Scene2d::Busy, 0),
        bench_2d_with(gpu, table, width, height, ticks, Scene2d::Demo, normal),
    ]
}

/// Times `ticks` ticks of the 3D bench scene in an `n`-cubed world. Returns
/// `None` when the device cannot hold a world that size.
pub fn bench_3d(gpu: &Gpu, table: &MaterialTable, n: u32, ticks: u32) -> Option<BenchResult> {
    let dims = [n, n, n];
    if !Sim3d::fits(gpu, dims) {
        return None;
    }
    let scene = pt_sim3d::scenes::bench(table, dims, 42);
    let mut sim = Sim3d::new(gpu, table.clone(), dims, 42);
    sim.upload(gpu, &scene.cells, &scene.temps);
    sim.run(gpu, 4);
    let start = Instant::now();
    sim.run(gpu, ticks);
    let ms = start.elapsed().as_secs_f64() * 1000.0 / ticks as f64;
    Some(BenchResult {
        label: format!("3D {n}^3"),
        cells: sim.cell_count(),
        ms_per_tick: ms,
    })
}

/// Formats results as a Markdown table.
pub fn table(gpu_name: &str, results: &[BenchResult]) -> String {
    let mut out =
        format!("GPU: {gpu_name}\n\n| World | Cells | ms per tick | Ticks per second | Sim fits 60 fps |\n|---|---:|---:|---:|---|\n");
    for r in results {
        out += &format!(
            "| {} | {:.2} M | {:.2} | {:.0} | {} |\n",
            r.label,
            r.cells as f64 / 1e6,
            r.ms_per_tick,
            r.ticks_per_second(),
            if r.fits_60fps() { "yes" } else { "no" }
        );
    }
    out
}
