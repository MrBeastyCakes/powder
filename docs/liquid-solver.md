# Liquid solver: design record

Draft, 2026-10-21. Liquids today are cellular rules with a pressure push
bolted on. This record proposes replacing them with a free-surface liquid
solver whose flow comes from pressure and gravity and whose character
(water, oil, honey, lava) comes from each material's viscosity and density.
It sets out the method, the gates each stage passes before the next starts,
and the experiments that decide whether it works. It follows the conventions
of the [air physics overhaul](https://github.com/MrBeastyCakes/powder/pull/12),
which lists liquid mechanics as a separate capability needing its own
validation; the two must agree on units, time and coupling.

## Why the cellular liquid falls short

`main` at `cb1f128` (which includes PR #13's settling stopgap) moves liquid
cells with block rules in `crates/pt-sim2d/src/sim2d.wgsl`:

| Behaviour | Consequence |
|---|---|
| A liquid cell can only trade places with open space or something lighter | Liquid inside a body never moves; all sideways transport happens in the top layer and at the edges. A real column slumps as a whole |
| Sideways speed is 8 signed bits per cell, pushed by the coarse column weight (`liquid_hvel`) | Only an approximation of hydrostatic pressure: no incompressibility, so no U-tube levelling through a submerged pipe, no pressure under a lid, no flow driven from below |
| Vertical speed is 8 unsigned bits (falling only) | Liquid cannot be thrown upward by pressure or splash |
| Random sideways spreading (`spread` in the material pack) | Levelling is part diffusion; its speed depends on the spread chance, not on physics |
| Per-tick drag constants (7/8 on the ground, 15/16 on liquid) | The only "viscosity" is a fudge shared by every liquid; honey and water can only differ through `spread` |
| Movement is at most one cell per pass | Fast flow needs many passes; the pass count is a performance and a physics setting at once |

PR #13 made a 64-cell water column settle in 2 s instead of 16, but by
tuning the top layer. It cannot produce pressure-driven bulk flow, and every
new liquid needs hand tuning.

## Direction

**Liquid is an incompressible Newtonian fluid with a free surface.** Its
velocity lives on a coarse staggered (MAC) grid; its mass stays in the fine
material cells, which are what the player sees and paints. Each step:

1. **Fine to coarse.** Each coarse cell's liquid fraction and mean density
   come from the fine cells under it; their velocities are transferred to
   the grid faces.
2. **Forces.** Gravity, and later the gas pressure at the surface.
3. **Viscosity.** Implicit diffusion of velocity with each material's
   dynamic viscosity, so honey and lava ooze while water splashes.
4. **Pressure.** A projection that makes the liquid's velocity
   divergence-free, with the free surface held at the surrounding gas
   pressure and walls as boundaries.
5. **Coarse to fine.** Cells take their velocity from the grid and move
   along it; mass is conserved exactly because cells move, never appear.

The fine grid keeps per-pixel detail, mixing of several liquids, and
everything else in the sandbox; the coarse grid supplies the physics that
cells cannot do alone.

### Decisions fixed by this record

| Decision | Contract |
|---|---|
| Physics | Incompressible, Newtonian, variable density (several liquids at once), free surface; surface tension, wetting and cavitation out of scope until separately qualified |
| Ownership | Liquid mass is the fine cells; the coarse grid owns liquid velocity only. No second liquid inventory |
| Material data | `viscosity` (dynamic, Pa s) joins `density` in the material schema, with a schema version bump and defaults for old packs. `spread` stops applying to liquids once the solver is on |
| Units and time | The fixed simulation clock and world scale from the air overhaul (1 mm per fine cell is its reference profile). Gravity in m/s^2, viscosity in Pa s; no per-tick constants |
| Grid | Same coarse resolution as the air grid to start (4 x 4 fine cells), but separate buffers and a separate solve: PR #11 showed that one shared solve cannot handle a 1000:1 density ratio |
| Air | Liquid is an obstacle to the air (as now). The gas pressure at the liquid surface becomes the surface boundary condition once the gas solver provides an absolute pressure |
| Legacy | The cellular liquid rules stay as a backend until the new solver passes its gates, so the app keeps working throughout |

## Method

### Pressure and the free surface

Use the standard MAC free-surface projection (Harlow and Welch 1965;
Foster and Metaxas 1996; Bridson, *Fluid Simulation for Computer Graphics*,
2nd ed., 2015): pressure unknowns only in coarse cells containing liquid,
with the surrounding gas pressure applied at the surface through a
ghost-fluid treatment that places the surface inside the cell from the
liquid fraction (Gibou, Fedkiw, Cheng and Kang 2002; Enright, Nguyen,
Gibou and Fedkiw 2003). Walls enter through face-area fractions (Batty,
Bertails and Bridson 2007), which keeps thin walls and slopes from
snapping to the coarse grid.

Volume drifts in every grid-and-particle liquid: cells crowd or spread
apart over time. Add a correction term to the projection's right-hand side
that pushes coarse cells whose liquid fraction exceeds 1 apart and pulls
under-full interior cells together, with a stated time constant, and
measure drift in the acceptance suite.

The solve itself is a gate (below). PR #11's 16 Jacobi iterations were not
enough even for the air; candidate methods are red-black Gauss-Seidel,
preconditioned conjugate gradient, and a multigrid preconditioner, compared
on residual per millisecond on the phone.

### Viscosity

Water is nearly inviscid at this scale (kinematic viscosity 1e-6 m^2/s);
honey and lava are not (honey roughly 2 to 10 Pa s, basaltic lava roughly
10 to 1000 Pa s, both strongly temperature-dependent; sources to be pinned
in the material data record). Explicit viscosity is limited to
dt < dx^2 / (4 nu): for 4 mm grid cells that is far below one tick for
honey and orders of magnitude below for lava. So viscosity is implicit:
the variational formulation of Batty and Bridson (2008) handles the free
surface correctly (liquid coils and buckles instead of sticking to the
air) and stays stable for any viscosity. A temperature-dependent viscosity
(lava stiffening as it cools) is a later extension using the same operator.

### Moving the cells

Cells need signed two-dimensional velocity: a separate per-cell velocity
buffer (two 16-bit floats, 4 bytes per cell, about 6 MiB at 1080 x 1440)
instead of the 8-bit fields packed into the cell word. Transfers between
cells and grid follow the affine particle-in-cell method (Jiang, Schroeder,
Selle, Teran and Stomakhin 2015), which keeps rotation and loses less
energy than plain PIC without FLIP's noise; the fine cells play the part
of particles fixed to a lattice.

A cell moving several cells per tick claims its target in a scatter pass
with a deterministic priority (for example, the higher hash wins), and
losers stay put; no floating-point atomics, so runs repeat exactly.
Powders sinking through liquid and gases bubbling out keep their existing
block rules at first, with the liquid velocity carrying them.

## Gates

Each stage ends with evidence. A failed gate changes the approach rather
than expanding an unvalidated implementation.

| Stage | Work | Gate |
|---|---|---|
| L0. Units | Agree the world scale, gravity and clock with the air overhaul; convert material viscosity data with cited sources | A written profile both solvers use; no per-tick physical constants in the new code |
| L1. CPU reference | A small f64 Rust implementation in `pt-core` (grid, free surface, implicit viscosity, projection), no GPU | Passes the verification cases below; method choices recorded with measured errors |
| L2. GPU feasibility | f32 compute shaders for the grid stages; compare pressure and viscosity solvers on the Pixel 10 Pro XL | Meets the device budget below on the reference scenes, with accuracy within the CPU reference's tolerances |
| L3. Cells on the grid | Per-cell velocity buffer, transfers, multi-cell moves, volume correction; legacy backend switch | Validation cases pass in the app; determinism, conservation and sleeping-chunk tests still pass |
| L4. Materials | `viscosity` in the schema; water, oil, lava (and a honey-like test liquid) tuned only through physical data | Layering by density and the viscous cases pass with real property values |
| L5. Gas coupling | Gas pressure as the surface condition once the gas solver supplies it | The air overhaul's coupled cases: trapped air resists, a sealed tank drains differently from a vented one |

## Acceptance suite

Verification checks that the equations are solved correctly; validation
checks that they predict the experiments. Tolerances are proposals to
tighten from the CPU reference, not numbers to tune toward.

| Experiment | Reference | What must hold |
|---|---|---|
| Still tank | Hydrostatics | Pressure equals rho g h; spurious velocity stays below a stated bound for 10 simulated minutes |
| Plane Poiseuille flow | Analytic parabolic profile and flux | Profile and flux error fall with grid refinement (verifies the viscosity operator) |
| Plane Couette flow | Analytic linear profile | As above, with a moving wall |
| Dam break | Martin and Moyce (1952) measured surge front and column height; Ritter's solution for the early front | Front position against nondimensional time within a stated band of the measurements |
| U-tube | Oscillation period 2 pi sqrt(L / 2g) for a low-viscosity liquid; overdamped return for a viscous one | Levels equalise through a submerged connection; period within a stated tolerance |
| Viscous spreading | Huppert (1982): a 2D viscous gravity current's front advances as t^(1/5) | Measured exponent and prefactor for a honey-like liquid |
| Two liquids | Density ordering | Oil over water at rest, with a flat interface and no mixing drift |
| Conservation | Exact cell counts | Liquid mass exactly conserved; volume drift below 1% over 10 minutes of sloshing |
| Determinism | Same seed, same device | Bit-identical replay |
| Existing behaviour | Current tests | Tank jet, settling flat, still and falling liquid leave the air still, sleeping chunks |

## Device budget

Targets until measured on the phone. The air overhaul budgets 8 ms per
tick for the whole simulation and 4 ms for gas and coupling; the liquid
solver gets 2 ms at the 95th percentile on the reference workload
(1080 x 1440 fine cells, 270 x 360 coarse cells, a scene half full of
liquid), within the 8 ms. Coarse cells without liquid cost nothing, and
chunks of resting liquid can sleep once their residual velocity falls
below a threshold. If accuracy and budget conflict, keep the physics and
shrink the world or the simulation rate, reported as such.

## Open questions

1. How far should liquids go beyond the gameplay favourites? Water, oil
   and lava are in the pack; honey or another very viscous liquid is the
   natural test of viscosity and a fun material.
2. Should lava's viscosity depend on temperature from the start, or after
   the basic solver lands? It is what makes lava flows look right, but
   it is a coupling with the heat model.
3. Bubbles: when gas is trapped in liquid, does it rise as cells (as now)
   or as a resolved second fluid? The cell approach is far cheaper and
   fine until the gas coupling stage.

## References

- F. H. Harlow and J. E. Welch, "Numerical calculation of time-dependent
  viscous incompressible flow of fluid with free surface", Physics of
  Fluids, 1965.
- N. Foster and D. Metaxas, "Realistic animation of liquids", Graphical
  Models and Image Processing, 1996.
- R. Bridson, *Fluid Simulation for Computer Graphics*, 2nd ed., CRC
  Press, 2015.
- F. Gibou, R. Fedkiw, L.-T. Cheng and M. Kang, "A second-order-accurate
  symmetric discretization of the Poisson equation on irregular domains",
  Journal of Computational Physics, 2002.
- D. Enright, D. Nguyen, F. Gibou and R. Fedkiw, "Using the particle level
  set method and a second order accurate pressure boundary condition for
  free surface flows", ASME/JSME Joint Fluids Summer Engineering
  Conference, 2003.
- C. Batty, F. Bertails and R. Bridson, "A fast variational framework for
  accurate solid-fluid coupling", ACM SIGGRAPH, 2007.
- C. Batty and R. Bridson, "Accurate viscous free surfaces for buckling,
  coiling, and rotating liquids", Symposium on Computer Animation, 2008.
- C. Jiang, C. Schroeder, A. Selle, J. Teran and A. Stomakhin, "The affine
  particle-in-cell method", ACM SIGGRAPH, 2015.
- J. C. Martin and W. J. Moyce, "An experimental study of the collapse of
  liquid columns on a rigid horizontal plane", Philosophical Transactions
  of the Royal Society A, 1952.
- H. E. Huppert, "The propagation of two-dimensional and axisymmetric
  viscous gravity currents over a rigid horizontal surface", Journal of
  Fluid Mechanics, 1982.

Citations are from memory and are checked against the papers before the
method is fixed at L1.
