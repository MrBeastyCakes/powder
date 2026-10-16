# Project goal

The user has adopted the air physics overhaul as the standing project goal.

Build a native mobile powder sandbox that earns its quality through scientific accuracy, conservation, stability, sustained device performance, and enjoyable experimentation. The initial target is a 2D Android simulation on the Pixel 10 Pro XL, with balanced wind, heat, pressure, and explosions.

Read [the accepted overhaul plan](docs/air-physics-overhaul.md) before changes to physics, simulation timing, gas/material ownership, or related architecture. Use its staged implementation and qualification gates to guide work within the user's requested scope.

## Requirements guiding implementation

- Treat air as a compressible gas mixture with canonical mass, momentum, energy, and composition. Derive density and thermodynamic state consistently from occupied volume and the equation of state.
- Model heat transport, viscosity, diffusion, humidity, phase exchange, combustion, and pressure waves through the plan's staged scientific capabilities and declared validity ranges.
- Conserve physical inventories across material interactions and record external tool, boundary, and source transfers. Use one energy reference without duplicate chemical or latent heat.
- Keep simulation time and continuous tool rates independent of rendering. Preserve gas connectivity through thin walls and account for geometry changes.
- Validate numerical steps before accepting them. Account for retries without duplicating sources or advancing simulated time.
- Verify physics against independent references and publish accuracy alongside performance. Treat 60 FPS and sharp-blast accuracy as targets until actual-device measurements qualify them.
- Keep the 2D physical assumptions explicit. Scientific claims for 3D turbulence, extreme regimes, liquid mechanics, and fracture require their own validation.
- Make experiments easy to build, inspect, save, replay, and share. Keep controls responsive and the scientific behavior understandable.
- Resolve numerical-method, topology, and device-feasibility gates before dependent production rewrites. Preserve the legacy backend during the staged migration.

The first foundation checkpoint is a fixed simulation clock, versioned physical state, and a sealed-chamber heating reference with independently predicted temperature and pressure, conserved mass, accounted energy, and reproducible save/load.
