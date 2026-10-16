# Air numerical-method gate: research record

2026-10-16. Supports change 3 of the [accepted overhaul](air-physics-overhaul.md). A bounded periodic 1D CPU comparator accompanies this record, **not a selected production solver or a completed mixture-flow qualification**. No device measurements accompany it.

## Executable comparison and observed limits

Run `cargo run -p pt-core --example air_flow_comparator` and
`cargo test -p pt-core --example air_flow_comparator`. The example derives a
first-order periodic uniform-grid pressure system by substituting the momentum
update into Cordier–Degond–Kumbaro equations (2.12)–(2.15), with alpha=1 and the
published Lax–Friedrichs explicit dissipation. It retains the old-time kinetic
energy in the closure, uses a small dense pivoted CPU solve, and reports both
stored-energy conservation and disagreement with new-time physical energy/EOS.
Scaled nondimensional units and constant gamma=1.4 keep this experiment separate
from the physical species/property model. It has no walls, sources, mixtures,
vacuum treatment, gravity, or production performance claim.

| Case | Resolution / epsilon | Normalized pressure error | Relative stored-energy drift | Maximum pressure/EOS discrepancy |
|---|---|---:|---:|---:|
| Small acoustic wave | 32 / 1 | 0.04555 | 5.51e-15 | 1.42e-11 |
| Small acoustic wave | 64 / 1 | 0.02317 | 1.08e-14 | 6.81e-12 |
| Small acoustic wave | 32 / 0.1 | 0.00644 | 7.28e-15 | 1.31e-11 |
| Small acoustic wave | 64 / 0.1 | 0.00296 | 1.60e-14 | 6.53e-12 |
| Underresolved acoustic step | 64 / 0.001 | 0.55561 | -5.44e-14 | 2.79e-9 |
| Low-Mach density contact | 64 / 0.001 | 1.70e-13 | 1.28e-13 | 1.55e-13 |

Acoustic error is pressure L1 divided by the initial pressure-wave amplitude;
the contact row instead reports maximum departure from constant unit pressure.
The small-wave cases compare the same analytical propagation interval across
resolutions. The underresolved case intentionally uses one much larger acoustic
step; it is a failure-of-accuracy demonstration, not a matched-runtime contest.
Stored energy is conservative to roundoff here, while the physical EOS defect
is nonzero. Four tests check uniform-flow preservation, transactional rejection,
small-wave convergence/conservation, and the distinction between implicit
stability and acoustic accuracy. These limited successes leave the production
method gate open: exact physical-energy coupling, mixture contacts, shock and
rarefaction cases, boundaries, and transport still require qualification.

## Recommendation and candidates

Start a small double-precision CPU reference with the degree-zero staggered scheme of **Tavelli–Dumbser (2017), §3.3, equations (55)–(58)** [1]. Spatial degree zero gives cell averages rather than high-order DG polynomials; temporal degree zero keeps the first comparison simple. Reproduce the paper's mesh/operators, pressure/energy coupling, and constant-heat-capacity benchmarks before changing geometry or thermodynamics. This is a reference experiment, not a commitment to triangular production grids. Its attraction is the conservative total-energy update and an outer Picard iteration that reconciles pressure, enthalpy, and physical kinetic energy.

| Published candidate | Useful property | Cost or unresolved issue for Powder |
|---|---|---|
| Tavelli–Dumbser 2017 [1]; staggered DG extension of Dumbser–Casulli 2016 [2] | Explicit material transport; implicit pressure and enthalpy transport; conservative mass, momentum, and total energy. Published acoustic, low-speed, and shock cases. | Primal/dual transfers and kinetic-energy evaluation must be reproduced. The paper assumes internal energy linear in pressure for its linear pressure solve. Variable heat capacity/species need an explicit extension and validation. |
| Dumbser–Casulli 2016 [2], semi-implicit FV with general EOS | Directly addresses finite volumes, staggered placement, and nonlinear thermodynamics. | Obtain and inspect its complete FV formulation before claiming a faithful implementation. Its DOI and repository record were verified; the repository PDF returned HTTP 403 during this research. |
| Cordier–Degond–Kumbaro 2012 [3], asymptotic-preserving all-speed scheme | Conservative pressure equation derived from momentum and energy; explicit transport and implicit acoustics; general-EOS formulation and low-Mach analysis. | Equations (2.12)–(2.22) use old-time kinetic energy in the thermodynamic closure and a pressure split parameter. Their closure cannot be silently replaced by new-time kinetic energy. The presented Navier–Stokes model omits viscous-work energy flux. |
| Boscheri–Pareschi 2021 [4], cell-centered pressure-based IMEX FV | Compact structured stencil, general EOS, high-order extensions, material-speed numerical viscosity; attractive GPU comparison. | Equations (26), (54)–(58) use a time-mixed kinetic term. Compact pressure stencil and its energy correction are integral to the method, not interchangeable Laplacians. Shock stabilization is separate; no mixture/vacuum positivity guarantee follows from the family name. |

