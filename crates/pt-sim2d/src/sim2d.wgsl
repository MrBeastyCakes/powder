// 2D simulation: one cell per screen pixel, y grows downward.
//
// Each tick runs one heat pass (read buffer A, write buffer B) and then a few
// movement passes in place on B. Movement uses a block cellular automaton:
// the world is cut into 2 x 2 blocks, each thread owns one block, and the
// block grid shifts by one cell between passes so material can cross block
// edges. No two threads touch the same cell, so there are no locks.
//
// Falling: before the block passes, one thread per column walks the column
// bottom to top and drops every unsupported grain or drop of liquid through
// the air below it, as far as its velocity carries it. Velocity lives in the
// cell's bits 16 to 23, in eighths of a cell per tick, and grows by GRAVITY
// each tick the cell falls freely, so falls accelerate. The block passes
// still do sinking through liquids, sliding, spreading and rising gas.
//
// Sleeping chunks: the world is also cut into 64 x 64 chunks. Any pass that
// moves a cell, or could have moved one but lost the dice roll, marks its
// chunk in `moved`. At the end of each tick `settle` wakes every chunk next
// to a marked one (a cell moves far less than a chunk in one tick), and
// `reset` clears the marks. Movement passes skip blocks in sleeping chunks,
// so a settled world costs only its heat pass.

struct Params {
    width: u32,
    height: u32,
    tick: u32,
    seed: u32,
    offset: u32,
    salt: u32,
    heat_speed: f32,
    _pad: u32,
}

@group(0) @binding(1) var<uniform> params: Params;
@group(0) @binding(2) var<storage, read> cells_in: array<u32>;
@group(0) @binding(3) var<storage, read> therm_in: array<vec2<f32>>;
@group(0) @binding(4) var<storage, read_write> cells_out: array<u32>;
@group(0) @binding(5) var<storage, read_write> therm_out: array<vec2<f32>>;
@group(0) @binding(6) var<storage, read_write> moved: array<atomic<u32>>;
@group(0) @binding(7) var<storage, read_write> awake: array<u32>;

const CHUNK: u32 = 64u;
// Velocity units are eighths of a cell per tick.
const GRAVITY: u32 = 3u;
const MAX_FALL: u32 = 160u;

fn vel_of(cell: u32) -> u32 { return (cell >> 16u) & 0xffu; }
fn with_vel(cell: u32, v: u32) -> u32 { return (cell & 0xff00ffffu) | (v << 16u); }

fn idx(x: u32, y: u32) -> u32 { return y * params.width + x; }

fn chunks_x() -> u32 { return (params.width + CHUNK - 1u) / CHUNK; }
fn chunks_y() -> u32 { return (params.height + CHUNK - 1u) / CHUNK; }
fn chunk_of(x: u32, y: u32) -> u32 { return (y / CHUNK) * chunks_x() + x / CHUNK; }

// Marks a chunk as moving. Most blocks in a busy chunk find it already
// marked, so they only read.
fn mark(c: u32) {
    if (atomicLoad(&moved[c]) == 0u) {
        atomicStore(&moved[c], 1u);
    }
}

@compute @workgroup_size(8, 8)
fn heat(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (gid.x >= params.width || gid.y >= params.height) { return; }
    let i = idx(gid.x, gid.y);
    let cell = cells_in[i];
    let mat = mat_of(cell);
    let th = therm_in[i];

    var flux = 0.0;
    var touches_air = false;
    var dirs = array<vec2<i32>, 4>(vec2<i32>(1, 0), vec2<i32>(-1, 0), vec2<i32>(0, 1), vec2<i32>(0, -1));
    for (var d = 0u; d < 4u; d++) {
        let p = vec2<i32>(gid.xy) + dirs[d];
        // The world's edges are insulated.
        if (p.x < 0 || p.y < 0 || p.x >= i32(params.width) || p.y >= i32(params.height)) { continue; }
        let j = idx(u32(p.x), u32(p.y));
        let n = mat_of(cells_in[j]);
        flux += conductance(mat, n, 0.24, params.heat_speed) * (therm_in[j].x - th.x);
        touches_air = touches_air || kind_of(n) == KIND_EMPTY || kind_of(n) == KIND_GAS;
    }

    var s = CellState(mat, th.x + flux / materials.m[mat].heat_capacity, th.y);
    s = resolve_phase(s);
    s = react(s, touches_air, hash3(i, params.tick, params.seed));
    // A cell that changes material starts at rest; otherwise it keeps its speed.
    cells_out[i] = select(make_cell(s.mat, shade_of(cell)), with_vel(make_cell(s.mat, shade_of(cell)), vel_of(cell)), s.mat == mat);
    therm_out[i] = vec2<f32>(s.temp, s.latent);
    // A phase change or reaction can set still material moving (ice melts).
    if (s.mat != mat) {
        mark(chunk_of(gid.x, gid.y));
    }
}

