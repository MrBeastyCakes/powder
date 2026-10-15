//! The touch UI: a keyboard-height control panel under the sim area (material
//! keys, tools, brush, time controls), a settings sheet, and a small stats
//! readout above the sim area.

use egui::{Color32, RichText, Vec2};

use crate::layout::Layout;
use crate::{Engine, ToolKind};

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
    pub bench: bool,
}

const PANEL_FILL: Color32 = Color32::from_rgb(12, 13, 18);
/// Material keys per row, like letters on a keyboard row.
const KEYS_PER_ROW: usize = 5;
/// Movement passes per tick the speed keys step through.
const MAX_SPEED: u32 = 8;

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
        style.spacing.button_padding = egui::vec2(6.0, 6.0);
        style.spacing.item_spacing = egui::vec2(6.0, 6.0);
        style.spacing.interact_size.y = 40.0;
        style.spacing.slider_width = 160.0;
        style.visuals.window_fill = Color32::from_rgba_unmultiplied(18, 20, 26, 235);
        style.visuals.panel_fill = PANEL_FILL;
    });
}

/// A keyboard key: fills its column and the row height.
fn key(ui: &mut egui::Ui, height: f32, button: egui::Button) -> egui::Response {
    ui.add_sized(Vec2::new(ui.available_width(), height), button)
}

/// A row of equal-width keys. `keys` draws key `i` into its column.
fn key_row(ui: &mut egui::Ui, n: usize, mut keys: impl FnMut(&mut egui::Ui, usize)) {
    ui.columns(n, |cols| {
        for (i, col) in cols.iter_mut().enumerate() {
            keys(col, i);
        }
    });
}

fn toggle_key(ui: &mut egui::Ui, height: f32, label: &str, on: bool) -> bool {
    let mut button = egui::Button::new(label).selected(on);
    if on {
        button = button.stroke(egui::Stroke::new(2.0, Color32::WHITE));
    }
    key(ui, height, button).clicked()
}

