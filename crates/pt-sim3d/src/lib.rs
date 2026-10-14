//! The 3D prototype: the shared material core on a voxel grid, simulated with
//! WGSL compute shaders and drawn by ray-marching the grid.

use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3};
use pt_core::gpu::{compute_pipeline, layout, material_buffer, Gpu, UNIFORM_SLOT};
pub use pt_core::Tool;
use pt_core::{fill_for, shader_source, MaterialTable};
use wgpu::util::DeviceExt;
use wgpu::ShaderStages as S;

pub mod scenes;

const PARAM_SLOTS: u64 = 128;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
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

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct BrushUniform {
    origin: [f32; 3],
    radius: f32,
    dir: [f32; 3],
    temp: f32,
    mat: u32,
    mode: u32,
    seed: u32,
    fill: f32,
    nx: u32,
    ny: u32,
    nz: u32,
    plane_y: f32,
    target: u32,
    extent: u32,
    _p: [u32; 2],
}

/// Where a 3D brush stroke lands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    /// On the first surface the touch ray hits, or the floor.
    Surface,
    /// On the horizontal drawing plane at `Sim3d::plane_y`.
    Plane,
}

/// A brush dab along a ray from the camera through the touch point.
#[derive(Debug, Clone, Copy)]
pub struct Stroke3 {
    pub origin: Vec3,
    pub dir: Vec3,
    pub radius: f32,
    pub tool: Tool,
    pub target: Target,
}

pub struct Sim3d {
    pub dims: [u32; 3],
    pub tick: u32,
    pub seed: u32,
    pub heat_speed: f32,
    pub move_substeps: u32,
    /// Height of the drawing plane, in voxels.
    pub plane_y: f32,
    pub table: MaterialTable,

    materials: wgpu::Buffer,
    cells: [wgpu::Buffer; 2],
    therm: [wgpu::Buffer; 2],
    params: wgpu::Buffer,
    brush: wgpu::Buffer,
    pick: wgpu::Buffer,
    current: usize,

    step_groups: [wgpu::BindGroup; 2],
    paint_groups: [wgpu::BindGroup; 2],
    heat_pipe: wgpu::ComputePipeline,
    move_pipe: wgpu::ComputePipeline,
    pick_pipe: wgpu::ComputePipeline,
    paint_pipe: wgpu::ComputePipeline,
    pending_stroke: Option<Stroke3>,
}

