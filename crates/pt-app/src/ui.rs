//! The touch UI: a material bar and controls along the bottom, a settings
//! sheet, and a small stats readout.

use egui::{Color32, RichText};
use pt_sim3d::Target;

use crate::{Engine, Mode, ToolKind};

pub struct Stats {
    pub gpu_name: String,
    pub frame_ms: f64,
    pub message: String,
    pub bench: Option<String>,
}

impl Stats {
    pub fn new(gpu_name: String) -> Stats {
        Stats {
            gpu_name,
            frame_ms: 16.7,
            message: String::new(),
            bench: None,
        }
    }

    pub fn frame(&mut self, ms: f64) {
        self.frame_ms = self.frame_ms * 0.95 + ms.min(250.0) * 0.05;
    }
}

/// What the UI asked for this frame, applied after it is drawn.
#[derive(Default)]
pub struct Actions {
    /// Areas the UI covers, in points; touches there do not paint.
    pub rects: Vec<egui::Rect>,
    /// Make a new 2D world: `Some(true)` with the demo scene, `Some(false)` empty.
    pub new_2d: Option<bool>,
    pub new_3d: Option<bool>,
    pub bench: bool,
}

fn swatch_text_color(rgb: [u8; 3]) -> Color32 {
    let l = 0.299 * rgb[0] as f32 + 0.587 * rgb[1] as f32 + 0.114 * rgb[2] as f32;
    if l > 140.0 {
        Color32::BLACK
    } else {
        Color32::WHITE
    }
}

fn touch_style(ctx: &egui::Context) {
    ctx.global_style_mut(|style| {
        style.spacing.button_padding = egui::vec2(12.0, 9.0);
        style.spacing.item_spacing = egui::vec2(8.0, 8.0);
        style.spacing.interact_size.y = 40.0;
        style.spacing.slider_width = 160.0;
        style.visuals.window_fill = Color32::from_rgba_unmultiplied(18, 20, 26, 235);
        style.visuals.panel_fill = Color32::from_rgba_unmultiplied(12, 13, 18, 215);
    });
}