pub fn draw(root: &mut egui::Ui, engine: &mut Engine) -> Actions {
    let ctx = &root.ctx().clone();
    touch_style(ctx);
    let ppp = ctx.pixels_per_point();
    let layout: Layout = engine.layout;
    let mut actions = Actions::default();

    // Bottom up: the strip over the gesture bar, then the control panel.
    let gap = egui::Panel::bottom("gesture-gap")
        .exact_size(layout.insets[1] as f32 / ppp)
        .resizable(false)
        .show_separator_line(false)
        .frame(egui::Frame::NONE.fill(PANEL_FILL))
        .show(root, |_| {});
    actions.rects.push(gap.response.rect);
    let panel = egui::Panel::bottom("controls")
        .exact_size(layout.panel_height as f32 / ppp)
        .resizable(false)
        .show_separator_line(false)
        .frame(egui::Frame::NONE.fill(PANEL_FILL).inner_margin(egui::Margin::symmetric(6, 8)))
        .show(root, |ui| controls(ui, engine));
    actions.rects.push(panel.response.rect);

    let s = &mut engine.settings;
    let fps = 1000.0 / engine.stats.frame_ms.max(0.001);
    let world = engine
        .world2d
        .as_ref()
        .map(|w| format!("{} x {}", w.sim.width, w.sim.height))
        .unwrap_or_default();
    egui::Area::new(egui::Id::new("stats"))
        .fixed_pos(egui::pos2(8.0, layout.insets[0] as f32 / ppp + 6.0))
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
        let width = (ctx.content_rect().width() - 32.0).min(420.0);
        let window = egui::Window::new("Settings")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .min_width(width)
            .max_width(width)
            .anchor(egui::Align2::CENTER_TOP, egui::vec2(0.0, layout.insets[0] as f32 / ppp + 40.0))
            .show(ctx, |ui| {
                egui::ScrollArea::vertical()
                    .max_height(ctx.content_rect().height() * 0.6)
                    .show(ui, |ui| {
                        // Sliders get their own line and the width that is
                        // left, so none runs past the edge of the screen.
                        let slider_width = (ui.available_width() - 70.0).max(60.0);
                        ui.spacing_mut().slider_width = slider_width;
                        ui.checkbox(&mut s.temperature_view, "Show temperature");
                        ui.checkbox(&mut s.replace, "Draw over existing material");
                        ui.label("Movement passes per tick");
                        ui.add(egui::Slider::new(&mut s.substeps, 1..=MAX_SPEED));
                        ui.label("Heat speed");
                        ui.add(egui::Slider::new(&mut s.heat_speed, 0.1..=4.0));
                        ui.separator();
                        ui.label("New world");
                        ui.horizontal(|ui| {
                            if ui.button("Demo").clicked() {
                                actions.new_2d = Some(true);
                            }
                            if ui.button("Empty").clicked() {
                                actions.new_2d = Some(false);
                            }
                        });
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

/// The control panel, laid out like a keyboard: rows of material keys, a
/// tool row, the brush, and a bottom row for time and the menu.
fn controls(ui: &mut egui::Ui, engine: &mut Engine) {
    let materials: Vec<u32> = (1..engine.table.len() as u32).collect();
    let material_rows = materials.len().div_ceil(KEYS_PER_ROW);
    let rows = material_rows + 3;
    let gap = ui.spacing().item_spacing.y;
    let row_h = ((ui.available_height() - gap * (rows - 1) as f32) / rows as f32).max(24.0);
    let s = &mut engine.settings;

    for chunk in materials.chunks(KEYS_PER_ROW) {
        key_row(ui, KEYS_PER_ROW, |col, i| {
            let Some(&id) = chunk.get(i) else { return };
            let def = engine.table.def(id);
            let rgb = engine.table.rgb(id);
            let selected = s.tool == ToolKind::Material && s.material == id;
            let mut button = egui::Button::new(RichText::new(&def.name).color(swatch_text_color(rgb)).strong())
                .fill(Color32::from_rgb(rgb[0], rgb[1], rgb[2]));
            if selected {
                button = button.stroke(egui::Stroke::new(3.0, Color32::WHITE));
            }
            if key(col, row_h, button).clicked() {
                s.material = id;
                s.tool = ToolKind::Material;
            }
        });
    }

    key_row(ui, 5, |col, i| match i {
        0 => {
            if toggle_key(col, row_h, "Heat", s.tool == ToolKind::Heat) {
                s.tool = ToolKind::Heat;
            }
        }
        1 => {
            if toggle_key(col, row_h, "Cool", s.tool == ToolKind::Cool) {
                s.tool = ToolKind::Cool;
            }
        }
        2 => {
            if toggle_key(col, row_h, "Erase", s.tool == ToolKind::Erase) {
                s.tool = ToolKind::Erase;
            }
        }
        3 => {
            if toggle_key(col, row_h, "Over", s.replace) {
                s.replace = !s.replace;
            }
        }
        _ => {
            if toggle_key(col, row_h, "Temp", s.temperature_view) {
                s.temperature_view = !s.temperature_view;
            }
        }
    });

    ui.horizontal(|ui| {
        ui.set_height(row_h);
        ui.label("Brush");
        ui.spacing_mut().slider_width = (ui.available_width() - 70.0).max(40.0);
        ui.add(egui::Slider::new(&mut s.brush_2d, 1.0..=150.0).logarithmic(true).integer());
    });

    key_row(ui, 6, |col, i| match i {
        0 => {
            if key(col, row_h, egui::Button::new(if s.paused { "Play" } else { "Pause" })).clicked() {
                s.paused = !s.paused;
            }
        }
        1 => {
            if col
                .add_enabled_ui(s.paused, |c| key(c, row_h, egui::Button::new("Step")))
                .inner
                .clicked()
            {
                s.step_once = true;
            }
        }
        2 => {
            if col
                .add_enabled_ui(s.substeps > 1, |c| key(c, row_h, egui::Button::new("Slower")))
                .inner
                .clicked()
            {
                s.substeps -= 1;
            }
        }
        3 => {
            col.add_sized(
                Vec2::new(col.available_width(), row_h),
                egui::Label::new(format!("Speed {}", s.substeps)),
            );
        }
        4 => {
            if col
                .add_enabled_ui(s.substeps < MAX_SPEED, |c| key(c, row_h, egui::Button::new("Faster")))
                .inner
                .clicked()
            {
                s.substeps += 1;
            }
        }
        _ => {
            if toggle_key(col, row_h, "Menu", s.show_menu) {
                s.show_menu = !s.show_menu;
            }
        }
    });
}
