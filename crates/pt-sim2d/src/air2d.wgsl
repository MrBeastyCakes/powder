// Air and liquid flow on a coarse grid: one air cell per 4 x 4 world cells.
//
// The grid is staggered: horizontal velocity lives on the left and right faces
// of each air cell, vertical velocity on the top and bottom faces, pressure at
// the centre. Velocity is in air cells per tick. Each tick:
//
//  1. `gather` sums up the world cells under each air cell, twice. For the
//     air it takes the mean density of the air and gases (lighter as they heat
//     up), or marks the cell blocked when at least half of it is solid, powder
//     or liquid: liquids move by their own rules, so to the air they are
//     obstacles like a pile of sand. For liquids it takes the mean density of
//     everything that flows, or solid when at least half is solid or powder.
//  2. `hydro` integrates the weight of each column from the top down, relative
//     to room-temperature air, once for each. That pressure exactly holds every
//     column up, so only differences between neighbouring columns push
//     sideways: cold air pushes in under hot air, and water pushes out through
//     a hole in its tank (sim2d.wgsl reads the liquid one).
//
//     Keeping liquid out of the air's weight matters: a blob of oil weighs
//     hundreds of times what the air beside it does, so counted in, it pushed
//     the air under it sideways hard enough that the pressure solve could not
//     cancel it, and any poured liquid raised a gale.
//  3. `advect` carries velocity along itself and adds that sideways push.
//  4. `divergence` and a few `jacobi` iterations find the pressure that keeps
//     the flow from piling up or emptying out, warm-started from last tick.
//  5. `project` subtracts that pressure's push, so flow converging under a hot
//     plume turns upward instead.
//
// The movement passes then let gases, air and liquids drift with the flow.
//
// Buffer `air` holds, in order: U and V (final face velocities, read by the
// movement passes), U2 and V2 (before the pressure solve), then per air cell
// RHO (density of everything that flows, or SOLID), PH (its column weight,
// which pushes liquids: see `heat` in sim2d.wgsl), DIV, ARHO (density of the
// air and gases, or SOLID where the air is blocked) and PA (their column
// weight, which pushes the air).

struct AirParams {
    width: u32,
    height: u32,
    aw: u32,
    ah: u32,
}

@group(0) @binding(1) var<uniform> ap: AirParams;
@group(0) @binding(2) var<storage, read> cells: array<u32>;
@group(0) @binding(3) var<storage, read> therm: array<vec2<f32>>;
@group(0) @binding(4) var<storage, read_write> air: array<f32>;
@group(0) @binding(5) var<storage, read> p_src: array<f32>;
@group(0) @binding(6) var<storage, read_write> p_dst: array<f32>;
@group(0) @binding(7) var<storage, read_write> moved: array<atomic<u32>>;

const AIR_CELL: u32 = 4u;
// Gravity in air cells per tick per tick: the fall pass's 3/8 world cell.
const AIR_GRAVITY: f32 = 0.09375;
// Room-temperature air, the density nothing floats or sinks in.
const AIR_REF: f32 = 1.2;
const ROOM_K: f32 = 293.15;
const SOLID: f32 = -1.0;
const MAX_SPEED: f32 = 2.0;
const DAMPING: f32 = 0.99;
// Flow slower than this leaves a sleeping chunk asleep.
const WAKE_SPEED: f32 = 0.02;
const CHUNK: u32 = 64u;

fn nu() -> u32 { return (ap.aw + 1u) * ap.ah; }
fn nv() -> u32 { return ap.aw * (ap.ah + 1u); }
fn nc() -> u32 { return ap.aw * ap.ah; }
fn u_at(i: u32, j: u32) -> u32 { return j * (ap.aw + 1u) + i; }
fn v_at(i: u32, j: u32) -> u32 { return nu() + j * ap.aw + i; }
fn u2_base() -> u32 { return nu() + nv(); }
fn rho_at(i: u32, j: u32) -> u32 { return 2u * (nu() + nv()) + j * ap.aw + i; }
fn ph_at(i: u32, j: u32) -> u32 { return rho_at(i, j) + nc(); }
fn div_at(i: u32, j: u32) -> u32 { return rho_at(i, j) + 2u * nc(); }
fn arho_at(i: u32, j: u32) -> u32 { return rho_at(i, j) + 3u * nc(); }
fn pa_at(i: u32, j: u32) -> u32 { return rho_at(i, j) + 4u * nc(); }
fn cell_at(i: u32, j: u32) -> u32 { return j * ap.aw + i; }

// The air's density at an air cell, or SOLID where the air is blocked.
fn rho(i: u32, j: u32) -> f32 { return air[arho_at(i, j)]; }