fn entry(binding: u32, resource: wgpu::BindingResource<'_>) -> wgpu::BindGroupEntry<'_> {
    wgpu::BindGroupEntry { binding, resource }
}

impl Sim3d {
    /// Bytes of GPU memory a world of this size needs (two copies of 12 bytes
    /// per voxel).
    pub fn memory_for(dims: [u32; 3]) -> u64 {
        dims.iter().map(|&d| d as u64).product::<u64>() * 24
    }

    /// Whether this device can hold a world of this size in one buffer.
    pub fn fits(gpu: &Gpu, dims: [u32; 3]) -> bool {
        let n = dims.iter().map(|&d| d as u64).product::<u64>();
        let limits = gpu.device.limits();
        n * 8 <= limits.max_storage_buffer_binding_size as u64 && n * 8 <= limits.max_buffer_size
    }

    pub fn new(gpu: &Gpu, table: MaterialTable, dims: [u32; 3], seed: u32) -> Sim3d {
        let device = &gpu.device;
        let n = dims.iter().map(|&d| d as u64).product::<u64>();
        let storage = wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::COPY_SRC;
        let make = |label: &str, bytes: u64| {
            device.create_buffer(&wgpu::BufferDescriptor { label: Some(label), size: bytes, usage: storage, mapped_at_creation: false })
        };
        let cells = [make("voxels a", n * 4), make("voxels b", n * 4)];
        let therm = [make("vtherm a", n * 8), make("vtherm b", n * 8)];
        let pick = make("pick", 16);
        let materials = material_buffer(device, &table);
        let params = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("sim3d params"),
            size: PARAM_SLOTS * UNIFORM_SLOT,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let brush = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("brush3d"),
            contents: bytemuck::bytes_of(&BrushUniform::zeroed()),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let step_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("sim3d step"),
            entries: &[
                layout::uniform(0, S::COMPUTE, false),
                layout::uniform(1, S::COMPUTE, true),
                layout::storage(2, S::COMPUTE, true),
                layout::storage(3, S::COMPUTE, true),
                layout::storage(4, S::COMPUTE, false),
                layout::storage(5, S::COMPUTE, false),
            ],
        });
        let paint_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("sim3d paint"),
            entries: &[
                layout::uniform(0, S::COMPUTE, false),
                layout::uniform(1, S::COMPUTE, false),
                layout::storage(2, S::COMPUTE, false),
                layout::storage(3, S::COMPUTE, false),
                layout::storage(4, S::COMPUTE, false),
            ],
        });
        let params_binding = wgpu::BindingResource::Buffer(wgpu::BufferBinding {
            buffer: &params,
            offset: 0,
            size: wgpu::BufferSize::new(std::mem::size_of::<Params>() as u64),
        });
        let step_groups = [0usize, 1].map(|src| {
            let dst = 1 - src;
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("sim3d step"),
                layout: &step_layout,
                entries: &[
                    entry(0, materials.as_entire_binding()),
                    entry(1, params_binding.clone()),
                    entry(2, cells[src].as_entire_binding()),
                    entry(3, therm[src].as_entire_binding()),
                    entry(4, cells[dst].as_entire_binding()),
                    entry(5, therm[dst].as_entire_binding()),
                ],
            })
        });
        let paint_groups = [0usize, 1].map(|i| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("sim3d paint"),
                layout: &paint_layout,
                entries: &[
                    entry(0, materials.as_entire_binding()),
                    entry(1, brush.as_entire_binding()),
                    entry(2, cells[i].as_entire_binding()),
                    entry(3, therm[i].as_entire_binding()),
                    entry(4, pick.as_entire_binding()),
                ],
            })
        });
        let sim_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("sim3d"),
            source: wgpu::ShaderSource::Wgsl(shader_source(include_str!("sim3d.wgsl")).into()),
        });
        let paint_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("paint3d"),
            source: wgpu::ShaderSource::Wgsl(shader_source(include_str!("paint3d.wgsl")).into()),
        });
        let step_pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("sim3d step"),
            bind_group_layouts: &[Some(&step_layout)],
            immediate_size: 0,
        });
        let paint_pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("sim3d paint"),
            bind_group_layouts: &[Some(&paint_layout)],
            immediate_size: 0,
        });

        let sim = Sim3d {
            dims,
            tick: 0,
            seed,
            heat_speed: 1.0,
            move_substeps: 4,
            plane_y: dims[1] as f32 * 0.25,
            heat_pipe: compute_pipeline(device, &step_pl, &sim_module, "heat"),
            move_pipe: compute_pipeline(device, &step_pl, &sim_module, "move_blocks"),
            pick_pipe: compute_pipeline(device, &paint_pl, &paint_module, "pick"),
            paint_pipe: compute_pipeline(device, &paint_pl, &paint_module, "paint"),
            table,
            materials,
            cells,
            therm,
            params,
            brush,
            pick,
            current: 0,
            step_groups,
            paint_groups,
            pending_stroke: None,
        };
        sim.clear(gpu);
        sim
    }

    pub fn cell_count(&self) -> u64 {
        self.dims.iter().map(|&d| d as u64).product()
    }

    pub fn index(&self, x: u32, y: u32, z: u32) -> usize {
        ((z as u64 * self.dims[1] as u64 + y as u64) * self.dims[0] as u64 + x as u64) as usize
    }

    pub fn clear(&self, gpu: &Gpu) {
        let n = self.cell_count() as usize;
        self.upload(gpu, &vec![0u32; n], &vec![20.0f32; n]);
    }

    pub fn upload(&self, gpu: &Gpu, cells: &[u32], temps: &[f32]) {
        let n = self.cell_count() as usize;
        assert_eq!(cells.len(), n);
        assert_eq!(temps.len(), n);
        let therm: Vec<[f32; 2]> = temps.iter().map(|&t| [t, 0.0]).collect();
        gpu.queue.write_buffer(&self.cells[self.current], 0, bytemuck::cast_slice(cells));
        gpu.queue.write_buffer(&self.therm[self.current], 0, bytemuck::cast_slice(&therm));
    }

    pub fn stroke(&mut self, stroke: Stroke3) {
        self.pending_stroke = Some(stroke);
    }

    /// Records any pending stroke and `ticks` ticks. Call once per submit.
    pub fn encode(&mut self, queue: &wgpu::Queue, encoder: &mut wgpu::CommandEncoder, ticks: u32) {
        if let Some(stroke) = self.pending_stroke.take() {
            self.encode_stroke(queue, encoder, stroke);
        }
        if ticks == 0 {
            return;
        }
        let per_tick = 1 + self.move_substeps as u64;
        assert!(ticks as u64 * per_tick <= PARAM_SLOTS, "too many ticks for one submit");
        let mut slots = vec![0u8; (ticks as u64 * per_tick * UNIFORM_SLOT) as usize];
        let mut slot = 0usize;
        for t in 0..ticks {
            let tick = self.tick.wrapping_add(t);
            for s in 0..per_tick as u32 {
                let p = Params {
                    nx: self.dims[0],
                    ny: self.dims[1],
                    nz: self.dims[2],
                    tick,
                    seed: self.seed,
                    offset: tick.wrapping_add(s) & 1,
                    salt: s.wrapping_mul(0x9e37_79b9),
                    heat_speed: self.heat_speed,
                };
                let at = slot * UNIFORM_SLOT as usize;
                slots[at..at + std::mem::size_of::<Params>()].copy_from_slice(bytemuck::bytes_of(&p));
                slot += 1;
            }
        }
        queue.write_buffer(&self.params, 0, &slots);

        let [nx, ny, nz] = self.dims;
        let cells = [nx.div_ceil(4), ny.div_ceil(4), nz.div_ceil(4)];
        let blocks = [(nx / 2 + 1).div_ceil(4), (ny / 2 + 1).div_ceil(4), (nz / 2 + 1).div_ceil(4)];
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("sim3d"), timestamp_writes: None });
        let mut slot = 0u32;
        for _ in 0..ticks {
            let group = &self.step_groups[self.current];
            pass.set_pipeline(&self.heat_pipe);
            pass.set_bind_group(0, group, &[slot * UNIFORM_SLOT as u32]);
            pass.dispatch_workgroups(cells[0], cells[1], cells[2]);
            slot += 1;
            pass.set_pipeline(&self.move_pipe);
            for _ in 0..self.move_substeps {
                pass.set_bind_group(0, group, &[slot * UNIFORM_SLOT as u32]);
                pass.dispatch_workgroups(blocks[0], blocks[1], blocks[2]);
                slot += 1;
            }
            self.current = 1 - self.current;
        }
        drop(pass);
        self.tick = self.tick.wrapping_add(ticks);
    }

    fn encode_stroke(&self, queue: &wgpu::Queue, encoder: &mut wgpu::CommandEncoder, s: Stroke3) {
        let (mode, mat) = s.tool.mode_and_material();
        let def = self.table.def(mat);
        let radius = s.radius.max(0.5);
        let extent = 2 * radius.ceil() as u32 + 2;
        let u = BrushUniform {
            origin: s.origin.to_array(),
            radius,
            dir: s.dir.normalize_or(Vec3::NEG_Y).to_array(),
            temp: def.spawn_temp,
            mat,
            mode,
            seed: self.tick.wrapping_mul(0x2545_f491) ^ self.seed,
            fill: fill_for(def.kind),
            nx: self.dims[0],
            ny: self.dims[1],
            nz: self.dims[2],
            plane_y: self.plane_y,
            target: match s.target {
                Target::Surface => 0,
                Target::Plane => 1,
            },
            extent,
            _p: [0; 2],
        };
        queue.write_buffer(&self.brush, 0, bytemuck::bytes_of(&u));
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("paint3d"), timestamp_writes: None });
        pass.set_bind_group(0, &self.paint_groups[self.current], &[]);
        pass.set_pipeline(&self.pick_pipe);
        pass.dispatch_workgroups(1, 1, 1);
        pass.set_pipeline(&self.paint_pipe);
        let groups = extent.div_ceil(4);
        pass.dispatch_workgroups(groups, groups, groups);
    }

    pub fn run(&mut self, gpu: &Gpu, ticks: u32) {
        let per_submit = (PARAM_SLOTS / (1 + self.move_substeps as u64)) as u32;
        let mut left = ticks;
        loop {
            let n = left.min(per_submit);
            let mut encoder = gpu.device.create_command_encoder(&Default::default());
            self.encode(&gpu.queue, &mut encoder, n);
            gpu.queue.submit([encoder.finish()]);
            left -= n;
            if left == 0 {
                break;
            }
        }
        gpu.wait();
    }

    pub fn read_cells(&self, gpu: &Gpu) -> Vec<u32> {
        gpu.read_buffer(&self.cells[self.current], 0, self.cell_count() as usize)
    }

    pub fn read_therm(&self, gpu: &Gpu) -> Vec<[f32; 2]> {
        gpu.read_buffer(&self.therm[self.current], 0, self.cell_count() as usize)
    }

    /// Where the last brush ray landed, if anywhere.
    pub fn read_pick(&self, gpu: &Gpu) -> Option<Vec3> {
        let p: Vec<[f32; 4]> = gpu.read_buffer(&self.pick, 0, 1);
        (p[0][3] > 0.5).then(|| Vec3::new(p[0][0], p[0][1], p[0][2]))
    }
}

