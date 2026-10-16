//! The shared core of both Phase 0 prototypes.
//!
//! - [`materials`]: the data-defined material format, its validation, and the
//!   compiled table the GPU reads.
//! - [`gpu`]: device setup shared by the app, the benchmark and the tests.
//! - [`COMMON_WGSL`]: shader code both simulations include, so heat, phase
//!   changes, reactions and colors behave the same in 2D and 3D.

pub mod gas;
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

/// What a brush does, in both prototypes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    /// Draw a material into empty cells.
    Paint(u32),
    /// Draw a material over anything.
    Replace(u32),
    Erase,
    /// Add energy, like a blowtorch.
    Heat,
    /// Remove energy.
    Cool,
}

impl Tool {
    /// The brush shaders' mode number and material id.
    pub fn mode_and_material(self) -> (u32, u32) {
        match self {
            Tool::Paint(m) => (0, m),
            Tool::Replace(m) => (1, m),
            Tool::Erase => (2, 0),
            Tool::Heat => (3, 0),
            Tool::Cool => (4, 0),
        }
    }
}

/// How full a fresh brush stroke of each kind is drawn: scattered grains look
/// like poured sand, solids and liquids are drawn solid.
pub fn fill_for(kind: Kind) -> f32 {
    match kind {
        Kind::Powder => 0.55,
        Kind::Gas => 0.35,
        _ => 1.0,
    }
}
