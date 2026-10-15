//! The 2D prototype: a falling-sand world at the screen's native resolution,
//! simulated with WGSL compute shaders.

use bytemuck::{Pod, Zeroable};
use pt_core::gpu::{compute_pipeline, layout, material_buffer, Gpu, UNIFORM_SLOT};
pub use pt_core::Tool;
use pt_core::{fill_for, shader_source, MaterialTable};
use wgpu::util::DeviceExt;
use wgpu::ShaderStages as S;

pub mod scenes;

/// Uniform slots available per submit: one heat pass plus the movement
/// passes for each tick.
const PARAM_SLOTS: u64 = 128;

/// Side of a sleeping chunk in cells; matches `CHUNK` in sim2d.wgsl.
pub const CHUNK: u32 = 64;

/// Block movement passes per tick unless the player changes it. Free falls
/// run in their own pass; these set how fast liquids spread, grains slide
/// and things sink through liquids, at most one cell per pass.
pub const DEFAULT_SUBSTEPS: u32 = 8;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
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

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct BrushUniform {
    ax: f32,
    ay: f32,
    bx: f32,
    by: f32,
    radius: f32,
    mat: u32,
    temp: f32,
    mode: u32,
    origin_x: i32,
    origin_y: i32,
    width: u32,
    height: u32,
    seed: u32,
    fill: f32,
    _p0: u32,
    _p1: u32,
}

/// One brush stroke segment in world cells.
#[derive(Debug, Clone, Copy)]
pub struct Stroke {
    pub from: [f32; 2],
    pub to: [f32; 2],
    pub radius: f32,
    pub tool: Tool,
}

pub struct Sim2d {
    pub width: u32,
    pub height: u32,
    pub tick: u32,
    pub seed: u32,
    /// Multiplier on every material's conductivity.
    pub heat_speed: f32,
    /// Movement passes per tick. Each pass moves a cell at most one cell, so
    /// this sets how fast things fall and flow.
    pub move_substeps: u32,
    pub table: MaterialTable,

    materials: wgpu::Buffer,
    cells: [wgpu::Buffer; 2],
    therm: [wgpu::Buffer; 2],
    /// Per chunk: something moved (bit 0) or warmed (bit 1) there this tick.
    moved: wgpu::Buffer,
    /// Per chunk: movement passes run there (bit 0), and for how many more
    /// ticks the heat pass runs (bits 1 and up).
    awake: wgpu::Buffer,
    params: wgpu::Buffer,
    brush: wgpu::Buffer,
    /// Index of the buffer pair holding the latest state.
    current: usize,

    step_groups: [wgpu::BindGroup; 2],
    paint_groups: [wgpu::BindGroup; 2],
    heat_pipe: wgpu::ComputePipeline,
    move_pipe: wgpu::ComputePipeline,
    fall_pipe: wgpu::ComputePipeline,
    settle_pipe: wgpu::ComputePipeline,
    reset_pipe: wgpu::ComputePipeline,
    paint_pipe: wgpu::ComputePipeline,
    pending_stroke: Option<Stroke>,
}

