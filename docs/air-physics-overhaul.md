# Air physics overhaul

Accepted planning baseline, 2026-10-16. Target: Pixel 10 Pro XL. Priority: a balanced sandbox with wind, heat, pressure, and explosions. Implementation has begun with the clock/input and sealed-gas foundations; see [implementation status](air-physics-progress.md). Production solver and performance claims remain subject to the explicit qualification gates below.

**Standing project goal, adopted by the user:** build a native mobile powder sandbox whose scientific accuracy, conservation, stability, sustained performance, and enjoyable experimentation are demonstrated by evidence. This plan is the accepted direction for that work. [Project instructions](../AGENTS.md) carry the goal forward for future work in this workspace.

## Direction

**Air is simulated matter: a compressible gas mixture that carries mass, momentum, thermal energy, and chemical species.** Its motion and thermodynamics drive the visible effects. The player should be able to build a chimney, heat a sealed chamber, puncture a pressure vessel, extinguish a fire by starving it of oxygen, or launch dust with a blast, and understand the result from the same physical rules.

Use one conservative, pressure-based compressible finite-volume gas model. Prototype a semi-implicit acoustic update so ordinary slow airflow does not require thousands of explicit sound-speed steps. The existing staggered layout is a candidate; retain it only if the selected published discretization and reference implementation justify its momentum/energy coupling. Scientific accuracy remains a measured requirement: a stable large step can still erase pressure waves.

Keep the fine material grid for grains and liquid detail. Gas composition, mass, and energy belong to the gas solver; smoke, flame, and visible steam become views of that state and suspended particles/droplets. This is a substantial change to the present gas-material representation.

The gameplay comes from responsive tools, useful measurements, clear effects, and interesting experiments. Physical coefficients and conservation rules should remain coherent across quality presets.

### What the world represents

The initial scientific contract is a planar 2D continuum: geometry and state are extruded through a saved slice thickness, with no front/back escape or variation. A sealed region in this view is sealed through that extrusion. If front/back heat loss or wall drag is added, it is an explicit modeled boundary term with an energy/momentum budget. A drawing cannot represent a real open room while silently assuming that nothing flows out of the screen.

Real 3D turbulence has vortex stretching and a different energy transfer across scales from 2D flow. Increasing resolution in the 2D solver will not recover that missing dimension. Validate 2D references as 2D; compare physical experiments only when geometry and out-of-plane assumptions match. A future 3D gas model can be a separate fidelity milestone after the mobile 2D system is measured. The existing shelved voxel prototype does not itself supply a validated 3D air solver.

### Decisions fixed by this plan

| Decision | Implementation contract |
|---|---|
| First supported physics | Nonreacting, single-phase ideal gas mixture with state-dependent thermal/transport properties; static impermeable geometry before moving material, phase change, or fire |
| Physical ownership | One canonical gas inventory and energy state per region; legacy gas labels/temperature/reactions cannot operate there in parallel |
| Energy convention | Species and phase reference energies include formation/phase offsets; derive temperature from thermal energy consistently and do not add the same chemical/latent release again |
| Time | Fixed simulation clock, timestamped input, time-based source rates, and accepted-step accounting; rendering never determines applied heat or reaction time |
| Initial boundaries | Explicit adiabatic, free-slip fixed walls for the first gas cases; gravity off in the uniform heating reference. Add no-slip and heat-conducting boundaries as tested capabilities |
| Geometry | Preserve fine-wall connectivity at every alignment; production cut-volume/refinement layout is selected by a prototype gate |
| Numerical acceptance | Commit only a converged, positive, source-accounted step; retries do not apply sources twice or advance simulation time |
| First playable scope | Static walls, a direct metered gas heater, a preset valve, a fan, pressure/temperature probes, and reproducible snapshots |
| Later qualification | Editable geometry, water/grain coupling, humidity/phase exchange, combustion, resolved blasts, and fracture each require their own gates |

The primary Pixel workload stays 1080 x 1440 fine cells. Use an initial reference-experiment profile of 1 mm per fine cell, a 1 m unit-depth extrusion, 293.15 K, 101325 Pa, and standard gravity of 9.80665 m/s^2 where enabled. Default ambient humidity is a specified 50% relative humidity computed from the selected saturation/property model; dry and humid benchmarks set their own composition explicitly. This profile is a declared experiment, not a measurement of the phone. Test the resulting physical scale and timestep cost before choosing additional production world presets. A saved experiment's scale never changes to improve its performance.

Three research decisions remain, with named deliverables rather than implicit assumptions: a numerical-method design record and CPU reference, a topology layout prototype, and an actual-device accuracy/performance profile. They are resolved before the corresponding production implementation; none needs the user to choose numerical details.

## What is present today

