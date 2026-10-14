// Shared by the 2D and 3D simulations. Must match `GpuMaterial` in materials.rs.

struct Material {
    kind: u32,
    color: u32,
    density: f32,
    heat_capacity: f32,

    conductivity: f32,
    friction: f32,
    spread: f32,
    glow: f32,

    up_temp: f32,
    up_into: u32,
    up_latent: f32,
    down_temp: f32,

    down_into: u32,
    down_latent: f32,
    burn_temp: f32,
    burn_into: u32,

    burn_heat: f32,
    burn_chance: f32,
    decay_into: u32,
    decay_chance: f32,
}

struct MaterialTable {
    m: array<Material, 128>,
}

@group(0) @binding(0) var<uniform> materials: MaterialTable;

const KIND_EMPTY: u32 = 0u;
const KIND_SOLID: u32 = 1u;
const KIND_POWDER: u32 = 2u;
const KIND_LIQUID: u32 = 3u;
const KIND_GAS: u32 = 4u;
const NONE: u32 = 0xffffffffu;
// Stands in for cells outside the world: never moves, never swaps.
const OUTSIDE: u32 = 0xffu;
const MIN_TEMP: f32 = -273.15;
const MAX_TEMP: f32 = 10000.0;

// A cell packs its material id (bits 0-7) and a random shade (bits 8-15) that
// travels with it, so grains keep their look as they move.
fn mat_of(cell: u32) -> u32 { return cell & 0xffu; }
fn shade_of(cell: u32) -> u32 { return (cell >> 8u) & 0xffu; }
fn make_cell(mat: u32, shade: u32) -> u32 { return (mat & 0xffu) | ((shade & 0xffu) << 8u); }

// Counter-based randomness: the same position, tick and seed always give the
// same number, so a save replays identically on the same device.
fn pcg(v: u32) -> u32 {
    let state = v * 747796405u + 2891336453u;
    let word = ((state >> ((state >> 28u) + 4u)) ^ state) * 277803737u;
    return (word >> 22u) ^ word;
}
fn hash3(a: u32, b: u32, c: u32) -> u32 { return pcg(a ^ pcg(b ^ pcg(c))); }
fn rand01(h: u32) -> f32 { return f32(h >> 8u) * (1.0 / 16777216.0); }

fn kind_of(m: u32) -> u32 {
    if (m >= 128u) { return KIND_SOLID; }
    return materials.m[m].kind;
}

fn movable(m: u32) -> bool {
    return m < 128u && materials.m[m].kind != KIND_SOLID;
}

// True when `a` should sink through `b`. Air counts, so gases lighter than air
// rise by having air sink past them.
fn heavier(a: u32, b: u32) -> bool {
    return movable(a) && movable(b) && materials.m[a].density > materials.m[b].density;
}

// Chance per pass that `a` sinks through `b`, from their density contrast.
fn sink_chance(a: u32, b: u32) -> f32 {
    let da = materials.m[a].density;
    let db = materials.m[b].density;
    // Capped below 1 so a falling mass breaks up naturally instead of
    // separating into regular rows.
    return clamp(2.0 * (da - db) / da, 0.05, 0.9);
}

// Heat flow per degree of difference between two neighbouring cells. Symmetric
// in a and b, so whatever one cell gains its neighbour loses. `limit` caps the
// rate below the explicit-integration stability bound for the number of
// neighbours (1/4 in 2D, 1/6 in 3D).
fn conductance(a: u32, b: u32, limit: f32, speed: f32) -> f32 {
    let ka = materials.m[a].conductivity;
    let kb = materials.m[b].conductivity;
    let sum = ka + kb;
    if (sum <= 0.0) { return 0.0; }
    let k = 2.0 * ka * kb / sum * speed;
    let cap = min(materials.m[a].heat_capacity, materials.m[b].heat_capacity) * limit;
    return min(k, cap);
}

struct CellState {
    mat: u32,
    temp: f32,
    // Energy stored toward a phase change: positive is progress toward the
    // material's heats_into, negative toward the reverse.
    latent: f32,
}

