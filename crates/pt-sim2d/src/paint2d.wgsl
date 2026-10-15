// Brush strokes: fills a capsule from `a` to `b` with the chosen tool.

struct Brush {
    ax: f32, ay: f32, bx: f32, by: f32,
    radius: f32, mat: u32, temp: f32, mode: u32,
    origin_x: i32, origin_y: i32, width: u32, height: u32,
    seed: u32, fill: f32, _p0: u32, _p1: u32,
}

const MODE_PAINT: u32 = 0u;
const MODE_REPLACE: u32 = 1u;
const MODE_ERASE: u32 = 2u;
const MODE_HEAT: u32 = 3u;
const MODE_COOL: u32 = 4u;

@group(0) @binding(1) var<uniform> brush: Brush;
@group(0) @binding(2) var<storage, read_write> cells: array<u32>;
@group(0) @binding(3) var<storage, read_write> therm: array<vec2<f32>>;
// The simulation's chunk marks and wake flags (see sim2d.wgsl): painted
// chunks wake up. The heat pass of the very next tick must run there, since
// only the buffer pair painted into holds the new cells.
@group(0) @binding(4) var<storage, read_write> moved: array<atomic<u32>>;
@group(0) @binding(5) var<storage, read_write> awake: array<u32>;

fn wake(x: i32, y: i32) {
    let cw = (brush.width + 63u) / 64u;
    let c = (u32(y) / 64u) * cw + u32(x) / 64u;
    atomicOr(&moved[c], 3u);
    // Every painting thread writes the same value, so the race is harmless.
    awake[c] = awake[c] | 4u;
}

fn segment_distance(p: vec2<f32>, a: vec2<f32>, b: vec2<f32>) -> f32 {
    let ab = b - a;
    let t = clamp(dot(p - a, ab) / max(dot(ab, ab), 1e-6), 0.0, 1.0);
    return length(p - (a + ab * t));
}

@compute @workgroup_size(8, 8)
fn paint(@builtin(global_invocation_id) gid: vec3<u32>) {
    let x = brush.origin_x + i32(gid.x);
    let y = brush.origin_y + i32(gid.y);
    if (x < 0 || y < 0 || x >= i32(brush.width) || y >= i32(brush.height)) { return; }
    let p = vec2<f32>(f32(x) + 0.5, f32(y) + 0.5);
    if (segment_distance(p, vec2<f32>(brush.ax, brush.ay), vec2<f32>(brush.bx, brush.by)) > brush.radius) { return; }

    let i = u32(y) * brush.width + u32(x);
    let h = hash3(u32(x) | (u32(y) << 16u), brush.seed, 0x7a1du);
    let current = mat_of(cells[i]);
    switch brush.mode {
        case MODE_HEAT: {
            therm[i].x = min(therm[i].x + 40.0 / materials.m[current].heat_capacity, MAX_TEMP);
            wake(x, y);
        }
        case MODE_COOL: {
            therm[i].x = max(therm[i].x - 40.0 / materials.m[current].heat_capacity, MIN_TEMP);
            wake(x, y);
        }
        case MODE_ERASE: {
            cells[i] = make_cell(0u, h >> 24u);
            therm[i] = vec2<f32>(20.0, 0.0);
            wake(x, y);
        }
        default: {
            if (brush.mode == MODE_PAINT && current != 0u) { return; }
            if (rand01(h) >= brush.fill) { return; }
            cells[i] = make_cell(brush.mat, h >> 24u);
            therm[i] = vec2<f32>(brush.temp, 0.0);
            wake(x, y);
        }
    }
}
