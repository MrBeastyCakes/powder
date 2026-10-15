//! Powder Toy Next, Phase 0 app: the 2D and 3D prototypes behind one touch UI.
//!
//! The target is Android (`android_main`, a Rust NativeActivity). The same
//! code also runs as a desktop window (`run` from `main.rs`) for development.
//! The screen is an upright 4:3 sim area, one cell per pixel, on a control
//! panel about as tall as a keyboard (see `layout`). The 3D prototype is
//! shelved: its code stays, but the app no longer offers it.

use std::sync::Arc;
use std::time::Instant;

use pt_core::gpu::Gpu;
use pt_core::{MaterialTable, Tool};
use pt_sim2d::{Overlay, Renderer2d, Sim2d, Stroke};
use pt_sim3d::{OrbitCamera, Renderer3d, Sim3d, Stroke3, Target, View3d};
use winit::application::ApplicationHandler;
use winit::dpi::PhysicalPosition;
use winit::event::{ElementState, MouseButton, MouseScrollDelta, TouchPhase, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::{Window, WindowId};

mod catalog;
mod layout;
mod ui;

use layout::Layout;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    TwoD,
    ThreeD,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolKind {
    Material,
    Heat,
    Cool,
    Erase,
    /// 3D only: one finger turns the camera instead of painting.
    Camera,
}

/// Everything the player can change from the UI.
pub struct Settings {
    pub mode: Mode,
    pub material: u32,
    pub tool: ToolKind,
    /// Brush radius in screen pixels (2D) or voxels (3D).
    pub brush_2d: f32,
    pub brush_3d: f32,
    pub replace: bool,
    pub paused: bool,
    pub step_once: bool,
    pub temperature_view: bool,
    pub substeps: u32,
    pub heat_speed: f32,
    pub size_3d: u32,
    pub target_3d: Target,
    pub show_menu: bool,
    /// The rail entry the material picker shows.
    pub category: catalog::Category,
    /// Materials painted with most recently, newest first.
    pub recent: Vec<u32>,
    /// The brush size popup above the panel.
    pub show_brush: bool,
}

impl Settings {
    fn tool(&self) -> Tool {
        match self.tool {
            ToolKind::Material if self.replace => Tool::Replace(self.material),
            ToolKind::Material | ToolKind::Camera => Tool::Paint(self.material),
            ToolKind::Heat => Tool::Heat,
            ToolKind::Cool => Tool::Cool,
            ToolKind::Erase => Tool::Erase,
        }
    }
}

struct World2d {
    sim: Sim2d,
    renderer: Renderer2d,
}

struct World3d {
    sim: Sim3d,
    renderer: Renderer3d,
    camera: OrbitCamera,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PointerId {
    Mouse,
    Touch(u64),
}

#[derive(Default)]
struct Pointers {
    touches: Vec<(u64, [f32; 2])>,
    mouse: [f32; 2],
    mouse_left: bool,
    mouse_right: bool,
    /// The pointer currently painting, and where it was last frame.
    paint: Option<(PointerId, [f32; 2])>,
    /// Two-finger gesture: last centroid and finger distance.
    gesture: Option<([f32; 2], f32)>,
}

/// GPU, worlds and UI state; survives the window going away on Android.
struct Engine {
    gpu: Gpu,
    table: MaterialTable,
    settings: Settings,
    world2d: Option<World2d>,
    world3d: Option<World3d>,
    egui_ctx: egui::Context,
    egui_renderer: egui_wgpu::Renderer,
    format: wgpu::TextureFormat,
    stats: ui::Stats,
    /// Screen rectangles (physical pixels) the UI covered last frame.
    ui_rects: Vec<egui::Rect>,
    /// This frame's screen layout.
    layout: Layout,
}

/// The window and its surface; recreated on every resume.
struct Screen {
    window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    egui_state: egui_winit::State,
}

#[derive(Default)]
struct App {
    engine: Option<Engine>,
    screen: Option<Screen>,
    pointers: Pointers,
    last_frame: Option<Instant>,
}

/// Runs the app until the window closes.
pub fn run(event_loop: EventLoop<()>) {
    event_loop.set_control_flow(ControlFlow::Poll);
    let mut app = App::default();
    event_loop.run_app(&mut app).expect("event loop failed");
}

#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
fn android_main(android_app: winit::platform::android::activity::AndroidApp) {
    use winit::platform::android::EventLoopBuilderExtAndroid;
    android_logger::init_once(
        android_logger::Config::default()
            .with_max_level(log::LevelFilter::Info)
            .with_tag("powder-toy"),
    );
    let event_loop = EventLoop::builder()
        .with_android_app(android_app)
        .build()
        .expect("create event loop");
    run(event_loop);
}

/// The layout for this window, kept clear of Android's status and gesture bars.
fn screen_layout(window: &Window, size: [u32; 2]) -> Layout {
    layout::layout(size, window.scale_factor() as f32, system_insets(window, size))
}

/// Rows the status bar (top) and gesture bar (bottom) cover, from the content
/// rectangle Android reports. Before Android reports one, nothing is reserved
/// here and the layout's minimum bottom gap applies.
#[cfg(target_os = "android")]
fn system_insets(window: &Window, size: [u32; 2]) -> [u32; 2] {
    use winit::platform::android::WindowExtAndroid;
    let r = window.content_rect();
    if r.right > r.left && r.bottom > r.top {
        [r.top.max(0) as u32, (size[1] as i32 - r.bottom).max(0) as u32]
    } else {
        [0, 0]
    }
}

#[cfg(not(target_os = "android"))]
fn system_insets(_window: &Window, _size: [u32; 2]) -> [u32; 2] {
    [0, 0]
}

fn surface_config(surface: &wgpu::Surface<'_>, gpu: &Gpu, size: [u32; 2]) -> wgpu::SurfaceConfiguration {
    let caps = surface.get_capabilities(&gpu.adapter);
    // Shaders write sRGB-encoded colors directly, so prefer a non-sRGB format.
    let format = caps.formats.iter().copied().find(|f| !f.is_srgb()).unwrap_or(caps.formats[0]);
    wgpu::SurfaceConfiguration {
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        format,
        width: size[0].max(1),
        height: size[1].max(1),
        present_mode: wgpu::PresentMode::Fifo,
        desired_maximum_frame_latency: 2,
        alpha_mode: caps.alpha_modes[0],
        view_formats: vec![],
        color_space: wgpu::SurfaceColorSpace::Auto,
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.screen.is_some() {
            return;
        }
        let attrs = Window::default_attributes()
            .with_title("Powder Toy Next")
            .with_inner_size(winit::dpi::LogicalSize::new(430, 932));
        let window = Arc::new(event_loop.create_window(attrs).expect("create window"));
        let size = window.inner_size();

        let instance = match &self.engine {
            Some(engine) => engine.gpu.instance.clone(),
            None => Gpu::instance(),
        };
        let surface = instance.create_surface(window.clone()).expect("create surface");
        if self.engine.is_none() {
            let gpu = pollster::block_on(Gpu::open(instance, Some(&surface))).expect("open GPU");
            let format = surface_config(&surface, &gpu, [size.width, size.height]).format;
            let table = MaterialTable::core();
            let egui_ctx = egui::Context::default();
            ui::install_fonts(&egui_ctx);
            let egui_renderer = egui_wgpu::Renderer::new(&gpu.device, format, egui_wgpu::RendererOptions::default());
            let sand = table.expect_id("sand");
            let settings = Settings {
                mode: Mode::TwoD,
                material: sand,
                tool: ToolKind::Material,
                brush_2d: 24.0,
                brush_3d: 3.0,
                replace: false,
                paused: false,
                step_once: false,
                temperature_view: false,
                substeps: 4,
                heat_speed: 1.0,
                size_3d: 128,
                target_3d: Target::Surface,
                show_menu: false,
                category: catalog::Category::Kind(table.def(sand).kind),
                recent: vec![],
                show_brush: false,
            };
            let stats = ui::Stats::new(gpu.adapter_name());
            self.engine = Some(Engine {
                gpu,
                table,
                settings,
                world2d: None,
                world3d: None,
                egui_ctx,
                egui_renderer,
                format,
                stats,
                ui_rects: vec![],
                layout: layout::layout([size.width, size.height], window.scale_factor() as f32, [0, 0]),
            });
        }
        let engine = self.engine.as_mut().unwrap();
        let config = surface_config(&surface, &engine.gpu, [size.width, size.height]);
        surface.configure(&engine.gpu.device, &config);
        if config.format != engine.format {
            engine.format = config.format;
            engine.egui_renderer = egui_wgpu::Renderer::new(&engine.gpu.device, config.format, egui_wgpu::RendererOptions::default());
            engine.world2d = None;
            engine.world3d = None;
        }
        let egui_state = egui_winit::State::new(
            engine.egui_ctx.clone(),
            egui::ViewportId::ROOT,
            &window,
            Some(window.scale_factor() as f32),
            None,
            Some(engine.gpu.device.limits().max_texture_dimension_2d as usize),
        );
        engine.layout = screen_layout(&window, [config.width, config.height]);
        if engine.world2d.is_none() {
            engine.new_world_2d(true);
        }
        window.request_redraw();
        self.screen = Some(Screen {
            window,
            surface,
            config,
            egui_state,
        });
    }

    fn suspended(&mut self, _event_loop: &ActiveEventLoop) {
        // Android destroys the native window when the app goes to the
        // background; the GPU and the worlds stay alive.
        self.screen = None;
        self.pointers = Pointers::default();
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        let (Some(engine), Some(screen)) = (self.engine.as_mut(), self.screen.as_mut()) else {
            return;
        };
        let egui_response = screen.egui_state.on_window_event(&screen.window, &event);

        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                screen.config.width = size.width.max(1);
                screen.config.height = size.height.max(1);
                screen.surface.configure(&engine.gpu.device, &screen.config);
            }
            WindowEvent::RedrawRequested => {
                let now = Instant::now();
                if let Some(last) = self.last_frame {
                    engine.stats.frame((now - last).as_secs_f64() * 1000.0);
                }
                self.last_frame = Some(now);
                engine.frame(screen, &mut self.pointers);
                screen.window.request_redraw();
            }
            WindowEvent::Touch(touch) => {
                let p = [touch.location.x as f32, touch.location.y as f32];
                self.pointers.on_touch(engine, touch.id, touch.phase, p);
            }
            WindowEvent::CursorMoved { position, .. } => self.pointers.on_mouse_move(engine, position),
            WindowEvent::MouseInput { state, button, .. } => {
                if !egui_response.consumed {
                    self.pointers.on_mouse_button(engine, button, state == ElementState::Pressed);
                } else if state == ElementState::Released {
                    self.pointers.on_mouse_button(engine, button, false);
                }
            }
            WindowEvent::MouseWheel { delta, .. } if !egui_response.consumed => {
                let lines = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y,
                    MouseScrollDelta::PixelDelta(p) => p.y as f32 / 40.0,
                };
                match engine.settings.mode {
                    Mode::TwoD => engine.settings.brush_2d = (engine.settings.brush_2d * 1.1f32.powf(lines)).clamp(1.0, 200.0),
                    Mode::ThreeD => {
                        if let Some(w) = engine.world3d.as_mut() {
                            w.camera.zoom(0.9f32.powf(lines));
                        }
                    }
                }
            }
            _ => {}
        }
    }
}