@compute @workgroup_size(64)
fn fall(@builtin(global_invocation_id) gid: vec3<u32>) {
    let x = gid.x;
    if (x >= params.width) { return; }
    let h = i32(params.height);
    // The lowest air cell in the run of air directly below the current cell,
    // or -1 when the cell below is not air.
    var space = -1;
    var y = h - 1;
    loop {
        if (y < 0) { break; }
        // Skip sleeping chunks a whole chunk at a time.
        if (y == h - 1 || u32(y) % CHUNK == CHUNK - 1u) {
            if (awake[chunk_of(x, u32(y))] == 0u) {
                y = i32((u32(y) / CHUNK) * CHUNK) - 1;
                space = -1;
                continue;
            }
        }
        let i = idx(x, u32(y));
        let c = cells_out[i];
        let k = kind_of(mat_of(c));
        if (k == KIND_EMPTY) {
            if (space < 0) { space = y; }
        } else if ((k == KIND_POWDER || k == KIND_LIQUID) && space > y) {
            let v = min(vel_of(c) + GRAVITY, MAX_FALL);
            // Round v / 8 up or down at random, in proportion, so slow cells
            // still creep down on average at the right speed.
            let dist = i32((v + (hash3(i, params.tick, params.seed ^ 0xfa11u) & 7u)) >> 3u);
            if (dist == 0) {
                cells_out[i] = with_vel(c, v);
                space = -1;
            } else {
                let land = min(y + dist, space);
                // Stopped short by something: the fall ends here.
                let nv = select(v, 0u, y + dist > space);
                let j = idx(x, u32(land));
                let t = therm_out[i];
                cells_out[i] = cells_out[j];
                therm_out[i] = therm_out[j];
                cells_out[j] = with_vel(c, nv);
                therm_out[j] = t;
                mark(chunk_of(x, u32(y)));
                mark(chunk_of(x, u32(land)));
                space = land - 1;
            }
        } else {
            // Resting on something: no speed left.
            if (vel_of(c) != 0u) { cells_out[i] = with_vel(c, 0u); }
            space = -1;
        }
        y -= 1;
    }
}

// End of tick: a chunk is awake if it or any neighbour moved this tick.
@compute @workgroup_size(8, 8)
fn settle(@builtin(global_invocation_id) gid: vec3<u32>) {
    let cw = chunks_x();
    let ch = chunks_y();
    if (gid.x >= cw || gid.y >= ch) { return; }
    var any = 0u;
    for (var dy = -1; dy <= 1; dy++) {
        for (var dx = -1; dx <= 1; dx++) {
            let x = i32(gid.x) + dx;
            let y = i32(gid.y) + dy;
            if (x >= 0 && y >= 0 && x < i32(cw) && y < i32(ch)) {
                any |= atomicLoad(&moved[u32(y) * cw + u32(x)]);
            }
        }
    }
    awake[gid.y * cw + gid.x] = any;
}

@compute @workgroup_size(8, 8)
fn reset(@builtin(global_invocation_id) gid: vec3<u32>) {
    let cw = chunks_x();
    if (gid.x >= cw || gid.y >= chunks_y()) { return; }
    atomicStore(&moved[gid.y * cw + gid.x], 0u);
}

var<private> blk_cell: array<u32, 4>;
var<private> blk_therm: array<vec2<f32>, 4>;
var<private> blk_mat: array<u32, 4>;
var<private> blk_moved: array<bool, 4>;

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

