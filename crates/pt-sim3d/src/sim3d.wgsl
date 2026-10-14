// 3D simulation: the same materials, heat and reactions as 2D on a voxel
// grid, y up. Movement uses 2 x 2 x 2 blocks with alternating offsets.

struct Params {
    nx: u32,
    ny: u32,
    nz: u32,
    tick: u32,
    seed: u32,
    offset: u32,
    salt: u32,
    heat_speed: f32,
}

@group(0) @binding(1) var<uniform> params: Params;
@group(0) @binding(2) var<storage, read> cells_in: array<u32>;
@group(0) @binding(3) var<storage, read> therm_in: array<vec2<f32>>;
@group(0) @binding(4) var<storage, read_write> cells_out: array<u32>;
@group(0) @binding(5) var<storage, read_write> therm_out: array<vec2<f32>>;

fn dims() -> vec3<i32> { return vec3<i32>(i32(params.nx), i32(params.ny), i32(params.nz)); }
fn inside(p: vec3<i32>) -> bool { return all(p >= vec3<i32>(0)) && all(p < dims()); }
fn vidx(p: vec3<i32>) -> u32 { return (u32(p.z) * params.ny + u32(p.y)) * params.nx + u32(p.x); }

@compute @workgroup_size(4, 4, 4)
fn heat(@builtin(global_invocation_id) gid: vec3<u32>) {
    let p = vec3<i32>(gid);
    if (!inside(p)) { return; }
    let i = vidx(p);
    let cell = cells_in[i];
    let mat = mat_of(cell);
    let th = therm_in[i];

    var flux = 0.0;
    var touches_air = false;
    var dirs = array<vec3<i32>, 6>(
        vec3<i32>(1, 0, 0), vec3<i32>(-1, 0, 0),
        vec3<i32>(0, 1, 0), vec3<i32>(0, -1, 0),
        vec3<i32>(0, 0, 1), vec3<i32>(0, 0, -1));
    for (var d = 0u; d < 6u; d++) {
        let q = p + dirs[d];
        if (!inside(q)) { continue; }
        let j = vidx(q);
        let n = mat_of(cells_in[j]);
        flux += conductance(mat, n, 0.16, params.heat_speed) * (therm_in[j].x - th.x);
        touches_air = touches_air || kind_of(n) == KIND_EMPTY || kind_of(n) == KIND_GAS;
    }

    var s = CellState(mat, th.x + flux / materials.m[mat].heat_capacity, th.y);
    s = resolve_phase(s);
    s = react(s, touches_air, hash3(i, params.tick, params.seed));
    cells_out[i] = make_cell(s.mat, shade_of(cell));
    therm_out[i] = vec2<f32>(s.temp, s.latent);
}

// Block slots: column c = dx + 2 * dz, bottom layer c, top layer c + 4.
var<private> blk_cell: array<u32, 8>;
var<private> blk_therm: array<vec2<f32>, 8>;
var<private> blk_mat: array<u32, 8>;
var<private> blk_moved: array<bool, 8>;

fn swap_cells(a: u32, b: u32) {
    let c = blk_cell[a]; blk_cell[a] = blk_cell[b]; blk_cell[b] = c;
    let t = blk_therm[a]; blk_therm[a] = blk_therm[b]; blk_therm[b] = t;
    let m = blk_mat[a]; blk_mat[a] = blk_mat[b]; blk_mat[b] = m;
    blk_moved[a] = true;
    blk_moved[b] = true;
}

fn flows_sideways(a: u32, b: u32) -> bool {
    let ka = kind_of(a);
    let kb = kind_of(b);
    let fluid_a = ka == KIND_LIQUID || ka == KIND_GAS;
    let fluid_b = kb == KIND_LIQUID || kb == KIND_GAS;
    let open_a = ka == KIND_EMPTY || ka == KIND_GAS;
    let open_b = kb == KIND_EMPTY || kb == KIND_GAS;
    return a != b && ((fluid_a && open_b) || (fluid_b && open_a));
}

fn slot_pos(base: vec3<i32>, k: u32) -> vec3<i32> {
    let c = k & 3u;
    return base + vec3<i32>(i32(c & 1u), i32(k >> 2u), i32(c >> 1u));
}