The last two are useful comparisons, but their published energy closures are a poor place to improvise when our first requirement is a canonical physical energy inventory. None of these citations establishes positivity, entropy behavior, multi-species contact accuracy, tiny cut-volume robustness, or mobile performance for our application.

## Stages that the CPU reference must reproduce

The following is a compact reading guide to [1], not a replacement for its operator definitions. Let `q = rho v` be momentum on the dual mesh, `U = rho E` total energy density on the primal mesh, `K = rho |v|²/2`, and `H = e + p/rho`. In its matrix notation, `M` are space-time mass matrices; `Q` is the pressure-gradient coupling; `E_ij` couples enthalpy/momentum to energy flux; `D_ij` is the divergence coupling. Their definitions and primal/dual averaging are given in §§3.1–3.3. Time and geometric factors are already included; do not insert another `dt` into these matrices.

For Picard iterate `k+1`, equations (55)–(58) amount to:

```text
M_i rho_i^(k+1) = M_i^- rho_i^n - F_rho,i^(k)

M_j q_j^(k+1) = M_j^- q_j^n - G_j^(k)
                  - Q_rj p_r^(k+1) - Q_lj p_l^(k+1)

M_i [rho e(p^(k+1), rho^(k+1)) + K_i^(k)]
  + sum_j E_ij H_j^(k) q_j^(k+1)
  = M_i^- U_i^n - F_K,i^(k) + sum_j D_ij [w_j^(k) + J_j^(k)]

M_i U_i^(n+1)
  = M_i^- U_i^n - F_K,i - sum_j E_ij H_j q_j^(k+1)
    + sum_j D_ij [w_j + J_j]
```

Here `F_rho` and `F_K` are the integrated explicit convective residuals, `G` includes momentum convection/viscous stress, `w = sigma v`, and `J = lambda grad(T)` is the paper's heat-flux quantity. Substitute the momentum update into the third equation to obtain the pressure system, exactly as in equation (56). Then update momentum, kinetic energy and enthalpy, and repeat. The final line is the conservative energy flux update; resetting energy from an independently solved temperature would bypass it. The paper uses a small fixed Picard count for its accuracy construction; Powder additionally needs a measured EOS/physical-energy consistency residual and rejection when the bounded iteration cannot meet it. That is an application requirement, not a convergence guarantee supplied by the paper.

For explicit convection, reproduce [1] equation (22): centered physical flux minus a Rusanov jump term with material normal speed. Do not substitute `|v|+c` everywhere: its low-Mach numerical diffusion changes the comparison. Conversely, omitting sound-speed dissipation does not prove strong-shock stability. The paper's §3.5 troubled-cell/artificial-viscosity treatment is a separate algorithm to reproduce and budget; it can reintroduce sound-speed-related costs. For the degree-zero reference, begin with first-order transport, and explicitly qualify its shock behavior before adding higher order.

At degree zero in time the paper reports a symmetric positive-definite pressure system; it does not establish this for arbitrary geometry, all thermodynamics, or higher temporal order. Verify symmetry/definiteness of the assembled CPU matrix before choosing conjugate gradients. Compare a direct small-system solution with iterative residuals first. Compare multigrid/preconditioning only after correctness is established.

## Thermodynamic and acceptance details required before extension

First reproduce constant `gamma` ideal-gas cases. Then introduce the foundation's actual EOS: at known `rho` and composition `Y`, set `T = p/(rho R_mix)` and evaluate `rho e(T,Y)` with the same species reference energies as the conserved inventory. Its pressure derivative at fixed `rho,Y` is `c_v(T,Y)/R_mix`; variable heat capacity makes the pressure equation nonlinear. A bracketed/damped nonlinear method and declared temperature range must be designed and checked against the reference energy inversion. The degree-zero linear-EOS paper alone does not validate this extension.

Use the same shared conservative mass flux to transport species, with face fractions summing to one; demonstrate nonnegative species and exact mixture consistency. Varying composition/heat capacity can generate pressure errors at advected material contacts even in conservative methods. Include a constant-pressure, constant-velocity composition-contact test before advertising mixture flow. No production positivity treatment is selected here. Initially reject invalid candidate steps and retry from the accepted state; a retry limit is required, and failure remains observable. Density/temperature floors cannot create hidden inventory. True vacuum and geometry deletion remain disabled until separately qualified.

Also test invariance under changing species reference-energy zeros: identical physical initial states must produce identical pressure/motion after the corresponding inventory transformation. Formation-energy offsets cannot simply be placed into an enthalpy coefficient whose positivity was assumed by the pressure solver. Any separation of transported reference energy from sensible/acoustic energy must retain exact conservative accounting; it needs a derivation before mixture production work.

Record mass/species, physical kinetic/internal/total energy, source/boundary flux, pressure-equation residual, EOS residual, and iteration counts for every accepted step. Conservation of a numerically defined energy alone does not demonstrate agreement with `rho e(T,Y) + rho |v|²/2`. Neither cellwise clipping nor blending two conservative solutions with different weights per cell automatically preserves global conservation; any limiter must show its shared-face accounting.