Reviewed `main` at `184439deedcfaa76d9904c53790f9a67885f9298` and [PR #11](https://github.com/MrBeastyCakes/powder/pull/11), head `d423ae52a137fd60d0cf11d379015a78f9754ff7`. The PR is an open prototype; this plan does not assume it is merged.

The PR already supplies a staggered grid at one air cell per 4 x 4 material cells, convection-like flow, gas transport, a liquid jet effect, and a Wind view. Useful pieces to retain include its grid plumbing, GPU dispatch integration, overlay, and regression scenes.

| Finding | Consequence | Source at PR head |
|---|---|---|
| Density is rebuilt each tick from material labels and temperature | Gas has no persistent inventory for compression or evacuation | `crates/pt-sim2d/src/air2d.wgsl:85` |
| The pressure solve targets zero divergence | Its arbitrary-gauge correction pressure cannot serve as absolute gas pressure | `crates/pt-sim2d/src/air2d.wgsl:214` |
| Column weight is computed separately, restarting below solids | It cannot represent pressure accumulated in a sealed chamber | `crates/pt-sim2d/src/air2d.wgsl:112` |
| A coarse cell becomes solid at 50% solid/powder occupancy | Thin walls and vents can change behavior with grid alignment | `crates/pt-sim2d/src/air2d.wgsl:108` |
| Liquids use column weight, excluding solved air pressure | Trapped air cannot correctly resist or drive liquid displacement | `crates/pt-sim2d/src/sim2d.wgsl:74` |
| Particle vertical speed is downward-only | Pressure blasts need signed two-dimensional material momentum | `crates/pt-sim2d/src/sim2d.wgsl:61` |
| Burning checks for neighboring open/gas space | There is no oxygen inventory to consume | `crates/pt-core/src/wgsl/common.wgsl:136` |
| Boiling swaps one water cell for one steam cell | The current density ratio cannot preserve physical water mass and expansion volume | `crates/pt-core/src/wgsl/common.wgsl:114`; `materials/core.toml:62,84` |
| One physics tick runs per drawn frame | Display rate affects physical speed; current speed controls chiefly change movement passes | `crates/pt-app/src/lib.rs:615`; `crates/pt-app/src/ui.rs:62` |
| Heat properties deliberately use tuned per-cell/per-tick units | Physical pressure requires a defined mass, length, time, and energy conversion | `materials/core.toml:7` |

The three PR air tests check plume direction, a visible water jet, and still-water stability. Keep their intent while adding thermodynamic and conservation tests. The repository already has a Pixel 10 Pro XL layout fixture with a 1080 x 1440 simulation area (`crates/pt-app/src/layout.rs:68`); this is the initial workload, rather than the full display height.

## Scientific contract

### State and units

Each gas control volume stores species masses, total gas energy, and the momentum/velocity quantities required by the selected scheme. Geometry supplies the actual available gas volume and connected faces. Compute:

```text
gas mass       m = sum(species masses)
density        rho = m / available gas volume
mass fraction  Y_i = m_i / m
pressure       p_abs = rho * R_mix(Y) * T_K       [initial ideal-mixture EOS]
gauge pressure p_gauge = p_abs - p_ambient
```

Obtain temperature by inverting the mixture internal-energy relation with temperature-dependent heat capacities. Track total energy consistently, including kinetic energy and pressure work. Density changes because mass or volume changes; heating a rigid sealed volume increases pressure without removing mass. Gravity and the resulting pressure/density field produce buoyancy.

The initial composition should represent nitrogen, oxygen, argon, carbon dioxide, and variable water vapor. Vapor can be transported as a noncondensing species in the first gas-only model; saturation and phase exchange become physical capabilities in the later humidity milestone. Combustion adds modeled fuel vapor and products; soot is a suspended particulate field. Property tables may combine inert species only after showing that mixture density and heat-capacity errors stay within the declared tolerance. Humidity is a composition variable, not an independent density multiplier.

Use Kelvin in thermodynamics, seconds, meters, kilograms, joules, and pascals internally, or an explicitly documented nondimensional equivalent. Keep Celsius as a display option. Save world-cell length, out-of-plane slice thickness, ambient state, gravity, and simulation time in world metadata. A 2D cell represents a finite physical volume; display DPI must not change that volume.

Validate the initial reference scale with a representative wind/vent test and save it with worlds. A change to air-grid resolution changes numerical resolution, while the physical size of the world stays fixed.

Use species/phase internal-energy references consistently across CPU and GPU: total gas energy contains reference internal energy plus sensible internal energy and kinetic energy. Recover temperature after subtracting kinetic energy and applying the same reference offsets. Reference-dependent internal energy can be negative; positivity constraints apply to physically valid temperature, density, and thermal state, not an arbitrary zero of total energy. Phase reference energies and pressure work must reproduce the intended latent enthalpy. Keep gravitational potential energy in the global ledger separately from local gas energy.

Initialize gas from composition, available volume, absolute pressure, and temperature through the EOS. The old `empty` cell label becomes geometric void in a new-model world. Its legacy fine-grid temperature is not another physical gas reservoir. Use a world-level backend/model version during migration: keep old worlds in the legacy backend until an explicit conversion is requested. An import can preserve visible geometry and approximate gas composition/temperature, but old gas-label counts cannot recover missing physical inventories. Report that conversion limitation and never silently treat an old world as an exact scientific snapshot.

### Properties and processes

| Property/process | Required treatment |
|---|---|
| Compressibility | Persistent gas inventory and an equation of state; heating, pumping, compression, and expansion all affect pressure |
| Heat capacity | Species/mixture `cv(T)` and `cp(T)`; consistent energy-to-temperature conversion |
| Heat transport | Advection, Fourier conduction, and energy exchanged with solids/liquids; conductivity depends on state |
| Viscosity | State-dependent dynamic viscosity, viscous stress, and conversion of dissipated kinetic energy to heat |
| Molecular diffusion | Conservative species diffusion with nonnegative species masses, zero net diffusive mixture mass flux, and associated energy transport |
| Buoyancy | Gravity with a balanced ambient pressure field and variable density; avoid a second independent gas-rise rule |
| Humidity | Vapor partial pressure, saturation over water/ice, relative humidity, evaporation, condensation, and latent heat |
| Combustion | Oxygen- and fuel-limited rates, chemical-energy release, gas products, soot, and heat loss |
| Pressure waves | Finite propagation, compression/rarefaction, reflection, transmission, and a separately validated shock regime |
| Interfaces | Pressure loads, gas displacement, interfacial heat transfer, liquid vapor exchange, and material-dependent drag |
| Radiation | Initially an explicitly budgeted thermal-radiation approximation; later soot/flame emission and absorption as required by the validated fire model |

Air is a mixture; its heat capacity, conductivity, viscosity, and density must not remain universal constants. Pure water vapor is invisible: visible white steam is condensed droplets, while smoke visibility comes primarily from particles. Rendering should preserve that distinction while remaining readable on a phone.

Scientific fidelity needs a declared validity range. Start validation around ordinary humid air and progressively include hot plumes, moderate pressure vessels, and combustion. Candidate reference points include 250, 293, 373, 1000, and 1800 K, and 0.2, 1, 2, and 5 bar, where the chosen property models apply. These are proposed test points, not an established accuracy envelope. Publish the resulting errors against reference properties.

The first EOS is an ideal mixture with variable heat capacity. Dense-gas corrections, dissociation at high temperature, rarefied flow, and extreme-pressure phase behavior require additional models and validation. Define those extensions in the property interface. Scientific claims for them begin when their reference cases pass. Molecular-scale simulation and full chemical kinetics are separate levels of approximation; a continuum gas model can still reproduce the intended macroscopic experiments accurately within its stated range.

## Solver design

### Conservative transport and pressure

Use the layout selected by the numerical-method gate and give the new gas state its own module and buffers. Exchange mass, species, momentum, and energy using shared face fluxes so neighboring volumes receive equal and opposite transfers. Use bounded conservative reconstruction; verify positivity without silently creating mass or energy. Semi-Lagrangian sampling can remain useful for visual tracers or a velocity predictor, but it cannot be the inventory-conservation mechanism.

Use a documented pressure-based compressible IMEX scheme: explicit material transport and a semi-implicit acoustic pressure/velocity/energy update. Pressure forces and pressure work must use consistent face quantities. The pressure correction is generally a Helmholtz-type solve with compressibility terms; adding an EOS field to the present incompressible Poisson solve is insufficient.

Absolute thermodynamic pressure has a physical reference. A numerical pressure correction may be stored separately. Only the physical pressure is used for probes, boiling, pumping, and material loads. A hydrostatic reference/perturbation split can improve floating-point precision, provided their sum remains consistent with the EOS and conserved state.

Select and cite an actual published all-speed pressure/energy discretization in a numerical-method design record before optimizing the WGSL implementation. The record must specify every update stage, face flux, pressure-work term, energy reference, positivity treatment, low-Mach limit, and step acceptance criterion. An IMEX family name alone does not define an implementable scheme. Require the CPU reference to pass nonuniform low-Mach heating/venting, an acoustic pulse, and shock-tube checks before choosing production GPU layouts.

Compare pressure residual and conservation error per millisecond for a multigrid-preconditioned method versus simpler iterations. A fixed 16 Jacobi passes is a baseline to measure, rather than a convergence guarantee. Test whether f32 pressure gradients and total-minus-kinetic energy recovery retain gentle airflow and small thermal changes. A reference-pressure split or auxiliary thermal variable must be reconciled to conservative energy if needed. The scheme must remain useful at low Mach number without excessive artificial diffusion of gentle wind.

Define the turbulence and unresolved mixing strategy explicitly. Begin with resolved flow and physical viscosity/diffusion; establish Reynolds, Rayleigh, Prandtl, Mach, and diffusion-related scales for the showcase scenes. Quantify excessive grid diffusion before adding a calibrated subgrid model. Any turbulence closure must document what it approximates in this 2D world and account for its energy transfers. Visual curl enhancement can be a separate artistic option; it is not evidence of correct turbulent transport.

An explicit compressible scheme at real sound speed is an expensive default. For illustration, a 4 mm air cell and sound speed near 343 m/s imply a CFL-0.5 acoustic step of roughly 6 microseconds: thousands of steps per 1/60 second. An implicit update avoids that stability restriction, but accurate wave shape can still require much smaller steps. Use error-driven event substeps for blasts and compare to reference solutions. Reduced sound speed, if offered as a stylized setting, changes the physical model and must be identified as such.

Explicit transport retains its own CFL restriction: 10 m/s flow across a 4 mm gas cell at CFL 0.5 needs a step no larger than 0.2 ms, about 84 substeps per reference tick. Qualify fast jets and blasts separately from ordinary flow. Full-world real-time accurate blasts are a research target, not a prerequisite assumed by the 4 ms gas budget. Preserve the physical model and show slower simulated time if an accurate event exceeds the device budget.

### Geometry, walls, and moving material

Build gas volume fractions and face apertures from the fine material geometry. Also represent connectivity inside a coarse cell: a one-pixel wall can divide that cell into two disconnected volumes. Face openness alone would reconnect them through a single pressure unknown.

Prototype disconnected cut volumes or local fine resolution at walls, vents, and liquid interfaces. Select the implementation with the seal tests and Pixel device measurements before committing the production data layout. Preserve one-cell walls at every alignment; do not make walls permeable as a quality shortcut. Handle tiny cut volumes with conservative merging/subcycling and documented connectivity rules.

Start with static impermeable walls, then add explicit permeability for porous material. Sand piles need a packing/permeability model rather than automatically becoming perfectly sealed blocks. A thin cap of sand and a rigid wall should behave differently.

Changes in liquid/solid occupancy must conservatively remap gas inventory and account for boundary work. Geometry cannot simply delete the gas in a newly occupied volume. A genuinely sealed gas pocket must resist displacement or compression; an outflow path must receive the displaced gas. Topology changes need split/merge tests.

Distinguish physical displacement from construction tools. During physical motion, retain every gas species and account for mechanical work; reject an impossible step that eliminates the final void containing finite gas mass. For construction, an explicitly destructive paint/replace operation may remove displaced gas to the external ledger with its species, energy, and momentum. Erasing material creates geometric void without automatically injecting ambient gas; subsequent redistribution/flow uses the existing inventory. These edits require robust vacuum-interface handling before they are enabled in a new-model world. Reset-to-atmosphere is a separate explicit source operation. Undo restores the complete prior state and ledger.

The first playable valve changes the permeability of an existing zero-thickness interface between already represented gas volumes. It does not add/remove gas volume or depend on a wall-painting remap. General editable geometry follows that checkpoint.

World edges have explicit modes: sealed, atmospheric open boundary with measured inflow/outflow, or periodic for tests. An atmospheric reservoir supplies its specified composition and temperature. Use characteristic/nonreflecting treatment where needed for the wave model and quantify residual reflection. Do not silently refill sealed interiors with ambient air.

### Coupling and chemistry

1. Add signed 2D particle velocity/impulse storage. Apply size-, density-, and relative-velocity-dependent drag; exchange the opposite impulse with gas. Keep an energy budget for drag dissipation. This allows pressure-driven gas to lift and throw dust.
2. Apply gas pressure loads to liquid boundaries and displacement back to gas. Verify hydrostatic liquid heads, trapped air, vented versus sealed drainage, and jets before replacing the PR's column-weight approximation. A full liquid solver remains a separate undertaking; document accuracy limits of the cellular liquid interface.
3. Exchange heat between gas and material without duplicating the fine-grid air temperature field. Exactly one field owns each parcel's physical energy; interpolation used for display must not create an additional heat reservoir.
4. Give condensed material physical mass and partial phase-change state. Evaporation removes actual water mass, adds identical vapor mass, and pays latent energy. Condensation reverses that transfer. A water cell turning into a same-sized steam label cannot represent the roughly thousandfold change in specific volume.
5. Use vapor partial pressure for evaporation/condensation equilibrium. Boiling depends on saturation vapor pressure versus local liquid pressure; it cannot use a universal 100 C threshold at all pressures. Include pressure and surface effects at the fidelity supported by the liquid model.
6. Migrate fire from a self-contained gas material to reacting fuel/oxygen plus energy and products. Represent solid fuel consumption and residual material, and preserve both total mass and elemental inventories across the reaction. Oxygen-starved fire weakens; ventilation changes the result. Subcycle stiff heat release rather than allowing negative reactants. Visible condensation also needs a droplet-size/settling model; a color field alone cannot predict where condensate goes.
7. Generate blasts through a specified energy release and optional reaction gas production. Pressure and momentum then emerge from the gas update. Adding a full radial kick as well as the same blast energy would double-count the source. Wall failure can use differential pressure, impulse, and material strength after fixed-wall loads are validated; free rigid bodies/fracture propagation are a later scope.

Keep material models data-driven. Introduce a versioned schema with thermodynamic/transport properties, species definitions, reaction yields, permeability, and strength as needed. Supply explicit conversions/defaults for old packs. Shared `pt-core` changes must preserve the shelved 3D prototype's tests or isolate legacy behavior until it is migrated.

## Time, performance, and numerical safeguards

Use a fixed simulation clock, initially 60 reference ticks per simulated second. Rendering may run at another rate. Speed controls scale simulated time across gas, falling material, heat, diffusion, reactions, and source tools together. Solver substeps and movement pass counts become numerical controls rather than the meaning of game speed.

Clock work includes input and tool semantics: queue timestamped stroke paths rather than overwriting a single pending stroke; integrate continuous heat/cool/pump/force tools over accepted simulation time; and distinguish construction events from continuous sources. Paused construction remains an explicit state edit, while continuous sources advance only during a step. A single-step command advances exactly one reference interval, including its required gas substeps. Resume never replays background wall time. Bound catch-up work and report achieved simulation speed when it falls behind. Preserve per-dispatch parameters across batches and respect the existing uniform-slot capacity; multiple encodes must not overwrite parameters still needed by an earlier dispatch.

Convert per-tick damping to a time-based decay and reaction chances to timestep-aware rates. Bound transport CFL, diffusion, reaction depletion, and pressure-solve error. Limit catch-up work; after backgrounding, discard wall-clock backlog. If real time cannot be maintained, expose the achieved simulation rate and retain coherent physics. Pause stops simulation work and uses event-driven redraw.

Retain a last accepted gas state. Each tentative step has a bounded solver budget and GPU-side residual/validity flag. Commit only when convergence, thermodynamic bounds, species positivity, and source consistency pass; otherwise retry from the accepted state with smaller steps or fewer simulated seconds per frame. Bound retries and report an unresolved failure instead of displaying corrupt state. The clock and source ledger advance only on acceptance. Asynchronous readback is for telemetry; it cannot establish whether the current step was safe to commit. Include rollback/scratch storage in the memory gate.

Conservation includes explicit source accounting: painting/removal, heaters, pumps, gravity/boundary work, radiation, and atmospheric fluxes. Follow the chosen species/phase reference-energy convention above; chemical and latent offsets must not also be added as duplicate heat reservoirs. Include gravitational potential energy or its explicitly integrated work. Any emergency clamp or lost energy must be observable in diagnostics. Near-vacuum needs a stated lower-validity bound and robust positivity handling; a density floor must not act as a hidden gas generator.

Conservation alone is insufficient. Check the direction of heat/species transfer and thermodynamic entropy behavior in isolated reference cases: spontaneous numerical cooling/heating, uphill diffusion, or negative entropy production require investigation. Sources and irreversible processes must have a coherent energy and entropy interpretation, with the appropriate reference assumptions.

### Proposed Pixel 10 Pro XL acceptance budgets

All timing numbers below are targets until measured on the actual phone. The repository's software Vulkan timings are not evidence for Android speed.

| Item | Initial target |
|---|---|
| Reference workload | 1080 x 1440 material cells; start comparison with 270 x 360 air cells plus topology detail |
| Balanced presentation / physics | 60 FPS / 60 Hz, where scientifically required gas substeps fit the budget |
| Whole simulation, P95 | At most 8 ms per reference tick |
| Gas plus coupling, P95 | At most 4 ms within that simulation budget |
| Frame critical path, P95 | At most 13 ms; measure actual CPU/GPU overlap and presentation deadlines |
| Sustained heavy scene | Ten minutes; fewer than 1% missed presentation deadlines, with at most 20% P95 tick-time degradation from minute 1 to minutes 8-10 |
| Persistent simulation memory | Initial ceiling 96 MiB at 1080 x 1440; 160 MiB for an optional 1440 x 1920 world, subject to profiling |

The current particle/thermal ping-pong uses 24 bytes per material cell. PR #11 adds about 36 bytes per 4 x 4 air cell plus boundary faces: roughly 38.94 MiB combined at 1080 x 1440. A planning reservation of 128 bytes per air cell for the replacement brings that to about 47.46 MiB. Another 12 bytes per material cell for coupling/partial phase state would bring it to about 65.26 MiB. These are allocation estimates, not measured totals; geometry components, solver scratch, rendering, UI, staging, and duplicate benchmark worlds need separate accounting and explicit caps.

Prototype the expensive state/geometry scheme early. If the accuracy and speed goals conflict, retain the scientific model and reduce the physical world size, visual cost, or exposed simulation rate. A 30 Hz battery profile needs separate equal-simulated-time accuracy tests; doubling `dt` is not automatically equivalent. Automatic visual scaling must preserve the saved world and wall connectivity. Avoid changing gas resolution in an ongoing experiment until conservative remapping and topology have been verified.

Expand `pt-bench` to record P50/P95/P99 tick/frame times, pressure residual, mass/energy errors, peak memory, achieved simulation time, and Android thermal status. Use GPU timestamps only when supported, with a labeled fallback. Keep reduced diagnostics asynchronous; avoid full-field readbacks during play. Run benchmarks in batches that keep the interface responsive and account for existing-world memory. Record battery/thermal comparisons under consistent brightness, charging, and starting conditions. Measure a plume, vent maze, boiling water, sealed heated chamber, settling dust, and repeated blasts, including drawing the app UI.

Capture actual adapter capabilities before allocating the production solver: storage sizes/binding counts, workgroup limits, precision, and negotiated optional features. Use a portable baseline without assuming f64, floating-point atomics, subgroups, or global workgroup barriers. The existing device requests no optional features, so timestamps need explicit capability negotiation. Keep new species/transport tables in separate storage buffers; widening the shared 128-entry uniform material table indiscriminately can exceed baseline limits. Preserve legacy 3D tables/helpers until explicitly migrated.

Sleeping material chunks may still respond to air. Pressure/temperature changes and impulses must wake affected material; do not skip a globally connected pressure solve merely because a particle chunk sleeps. Consider sparse gas work only after its conservation and pressure communication are validated.

## Implementation sequence

Use small reviewable changes. The local folders used in planning are verified source snapshots, not Git checkouts. Before implementation, fetch current upstream refs, reconcile the latest main/air work, and record the actual chosen base commit. Reuse useful PR #11 infrastructure without assuming it is merged or its physics already establishes the new model. Keep the legacy backend working while the new backend passes its release gates.

| Change | Main work and locations | Completion gate |
|---|---|---|
| 0. Establish the base | Upstream refs, existing tests/Android packaging, backend and shared-3D boundaries | Exact implementation base and reused prototype pieces recorded; preserve existing applicable checks |
| 1. Clock and source events | `pt-app/src/lib.rs`, `ui.rs`, brush/tool scheduling, bounded parameter batches | Same timestamped input and source ledger at equal simulated time under 30/60/120 rendering; pause/step/resume and batch limits pass |
| 2. Units, state, and heating reference | Versioned world metadata, species/EOS properties in `pt-core`, CPU sealed-chamber model and snapshot | Accounted heater raises temperature/pressure correctly, mass stays fixed, no unintended motion, snapshot round-trip reproduces the state |
| 3. Numerical-method selection | Cited method design record and CPU flow implementation; independent reference cases | Low-Mach flow, conservation, positivity, acoustic and shock behavior quantified; one consistent pressure/energy scheme selected |
| 4. GPU and topology feasibility | Prototype geometry/flux/solve buffers, f32 precision, limits, rollback, Pixel benchmarking | Select production topology and solve layout from measured seal tests, error, time, memory, and capability constraints |
| 5. Conservative static-geometry gas | New `pt-sim2d/src/air/` module/shaders replacing prototype stages | CPU/reference comparisons, hydrostatic rest, metered heating, vent flow, accepted-step logic, and all wall alignments pass |
| 6. First playable checkpoint | Direct gas heater, preset valve, fan, pressure/temperature probes, reproducible saved experiments | Heated sealed box raises pressure; opening the preset valve creates a directed gas jet; heated open gas forms a plume |
| 7. Editable geometry and material exchange | Construction ledger, conservative physical remapping, signed material momentum, thermal/liquid interfaces | Split/merge/final-void cases are defined; pressure resists water displacement and lifts grains; opposite exchange and inventories pass |
| 8. Humidity, steam, and fire | Saturation/phase reference energies, droplet model, oxygen-limited reactions and versioned material data | Condensation, evaporation cooling, pressure-dependent boiling approximation, oxygen starvation, and elemental/energy balances validated |
| 9. Blasts and release qualification | Conservative rapid release, wave/shock validation, pressure loads, full phone/UX suite | Publish accuracy and sustained performance for the selected profile; expose reduced simulation speed when an event exceeds its real-time budget |

Changes 3 and 4 are bounded research gates. Their deliverables include the selected method/layout, executable evidence, and a decision record; a failed feasibility case changes the production approach instead of expanding an unvalidated implementation. Pixel timing is completed when the APK can run on the user's actual device; cloud CI cannot supply that result. Device access does not block the independent clock, CPU-reference, and portable GPU work.

The first playable checkpoint uses a direct heat source measured in watts, so it does not depend on later gas-to-solid thermal exchange. Its valve changes existing connectivity, so it does not depend on later editable geometry. Uniform sealed heating alone verifies state and energy accounting; it does not validate the flow or pressure solver.

The first qualified release includes static-wall gas experiments, heating/fan/vent sources, probes, physical snapshots, and the published operating range. It can retain legacy play in a separate backend while material coupling develops. Accurate water jets/boiling, arbitrary chemistry, pressure-vessel fragmentation, 3D turbulence, and real-time sharp shocks are separately qualified capabilities.

### Acceptance suite

These are proposed engineering gates, to be tightened or revised from a reference implementation rather than chosen to fit an unstable result.

| Experiment | What must be established |
|---|---|
| Closed inert gas, no external sources | Relative mass drift below 1e-4 and total-energy drift below 1e-3 over 1000 controlled steps; account for gravity and walls if enabled |
| Uniform sealed heating | Fixed mass/volume/composition: `p2/p1 = T2/T1` in Kelvin within 2% |
| Controlled piston | Isothermal `pV` and constant-gamma adiabatic `pV^gamma` within 3%; variable-heat-capacity tests use the appropriate reference instead |
| Free expansion into an evacuated rigid chamber | Mass/energy conserved; ideal gas has the expected final pressure and unchanged equilibrium temperature for an insulated rigid system |
| Resting atmosphere | No growing spurious circulation; vertical pressure supports weight; equilibrium remains stable under timestep/refinement changes |
| Transport reference cases | Couette/Poiseuille flow for viscosity, conduction/diffusion solutions for thermal/species transport, and natural-convection references with matched dimensionless scales |
| Advection and mixing | Species stay nonnegative, sum to mixture mass, and conserve mass; mixing and heat transport improve under refinement |
| Humid air and phase exchange | Density/property and saturation/dew-point checks against reference data; condensation depends on vapor partial pressure; latent energy and water mass balance |
| Fine walls and vents | Seals at every x/y offset modulo 4, including internal coarse-cell dividers; quantify 1- and 2-cell apertures, corners, and split/merge events |
| Material exchange | Opposite drag impulses; trapped bubbles resist compression/displacement; vented tank drains differently from a sealed one; no gas disappears under moving liquid |
| Oxygen-limited flame | Fuel and oxygen loss match specified product yields and released chemical energy; a sealed chamber eventually starves combustion |
| Small acoustic pulse | Wave speed agrees with the mixture sound speed; report phase/amplitude error at the chosen grid and timestep |
| Compressible vent | Resolved mass flow, pressure ratio, and transition to choked outflow agree with the selected reference over its stated validity range |
| Sod shock tube | Compare density, velocity, and pressure to an exact/reference solution; positive density/internal energy; error improves with refinement |
| Blast | Measure overpressure and impulse, reflections, and energy conservation. For the ideal strong-blast 2D cylindrical reference, radius scales as `t^(1/2)`; do not apply a 3D spherical exponent |
| Frame-rate independence | Compare equal simulated time across render rates and validated timestep profiles; physical observables agree within specified tolerances |
| Replay | Repeat inputs/seeds on the same configuration; require reproducibility there and use tolerances across different GPU backends |

Retain applicable material conservation, heat/latent-energy, movement, Android packaging, and UI regressions. Update tests that currently count gas pixels to measure canonical gas mass. Validate shader compilation and software Vulkan cases in CI, then collect actual Android performance evidence. This planning work has not run the future acceptance suite.

## Making the science enjoyable

Give players a small set of tools with physical meanings: a directional fan applies force, a pump moves gas between endpoints or a reservoir, a heater adds energy, a vent changes a connection, and an ignition tool supplies a local energy input. A vacuum tool removes gas inventory; erasing a wall opens space without automatically resetting nearby pressure. Painting/removal is an explicit external mass/energy source or sink in the ledger.

Provide temperature, gauge-pressure, velocity, oxygen, humidity, and density views. A hold-to-inspect probe shows physical values and the current simulation rate. Keep everyday controls compact and put detailed measurements in an inspector. Slow motion and frame stepping help players see real processes that happen faster than a display frame.

Ship experiments rather than empty feature lists:

- A hot plate and chimney showing convection, circulation, and ventilation.
- A heated sealed chamber whose vent releases a pressure-driven jet.
- A humid warm chamber with a cold window producing condensation.
- A fire with an adjustable air supply, soot, and oxygen starvation.
- A pressure pulse around an obstacle and a blast that throws loose grains.
- A fan blowing through porous sand, followed by dust lifting when drag becomes strong enough.

Tune the size and initial conditions of these scenes so the effects are readable within seconds. Optional sandbox boosts should act as visible sources or clearly labeled artistic forces with accounted energy. Rendering can exaggerate tracer visibility without changing the gas's thermodynamic state.

## Making the ambition measurable

The goal is a physically credible, reproducible, enjoyable 2D mobile sandbox within a declared operating range. Publish the scope and benchmark results so claims of improvement can be checked. A single universal ranking hides tradeoffs; use a scorecard with accuracy and stability as release gates, followed by performance and player experience.

### Important dependencies outside the air solver

- **Liquid mechanics:** pressure transmission, incompressibility over its stated range, viscosity, free surfaces, and gas entrainment. Surface tension, droplet size, wetting, and cavitation matter when extending to small jets, bubbles, or low-pressure liquid experiments. Track these as material-coupling capabilities; a calibrated gas model cannot correct inaccurate liquid boundary motion on its own.
- **Solid and granular mechanics:** support/contact, friction and angle of repose, permeability, thermal expansion, and strength under pressure/impulse. Static walls can carry measured loads initially. Fracture and moving rigid structures require explicit mechanical models before advertising pressure-vessel destruction as predictive.
- **Boundary conditions and transport:** wall heat capacity and conductivity, insulation, slip/no-slip assumptions, thermal radiation, and front/back closure. These often control an experiment as strongly as the gas EOS.
- **Usable experiments:** repeatable construction, measurement tools, reliable undo/redo, full-state save/load, and the ability to share the exact initial conditions. A compelling scientific sandbox should make testing an idea easy.

### Verification, validation, and provenance

Separate verification (solving the chosen equations correctly) from validation (those equations predicting the intended physical experiments). CPU/GPU agreement is useful verification but can share a modeling error. Use analytic solutions and independent reference implementations, then compare the supported physical regime to measured data with uncertainty.

For each property table and benchmark, record its source/version/license, units, applicable pressure/temperature/composition range, geometry, boundary conditions, timestep/grid, reference uncertainty, metric, and tolerance. Candidate sources include established thermodynamic/transport reference tables for air species and IAPWS formulations for water/steam; verify the selected datasets and their valid ranges during implementation. Do not call a benchmark scientific solely because it looks plausible.

Match dimensionless scales and geometry when comparing scenes. Keep reference datasets and immutable test scenes beside the benchmark definitions. Publish failures and accuracy-versus-runtime curves, not only the fastest preset. Existing powder games can be compared for interface and play tasks; their behavior is not the ground truth for fluid physics.

| Dimension | Proposed release evidence |
|---|---|
| Gas thermodynamics | EOS/property checks, sealed heating within 2%, compression within 3%, and mass/energy gates above |
| Flow and boundaries | Zero modeled transfer across impermeable walls; resolved vent mass-flow within an initial 5% reference target; specify minimum aperture resolution for quantitative accuracy |
| Waves and blasts | For a specified resolved pulse, initial targets of under 3% arrival-time error and under 10% pressure-impulse error; add shock-profile norms and refinement studies before claiming sharp-shock accuracy |
| Coupled matter | Physical inventory transfers, pressure-driven liquid tests, interfacial heat/latent-energy tests, and measured particle impulse exchange |
| Sustained mobile operation | Ten-minute Pixel scene suite meeting the selected profile; record speed, thermal status, power, and memory alongside accuracy |
| Reproducible experiments | Full-state round-trip and replay tests, a versioned world format, shareable initial conditions, and complete source/settings metadata |
| Construction and enjoyment | Observe first-time players build a chimney and vented chamber, inspect pressure, and save/share a scene; record task completion time, errors, and discoveries; set product thresholds from an initial usability study |

All new numeric accuracy values are proposed gates requiring reference cases; a one-pixel vent can have correct connectivity without a quantitatively resolved jet. Keep those two claims separate. Specify cells per resolved wavelength and event timestep limits for wave benchmarks, and report peak overpressure as well as impulse. Normalize event-energy error to the injected blast energy or acoustic perturbation energy, as applicable: the much larger ambient thermal reservoir can hide a failed blast in a small total-energy percentage. Track pressure offset and spurious speed separately, and require spatial/temporal convergence where meaningful. A preset qualifies only if it meets both its physics and performance gates. Faster rendering obtained by advancing less simulated time must be reported as slower simulation.

### Experiment and world infrastructure

Save the complete physical state: gas inventories/energy/momentum, condensed material and partial phases, pending sources, simulation time, world scale, boundary conditions, model/data versions, and random seed. Define portable replay as events timestamped in simulation time plus model settings. Deterministic same-device replay and cross-device physical agreement are separate requirements.

Provide persistent pressure/temperature/flow probes with graphs, units, color legends, an accessible palette, and exportable data. Offer clean initial-state templates, robust undo, and save migrations. Undo must restore inventories and connections, not just the painted pixels. Keep the app usable offline once installed; a local experiment should remain available without a service dependency.

Treat material/mod inputs as versioned scientific data: validate ranges and units, reject inconsistent reaction yields, and report unsupported model regimes. Make numerical diagnostics available to developers without putting solver internals into the normal play flow.

## Recommended first change

Start changes 1 and 2 after confirming the upstream base. Use a small fixed Cartesian chamber, static adiabatic walls, gravity off, canonical nonreacting species masses/momentum/energy, and a uniform heater specified in joules per simulated second. For a constant-cv test, independently predict `T2 = T1 + Q/(m*cv)` and `p2/p1 = T2/T1`; add independent variable-cv energy-integration/property checks.

The completion evidence is unchanged mass, `delta U = injected energy`, the expected pressure and temperature, zero unintended motion, equal outcomes/source budgets over equal simulated time at different rendering rates, and full physical-state save/load. Keep the existing app available during this work. No production shader rewrite, material migration, or claim of validated airflow belongs in this first change.
