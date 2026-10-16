//! Powder Toy Next, Phase 0 app: the 2D and 3D prototypes behind one touch UI.
//!
//! The target is Android (`android_main`, a Rust NativeActivity). The same
//! code also runs as a desktop window (`run` from `main.rs`) for development.
//! The screen is an upright 4:3 sim area, one cell per pixel, on a control
//! panel about as tall as a keyboard (see `layout`). The 3D prototype is
//! shelved: its code stays, but the app no longer offers it.

use std::sync::Arc;
use std::time::{Duration, Instant};

use pt_core::gpu::Gpu;
use pt_core::{MaterialTable, Tool};
use pt_sim2d::{Overlay, Renderer2d, Sim2d, Stroke, TickInput, TimedStroke};
use pt_sim3d::{OrbitCamera, Renderer3d, Sim3d, Stroke3, Target, View3d};
use winit::application::ApplicationHandler;
use winit::dpi::PhysicalPosition;
use winit::event::{ElementState, MouseButton, MouseScrollDelta, TouchPhase, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::{Window, WindowId};

mod catalog;
mod clock;
mod input;
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
    /// Playback speed scales complete fixed ticks, including heat and reactions.
    pub simulation_speed: f64,
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
    /// The pointer currently painting, and its latest event position.
    paint: Option<(PointerId, [f32; 2])>,
    timeline: input::InputTimeline,
    last_sample: Option<input::BrushSample>,
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
    epoch: Instant,
    clock: clock::SimulationClock,
    repaint_delay: Duration,
}

/// The window and its surface; recreated on every resume.
struct Screen {
    window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    egui_state: egui_winit::State,
    /// System bar insets in pixels, once Android has reported them.
    insets: Option<[u32; 2]>,
}

#[derive(Default)]
struct App {
    engine: Option<Engine>,
    screen: Option<Screen>,
    pointers: Pointers,
    last_frame: Option<Instant>,
    next_repaint: Option<Instant>,
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
    let _ = ANDROID_APP.set(android_app.clone());
    let event_loop = EventLoop::builder()
        .with_android_app(android_app)
        .build()
        .expect("create event loop");
    run(event_loop);
}

/// The app handle, for asking Android about the window from Java.
#[cfg(target_os = "android")]
static ANDROID_APP: std::sync::OnceLock<winit::platform::android::activity::AndroidApp> = std::sync::OnceLock::new();

/// Status bar height in points assumed until Android reports the real one.
#[cfg(target_os = "android")]
const FALLBACK_TOP_POINTS: f32 = 48.0;
#[cfg(not(target_os = "android"))]
const FALLBACK_TOP_POINTS: f32 = 0.0;

/// The layout for this window, kept clear of the status bar, camera cutout
/// and gesture bar. `insets` are the measured ones, once known.
fn screen_layout(window: &Window, size: [u32; 2], insets: Option<[u32; 2]>) -> Layout {
    let scale = window.scale_factor() as f32;
    let insets = insets.unwrap_or([(FALLBACK_TOP_POINTS * scale).round() as u32, 0]);
    layout::layout(size, scale, insets)
}

/// Rows the status bar and camera cutout (top) and the gesture or navigation
/// bar (bottom) cover, asked of the window's decor view. `None` until the
/// view is attached, or if Java says no.
///
/// The app draws edge to edge (Android 15 enforces it), so the content
/// rectangle NativeActivity reports is the whole screen and cannot be used.
#[cfg(target_os = "android")]
fn system_insets() -> Option<[u32; 2]> {
    use jni::objects::JObject;
    use jni::refs::Global;
    use jni::{jni_sig, jni_str};

    let app = ANDROID_APP.get()?;
    let vm = unsafe { jni::JavaVM::from_raw(app.vm_as_ptr().cast()) };
    let query = |env: &mut jni::Env| -> jni::errors::Result<Option<[u32; 2]>> {
        let raw = app.activity_as_ptr() as jni::sys::jobject;
        // SAFETY: a global reference `app` owns, valid while `app` lives.
        let activity = unsafe { env.as_cast_raw::<Global<JObject>>(&raw)? };
        let window = env
            .call_method(activity.as_ref(), jni_str!("getWindow"), jni_sig!("()Landroid/view/Window;"), &[])?
            .l()?;
        let decor = env
            .call_method(&window, jni_str!("getDecorView"), jni_sig!("()Landroid/view/View;"), &[])?
            .l()?;
        let insets = env
            .call_method(
                &decor,
                jni_str!("getRootWindowInsets"),
                jni_sig!("()Landroid/view/WindowInsets;"),
                &[],
            )?
            .l()?;
        if insets.is_null() {
            return Ok(None);
        }
        // Stable insets count the bars even while they are hidden; these
        // calls exist on every Android the app supports (API 28 and up).
        let mut top = env.call_method(&insets, jni_str!("getStableInsetTop"), jni_sig!("()I"), &[])?.i()?;
        let bottom = env
            .call_method(&insets, jni_str!("getStableInsetBottom"), jni_sig!("()I"), &[])?
            .i()?;
        let cutout = env
            .call_method(
                &insets,
                jni_str!("getDisplayCutout"),
                jni_sig!("()Landroid/view/DisplayCutout;"),
                &[],
            )?
            .l()?;
        if !cutout.is_null() {
            top = top.max(env.call_method(&cutout, jni_str!("getSafeInsetTop"), jni_sig!("()I"), &[])?.i()?);
        }
        Ok(Some([top.max(0) as u32, bottom.max(0) as u32]))
    };
    let result = vm.attach_current_thread(|env| -> jni::errors::Result<Option<[u32; 2]>> {
        let r = query(env);
        if r.is_err() {
            env.exception_clear();
        }
        Ok(r.unwrap_or(None))
    });
    match result {
        Ok(insets) => insets,
        Err(err) => {
            log::warn!("could not read window insets: {err}");
            None
        }
    }
}

