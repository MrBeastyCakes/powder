//! The shared core of both Phase 0 prototypes.
//!
//! - [`materials`]: the data-defined material format, its validation, and the
//!   compiled table the GPU reads.
//! - [`gpu`]: device setup shared by the app, the benchmark and the tests.
//! - [`COMMON_WGSL`]: shader code both simulations include, so heat, phase
//!   changes, reactions and colors behave the same in 2D and 3D.

pub mod gpu;
pub mod materials;

pub use materials::{GpuMaterial, Kind, MaterialError, MaterialTable, MAX_MATERIALS, NONE};

/// WGSL shared by the 2D and 3D simulations: the material table binding,
/// hashing, heat exchange, phase changes, reactions and color.
pub const COMMON_WGSL: &str = include_str!("wgsl/common.wgsl");

/// Joins the shared WGSL with a simulation's own shader source.
pub fn shader_source(sim_wgsl: &str) -> String {
    format!("{COMMON_WGSL}\n{sim_wgsl}")
}
