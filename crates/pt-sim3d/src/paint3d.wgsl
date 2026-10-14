// 3D brush: `pick` casts the touch ray into the voxel grid to find where to
// paint (on the first surface hit, or on a movable horizontal plane), then
// `paint` fills a sphere there.

struct Brush {
    ox: f32, oy: f32, oz: f32, radius: f32,
    dx: f32, dy: f32, dz: f32, temp: f32,
    mat: u32, mode: u32, seed: u32, fill: f32,
    nx: u32, ny: u32, nz: u32, plane_y: f32,
    aim: u32, extent: u32, _p0: u32, _p1: u32,
}

const MODE_PAINT: u32 = 0u;
const MODE_ERASE: u32 = 2u;
const MODE_HEAT: u32 = 3u;
const MODE_COOL: u32 = 4u;
const TARGET_SURFACE: u32 = 0u;
const TARGET_PLANE: u32 = 1u;

@group(0) @binding(1) var<uniform> brush: Brush;
@group(0) @binding(2) var<storage, read_write> cells: array<u32>;
@group(0) @binding(3) var<storage, read_write> therm: array<vec2<f32>>;
// xyz: brush center in voxels, w: 1 when the ray found a spot.
@group(0) @binding(4) var<storage, read_write> pick_out: vec4<f32>;

fn bdims() -> vec3<i32> { return vec3<i32>(i32(brush.nx), i32(brush.ny), i32(brush.nz)); }
fn binside(p: vec3<i32>) -> bool { return all(p >= vec3<i32>(0)) && all(p < bdims()); }
fn bidx(p: vec3<i32>) -> u32 { return (u32(p.z) * brush.ny + u32(p.y)) * brush.nx + u32(p.x); }

fn box_hit(ro: vec3<f32>, rd: vec3<f32>, size: vec3<f32>) -> vec2<f32> {
    let inv = 1.0 / rd;
    let a = (vec3<f32>(0.0) - ro) * inv;
    let b = (size - ro) * inv;
    let t0 = max(max(min(a.x, b.x), min(a.y, b.y)), min(a.z, b.z));
    let t1 = min(min(max(a.x, b.x), max(a.y, b.y)), max(a.z, b.z));
    return vec2<f32>(t0, t1);
}

@compute @workgroup_size(1)
fn pick() {
    let ro = vec3<f32>(brush.ox, brush.oy, brush.oz);
    let rd = safe_dir(normalize(vec3<f32>(brush.dx, brush.dy, brush.dz)));
    let size = vec3<f32>(bdims());
    pick_out = vec4<f32>(0.0);

    if (brush.aim == TARGET_PLANE) {
        let t = (brush.plane_y + 0.5 - ro.y) / rd.y;
        let p = ro + rd * t;
        if (t > 0.0 && p.x >= 0.0 && p.z >= 0.0 && p.x <= size.x && p.z <= size.z) {
            pick_out = vec4<f32>(p, 1.0);
        }
        return;
    }

    let span = box_hit(ro, rd, size);
    if (span.x > span.y || span.y < 0.0) { return; }
    let t = max(span.x, 0.0) + 1e-3;
    var cell = clamp(vec3<i32>(floor(ro + rd * t)), vec3<i32>(0), bdims() - 1);
    let step = vec3<i32>(sign(rd));
    let delta = abs(1.0 / rd);
    var tmax = (vec3<f32>(cell) + max(vec3<f32>(step), vec3<f32>(0.0)) - ro) / rd;
    var normal = vec3<i32>(0, 1, 0);
    let steps = brush.nx + brush.ny + brush.nz;
    for (var s = 0u; s < steps; s++) {
        let k = kind_of(mat_of(cells[bidx(cell)]));
        if (k != KIND_EMPTY && k != KIND_GAS) {
            pick_out = vec4<f32>(vec3<f32>(cell + normal) + 0.5, 1.0);
            return;
        }
        if (tmax.x < tmax.y && tmax.x < tmax.z) {
            cell.x += step.x; tmax.x += delta.x; normal = vec3<i32>(-step.x, 0, 0);
        } else if (tmax.y < tmax.z) {
            cell.y += step.y; tmax.y += delta.y; normal = vec3<i32>(0, -step.y, 0);
        } else {
            cell.z += step.z; tmax.z += delta.z; normal = vec3<i32>(0, 0, -step.z);
        }
        if (!binside(cell)) {
            // Fell through the floor of the world: paint on the floor.
            if (cell.y < 0) {
                pick_out = vec4<f32>(vec3<f32>(cell + normal) + 0.5, 1.0);
            }
            return;
        }
    }
}

@compute @workgroup_size(4, 4, 4)
fn paint(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (pick_out.w < 0.5 || any(gid >= vec3<u32>(brush.extent))) { return; }
    let center = pick_out.xyz;
    let origin = vec3<i32>(floor(center - brush.radius));
    let p = origin + vec3<i32>(gid);
    if (!binside(p) || length(vec3<f32>(p) + 0.5 - center) > brush.radius) { return; }

    let i = bidx(p);
    let h = hash3(u32(p.x) | (u32(p.y) << 10u) | (u32(p.z) << 20u), brush.seed, 0x7a1du);
    let current = mat_of(cells[i]);
    switch brush.mode {
        case MODE_HEAT: {
            therm[i].x = min(therm[i].x + 40.0 / materials.m[current].heat_capacity, MAX_TEMP);
        }
        case MODE_COOL: {
            therm[i].x = max(therm[i].x - 40.0 / materials.m[current].heat_capacity, MIN_TEMP);
        }
        case MODE_ERASE: {
            cells[i] = make_cell(0u, h >> 24u);
            therm[i] = vec2<f32>(20.0, 0.0);
        }
        default: {
            if (brush.mode == MODE_PAINT && current != 0u) { return; }
            if (rand01(h) >= brush.fill) { return; }
            cells[i] = make_cell(brush.mat, h >> 24u);
            therm[i] = vec2<f32>(brush.temp, 0.0);
        }
    }
}