fn wake_at(x: u32, y: u32) {
    let cw = (ap.width + CHUNK - 1u) / CHUNK;
    let c = (min(y, ap.height - 1u) / CHUNK) * cw + min(x, ap.width - 1u) / CHUNK;
    if (atomicLoad(&moved[c]) == 0u) {
        atomicStore(&moved[c], 1u);
    }
}

@compute @workgroup_size(8, 8)
fn gather(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (gid.x >= ap.aw || gid.y >= ap.ah) { return; }
    var solid = 0u;
    var liquid = 0u;
    var mass = 0.0;
    var n = 0.0;
    var gas_mass = 0.0;
    var gas_n = 0.0;
    for (var dy = 0u; dy < AIR_CELL; dy++) {
        for (var dx = 0u; dx < AIR_CELL; dx++) {
            let x = gid.x * AIR_CELL + dx;
            let y = gid.y * AIR_CELL + dy;
            if (x >= ap.width || y >= ap.height) { solid++; continue; }
            let i = y * ap.width + x;
            let m = mat_of(cells[i]);
            let k = kind_of(m);
            // Piles of powder block the air like walls do.
            if (k == KIND_SOLID || k == KIND_POWDER) { solid++; continue; }
            var r = materials.m[m].density;
            if (k == KIND_LIQUID) {
                liquid++;
            } else {
                r *= ROOM_K / max(therm[i].x + 273.15, 1.0);
                gas_mass += r;
                gas_n += 1.0;
            }
            mass += r;
            n += 1.0;
        }
    }
    let half = AIR_CELL * AIR_CELL;
    air[rho_at(gid.x, gid.y)] = select(mass / max(n, 1.0), SOLID, solid * 2u >= half);
    air[arho_at(gid.x, gid.y)] = select(gas_mass / max(gas_n, 1.0), SOLID, (solid + liquid) * 2u >= half);
}

// Integrates column `i` of the densities at `src` (offset per air cell)
// into column weights at `dst`.
fn column_weight(i: u32, src: u32, dst: u32) {
    var p = 0.0;
    var prev = 0.0;
    var open = false;
    for (var j = 0u; j < ap.ah; j++) {
        let c = j * ap.aw + i;
        let r = air[src + c];
        if (r < 0.0) {
            // Solids carry their own weight; pressure starts over below them.
            air[dst + c] = 0.0;
            open = false;
            continue;
        }
        let rp = r - AIR_REF;
        p = select(AIR_GRAVITY * rp * 0.5, p + AIR_GRAVITY * (prev + rp) * 0.5, open);
        air[dst + c] = p;
        prev = rp;
        open = true;
    }
}

@compute @workgroup_size(64)
fn hydro(@builtin(global_invocation_id) gid: vec3<u32>) {
    let i = gid.x;
    if (i >= ap.aw) { return; }
    column_weight(i, rho_at(0u, 0u), ph_at(0u, 0u));
    column_weight(i, arho_at(0u, 0u), pa_at(0u, 0u));
}

fn sample_u(pos: vec2<f32>) -> f32 {
    let f = vec2<f32>(pos.x, pos.y - 0.5);
    let i0 = clamp(floor(f), vec2<f32>(0.0), vec2<f32>(f32(ap.aw), f32(ap.ah - 1u)));
    let t = clamp(f - i0, vec2<f32>(0.0), vec2<f32>(1.0));
    let a = vec2<u32>(i0);
    let b = vec2<u32>(min(a.x + 1u, ap.aw), min(a.y + 1u, ap.ah - 1u));
    let top = mix(air[u_at(a.x, a.y)], air[u_at(b.x, a.y)], t.x);
    let bot = mix(air[u_at(a.x, b.y)], air[u_at(b.x, b.y)], t.x);
    return mix(top, bot, t.y);
}

fn sample_v(pos: vec2<f32>) -> f32 {
    let f = vec2<f32>(pos.x - 0.5, pos.y);
    let i0 = clamp(floor(f), vec2<f32>(0.0), vec2<f32>(f32(ap.aw - 1u), f32(ap.ah)));
    let t = clamp(f - i0, vec2<f32>(0.0), vec2<f32>(1.0));
    let a = vec2<u32>(i0);
    let b = vec2<u32>(min(a.x + 1u, ap.aw - 1u), min(a.y + 1u, ap.ah));
    let top = mix(air[v_at(a.x, a.y)], air[v_at(b.x, a.y)], t.x);
    let bot = mix(air[v_at(a.x, b.y)], air[v_at(b.x, b.y)], t.x);
    return mix(top, bot, t.y);
}

fn u_open(i: u32, j: u32) -> bool {
    return i > 0u && i < ap.aw && rho(i - 1u, j) >= 0.0 && rho(i, j) >= 0.0;
}