impl Sim2d {
    pub fn new(gpu: &Gpu, table: MaterialTable, width: u32, height: u32, seed: u32) -> Sim2d {
        let device = &gpu.device;
        let cell_count = (width as u64) * (height as u64);
        let storage = wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::COPY_SRC;
        let make = |label: &str, bytes: u64| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size: bytes,
                usage: storage,
                mapped_at_creation: false,
            })
        };
        let cells = [make("cells a", cell_count * 4), make("cells b", cell_count * 4)];
        let therm = [make("therm a", cell_count * 8), make("therm b", cell_count * 8)];
        let chunk_count = (width.div_ceil(CHUNK) * height.div_ceil(CHUNK)) as u64;
        let moved = make("chunks moved", chunk_count * 4);
        let awake = make("chunks awake", chunk_count * 4);
        let materials = material_buffer(device, &table);
        let params = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("sim2d params"),
            size: PARAM_SLOTS * UNIFORM_SLOT,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let brush = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("brush"),
            contents: bytemuck::bytes_of(&BrushUniform::zeroed()),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let step_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("sim2d step"),
            entries: &[
                layout::uniform(0, S::COMPUTE, false),
                layout::uniform(1, S::COMPUTE, true),
                layout::storage(2, S::COMPUTE, true),
                layout::storage(3, S::COMPUTE, true),
                layout::storage(4, S::COMPUTE, false),
                layout::storage(5, S::COMPUTE, false),
                layout::storage(6, S::COMPUTE, false),
                layout::storage(7, S::COMPUTE, false),
            ],
        });
        let paint_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("sim2d paint"),
            entries: &[
                layout::uniform(0, S::COMPUTE, false),
                layout::uniform(1, S::COMPUTE, false),
                layout::storage(2, S::COMPUTE, false),
                layout::storage(3, S::COMPUTE, false),
                layout::storage(4, S::COMPUTE, false),
                layout::storage(5, S::COMPUTE, false),
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
                label: Some("sim2d step"),
                layout: &step_layout,
                entries: &[
                    entry(0, materials.as_entire_binding()),
                    entry(1, params_binding.clone()),
                    entry(2, cells[src].as_entire_binding()),
                    entry(3, therm[src].as_entire_binding()),
                    entry(4, cells[dst].as_entire_binding()),
                    entry(5, therm[dst].as_entire_binding()),
                    entry(6, moved.as_entire_binding()),
                    entry(7, awake.as_entire_binding()),
                ],
            })
        });
        let paint_groups = [0usize, 1].map(|i| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("sim2d paint"),
                layout: &paint_layout,
                entries: &[
                    entry(0, materials.as_entire_binding()),
                    entry(1, brush.as_entire_binding()),
                    entry(2, cells[i].as_entire_binding()),
                    entry(3, therm[i].as_entire_binding()),
                    entry(4, moved.as_entire_binding()),
                    entry(5, awake.as_entire_binding()),
                ],
            })
        });

        let sim_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("sim2d"),
            source: wgpu::ShaderSource::Wgsl(shader_source(include_str!("sim2d.wgsl")).into()),
        });
        let paint_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("paint2d"),
            source: wgpu::ShaderSource::Wgsl(shader_source(include_str!("paint2d.wgsl")).into()),
        });
        let step_pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("sim2d step"),
            bind_group_layouts: &[Some(&step_layout)],
            immediate_size: 0,
        });
        let paint_pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("sim2d paint"),
            bind_group_layouts: &[Some(&paint_layout)],
            immediate_size: 0,
        });

        let sim = Sim2d {
            width,
            height,
            tick: 0,
            seed,
            heat_speed: 1.0,
            move_substeps: DEFAULT_SUBSTEPS,
            heat_pipe: compute_pipeline(device, &step_pl, &sim_module, "heat"),
            move_pipe: compute_pipeline(device, &step_pl, &sim_module, "move_blocks"),
            fall_pipe: compute_pipeline(device, &step_pl, &sim_module, "fall"),
            settle_pipe: compute_pipeline(device, &step_pl, &sim_module, "settle"),
            reset_pipe: compute_pipeline(device, &step_pl, &sim_module, "reset"),
            paint_pipe: compute_pipeline(device, &paint_pl, &paint_module, "paint"),
            table,
            materials,
            cells,
            therm,
            moved,
            awake,
            params,
            brush,
            current: 0,
            step_groups,
            paint_groups,
            pending_stroke: None,
        };
        sim.clear(gpu);
        sim
    }

    pub fn cell_count(&self) -> u64 {
        self.width as u64 * self.height as u64
    }

    /// The chunk grid's size in chunks.
    pub fn chunk_dims(&self) -> [u32; 2] {
        [self.width.div_ceil(CHUNK), self.height.div_ceil(CHUNK)]
    }

    /// Fills the world with air at room temperature.
    pub fn clear(&self, gpu: &Gpu) {
        let n = self.cell_count() as usize;
        self.upload(gpu, &vec![0u32; n], &vec![20.0f32; n]);
    }

    /// Replaces the whole world. `cells` are packed cells (see `make_cell`),
    /// `temps` are degrees Celsius.
    pub fn upload(&self, gpu: &Gpu, cells: &[u32], temps: &[f32]) {
        let n = self.cell_count() as usize;
        assert_eq!(cells.len(), n);
        assert_eq!(temps.len(), n);
        let therm: Vec<[f32; 2]> = temps.iter().map(|&t| [t, 0.0]).collect();
        gpu.queue.write_buffer(&self.cells[self.current], 0, bytemuck::cast_slice(cells));
        gpu.queue.write_buffer(&self.therm[self.current], 0, bytemuck::cast_slice(&therm));
        self.wake_all(gpu);
    }

    /// Wakes every chunk, so the next tick looks at the whole world.
    pub fn wake_all(&self, gpu: &Gpu) {
        let [cw, ch] = self.chunk_dims();
        let n = (cw * ch) as usize;
        gpu.queue.write_buffer(&self.moved, 0, bytemuck::cast_slice(&vec![3u32; n]));
        gpu.queue
            .write_buffer(&self.awake, 0, bytemuck::cast_slice(&vec![1u32 | 2 << 1; n]));
    }

    /// Queues a brush stroke; it is applied at the start of the next
    /// [`encode`](Self::encode). One stroke per frame is enough for one finger,
    /// and longer moves are drawn as a capsule so nothing is skipped.
    pub fn stroke(&mut self, stroke: Stroke) {
        self.pending_stroke = Some(stroke);
    }

    /// Records any pending stroke and `ticks` simulation ticks. Call once per
    /// queue submit: the tick parameters share one uniform buffer.
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
        let mut push = |p: Params| {
            let at = slot * UNIFORM_SLOT as usize;
            slots[at..at + std::mem::size_of::<Params>()].copy_from_slice(bytemuck::bytes_of(&p));
            slot += 1;
        };
        for t in 0..ticks {
            let tick = self.tick.wrapping_add(t);
            for s in 0..per_tick as u32 {
                push(Params {
                    width: self.width,
                    height: self.height,
                    tick,
                    seed: self.seed,
                    // Pass 0 is heat; movement passes alternate the block grid.
                    offset: (tick.wrapping_add(s)) & 1,
                    salt: s.wrapping_mul(0x9e37_79b9),
                    heat_speed: self.heat_speed,
                    _pad: 0,
                });
            }
        }
        queue.write_buffer(&self.params, 0, &slots);

        let cells_x = self.width.div_ceil(8);
        let cells_y = self.height.div_ceil(8);
        let blocks_x = (self.width / 2 + 1).div_ceil(8);
        let blocks_y = (self.height / 2 + 1).div_ceil(8);
        let [cw, ch] = self.chunk_dims();
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("sim2d"),
            timestamp_writes: None,
        });
        let mut slot = 0u32;
        for _ in 0..ticks {
            let group = &self.step_groups[self.current];
            pass.set_pipeline(&self.heat_pipe);
            pass.set_bind_group(0, group, &[slot * UNIFORM_SLOT as u32]);
            pass.dispatch_workgroups(cells_x, cells_y, 1);
            pass.set_pipeline(&self.fall_pipe);
            pass.dispatch_workgroups(self.width.div_ceil(64), 1, 1);
            slot += 1;
            pass.set_pipeline(&self.move_pipe);
            for _ in 0..self.move_substeps {
                pass.set_bind_group(0, group, &[slot * UNIFORM_SLOT as u32]);
                pass.dispatch_workgroups(blocks_x, blocks_y, 1);
                slot += 1;
            }
            // The chunk passes only read the world size, so any slot will do.
            let last = (slot - 1) * UNIFORM_SLOT as u32;
            pass.set_bind_group(0, group, &[last]);
            pass.set_pipeline(&self.settle_pipe);
            pass.dispatch_workgroups(cw.div_ceil(8), ch.div_ceil(8), 1);
            pass.set_pipeline(&self.reset_pipe);
            pass.dispatch_workgroups(cw.div_ceil(8), ch.div_ceil(8), 1);
            self.current = 1 - self.current;
        }
        drop(pass);
        self.tick = self.tick.wrapping_add(ticks);
    }

    fn encode_stroke(&self, queue: &wgpu::Queue, encoder: &mut wgpu::CommandEncoder, s: Stroke) {
        let r = s.radius.max(0.5);
        let min_x = (s.from[0].min(s.to[0]) - r).floor() as i32;
        let min_y = (s.from[1].min(s.to[1]) - r).floor() as i32;
        let max_x = (s.from[0].max(s.to[0]) + r).ceil() as i32;
        let max_y = (s.from[1].max(s.to[1]) + r).ceil() as i32;
        let (mode, mat) = s.tool.mode_and_material();
        let def = self.table.def(mat);
        let uniform = BrushUniform {
            ax: s.from[0],
            ay: s.from[1],
            bx: s.to[0],
            by: s.to[1],
            radius: r,
            mat,
            temp: def.spawn_temp,
            mode,
            origin_x: min_x,
            origin_y: min_y,
            width: self.width,
            height: self.height,
            seed: self.tick.wrapping_mul(0x2545_f491) ^ self.seed,
            fill: fill_for(def.kind),
            _p0: 0,
            _p1: 0,
        };
        queue.write_buffer(&self.brush, 0, bytemuck::bytes_of(&uniform));
        let w = (max_x - min_x + 1) as u32;
        let h = (max_y - min_y + 1) as u32;
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("paint2d"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&self.paint_pipe);
        pass.set_bind_group(0, &self.paint_groups[self.current], &[]);
        pass.dispatch_workgroups(w.div_ceil(8), h.div_ceil(8), 1);
    }

    /// Runs `ticks` ticks and waits for the GPU. For tests and benchmarks.
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

    /// Reads which chunks run movement passes next tick, row by row. Slow.
    pub fn read_awake(&self, gpu: &Gpu) -> Vec<bool> {
        self.read_awake_raw(gpu).into_iter().map(|a| a & 1 != 0).collect()
    }

    /// Reads which chunks run the heat pass next tick, row by row. Slow.
    pub fn read_heat_awake(&self, gpu: &Gpu) -> Vec<bool> {
        self.read_awake_raw(gpu).into_iter().map(|a| a >> 1 != 0).collect()
    }

    fn read_awake_raw(&self, gpu: &Gpu) -> Vec<u32> {
        let [cw, ch] = self.chunk_dims();
        gpu.read_buffer(&self.awake, 0, (cw * ch) as usize)
    }

    /// Reads the current cells back. Slow.
    pub fn read_cells(&self, gpu: &Gpu) -> Vec<u32> {
        gpu.read_buffer(&self.cells[self.current], 0, self.cell_count() as usize)
    }

    /// Reads the current temperature and stored latent heat back. Slow.
    pub fn read_therm(&self, gpu: &Gpu) -> Vec<[f32; 2]> {
        gpu.read_buffer(&self.therm[self.current], 0, self.cell_count() as usize)
    }

    /// Reads one cell back: material id, temperature, stored latent heat.
    pub fn inspect(&self, gpu: &Gpu, x: u32, y: u32) -> Option<(u32, f32, f32)> {
        if x >= self.width || y >= self.height {
            return None;
        }
        let i = y as u64 * self.width as u64 + x as u64;
        let cell: Vec<u32> = gpu.read_buffer(&self.cells[self.current], i * 4, 1);
        let th: Vec<[f32; 2]> = gpu.read_buffer(&self.therm[self.current], i * 8, 1);
        Some((cell[0] & 0xff, th[0][0], th[0][1]))
    }

    pub fn current_buffers(&self) -> (&wgpu::Buffer, &wgpu::Buffer) {
        (&self.cells[self.current], &self.therm[self.current])
    }

    fn buffers(&self, i: usize) -> (&wgpu::Buffer, &wgpu::Buffer) {
        (&self.cells[i], &self.therm[i])
    }
}