pub fn draw(root: &mut egui::Ui, engine: &mut Engine) -> Actions {
    let ctx = &root.ctx().clone();
    touch_style(ctx);
    let mut actions = Actions::default();
    let s = &mut engine.settings;

    let panel = egui::Panel::bottom("controls").show(root, |ui| {
        ui.add_space(6.0);
        egui::ScrollArea::horizontal().id_salt("materials").show(ui, |ui| {
            ui.horizontal(|ui| {
                for id in 1..engine.table.len() as u32 {
                    let def = engine.table.def(id);
                    let rgb = engine.table.rgb(id);
                    let selected = s.tool == ToolKind::Material && s.material == id;
                    let mut button = egui::Button::new(RichText::new(&def.name).color(swatch_text_color(rgb)).strong())
                        .fill(Color32::from_rgb(rgb[0], rgb[1], rgb[2]));
                    if selected {
                        button = button.stroke(egui::Stroke::new(3.0, Color32::WHITE));
                    }
                    if ui.add(button).clicked() {
                        s.material = id;
                        s.tool = ToolKind::Material;
                    }
                }
                ui.separator();
                for (kind, label) in [(ToolKind::Heat, "Heat"), (ToolKind::Cool, "Cool"), (ToolKind::Erase, "Erase")] {
                    if ui.selectable_label(s.tool == kind, label).clicked() {
                        s.tool = kind;
                    }
                }
                if s.mode == Mode::ThreeD && ui.selectable_label(s.tool == ToolKind::Camera, "Turn").clicked() {
                    s.tool = ToolKind::Camera;
                }
            });
        });
        ui.horizontal_wrapped(|ui| {
            if ui.selectable_label(s.mode == Mode::TwoD, "2D").clicked() {
                s.mode = Mode::TwoD;
                if s.tool == ToolKind::Camera {
                    s.tool = ToolKind::Material;
                }
            }
            if ui.selectable_label(s.mode == Mode::ThreeD, "3D").clicked() {
                s.mode = Mode::ThreeD;
            }
            ui.separator();
            if ui.button(if s.paused { "Play" } else { "Pause" }).clicked() {
                s.paused = !s.paused;
            }
            if s.paused && ui.button("Step").clicked() {
                s.step_once = true;
            }
            if ui.button("Menu").clicked() {
                s.show_menu = !s.show_menu;
            }
            ui.spacing_mut().slider_width = (ui.available_width() - 70.0).clamp(80.0, 220.0);
            match s.mode {
                Mode::TwoD => ui.add(egui::Slider::new(&mut s.brush_2d, 1.0..=150.0).logarithmic(true).text("Brush")),
                Mode::ThreeD => ui.add(egui::Slider::new(&mut s.brush_3d, 0.5..=20.0).text("Brush")),
            };
        });
        ui.add_space(10.0);
    });
    actions.rects.push(panel.response.rect);

    let fps = 1000.0 / engine.stats.frame_ms.max(0.001);
    let world = match s.mode {
        Mode::TwoD => engine.world2d.as_ref().map(|w| format!("{} x {}", w.sim.width, w.sim.height)),
        Mode::ThreeD => engine.world3d.as_ref().map(|w| format!("{}^3", w.sim.dims[0])),
    }
    .unwrap_or_default();
    egui::Area::new(egui::Id::new("stats"))
        .fixed_pos(egui::pos2(8.0, 36.0))
        .interactable(false)
        .show(ctx, |ui| {
            ui.label(
                RichText::new(format!("{fps:.0} fps  {:.1} ms  {world}", engine.stats.frame_ms))
                    .monospace()
                    .color(Color32::from_gray(200)),
            );
            if !engine.stats.message.is_empty() {
                ui.label(RichText::new(&engine.stats.message).color(Color32::from_rgb(255, 190, 90)));
            }
        });

    if s.show_menu {
        let mut open = true;
        let window = egui::Window::new("Settings")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_TOP, egui::vec2(0.0, 60.0))
            .show(ctx, |ui| {
                egui::ScrollArea::vertical()
                    .max_height(ctx.content_rect().height() * 0.6)
                    .show(ui, |ui| {
                        ui.checkbox(&mut s.temperature_view, "Show temperature");
                        ui.checkbox(&mut s.replace, "Draw over existing material");
                        ui.add(egui::Slider::new(&mut s.substeps, 1..=8).text("Movement passes per tick"));
                        ui.add(egui::Slider::new(&mut s.heat_speed, 0.1..=4.0).text("Heat speed"));
                        ui.separator();
                        ui.label("2D world");
                        ui.horizontal(|ui| {
                            if ui.button("Demo").clicked() {
                                actions.new_2d = Some(true);
                            }
                            if ui.button("Empty").clicked() {
                                actions.new_2d = Some(false);
                            }
                        });
                        ui.separator();
                        ui.label("3D world");
                        ui.horizontal(|ui| {
                            for n in [64, 96, 128, 192, 256] {
                                if ui.selectable_label(s.size_3d == n, format!("{n}")).clicked() {
                                    s.size_3d = n;
                                }
                            }
                        });
                        ui.horizontal(|ui| {
                            if ui.button("Demo").clicked() {
                                actions.new_3d = Some(true);
                            }
                            if ui.button("Empty").clicked() {
                                actions.new_3d = Some(false);
                            }
                        });
                        ui.horizontal(|ui| {
                            ui.label("Draw on");
                            if ui.selectable_label(s.target_3d == Target::Surface, "Surfaces").clicked() {
                                s.target_3d = Target::Surface;
                            }
                            if ui.selectable_label(s.target_3d == Target::Plane, "Plane").clicked() {
                                s.target_3d = Target::Plane;
                            }
                        });
                        if s.target_3d == Target::Plane {
                            if let Some(w) = engine.world3d.as_mut() {
                                let top = w.sim.dims[1] as f32 - 1.0;
                                ui.add(egui::Slider::new(&mut w.sim.plane_y, 0.0..=top).text("Plane height"));
                            }
                        }
                        ui.separator();
                        ui.label(RichText::new(&engine.stats.gpu_name).small());
                        if ui
                            .button("Run benchmark")
                            .on_hover_text("Takes a few seconds; the screen freezes while it runs")
                            .clicked()
                        {
                            actions.bench = true;
                        }
                        if let Some(report) = &engine.stats.bench {
                            ui.label(RichText::new(report).monospace().small());
                            if ui.button("Copy results").clicked() {
                                ui.ctx().copy_text(report.clone());
                            }
                        }
                    });
            });
        if let Some(w) = window {
            actions.rects.push(w.response.rect);
        }
        s.show_menu = open;
    }
    actions
}