fn v_open(i: u32, j: u32) -> bool {
    return j > 0u && j < ap.ah && rho(i, j - 1u) >= 0.0 && rho(i, j) >= 0.0;
}

// One thread per face position; each does a horizontal and a vertical face.
@compute @workgroup_size(8, 8)
fn advect(@builtin(global_invocation_id) gid: vec3<u32>) {
    let i = gid.x;
    let j = gid.y;
    let base = u2_base();
    if (i <= ap.aw && j < ap.ah) {
        var u = 0.0;
        if (u_open(i, j)) {
            let pos = vec2<f32>(f32(i), f32(j) + 0.5);
            let back = pos - vec2<f32>(air[u_at(i, j)], sample_v(pos));
            // The sideways push from columns of different weight.
            let w = 2.0 / (rho(i - 1u, j) + rho(i, j));
            u = DAMPING * sample_u(back);
            u -= w * (air[pa_at(i, j)] - air[pa_at(i - 1u, j)]);
        }
        air[base + u_at(i, j)] = u;
    }
    if (i < ap.aw && j <= ap.ah) {
        var v = 0.0;
        if (v_open(i, j)) {
            let pos = vec2<f32>(f32(i) + 0.5, f32(j));
            let back = pos - vec2<f32>(sample_u(pos), air[v_at(i, j)]);
            // Gravity and the column weight cancel exactly here.
            v = DAMPING * sample_v(back);
        }
        air[base + v_at(i, j)] = v;
    }
}

@compute @workgroup_size(8, 8)
fn divergence(@builtin(global_invocation_id) gid: vec3<u32>) {
    let i = gid.x;
    let j = gid.y;
    if (i >= ap.aw || j >= ap.ah) { return; }
    let b = u2_base();
    var d = 0.0;
    if (rho(i, j) >= 0.0) {
        d = air[b + u_at(i + 1u, j)] - air[b + u_at(i, j)] + air[b + v_at(i, j + 1u)] - air[b + v_at(i, j)];
    }
    air[div_at(i, j)] = d;
}

fn face_weight(r: f32, i: i32, j: i32) -> f32 {
    if (i < 0 || j < 0 || i >= i32(ap.aw) || j >= i32(ap.ah)) { return 0.0; }
    let rn = rho(u32(i), u32(j));
    if (rn < 0.0) { return 0.0; }
    return 2.0 / (r + rn);
}

@compute @workgroup_size(8, 8)
fn jacobi(@builtin(global_invocation_id) gid: vec3<u32>) {
    let i = gid.x;
    let j = gid.y;
    if (i >= ap.aw || j >= ap.ah) { return; }
    let r = rho(i, j);
    if (r < 0.0) {
        p_dst[cell_at(i, j)] = 0.0;
        return;
    }
    var sw = 0.0;
    var sp = 0.0;
    var dirs = array<vec2<i32>, 4>(vec2<i32>(1, 0), vec2<i32>(-1, 0), vec2<i32>(0, 1), vec2<i32>(0, -1));
    for (var d = 0u; d < 4u; d++) {
        let n = vec2<i32>(i32(i), i32(j)) + dirs[d];
        let w = face_weight(r, n.x, n.y);
        if (w > 0.0) {
            sw += w;
            sp += w * p_src[cell_at(u32(n.x), u32(n.y))];
        }
    }
    p_dst[cell_at(i, j)] = select(0.0, (sp - air[div_at(i, j)]) / sw, sw > 0.0);
}

@compute @workgroup_size(8, 8)
fn project(@builtin(global_invocation_id) gid: vec3<u32>) {
    let i = gid.x;
    let j = gid.y;
    let b = u2_base();
    var fast = false;
    if (i <= ap.aw && j < ap.ah) {
        var u = 0.0;
        if (u_open(i, j)) {
            let w = 2.0 / (rho(i - 1u, j) + rho(i, j));
            u = clamp(air[b + u_at(i, j)] - w * (p_src[cell_at(i, j)] - p_src[cell_at(i - 1u, j)]), -MAX_SPEED, MAX_SPEED);
        }
        air[u_at(i, j)] = u;
        fast = fast || abs(u) > WAKE_SPEED;
    }
    if (i < ap.aw && j <= ap.ah) {
        var v = 0.0;
        if (v_open(i, j)) {
            let w = 2.0 / (rho(i, j - 1u) + rho(i, j));
            v = clamp(air[b + v_at(i, j)] - w * (p_src[cell_at(i, j)] - p_src[cell_at(i, j - 1u)]), -MAX_SPEED, MAX_SPEED);
        }
        air[v_at(i, j)] = v;
        fast = fast || abs(v) > WAKE_SPEED;
    }
    // Moving air wakes the world cells it can carry.
    if (fast) {
        wake_at(i * AIR_CELL, j * AIR_CELL);
    }
}
