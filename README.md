# Powder Toy Next: Phase 0

An Android-native, GPU-simulated falling-sand sandbox. Phase 0 builds two
prototypes on one shared material system, a 2D world at the phone's native
resolution (one cell per screen pixel) and a 3D voxel world, so the direction
for every later phase can be chosen from measurements and playtests.

## What is here

| Path | What it is |
|---|---|
| `materials/core.toml` | The 10 built-in materials (plus air) as data: density, heat capacity, conductivity, phase changes, burning, decay |
| `crates/pt-core` | Loads and validates material packs, compiles them into the GPU table, and holds the WGSL both simulations share (heat, phase changes, reactions, color) |
| `crates/pt-sim2d` | The 2D simulation and renderer (WGSL compute and fragment shaders) |
| `crates/pt-sim3d` | The 3D voxel simulation, brush picking and ray-marched renderer |
| `crates/pt-app` | The touch app: an upright 4:3 sim area on a keyboard-height control panel, with an on-device benchmark. The 3D prototype is shelved and no longer offered in the app |
| `crates/pt-bench` | The standard benchmark and a screenshot tool, runnable headless |
| `mobile/android` | Android packaging (native Rust NativeActivity, Vulkan); see its README |

## How the simulation works

- **Materials as data.** No material has its own code. Behavior follows from
  `kind` (empty, solid, powder, liquid, gas), `density`, the heat properties,
  and optional `heats_into`, `burns` and `decays`. Reverse phase changes are
  derived: water `heats_into` steam at 100, so steam condenses back at 100.
- **Movement.** A block cellular automaton on 2 x 2 (2D) or 2 x 2 x 2 (3D)
  blocks with alternating offsets, so every block updates in parallel on the
  GPU with no locks. Denser material sinks through lighter material, faster
  the bigger the density difference; powders slide off piles unless friction
  holds them; liquids and gases spread sideways.
- **Heat.** Energy-conserving diffusion with an enthalpy method: a cell at its
  melting point absorbs latent heat before it changes, so melting ice cools
  the water around it and boiling takes time. A test checks energy is
  conserved to 0.1% while ice melts.
- **Reactions.** Burning needs the ignition temperature and open space next to
  the cell, and adds the material's stored energy; decay lets fire die out.
- **Determinism.** Randomness is a hash of position, tick and seed, so the same
  world and inputs replay identically on the same device (tested).

## Run it

Android is the target (see `mobile/android/README.md`). For development the
same app also runs in a desktop window:

```sh
cargo run --release -p pt-app
```

Left-drag paints and the mouse wheel changes the brush. On a phone, one
finger paints and pinch changes the brush.

Tests (they need a Vulkan device; on a machine without a GPU, install
`mesa-vulkan-drivers` for the lavapipe software device):

```sh
cargo test --workspace
```

Benchmark and screenshots:

```sh
cargo run --release -p pt-bench                      # phone-sized 2D worlds, 3D at 64 to 256 cubed
cargo run --release -p pt-bench -- --screenshots out # PNGs of both prototypes
```

On a phone, open Menu, then Run benchmark, then Copy results. It measures the
same scenes as `pt-bench`.

## Phase 0 status

Done:

- Shared material core with 10 materials, schema versioning and validation
- 2D world at native resolution with movement, heat, phase changes, burning
- 3D voxel world with the same core, ray-marched rendering, painting on
  surfaces or on a movable plane
- Touch app with material bar, heat and cool tools, temperature view, pause and
  step, 2D and 3D switch, world size picker and on-device benchmark
- 21 tests (7 material, 9 2D, 5 3D) passing on a software GPU
- Android target type-checks; APK packaging set up with cargo-apk

Next:

- First APK build on real hardware, then benchmarks on three Android phone tiers
- Sleeping chunks (skip regions where nothing moves), the biggest expected speedup
- Empty-space skipping for the 3D ray march
- Magnifier loupe for 1-pixel precision under a finger
- Faster movement at native resolution (see findings)

## Findings so far

- **Movement speed at native resolution.** Cells move at most one cell per
  movement pass. With 4 passes per tick, sand falls 240 pixels a second, so a
  full phone-height drop takes about 10 seconds. More passes cost GPU time.
  Velocity-based movement (the Phase 1 momentum layer) is the real fix and may
  need to come earlier.
- **Hot air does not rise yet.** Heat moves with cells and by conduction only,
  so air heated by lava stays where it is. Buoyant hot air needs the Phase 1
  air layer.
- **Software-GPU numbers are not phone numbers.** On the cloud machine's
  4-core software Vulkan device a 1080 x 2400 world takes about 90 ms per
  tick. The on-device benchmark exists to get real numbers.