// Enthalpy-style phase change. A cell at its melting point stays there while
// it absorbs the latent heat, so melting ice cools what is around it and
// boiling takes time. Energy is conserved through every change.
fn resolve_phase(s: CellState) -> CellState {
    let m = materials.m[s.mat];
    var t = s.temp + s.latent / m.heat_capacity;
    if (t > m.up_temp) {
        let stored = (t - m.up_temp) * m.heat_capacity;
        if (stored >= m.up_latent) {
            let n = materials.m[m.up_into];
            return CellState(m.up_into, clamp(m.up_temp + (stored - m.up_latent) / n.heat_capacity, MIN_TEMP, MAX_TEMP), 0.0);
        }
        return CellState(s.mat, m.up_temp, stored);
    }
    if (t < m.down_temp) {
        let stored = (t - m.down_temp) * m.heat_capacity;
        if (-stored >= m.down_latent) {
            let n = materials.m[m.down_into];
            return CellState(m.down_into, clamp(m.down_temp + (stored + m.down_latent) / n.heat_capacity, MIN_TEMP, MAX_TEMP), 0.0);
        }
        return CellState(s.mat, m.down_temp, stored);
    }
    return CellState(s.mat, clamp(t, MIN_TEMP, MAX_TEMP), 0.0);
}

// Burning (needs open space next to the cell) adds the material's stored chemical
// energy; decay keeps the temperature and lets the energy leave as light.
fn react(s: CellState, touches_air: bool, h: u32) -> CellState {
    let m = materials.m[s.mat];
    if (m.burn_into != NONE && touches_air && s.temp >= m.burn_temp && rand01(h) < m.burn_chance) {
        let n = materials.m[m.burn_into];
        let energy = (s.temp + s.latent / m.heat_capacity) * m.heat_capacity + m.burn_heat;
        return CellState(m.burn_into, clamp(energy / n.heat_capacity, MIN_TEMP, MAX_TEMP), 0.0);
    }
    if (m.decay_into != NONE && rand01(pcg(h)) < m.decay_chance) {
        return CellState(m.decay_into, s.temp, 0.0);
    }
    return s;
}

fn srgb_of(m: u32) -> vec3<f32> {
    return unpack4x8unorm(materials.m[m].color).rgb;
}

// Rough blackbody ramp from dull red to white, with its intensity.
fn blackbody(t: f32) -> vec4<f32> {
    let a = clamp((t - 500.0) / 500.0, 0.0, 1.0);
    let b = clamp((t - 1000.0) / 600.0, 0.0, 1.0);
    let c = clamp((t - 1600.0) / 1400.0, 0.0, 1.0);
    let col = mix(mix(mix(vec3<f32>(0.5, 0.02, 0.0), vec3<f32>(1.0, 0.35, 0.05), a),
                      vec3<f32>(1.0, 0.8, 0.35), b),
                  vec3<f32>(1.0, 0.97, 0.9), c);
    let intensity = smoothstep(480.0, 1100.0, t);
    return vec4<f32>(col, intensity);
}

// Display color of a cell in sRGB: base color varied by its shade, glowing by
// temperature like real hot matter.
fn cell_color(cell: u32, temp: f32) -> vec3<f32> {
    let m = mat_of(cell);
    let base = srgb_of(m) * (0.86 + 0.28 * f32(shade_of(cell)) / 255.0);
    if (m == 0u) { return base; }
    let bb = blackbody(temp);
    let glow = materials.m[m].glow;
    var col = mix(base, bb.rgb, bb.a * 0.85);
    // Self-lit materials (fire) brighten with their own heat.
    col = col + base * glow * 0.35 * bb.a;
    return min(col, vec3<f32>(1.0));
}

// Blue-to-red overlay for the temperature view.
fn heat_map(t: f32) -> vec3<f32> {
    let x = clamp((t + 50.0) / 1350.0, 0.0, 1.0);
    let y = sqrt(x);
    return clamp(vec3<f32>(1.6 * y - 0.3, 1.0 - abs(1.6 * y - 0.9) * 1.4, 1.0 - 2.2 * y), vec3<f32>(0.0), vec3<f32>(1.0));
}

// Ray directions with an exactly-zero component would divide by zero in a
// voxel walk; nudge those components to a tiny positive value.
fn safe_dir(d: vec3<f32>) -> vec3<f32> {
    return select(d, vec3<f32>(1e-6), abs(d) < vec3<f32>(1e-6));
}