/// An orbit camera around the middle of the world.
#[derive(Debug, Clone, Copy)]
pub struct OrbitCamera {
    pub yaw: f32,
    pub pitch: f32,
    pub distance: f32,
    pub target: Vec3,
    pub fov_y: f32,
}

impl OrbitCamera {
    pub fn looking_at(dims: [u32; 3]) -> OrbitCamera {
        let size = Vec3::new(dims[0] as f32, dims[1] as f32, dims[2] as f32);
        OrbitCamera {
            yaw: 0.7,
            pitch: 0.45,
            distance: size.length() * 1.15,
            target: size * Vec3::new(0.5, 0.35, 0.5),
            fov_y: 50f32.to_radians(),
        }
    }

    pub fn eye(&self) -> Vec3 {
        let dir = Vec3::new(self.pitch.cos() * self.yaw.sin(), self.pitch.sin(), self.pitch.cos() * self.yaw.cos());
        self.target + dir * self.distance
    }

    pub fn view_proj(&self, aspect: f32) -> Mat4 {
        let view = glam::camera::rh::view::look_at_mat4(self.eye(), self.target, Vec3::Y);
        let proj = glam::camera::rh::proj::directx::perspective(self.fov_y, aspect, 0.5, self.distance * 4.0 + 1000.0);
        proj * view
    }