impl Pointers {
    fn over_ui(engine: &Engine, p: [f32; 2]) -> bool {
        engine.ui_rects.iter().any(|r| r.contains(egui::pos2(p[0], p[1])))
    }

    fn paints(engine: &Engine) -> bool {
        !(engine.settings.mode == Mode::ThreeD && engine.settings.tool == ToolKind::Camera)
    }

    fn on_touch(&mut self, engine: &mut Engine, id: u64, phase: TouchPhase, p: [f32; 2]) {
        match phase {
            TouchPhase::Started => {
                if Pointers::over_ui(engine, p) {
                    return;
                }
                self.touches.push((id, p));
                if self.touches.len() == 1 && Pointers::paints(engine) {
                    self.paint = Some((PointerId::Touch(id), p));
                } else {
                    // A second finger turns a stroke into a camera gesture.
                    self.paint = None;
                    self.gesture = None;
                }
            }
            TouchPhase::Moved => {
                let Some(t) = self.touches.iter_mut().find(|t| t.0 == id) else {
                    return;
                };
                let prev = t.1;
                t.1 = p;
                match self.touches.len() {
                    1 if self.paint.is_none() => {
                        if let Some(w) = engine.world3d.as_mut().filter(|_| engine.settings.mode == Mode::ThreeD) {
                            w.camera.orbit(p[0] - prev[0], p[1] - prev[1]);
                        }
                    }
                    2 => {
                        let (a, b) = (self.touches[0].1, self.touches[1].1);
                        let centroid = [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0];
                        let dist = ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt().max(1.0);
                        if let (Some((c0, d0)), Some(w)) = (self.gesture, engine.world3d.as_mut()) {
                            if engine.settings.mode == Mode::ThreeD {
                                w.camera.orbit(centroid[0] - c0[0], centroid[1] - c0[1]);
                                w.camera.zoom(d0 / dist);
                            }
                        }
                        if engine.settings.mode == Mode::TwoD {
                            if let Some((_, d0)) = self.gesture {
                                engine.settings.brush_2d = (engine.settings.brush_2d * dist / d0).clamp(1.0, 200.0);
                            }
                        }
                        self.gesture = Some((centroid, dist));
                    }
                    _ => {}
                }
            }
            TouchPhase::Ended | TouchPhase::Cancelled => {
                self.touches.retain(|t| t.0 != id);
                if self.paint.is_some_and(|(pid, _)| pid == PointerId::Touch(id)) {
                    self.paint = None;
                }
                if self.touches.len() < 2 {
                    self.gesture = None;
                }
            }
        }
    }