#[cfg(not(target_os = "android"))]
fn system_insets() -> Option<[u32; 2]> {
    Some([0, 0])
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
                simulation_speed: 1.0,
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
                epoch: Instant::now(),
                clock: clock::SimulationClock::default(),
                repaint_delay: Duration::ZERO,
            });
        }
        let engine = self.engine.as_mut().unwrap();
        engine.clock.reset(engine.epoch.elapsed().as_secs_f64());
        self.last_frame = None;
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
        let insets = system_insets();
        engine.layout = screen_layout(&window, [config.width, config.height], insets);
        if engine.world2d.is_none() {
            engine.new_world_2d(true);
        }
        window.request_redraw();
        event_loop.set_control_flow(ControlFlow::Poll);
        self.next_repaint = None;
        self.screen = Some(Screen {
            window,
            surface,
            config,
            egui_state,
            insets,
        });
    }

    fn suspended(&mut self, event_loop: &ActiveEventLoop) {
        // Android destroys the native window when the app goes to the
        // background; the GPU and the worlds stay alive.
        self.screen = None;
        self.pointers = Pointers::default();
        self.last_frame = None;
        self.next_repaint = None;
        event_loop.set_control_flow(ControlFlow::Wait);
        if let Some(engine) = self.engine.as_mut() {
            engine.clock.reset(engine.epoch.elapsed().as_secs_f64());
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if self.next_repaint.is_some_and(|deadline| deadline <= Instant::now()) {
            self.next_repaint = None;
            event_loop.set_control_flow(ControlFlow::Wait);
            if let Some(screen) = self.screen.as_ref() {
                screen.window.request_redraw();
            }
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        let (Some(engine), Some(screen)) = (self.engine.as_mut(), self.screen.as_mut()) else {
            return;
        };
        let egui_response = screen.egui_state.on_window_event(&screen.window, &event);
        let input_or_window_change = !matches!(event, WindowEvent::RedrawRequested);

        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                screen.config.width = size.width.max(1);
                screen.config.height = size.height.max(1);
                screen.surface.configure(&engine.gpu.device, &screen.config);
                screen.insets = None;
            }
            WindowEvent::Focused(_) | WindowEvent::Occluded(_) => {
                self.pointers = Pointers::default();
                engine.clock.reset(engine.epoch.elapsed().as_secs_f64());
            }
            WindowEvent::RedrawRequested => {
                let now = Instant::now();
                if let Some(last) = self.last_frame {
                    engine.stats.frame((now - last).as_secs_f64() * 1000.0);
                }
                self.last_frame = Some(now);
                let rendered = engine.frame(screen, &mut self.pointers);
                if !rendered || engine.settings.paused {
                    self.last_frame = None;
                    self.next_repaint = if engine.repaint_delay == Duration::MAX {
                        None
                    } else {
                        Instant::now().checked_add(engine.repaint_delay)
                    };
                    event_loop.set_control_flow(self.next_repaint.map_or(ControlFlow::Wait, ControlFlow::WaitUntil));
                } else {
                    self.next_repaint = None;
                    event_loop.set_control_flow(ControlFlow::Poll);
                    screen.window.request_redraw();
                }
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
                self.pointers.refresh(engine);
            }
            _ => {}
        }
        if input_or_window_change {
            screen.window.request_redraw();
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

    fn record(&mut self, engine: &mut Engine, p: [f32; 2]) {
        let position = match engine.settings.mode {
            Mode::TwoD => {
                let Some(w) = engine.world2d.as_ref() else { return };
                let [sx, sy, sw, sh] = engine.layout.sim;
                let offset = [
                    sx as i32 + (sw as i32 - w.sim.width as i32) / 2,
                    sy as i32 + (sh as i32 - w.sim.height as i32) / 2,
                ];
                [p[0] - offset[0] as f32, p[1] - offset[1] as f32]
            }
            Mode::ThreeD => p,
        };
        let sample = input::BrushSample {
            position,
            radius: if engine.settings.mode == Mode::TwoD {
                engine.settings.brush_2d
            } else {
                engine.settings.brush_3d
            },
            tool: engine.settings.tool(),
            mode: engine.settings.mode,
            target: engine.settings.target_3d,
        };
        if let Some((id, _)) = self.paint {
            self.paint = Some((id, p));
        }
        if self.last_sample == Some(sample) {
            return;
        }
        self.last_sample = Some(sample);
        self.timeline.sample(engine.epoch.elapsed().as_secs_f64(), sample);
        if engine.settings.tool == ToolKind::Material && engine.settings.recent.first() != Some(&engine.settings.material) {
            catalog::remember(&mut engine.settings.recent, engine.settings.material);
        }
    }

    fn end(&mut self, engine: &Engine) {
        self.timeline.end(engine.epoch.elapsed().as_secs_f64());
        self.paint = None;
        self.last_sample = None;
    }

    fn refresh(&mut self, engine: &mut Engine) {
        if let Some((_, p)) = self.paint {
            self.record(engine, p);
        }
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
                    self.record(engine, p);
                } else {
                    // A second finger turns a stroke into a camera gesture.
                    self.end(engine);
                    self.gesture = None;
                }
            }
            TouchPhase::Moved => {
                let Some(t) = self.touches.iter_mut().find(|t| t.0 == id) else {
                    return;
                };
                let prev = t.1;
                t.1 = p;
                if self.paint.is_some_and(|(pid, _)| pid == PointerId::Touch(id)) {
                    self.record(engine, p);
                }
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
                if self.paint.is_some_and(|(pid, _)| pid == PointerId::Touch(id)) {
                    if phase == TouchPhase::Ended && self.paint.is_some_and(|(_, last)| last != p) {
                        self.record(engine, p);
                    }
                    self.end(engine);
                }
                self.touches.retain(|t| t.0 != id);
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
        if self.paint.is_some_and(|(id, _)| id == PointerId::Mouse) {
            self.record(engine, p);
        }
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
                    self.record(engine, self.mouse);
                } else if !down && self.paint.is_some_and(|(id, _)| id == PointerId::Mouse) {
                    self.end(engine);
                }
            }
            MouseButton::Right => self.mouse_right = down,
            _ => {}
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
        sim.move_substeps = pt_sim2d::DEFAULT_SUBSTEPS;
        let renderer = Renderer2d::new(&self.gpu, &sim, self.format);
        log::info!("2D world {} x {} ({:.1} M cells)", size[0], size[1], sim.cell_count() as f64 / 1e6);
        self.world2d = Some(World2d { sim, renderer });
        self.clock = clock::SimulationClock::default();
        self.clock.reset(self.epoch.elapsed().as_secs_f64());
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
        sim.move_substeps = pt_sim2d::DEFAULT_SUBSTEPS;
        let renderer = Renderer3d::new(&self.gpu, &sim, self.format);
        log::info!("3D world {n}^3 ({:.1} M voxels)", sim.cell_count() as f64 / 1e6);
        self.world3d = Some(World3d {
            sim,
            renderer,
            camera: OrbitCamera::looking_at(dims),
        });
    }

    fn frame(&mut self, screen: &mut Screen, pointers: &mut Pointers) -> bool {
        // A hidden or invalid surface waits for a window/input event. Failed
        // acquisition must not leave an immediate paused repaint deadline.
        self.repaint_delay = Duration::MAX;
        let size = [screen.config.width, screen.config.height];
        if screen.insets.is_none() {
            screen.insets = system_insets();
        }
        self.layout = screen_layout(&screen.window, size, screen.insets);

        // Get the surface before running the UI: skipping a frame after the UI
        // ran would drop its texture updates (the font atlas among them).
        let frame = match screen.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(f) | wgpu::CurrentSurfaceTexture::Suboptimal(f) => f,
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                screen.surface.configure(&self.gpu.device, &screen.config);
                self.repaint_delay = Duration::from_millis(16);
                return false;
            }
            wgpu::CurrentSurfaceTexture::Timeout => {
                self.repaint_delay = Duration::from_millis(50);
                return false;
            }
            _ => return false,
        };

        // UI first, so this frame's input knows where the UI is.
        let raw_input = screen.egui_state.take_egui_input(&screen.window);
        let mut actions = ui::Actions::default();
        let mut output = self.egui_ctx.clone().run_ui(raw_input, |ui| {
            actions = ui::draw(ui, self);
        });
        self.repaint_delay = output
            .viewport_output
            .get(&egui::ViewportId::ROOT)
            .map_or(Duration::MAX, |viewport| viewport.repaint_delay);
        screen
            .egui_state
            .handle_platform_output(&screen.window, output.platform_output.clone());
        let ppp = output.pixels_per_point;
        self.ui_rects = actions
            .rects
            .iter()
            .map(|r| egui::Rect::from_min_max((r.min.to_vec2() * ppp).to_pos2(), (r.max.to_vec2() * ppp).to_pos2()))
            .collect();
        if actions.new_2d.is_some() {
            *pointers = Pointers::default();
        }
        self.apply(actions);
        pointers.refresh(self);
        if self.settings.mode == Mode::ThreeD && self.world3d.is_none() {
            self.new_world_3d(self.settings.size_3d, true);
            if self.world3d.is_none() {
                self.settings.mode = Mode::TwoD;
            }
        }

        let now = self.epoch.elapsed().as_secs_f64();
        let plan = self.clock.frame(
            now,
            self.settings.simulation_speed,
            self.settings.paused,
            std::mem::take(&mut self.settings.step_once),
        );
        self.stats.simulated_seconds = self.clock.simulated_seconds;
        self.stats.simulation_limited = plan.dropped_seconds > 0.0;
        let mut inputs: Vec<(Vec<input::BrushSegment>, bool)> =
            plan.ticks.into_iter().map(|tick| (pointers.timeline.tick(tick), true)).collect();
        if self.settings.paused && inputs.is_empty() {
            inputs.push((pointers.timeline.construction(now), false));
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
                let actions: Vec<TickInput> = inputs
                    .iter()
                    .map(|(strokes, advance)| TickInput {
                        advance: *advance,
                        strokes: strokes
                            .iter()
                            .filter(|s| s.sample.mode == Mode::TwoD)
                            .map(|s| TimedStroke {
                                stroke: Stroke {
                                    from: s.from,
                                    to: s.sample.position,
                                    radius: s.sample.radius,
                                    tool: s.sample.tool,
                                },
                                duration_seconds: s.duration_seconds,
                            })
                            .collect(),
                    })
                    .collect();
                w.sim.move_substeps = pt_sim2d::DEFAULT_SUBSTEPS;
                w.sim.heat_speed = self.settings.heat_speed;
                w.sim.encode_actions(&self.gpu.queue, &mut encoder, &actions);
                let overlay = if self.settings.temperature_view {
                    Overlay::Temperature
                } else {
                    Overlay::None
                };
                w.renderer.draw(&self.gpu.queue, &mut encoder, &target, &w.sim, offset, overlay);
            }
            Mode::ThreeD => {
                let w = self.world3d.as_mut().expect("3D world exists");
                w.sim.move_substeps = pt_sim2d::DEFAULT_SUBSTEPS;
                w.sim.heat_speed = self.settings.heat_speed;
                // This shelved backend has one brush uniform. Submit before
                // reusing it so batches never overwrite an earlier stroke.
                for (strokes, advance) in &inputs {
                    for stroke in strokes.iter().filter(|s| s.sample.mode == Mode::ThreeD) {
                        let (origin, dir) = w.camera.ray(stroke.sample.position, [size[0] as f32, size[1] as f32]);
                        w.sim.timed_stroke(
                            Stroke3 {
                                origin,
                                dir,
                                radius: stroke.sample.radius,
                                tool: stroke.sample.tool,
                                target: stroke.sample.target,
                            },
                            stroke.duration_seconds,
                        );
                        let mut step = self.gpu.device.create_command_encoder(&Default::default());
                        w.sim.encode(&self.gpu.queue, &mut step, 0);
                        self.gpu.queue.submit([step.finish()]);
                    }
                    if *advance {
                        let mut step = self.gpu.device.create_command_encoder(&Default::default());
                        w.sim.encode(&self.gpu.queue, &mut step, 1);
                        self.gpu.queue.submit([step.finish()]);
                    }
                }
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
        true
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
        self.clock.reset(self.epoch.elapsed().as_secs_f64());
    }
}