fn entry(binding: u32, resource: wgpu::BindingResource<'_>) -> wgpu::BindGroupEntry<'_> {
    wgpu::BindGroupEntry { binding, resource }
}

/// Packs a material id and shade into a cell, as the shaders do.
pub fn make_cell(mat: u32, shade: u8) -> u32 {
    (mat & 0xff) | (shade as u32) << 8
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct ViewUniform {
    world_w: u32,
    world_h: u32,
    offset_x: i32,
    offset_y: i32,
    overlay: u32,
    _p: [u32; 3],
}

/// What the 2D view shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Overlay {
    None,
    Temperature,
}

/// Draws a [`Sim2d`] to a render target, one cell per pixel.
pub struct Renderer2d {
    pipeline: wgpu::RenderPipeline,
    view: wgpu::Buffer,
    groups: [wgpu::BindGroup; 2],
}

impl Renderer2d {
    pub fn new(gpu: &Gpu, sim: &Sim2d, format: wgpu::TextureFormat) -> Renderer2d {
        let device = &gpu.device;
        let view = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("view2d"),
            size: std::mem::size_of::<ViewUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("render2d"),
            entries: &[
                layout::uniform(0, S::FRAGMENT, false),
                layout::uniform(1, S::FRAGMENT, false),
                layout::storage(2, S::FRAGMENT, true),
                layout::storage(3, S::FRAGMENT, true),
            ],
        });
        let groups = [0usize, 1].map(|i| {
            let (cells, therm) = sim.buffers(i);
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("render2d"),
                layout: &bgl,
                entries: &[
                    entry(0, sim.materials.as_entire_binding()),
                    entry(1, view.as_entire_binding()),
                    entry(2, cells.as_entire_binding()),
                    entry(3, therm.as_entire_binding()),
                ],
            })
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("render2d"),
            source: wgpu::ShaderSource::Wgsl(shader_source(include_str!("render2d.wgsl")).into()),
        });
        let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("render2d"),
            bind_group_layouts: &[Some(&bgl)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("render2d"),
            layout: Some(&pl),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[],
            },
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
        Renderer2d { pipeline, view, groups }
    }

    /// Draws into `target`. The world's top-left cell lands on pixel `offset`.
    pub fn draw(
        &self,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        sim: &Sim2d,
        offset: [i32; 2],
        overlay: Overlay,
    ) {
        let v = ViewUniform {
            world_w: sim.width,
            world_h: sim.height,
            offset_x: offset[0],
            offset_y: offset[1],
            overlay: overlay as u32,
            _p: [0; 3],
        };
        queue.write_buffer(&self.view, 0, bytemuck::bytes_of(&v));
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("render2d"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
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
