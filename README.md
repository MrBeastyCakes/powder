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
| `crates/pt-app` | The touch app: an upright 4:3 sim area on a keyboard-height control panel (a catalog: category rail and material list, built from the pack's kinds and tags), with an on-device benchmark. The 3D prototype is shelved and no longer offered in the app |
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
- **Air and pressure (2D).** A coarse grid, one air cell per 4 x 4 world
  cells, carries the air's flow. Each tick it weighs every column (hot air is
  lighter), pushes sideways where neighbouring columns weigh differently, and
  solves for the pressure that keeps the flow from piling up, so cold air
  slides in under hot air and the hot air rises. Air and gases drift with the
  flow. Liquids keep a sideways speed of their own, pushed by the weight of
  the liquid above, so water jets out of a hole in its tank. The Wind view
  shows the flow: hue for direction, brightness for speed.
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
- Touch app with a catalog picker (a rail of Recent, Tools and every kind and
  tag in the pack, and a material list whose descriptions come from the pack),
  heat and cool tools, temperature view, pause, step and speed, and an
  on-device benchmark. UI fonts are IBM Plex, subset and renamed under the
  SIL Open Font License (`crates/pt-app/fonts`)
- 31 tests (8 material, 9 app, 9 2D, 5 3D) passing on a software GPU
- Android target type-checks; APK packaging set up with cargo-apk

Next:

- First APK build on real hardware, then benchmarks on three Android phone tiers
- Empty-space skipping for the 3D ray march
- Magnifier loupe for 1-pixel precision under a finger
- Faster movement at native resolution (see findings)

## Findings so far

- **Movement speed at native resolution.** The block passes move a cell at
  most one cell per pass, so free falls now have a pass of their own: one
  thread per column drops each unsupported grain or drop through the air
  below it as far as its velocity carries it, and the velocity (stored in the
  cell) grows every tick it falls, so falls accelerate. Dropped sand crosses
  the 1,440-row sim area in about 1.6 seconds (tested under 2). The 8 block
  passes per tick handle sinking through liquids, sliding and spreading.
  Sleeping chunks skip all of it where nothing moves: a 64 x 64 chunk sleeps
  until something nearby moves, is painted, or changes phase.
- **First pass at air.** Hot air over lava rises, a tank jets through a hole,
  and still water leaves the air still (all tested). Water still drains from
  a hole only as fast as air bubbles can climb into the tank, so jets are a
  spray rather than a solid stream, and pressure does not yet shift boiling
  or melting points.
- **Software-GPU numbers are not phone numbers.** On the cloud machine's
  4-core software Vulkan device a 1080 x 2400 world takes about 90 ms per
  tick. The on-device benchmark exists to get real numbers.
