// Ray-marches the voxel grid: one ray per screen pixel, stepping voxel by
// voxel (Amanatides-Woo). Gases are drawn as glowing haze.

struct Camera {
    inv_view_proj: mat4x4<f32>,
    eye: vec4<f32>,
    dims: vec4<u32>,
    // x: plane height, y: brush radius, z: show plane, w: overlay
    extra: vec4<f32>,
    // x, y: viewport size in pixels
    viewport: vec4<f32>,
}

@group(0) @binding(1) var<uniform> cam: Camera;
@group(0) @binding(2) var<storage, read> cells: array<u32>;
@group(0) @binding(3) var<storage, read> therm: array<vec2<f32>>;
@group(0) @binding(4) var<storage, read> pick_in: vec4<f32>;

fn rdims() -> vec3<i32> { return vec3<i32>(cam.dims.xyz); }
fn rinside(p: vec3<i32>) -> bool { return all(p >= vec3<i32>(0)) && all(p < rdims()); }
fn ridx(p: vec3<i32>) -> u32 { return (u32(p.z) * cam.dims.y + u32(p.y)) * cam.dims.x + u32(p.x); }

@vertex
fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let uv = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    return vec4<f32>(uv * 2.0 - 1.0, 0.0, 1.0);
}

fn background(ro: vec3<f32>, rd: vec3<f32>) -> vec3<f32> {
    let size = vec3<f32>(rdims());
    if (rd.y < 0.0) {
        let t = -ro.y / rd.y;
        let p = ro + rd * t;
        if (p.x >= 0.0 && p.z >= 0.0 && p.x <= size.x && p.z <= size.z) {
            let check = (i32(floor(p.x / 8.0)) + i32(floor(p.z / 8.0))) & 1;
            return select(vec3<f32>(0.10, 0.11, 0.13), vec3<f32>(0.13, 0.14, 0.17), check == 1);
        }
    }
    let up = clamp(rd.y * 0.5 + 0.5, 0.0, 1.0);
    return mix(vec3<f32>(0.03, 0.03, 0.05), vec3<f32>(0.10, 0.13, 0.20), up);
}

@fragment
fn fs(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let ndc = vec2<f32>(pos.x / cam.viewport.x * 2.0 - 1.0, 1.0 - pos.y / cam.viewport.y * 2.0);
    let near = cam.inv_view_proj * vec4<f32>(ndc, 0.0, 1.0);
    let far = cam.inv_view_proj * vec4<f32>(ndc, 1.0, 1.0);
    let ro = near.xyz / near.w;
    let rd = safe_dir(normalize(far.xyz / far.w - ro));
    let size = vec3<f32>(rdims());

    var haze = vec3<f32>(0.0);
    var haze_a = 0.0;
    var color = background(ro, rd);
    var hit_t = 1e30;

    let inv = 1.0 / rd;
    let a = -ro * inv;
    let b = (size - ro) * inv;
    let t0 = max(max(min(a.x, b.x), min(a.y, b.y)), min(a.z, b.z));
    let t1 = min(min(max(a.x, b.x), max(a.y, b.y)), max(a.z, b.z));
    if (t0 <= t1 && t1 > 0.0) {
        let t = max(t0, 0.0) + 1e-3;
        var cell = clamp(vec3<i32>(floor(ro + rd * t)), vec3<i32>(0), rdims() - 1);
        let step = vec3<i32>(sign(rd));
        let delta = abs(inv);
        var tmax = (vec3<f32>(cell) + max(vec3<f32>(step), vec3<f32>(0.0)) - ro) * inv;
        var normal = vec3<f32>(0.0, 1.0, 0.0);
        if (t0 > 0.0) {
            // Entry face of the world box.
            if (t0 == min(a.x, b.x)) { normal = vec3<f32>(-sign(rd.x), 0.0, 0.0); }
            else if (t0 == min(a.z, b.z)) { normal = vec3<f32>(0.0, 0.0, -sign(rd.z)); }
            else { normal = vec3<f32>(0.0, -sign(rd.y), 0.0); }
        }
        var t_cur = max(t0, 0.0);
        let steps = cam.dims.x + cam.dims.y + cam.dims.z;
        for (var s = 0u; s < steps; s++) {
            let i = ridx(cell);
            let c = cells[i];
            let m = mat_of(c);
            if (m != 0u) {
                let temp = therm[i].x;
                var col = cell_color(c, temp);
                if (cam.extra.w > 0.5) { col = heat_map(temp); }
                if (kind_of(m) == KIND_GAS) {
                    let alpha = 0.10 + 0.25 * materials.m[m].glow;
                    haze += (1.0 - haze_a) * alpha * col;
                    haze_a += (1.0 - haze_a) * alpha;
                    if (haze_a > 0.97) { break; }
                } else {
                    let light = normalize(vec3<f32>(0.45, 0.85, 0.3));
                    let diffuse = max(dot(normal, light), 0.0);
                    let face = 0.55 + 0.45 * diffuse + 0.08 * normal.z;
                    let emissive = blackbody(temp).a + materials.m[m].glow * 0.5;
                    color = col * mix(face, 1.0, clamp(emissive, 0.0, 1.0));
                    // Brush cursor.
                    let center = pick_in.xyz;
                    if (pick_in.w > 0.5 && length(vec3<f32>(cell) + 0.5 - center) <= cam.extra.y + 1.0) {
                        color = mix(color, vec3<f32>(1.0), 0.25);
                    }
                    hit_t = t_cur;
                    break;
                }
            }
            if (tmax.x < tmax.y && tmax.x < tmax.z) {
                t_cur = tmax.x; cell.x += step.x; tmax.x += delta.x; normal = vec3<f32>(-f32(step.x), 0.0, 0.0);
            } else if (tmax.y < tmax.z) {
                t_cur = tmax.y; cell.y += step.y; tmax.y += delta.y; normal = vec3<f32>(0.0, -f32(step.y), 0.0);
            } else {
                t_cur = tmax.z; cell.z += step.z; tmax.z += delta.z; normal = vec3<f32>(0.0, 0.0, -f32(step.z));
            }
            if (!rinside(cell)) { break; }
        }
    }

    // The drawing plane, shown as a translucent grid where the ray crosses it.
    if (cam.extra.z > 0.5 && abs(rd.y) > 1e-4) {
        let tp = (cam.extra.x + 0.5 - ro.y) / rd.y;
        let p = ro + rd * tp;
        if (tp > 0.0 && tp < hit_t && p.x >= 0.0 && p.z >= 0.0 && p.x <= size.x && p.z <= size.z) {
            let g = fract(p.xz / 8.0);
            let line = select(0.10, 0.35, any(g < vec2<f32>(0.06)));
            color = mix(color, vec3<f32>(0.35, 0.7, 1.0), line);
        }
    }

    color = color * (1.0 - haze_a) + haze;
    return vec4<f32>(color, 1.0);
}
