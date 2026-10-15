// Draws the world 1:1, one cell per screen pixel.

struct View {
    world_w: u32,
    world_h: u32,
    offset_x: i32,
    offset_y: i32,
    overlay: u32,
    _p0: u32,
    _p1: u32,
    _p2: u32,
}

@group(0) @binding(1) var<uniform> view: View;
@group(0) @binding(2) var<storage, read> cells: array<u32>;
@group(0) @binding(3) var<storage, read> therm: array<vec2<f32>>;
// Face velocities from air2d.wgsl: U then V, in air cells per tick.
@group(0) @binding(4) var<storage, read> air: array<f32>;

const AIR_CELL: u32 = 4u;

fn wind_color(p: vec2<i32>, base: vec3<f32>) -> vec3<f32> {
    let aw = (view.world_w + AIR_CELL - 1u) / AIR_CELL;
    let ah = (view.world_h + AIR_CELL - 1u) / AIR_CELL;
    let i = u32(p.x) / AIR_CELL;
    let j = u32(p.y) / AIR_CELL;
    let u = 0.5 * (air[j * (aw + 1u) + i] + air[j * (aw + 1u) + i + 1u]);
    let nu = (aw + 1u) * ah;
    let v = 0.5 * (air[nu + j * aw + i] + air[nu + (j + 1u) * aw + i]);
    // Speed in world cells per tick; 2 and up shows at full brightness.
    let speed = length(vec2<f32>(u, v)) * f32(AIR_CELL);
    let a = atan2(-v, u) / 6.2831853;
    let hue = clamp(abs(fract(a + vec3<f32>(0.0, 2.0 / 3.0, 1.0 / 3.0)) * 6.0 - 3.0) - 1.0, vec3<f32>(0.0), vec3<f32>(1.0));
    let k = sqrt(clamp(speed / 2.0, 0.0, 1.0));
    return mix(base * 0.35, hue, k * 0.85);
}

@vertex
fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let uv = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    return vec4<f32>(uv * 2.0 - 1.0, 0.0, 1.0);
}

@fragment
fn fs(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let p = vec2<i32>(floor(pos.xy)) - vec2<i32>(view.offset_x, view.offset_y);
    if (p.x < 0 || p.y < 0 || p.x >= i32(view.world_w) || p.y >= i32(view.world_h)) {
        return vec4<f32>(0.02, 0.02, 0.03, 1.0);
    }
    let i = u32(p.y) * view.world_w + u32(p.x);
    let cell = cells[i];
    let t = therm[i].x;
    if (view.overlay == 1u) {
        let base = select(0.25, 0.6, mat_of(cell) != 0u);
        return vec4<f32>(heat_map(t) * base + heat_map(t) * 0.4, 1.0);
    }
    if (view.overlay == 2u) {
        return vec4<f32>(wind_color(p, cell_color(cell, t)), 1.0);
    }
    return vec4<f32>(cell_color(cell, t), 1.0);
}