    fn on_mouse_move(&mut self, engine: &mut Engine, position: PhysicalPosition<f64>) {
        let p = [position.x as f32, position.y as f32];
        let prev = self.mouse;
        self.mouse = p;
        let orbiting = self.mouse_right || (self.mouse_left && !Pointers::paints(engine));
        if orbiting && engine.settings.mode == Mode::ThreeD {
            if let Some(w) = engine.world3d.as_mut() {
                w.camera.orbit(p[0] - prev[0], p[1] - prev[1]);
            }
        }
    }

    fn on_mouse_button(&mut self, engine: &mut Engine, button: MouseButton, down: bool) {
        match button {
            MouseButton::Left => {
                self.mouse_left = down;
                if down && Pointers::paints(engine) && !Pointers::over_ui(engine, self.mouse) {
                    self.paint = Some((PointerId::Mouse, self.mouse));
                } else if !down && self.paint.is_some_and(|(id, _)| id == PointerId::Mouse) {
                    self.paint = None;
                }
            }
            MouseButton::Right => self.mouse_right = down,
            _ => {}
        }
    }

    fn position(&self, id: PointerId) -> Option<[f32; 2]> {
        match id {
            PointerId::Mouse => Some(self.mouse),
            PointerId::Touch(t) => self.touches.iter().find(|x| x.0 == t).map(|x| x.1),
        }
    }
}