@compute @workgroup_size(4, 4, 4)
fn move_blocks(@builtin(global_invocation_id) gid: vec3<u32>) {
    let off = i32(params.offset);
    let base = vec3<i32>(gid) * 2 - vec3<i32>(off);
    if (any(base >= dims())) { return; }

    var valid: array<bool, 8>;
    for (var k = 0u; k < 8u; k++) {
        let p = slot_pos(base, k);
        blk_moved[k] = false;
        valid[k] = inside(p);
        if (valid[k]) {
            let i = vidx(p);
            blk_cell[k] = cells_out[i];
            blk_therm[k] = therm_out[i];
            blk_mat[k] = mat_of(blk_cell[k]);
        } else {
            blk_cell[k] = OUTSIDE;
            blk_therm[k] = vec2<f32>(0.0);
            blk_mat[k] = OUTSIDE;
        }
    }

    let h = hash3(gid.x | (gid.y << 10u) | (gid.z << 20u), params.tick, params.seed ^ params.salt);
    let flip = h & 3u;   // random mirror of the block in x and z
    let rot = (h >> 2u) % 3u;

    // 1. Fall.
    for (var i = 0u; i < 4u; i++) {
        let c = i ^ flip;
        if (heavier(blk_mat[c + 4u], blk_mat[c]) && rand01(pcg(h + i)) < sink_chance(blk_mat[c + 4u], blk_mat[c])) {
            swap_cells(c + 4u, c);
        }
    }

    // 2. Slide diagonally down (grains, liquids) or up (gases).
    for (var i = 0u; i < 4u; i++) {
        let c = i ^ flip;
        let top = c + 4u;
        let k = kind_of(blk_mat[top]);
        if ((k == KIND_POWDER || k == KIND_LIQUID) && !blk_moved[top] && !heavier(blk_mat[top], blk_mat[c])) {
            let stick = select(0.0, materials.m[blk_mat[top]].friction, k == KIND_POWDER);
            if (rand01(pcg(h ^ (0x9e37u + i))) >= stick) {
                for (var j = 0u; j < 3u; j++) {
                    let o = c ^ (((j + rot) % 3u) + 1u);
                    if (!blk_moved[o] && heavier(blk_mat[top], blk_mat[o])) {
                        swap_cells(top, o);
                        break;
                    }
                }
            }
        }
        if (kind_of(blk_mat[c]) == KIND_GAS && !blk_moved[c] && !heavier(blk_mat[top], blk_mat[c])) {
            for (var j = 0u; j < 3u; j++) {
                let o = (c ^ (((j + rot) % 3u) + 1u)) + 4u;
                if (!blk_moved[o] && heavier(blk_mat[o], blk_mat[c])) {
                    swap_cells(c, o);
                    break;
                }
            }
        }
    }

    // 3. Spread sideways within each layer, pairing cells along x, z or the
    //    diagonal at random.
    let pairing = (h >> 4u) % 3u;
    var pa = array<u32, 2>(0u, 2u);
    var pb = array<u32, 2>(1u, 3u);
    if (pairing == 1u) { pa = array<u32, 2>(0u, 1u); pb = array<u32, 2>(2u, 3u); }
    if (pairing == 2u) { pa = array<u32, 2>(0u, 1u); pb = array<u32, 2>(3u, 2u); }
    for (var layer = 0u; layer < 8u; layer += 4u) {
        for (var q = 0u; q < 2u; q++) {
            let a = pa[q] + layer;
            let b = pb[q] + layer;
            if (!blk_moved[a] && !blk_moved[b] && flows_sideways(blk_mat[a], blk_mat[b])) {
                let sa = select(0.0, materials.m[blk_mat[a]].spread, blk_mat[a] < 128u);
                let sb = select(0.0, materials.m[blk_mat[b]].spread, blk_mat[b] < 128u);
                if (rand01(pcg(h ^ (0x51edu + layer + q))) < max(sa, sb)) {
                    swap_cells(a, b);
                }
            }
        }
    }

    for (var k = 0u; k < 8u; k++) {
        if (valid[k] && blk_moved[k]) {
            let i = vidx(slot_pos(base, k));
            cells_out[i] = blk_cell[k];
            therm_out[i] = blk_therm[k];
        }
    }
}
