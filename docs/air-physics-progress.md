# Air physics implementation status

The first implementation follows the [accepted plan](air-physics-overhaul.md)
and the standing goal in [AGENTS.md](../AGENTS.md). It is based on upstream
`main` commit `184439deedcfaa76d9904c53790f9a67885f9298`. PR #11's separate
experimental air branch was still open at the start of this work.

## Implemented foundation

- A fixed 60 Hz reference clock, with playback speed applied to complete
  simulation ticks rather than just movement passes.
- Timestamped brush paths and duration-based heat/cool exposure in both
  existing simulation backends. Construction while paused does not advance
  time; a manual step advances one reference interval.
- Bounded overload work, explicit slowdown indication, and no catch-up after
  suspension, focus changes, or an on-device benchmark.
- Event-driven paused rendering and immediate source updates when a stationary
  brush changes tool, material, or radius.
- Command-ordered immutable parameter snapshots so queued strokes and repeated
  encodes in one GPU submission retain their own uniforms.
- A separate physical-gas reference with canonical species masses, planar
  momentum, total energy, SI world metadata, source accounting, transactional
  heater steps, and validated versioned TOML snapshots.
- Temperature-dependent ideal-mixture heat capacities for N2, O2, Ar, CO2,
  and H2O from a pinned NASA7 dataset, with one formation-energy convention.
  Independent NIST property and integrated-energy checks are included.

The gas reference is a uniform, fixed-volume, adiabatic chamber with gravity
disabled. Its initial preset is explicitly dry air at 293.15 K and 101325 Pa,
with 1 mm world cells and 1 m unit depth. It uses a declared 200–1000 K property
range and treats any water vapor as noncondensing. This dry reference is an
initial experiment, not the eventual humid ambient world preset.

The existing cellular simulation remains the app's runtime backend. It does
not run a second physical gas inventory alongside legacy gas labels. Gas-flow
conservation, pressure waves, material exchange, humidity equilibrium, and
combustion remain later capabilities.

## Reproduce the foundation checks

Validation completed for the foundation: all 60 workspace tests pass, including
13 scientific gas-reference tests and the 2D/3D GPU input regressions. Four
additional tests qualify the limited CPU flow comparator below. Formatting,
Clippy with warnings denied, and the `aarch64-linux-android` app type-check pass.
The sealed-chamber example also runs successfully. GPU checks used Mesa
lavapipe in the cloud workspace; no Pixel performance measurement is claimed.

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --all-targets --locked
cargo check -p pt-app --target aarch64-linux-android --locked
cargo run -p pt-core --example sealed_chamber --locked
```

The GPU tests require Vulkan or another supported GPU backend. CI uses
lavapipe; the gas reference tests themselves do not open a GPU.

The sealed-chamber example uses the constant-heat-capacity analytical model:
1000 W for 10 simulated seconds injects 10,000 J into a 1 m³ chamber.

| Quantity | Before | After |
|---|---:|---:|
| Mass, kg | 1.204153347024 | 1.204153347024 |
| Temperature, K | 293.150000 | 304.721496 |
| Absolute pressure, Pa | 101325.000000 | 105324.596781 |
| Momentum, kg m/s | 0 | 0 |

The predicted and computed temperature/pressure agree; the physical snapshot
round-trips exactly. Variable-capacity tests separately compare NASA fits and
integrated internal-energy changes to NIST Shomate references at 500–600 K,
inside the valid ranges of both datasets. These are thermodynamic/property
checks, not validation of spatial flow or device speed.

## CPU numerical-method comparison

`cargo run -p pt-core --example air_flow_comparator` runs a bounded periodic 1D
constant-gamma comparison derived from Cordier–Degond–Kumbaro (2012). It preserves
the paper's frozen-kinetic-energy closure and reports both its conserved stored
energy and its discrepancy from new-time physical kinetic/internal energy.
It is independent of the application backend and the variable-cp gas model.

Uniform low-Mach flow, step rejection, acoustic spatial refinement, and a
deliberately underresolved acoustic step are covered by four example tests;
CI now runs examples with `--all-targets`. In one acoustic comparison, increasing
32 cells to 64 reduced normalized pressure error from 0.00644 to 0.00296. A
large implicit acoustic step remained stable and conservative while producing
0.556 normalized pressure error. This demonstrates why solver stability cannot
qualify wave fidelity. The [method research record](air-solver-method-gate.md)
states its source, assumptions, measurements, and remaining method-selection gate.

## Next qualification gates

1. Select and reproduce a published all-speed pressure/energy method on the CPU.
   Test low-Mach behavior, conservation, positivity, acoustic propagation, and
   shock tubes before committing a production GPU layout.
2. Prototype topology-aware gas volumes and f32 GPU solves. Preserve one-cell
   wall connectivity at every alignment, and measure real resource costs.
3. Qualify the first playable gas checkpoint: a metered gas heater, preset
   valve, fan, pressure/temperature probes, and reproducible saved experiments.
4. Add editable geometry and two-way material exchange, then phase change and
   oxygen-limited chemistry, then separately qualified blasts and material loads.

Actual sustained Pixel 10 Pro XL performance requires running the APK on that
device. Cloud software Vulkan tests establish shader correctness and
regressions; they do not establish the proposed 60 FPS or blast-accuracy targets.