impl Engine {
    /// A new 2D world the size of the layout's sim area.
    fn new_world_2d(&mut self, demo: bool) {
        let size = [self.layout.sim[2].max(2), self.layout.sim[3].max(2)];
        let mut sim = Sim2d::new(&self.gpu, self.table.clone(), size[0], size[1], 0x5eed);
        if demo {
            let scene = pt_sim2d::scenes::demo(&self.table, size[0], size[1], 1);
            sim.upload(&self.gpu, &scene.cells, &scene.temps);
        }
        sim.move_substeps = self.settings.substeps;
        let renderer = Renderer2d::new(&self.gpu, &sim, self.format);
        log::info!("2D world {} x {} ({:.1} M cells)", size[0], size[1], sim.cell_count() as f64 / 1e6);
        self.world2d = Some(World2d { sim, renderer });
    }

    fn new_world_3d(&mut self, n: u32, demo: bool) {
        let dims = [n, n, n];
        if !Sim3d::fits(&self.gpu, dims) {
            self.stats.message = format!("A {n}^3 world does not fit in this GPU's buffer limits");
            return;
        }
        // Free the old world first: phones are short on memory.
        self.world3d = None;
        let mut sim = Sim3d::new(&self.gpu, self.table.clone(), dims, 0x5eed);
        if demo {
            let scene = pt_sim3d::scenes::demo(&self.table, dims, 1);
            sim.upload(&self.gpu, &scene.cells, &scene.temps);
        }
        sim.move_substeps = self.settings.substeps;
        let renderer = Renderer3d::new(&self.gpu, &sim, self.format);
        log::info!("3D world {n}^3 ({:.1} M voxels)", sim.cell_count() as f64 / 1e6);
        self.world3d = Some(World3d {
            sim,
            renderer,
            camera: OrbitCamera::looking_at(dims),
        });
    }