@compute @workgroup_size(8, 8)
fn move_blocks(@builtin(global_invocation_id) gid: vec3<u32>) {
    let off = i32(params.offset);
    let bx = i32(gid.x) * 2 - off;
    let by = i32(gid.y) * 2 - off;
    if (bx >= i32(params.width) || by >= i32(params.height)) { return; }

    // A block on a chunk edge belongs to both chunks it touches.
    let c0 = chunk_of(u32(max(bx, 0)), u32(max(by, 0)));
    let c1 = chunk_of(u32(min(bx + 1, i32(params.width) - 1)), u32(min(by + 1, i32(params.height) - 1)));
    if (awake[c0] == 0u && awake[c1] == 0u) { return; }
    // Set when anything here moved or could have; keeps the chunk awake.
    var busy = false;

    // Block slots: 0 top-left, 1 top-right, 2 bottom-left, 3 bottom-right.
    var valid = array<bool, 4>(false, false, false, false);
    for (var k = 0u; k < 4u; k++) {
        let x = bx + i32(k & 1u);
        let y = by + i32(k >> 1u);
        blk_moved[k] = false;
        if (x >= 0 && y >= 0 && x < i32(params.width) && y < i32(params.height)) {
            let i = idx(u32(x), u32(y));
            valid[k] = true;
            blk_cell[k] = cells_out[i];
            blk_therm[k] = therm_out[i];
            blk_mat[k] = mat_of(blk_cell[k]);
        } else {
            blk_cell[k] = OUTSIDE;
            blk_therm[k] = vec2<f32>(0.0);
            blk_mat[k] = OUTSIDE;
        }
    }

    let h = hash3(gid.x | (gid.y << 16u), params.tick, params.seed ^ params.salt);
    // Mirror the block at random so nothing drifts left or right.
    let l = h & 1u;
    let r = 1u - l;
    let cols = array<u32, 2>(l, r);

    // 1. Sink: anything denser sinks through liquid or gas below it, faster the
    //    bigger the density difference (sand drops through water, stone sinks
    //    slowly through lava, warm fire drifts up rather than shooting up).
    for (var c = 0u; c < 2u; c++) {
        let top = cols[c];
        let bot = top + 2u;
        // Grains and liquids falling through air belong to the fall pass,
        // which accelerates them.
        let kt = kind_of(blk_mat[top]);
        let free_fall = (kt == KIND_POWDER || kt == KIND_LIQUID) && kind_of(blk_mat[bot]) == KIND_EMPTY;
        let sinks = heavier(blk_mat[top], blk_mat[bot]) && !free_fall;
        busy = busy || sinks;
        if (sinks && rand01(pcg(h + c)) < sink_chance(blk_mat[top], blk_mat[bot])) {
            swap_cells(top, bot);
        }
    }

    // 2. Slide: grains and liquids blocked below slip diagonally; gases
    //    blocked above rise diagonally.
    for (var c = 0u; c < 2u; c++) {
        let top = cols[c];
        let other = cols[1u - c];
        let k = kind_of(blk_mat[top]);
        if ((k == KIND_POWDER || k == KIND_LIQUID) && !blk_moved[top] && !blk_moved[other + 2u]) {
            if (!heavier(blk_mat[top], blk_mat[top + 2u]) && heavier(blk_mat[top], blk_mat[other + 2u])) {
                busy = true;
                let stick = select(0.0, materials.m[blk_mat[top]].friction, k == KIND_POWDER);
                if (rand01(pcg(h ^ (0x9e37u + c))) >= stick) {
                    swap_cells(top, other + 2u);
                }
            }
        }
        let bot = top + 2u;
        if (kind_of(blk_mat[bot]) == KIND_GAS && !blk_moved[bot] && !blk_moved[other]) {
            if (!heavier(blk_mat[top], blk_mat[bot]) && heavier(blk_mat[other], blk_mat[bot])) {
                busy = true;
                swap_cells(bot, other);
            }
        }
    }

    // 3. Spread: liquids and gases flow sideways into open space.
    for (var row = 0u; row < 4u; row += 2u) {
        let a = l + row;
        let b = r + row;
        let flows = flows_sideways(blk_mat[a], blk_mat[b]);
        busy = busy || flows;
        if (!blk_moved[a] && !blk_moved[b] && flows) {
            let sa = select(0.0, materials.m[blk_mat[a]].spread, blk_mat[a] < 128u);
            let sb = select(0.0, materials.m[blk_mat[b]].spread, blk_mat[b] < 128u);
            if (rand01(pcg(h ^ (0x51edu + row))) < max(sa, sb)) {
                swap_cells(a, b);
            }
        }
    }

    for (var k = 0u; k < 4u; k++) {
        if (valid[k] && blk_moved[k]) {
            let i = idx(u32(bx + i32(k & 1u)), u32(by + i32(k >> 1u)));
            cells_out[i] = blk_cell[k];
            therm_out[i] = blk_therm[k];
        }
    }
    if (busy) {
        mark(c0);
    }
}