    /// The ray from the camera through a pixel.
    pub fn ray(&self, pixel: [f32; 2], viewport: [f32; 2]) -> (Vec3, Vec3) {
        let inv = self.view_proj(viewport[0] / viewport[1]).inverse();
        let ndc = [pixel[0] / viewport[0] * 2.0 - 1.0, 1.0 - pixel[1] / viewport[1] * 2.0];
        let near = inv.project_point3(Vec3::new(ndc[0], ndc[1], 0.0));
        let far = inv.project_point3(Vec3::new(ndc[0], ndc[1], 1.0));
        (near, (far - near).normalize())
    }

    pub fn orbit(&mut self, dx: f32, dy: f32) {
        self.yaw -= dx * 0.008;
        self.pitch = (self.pitch + dy * 0.008).clamp(-0.2, 1.5);
    }

    pub fn zoom(&mut self, factor: f32) {
        self.distance = (self.distance * factor).clamp(10.0, 2000.0);
    }
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct CameraUniform {
    inv_view_proj: [[f32; 4]; 4],
    eye: [f32; 4],
    dims: [u32; 4],
    extra: [f32; 4],
    viewport: [f32; 4],
}

/// Draws a [`Sim3d`] by ray-marching it.
pub struct Renderer3d {
    pipeline: wgpu::RenderPipeline,
    camera: wgpu::Buffer,
    groups: [wgpu::BindGroup; 2],
}

/// What the 3D view shows besides the voxels.
#[derive(Debug, Clone, Copy, Default)]
pub struct View3d {
    pub show_plane: bool,
    pub brush_radius: f32,
    pub temperature: bool,
}

impl Renderer3d {
    pub fn new(gpu: &Gpu, sim: &Sim3d, format: wgpu::TextureFormat) -> Renderer3d {
        let device = &gpu.device;
        let camera = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("camera3d"),
            size: std::mem::size_of::<CameraUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("render3d"),
            entries: &[
                layout::uniform(0, S::FRAGMENT, false),
                layout::uniform(1, S::FRAGMENT, false),
                layout::storage(2, S::FRAGMENT, true),
                layout::storage(3, S::FRAGMENT, true),
                layout::storage(4, S::FRAGMENT, true),
            ],
        });
        let groups = [0usize, 1].map(|i| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("render3d"),
                layout: &bgl,
                entries: &[
                    entry(0, sim.materials.as_entire_binding()),
                    entry(1, camera.as_entire_binding()),
                    entry(2, sim.cells[i].as_entire_binding()),
                    entry(3, sim.therm[i].as_entire_binding()),
                    entry(4, sim.pick.as_entire_binding()),
                ],
            })
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("render3d"),
            source: wgpu::ShaderSource::Wgsl(shader_source(include_str!("render3d.wgsl")).into()),
        });
        let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("render3d"),
            bind_group_layouts: &[Some(&bgl)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("render3d"),
            layout: Some(&pl),
            vertex: wgpu::VertexState { module: &module, entry_point: Some("vs"), compilation_options: Default::default(), buffers: &[] },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(format.into())],
            }),
            multiview_mask: None,
            cache: None,
        });
        Renderer3d { pipeline, camera, groups }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn draw(
        &self,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        viewport: [u32; 2],
        sim: &Sim3d,
        camera: &OrbitCamera,
        view: View3d,
    ) {
        let aspect = viewport[0] as f32 / viewport[1].max(1) as f32;
        let u = CameraUniform {
            inv_view_proj: camera.view_proj(aspect).inverse().to_cols_array_2d(),
            eye: camera.eye().extend(1.0).to_array(),
            dims: [sim.dims[0], sim.dims[1], sim.dims[2], 0],
            extra: [sim.plane_y, view.brush_radius, view.show_plane as u32 as f32, view.temperature as u32 as f32],
            viewport: [viewport[0] as f32, viewport[1] as f32, 0.0, 0.0],
        };
        queue.write_buffer(&self.camera, 0, bytemuck::bytes_of(&u));
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("render3d"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.groups[sim.current], &[]);
        pass.draw(0..3, 0..1);
    }
}