    fn frame(&mut self, screen: &mut Screen, pointers: &mut Pointers) {
        let size = [screen.config.width, screen.config.height];
        self.layout = screen_layout(&screen.window, size);

        // Get the surface before running the UI: skipping a frame after the UI
        // ran would drop its texture updates (the font atlas among them).
        let frame = match screen.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(f) | wgpu::CurrentSurfaceTexture::Suboptimal(f) => f,
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                screen.surface.configure(&self.gpu.device, &screen.config);
                return;
            }
            _ => return,
        };

        // UI first, so this frame's input knows where the UI is.
        let raw_input = screen.egui_state.take_egui_input(&screen.window);
        let mut actions = ui::Actions::default();
        let mut output = self.egui_ctx.clone().run_ui(raw_input, |ui| {
            actions = ui::draw(ui, self);
        });
        screen
            .egui_state
            .handle_platform_output(&screen.window, output.platform_output.clone());
        let ppp = output.pixels_per_point;
        self.ui_rects = actions
            .rects
            .iter()
            .map(|r| egui::Rect::from_min_max((r.min.to_vec2() * ppp).to_pos2(), (r.max.to_vec2() * ppp).to_pos2()))
            .collect();
        self.apply(actions);
        if self.settings.mode == Mode::ThreeD && self.world3d.is_none() {
            self.new_world_3d(self.settings.size_3d, true);
            if self.world3d.is_none() {
                self.settings.mode = Mode::TwoD;
            }
        }

        let ticks = if !self.settings.paused || std::mem::take(&mut self.settings.step_once) {
            1
        } else {
            0
        };
        let tool = self.settings.tool();
        let paint_pos = pointers.paint.and_then(|(id, last)| pointers.position(id).map(|p| (id, last, p)));
        if let Some((id, _, p)) = paint_pos {
            pointers.paint = Some((id, p));
            // Recent lists what was painted with, not everything tapped while browsing.
            if self.settings.tool == ToolKind::Material && self.settings.recent.first() != Some(&self.settings.material) {
                catalog::remember(&mut self.settings.recent, self.settings.material);
            }
        }

        let target = frame.texture.create_view(&Default::default());
        let mut encoder = self.gpu.device.create_command_encoder(&Default::default());

        match self.settings.mode {
            Mode::TwoD => {
                let w = self.world2d.as_mut().expect("2D world exists");
                // The world fills the sim area; one made for a different
                // screen sits centred in it.
                let [sx, sy, sw, sh] = self.layout.sim;
                let offset = [
                    sx as i32 + (sw as i32 - w.sim.width as i32) / 2,
                    sy as i32 + (sh as i32 - w.sim.height as i32) / 2,
                ];
                if let Some((_, last, p)) = paint_pos {
                    let to_world = |q: [f32; 2]| [q[0] - offset[0] as f32, q[1] - offset[1] as f32];
                    w.sim.stroke(Stroke {
                        from: to_world(last),
                        to: to_world(p),
                        radius: self.settings.brush_2d,
                        tool,
                    });
                }
                w.sim.move_substeps = self.settings.substeps;
                w.sim.heat_speed = self.settings.heat_speed;
                w.sim.encode(&self.gpu.queue, &mut encoder, ticks);
                let overlay = if self.settings.temperature_view {
                    Overlay::Temperature
                } else {
                    Overlay::None
                };
                w.renderer.draw(&self.gpu.queue, &mut encoder, &target, &w.sim, offset, overlay);
            }
            Mode::ThreeD => {
                let w = self.world3d.as_mut().expect("3D world exists");
                if let Some((_, _, p)) = paint_pos {
                    let (origin, dir) = w.camera.ray(p, [size[0] as f32, size[1] as f32]);
                    w.sim.stroke(Stroke3 {
                        origin,
                        dir,
                        radius: self.settings.brush_3d,
                        tool,
                        target: self.settings.target_3d,
                    });
                }
                w.sim.move_substeps = self.settings.substeps;
                w.sim.heat_speed = self.settings.heat_speed;
                w.sim.encode(&self.gpu.queue, &mut encoder, ticks);
                let view = View3d {
                    show_plane: self.settings.target_3d == Target::Plane,
                    brush_radius: self.settings.brush_3d,
                    temperature: self.settings.temperature_view,
                };
                w.renderer
                    .draw(&self.gpu.queue, &mut encoder, &target, size, &w.sim, &w.camera, view);
            }
        }

        // UI on top.
        let screen_desc = egui_wgpu::ScreenDescriptor {
            size_in_pixels: size,
            pixels_per_point: ppp,
        };
        let jobs = self.egui_ctx.tessellate(output.shapes, ppp);
        for (id, deltas) in &output.textures_delta.set {
            for delta in deltas {
                self.egui_renderer.update_texture(&self.gpu.device, &self.gpu.queue, *id, delta);
            }
        }
        let extra = self
            .egui_renderer
            .update_buffers(&self.gpu.device, &self.gpu.queue, &mut encoder, &jobs, &screen_desc);
        {
            let pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("egui"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &target,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            let mut pass = pass.forget_lifetime();
            self.egui_renderer.render(&mut pass, &jobs, &screen_desc);
        }
        for id in &output.textures_delta.free {
            self.egui_renderer.free_texture(id);
        }
        // Every delta is applied above; clearing says so (egui checks in debug builds).
        output.textures_delta.clear();
        self.gpu.queue.submit(extra.into_iter().chain([encoder.finish()]));
        screen.window.pre_present_notify();
        self.gpu.queue.present(frame);
    }

    fn apply(&mut self, actions: ui::Actions) {
        if let Some(demo) = actions.new_2d {
            self.world2d = None;
            self.new_world_2d(demo);
        }
        if actions.bench {
            self.run_benchmark();
        }
    }

    /// The same 2D measurement as `pt-bench`, on this device, at the size of
    /// this screen's sim area. (The 3D prototype is shelved, so it is not
    /// measured here; `pt-bench` still covers it.)
    fn run_benchmark(&mut self) {
        let [_, _, w, h] = self.layout.sim;
        let results = vec![pt_bench::bench_2d(&self.gpu, &self.table, w.max(2), h.max(2), 120)];
        let report = pt_bench::table(&self.gpu.adapter_name(), &results);
        log::info!("benchmark\n{report}");
        self.stats.bench = Some(report);
    }
}