## CPU experiment sequence and exit decision

1. **Constant-state and sealed heating:** no sources, then metered uniform energy input; periodic and rigid adiabatic walls. Establish zero spurious flow and independent EOS/energy accounting before pressure-driven flow.
2. **Contact/advection and low-Mach vortex:** constant pressure/velocity density contact, then a smooth vortex at successively lower Mach number. Measure pressure noise, kinetic-energy decay, convergence, and cost. Repeat contacts after adding species/variable heat capacity.
3. **Nonuniform heating and vent:** localized metered heat in a sealed box, then a zero-thickness valve between pre-existing volumes. Check integrated chamber energy/mass, low-Mach expansion and vent flow against a refined independent compressible reference. Uniform heating is insufficient evidence for this case.
4. **Small acoustic pulse:** compare `c² = gamma R T` for the constant-heat-capacity gas, reflected-wave phase/amplitude, and error under independent spatial/time refinement. Test large material-CFL steps explicitly to expose acoustic damping.
5. **Sod and rarefaction:** exact ideal-gas Riemann solution; shock/contact positions and L1 errors at several resolutions, positivity and source-free conservation. Strong rarefaction approaches the declared low-density limit; it must not silently install a density floor.
6. **Mixture and transport extension:** variable-heat-capacity heating/vent/contact cases, then analytic conduction, diffusion, and viscous cases. Add gravity only with a hydrostatic-rest test and explicit gravitational-work ledger.

For each case save full parameters, initial/final state, reference provenance, error norms, residual/iteration history, and equal-simulated-time results. Start with the accepted plan's conservation/heating thresholds; report observed convergence rather than inventing passing thresholds afterward. Shock and acoustic errors need published numerical results before their release tolerances are fixed. **Exit change 3 only after a specific version of the stages, energy closure, boundaries, limiter, EOS, and timestep rules passes these comparisons.** A failed candidate leads to another CPU comparison, not a GPU rewrite.

## Mobile feasibility implications

At 4 mm spacing, `c = 343 m/s` and acoustic CFL 0.5 give `dt ≈ 5.83 microseconds`, about **2,859 explicit steps per 1/60 second**. Semi-implicit acoustics removes that stability limit; it does not preserve a sharp waveform at arbitrary `dt`. Material transport still costs about **84 steps per tick at 10 m/s**, before pressure iterations. At 270 × 360 gas cells this is already over eight million cell updates per tick per pass. A sharp blast adds wave-accuracy requirements. The proposed 4 ms gas budget therefore needs measurement; it cannot be inferred from unconditional acoustic stability.

The GPU gate should compare f64 CPU and f32 GPU observables, not just field finiteness. Around 101325 Pa, adjacent f32 values are about 0.0078 Pa apart; the dynamic pressure of 0.1 m/s air is only about 0.006 Pa. A reference-pressure split may be necessary for gentle winds and must retain the EOS/global energy ledger. Benchmark pressure residual **and physical error per millisecond**, with complete Picard/nonlinear solves, geometry transfers, rollback storage and reductions included. Portable WGSL kernels must use multiple dispatches for global dependencies and cannot assume f64, floating-point atomics or cross-workgroup barriers. Actual Pixel measurements and all-alignment thin-wall tests select the production layout afterward.

## Primary sources

1. M. Tavelli and M. Dumbser, *A pressure-based semi-implicit space–time discontinuous Galerkin method on staggered unstructured meshes for the solution of the compressible Navier–Stokes equations at all Mach numbers*, JCP 341 (2017), 341–376. [DOI](https://doi.org/10.1016/j.jcp.2017.03.030), [inspected author manuscript](https://arxiv.org/abs/1612.08866). Especially §§3.2–3.5 and equations (22), (49)–(58).
2. M. Dumbser and V. Casulli, *A conservative, weakly nonlinear semi-implicit finite volume scheme for the compressible Navier–Stokes equations with general equation of state*, AMC 272 (2016), 479–497. [DOI](https://doi.org/10.1016/j.amc.2015.08.042), [repository record](https://hdl.handle.net/11572/148704). Full method not independently inspected in this pass.
3. F. Cordier, P. Degond and A. Kumbaro, *An Asymptotic-Preserving all-speed scheme for the Euler and Navier–Stokes equations*, JCP 231 (2012), 5685–5704. [DOI](https://doi.org/10.1016/j.jcp.2012.04.025), [inspected manuscript](https://arxiv.org/abs/1108.2876).
4. W. Boscheri and L. Pareschi, *High order pressure-based semi-implicit IMEX schemes for the 3D Navier–Stokes equations at all Mach numbers*, JCP 434 (2021), 110206. [DOI](https://doi.org/10.1016/j.jcp.2021.110206), [inspected manuscript](https://arxiv.org/abs/2008.01789). Especially §§3.1–3.3, §3.6 and equations (26), (54)–(61).
