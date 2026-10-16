//! The touch UI: a keyboard-height control panel under the sim area, a
//! settings sheet, and a small stats readout above the sim area.
//!
//! The panel is a catalog: a top strip for time, the brush and the menu; a
//! rail of categories down the left (Recent, Tools, then every kind and tag
//! the material pack uses); and the chosen category as a list on the right.

use std::sync::Arc;

use egui::scroll_area::ScrollBarVisibility;
use egui::text::{LayoutJob, TextFormat, TextWrapping};
use egui::{pos2, vec2, Color32, FontFamily, FontId, Painter, Pos2, Rect, Response, RichText, Sense, Shape, Stroke, StrokeKind, Vec2};
use pt_core::Kind;

use crate::catalog::{self, Category};
use crate::layout::Layout;
use crate::{Engine, Settings, ToolKind};

pub struct Stats {
    pub gpu_name: String,
    pub frame_ms: f64,
    pub message: String,
    pub bench: Option<String>,
    pub simulated_seconds: f64,
    pub simulation_limited: bool,
}

impl Stats {
    pub fn new(gpu_name: String) -> Stats {
        Stats {
            gpu_name,
            frame_ms: 16.7,
            message: String::new(),
            bench: None,
            simulated_seconds: 0.0,
            simulation_limited: false,
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

// The palette: a graphite panel, raised keys, and one accent for whatever is selected.
const PANEL: Color32 = Color32::from_rgb(0x14, 0x16, 0x1d);
const KEY: Color32 = Color32::from_rgb(0x1d, 0x20, 0x29);
const RAISED: Color32 = Color32::from_rgb(0x26, 0x2a, 0x35);
const TOOL_SWATCH: Color32 = Color32::from_rgb(0x2a, 0x2e, 0x3a);
const LINE: Color32 = RAISED;
const TEXT: Color32 = Color32::from_rgb(0xe8, 0xe9, 0xee);
const MUTED: Color32 = Color32::from_rgb(0x9a, 0xa0, 0xad);
const DIM: Color32 = Color32::from_rgb(0x8b, 0x92, 0xa0);
const ACCENT: Color32 = Color32::from_rgb(0xff, 0xb0, 0x30);

/// Playback speed changes the number of complete 1/60-second physics ticks.
const SPEEDS: [f64; 5] = [0.25, 0.5, 1.0, 1.5, 2.0];
const SPEED_LABELS: [&str; 5] = ["¼×", "½×", "1×", "1½×", "2×"];
/// Height of the top strip, its bottom rule included.
const STRIP_H: f32 = 53.0;
const RAIL_W: f32 = 68.0;
const KEY_H: f32 = 44.0;
const ROW_H: f32 = 48.0;

const SANS: &str = "powder-ui-sans";
const SANS_STRONG: &str = "powder-ui-sans-semibold";
const MONO: &str = "powder-ui-mono";
/// The font family for names and labels.
const STRONG: &str = "strong";

/// Installs the UI fonts: IBM Plex, renamed "Powder UI" because the files are
/// subset (see `fonts/OFL.txt`). egui's own fonts stay behind them as fallbacks.
pub fn install_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    let faces: [(&str, &'static [u8]); 3] = [
        (SANS, include_bytes!("../fonts/PowderUISans-Regular.ttf")),
        (SANS_STRONG, include_bytes!("../fonts/PowderUISans-SemiBold.ttf")),
        (MONO, include_bytes!("../fonts/PowderUIMono-Medium.ttf")),
    ];
    for (name, bytes) in faces {
        fonts.font_data.insert(name.into(), Arc::new(egui::FontData::from_static(bytes)));
    }
    let families = &mut fonts.families;
    families.entry(FontFamily::Proportional).or_default().insert(0, SANS.into());
    families.entry(FontFamily::Monospace).or_default().insert(0, MONO.into());
    let mut strong = vec![SANS_STRONG.to_string()];
    strong.extend(families[&FontFamily::Proportional].iter().cloned());
    families.insert(FontFamily::Name(STRONG.into()), strong);
    ctx.set_fonts(fonts);
}

fn strong(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(STRONG.into()))
}

fn touch_style(ctx: &egui::Context) {
    ctx.global_style_mut(|style| {
        use egui::TextStyle;
        style.text_styles = [
            (TextStyle::Small, FontId::proportional(12.0)),
            (TextStyle::Body, FontId::proportional(15.0)),
            (TextStyle::Button, FontId::proportional(15.0)),
            (TextStyle::Monospace, FontId::monospace(13.0)),
            (TextStyle::Heading, strong(18.0)),
        ]
        .into();
        style.spacing.button_padding = egui::vec2(10.0, 8.0);
        style.spacing.item_spacing = egui::vec2(8.0, 8.0);
        style.spacing.interact_size.y = 40.0;
        style.spacing.slider_width = 160.0;
        style.visuals.window_fill = KEY;
        style.visuals.window_stroke = Stroke::new(1.0, RAISED);
        style.visuals.window_corner_radius = 16.into();
        style.visuals.panel_fill = PANEL;
        style.visuals.selection.bg_fill = ACCENT;
        style.visuals.selection.stroke = Stroke::new(1.0, PANEL);
        style.visuals.slider_trailing_fill = true;
        style.visuals.override_text_color = Some(TEXT);
    });
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
        .frame(egui::Frame::NONE.fill(PANEL))
        .show(root, |_| {});
    actions.rects.push(gap.response.rect);
    let panel = egui::Panel::bottom("controls")
        .exact_size(layout.panel_height as f32 / ppp)
        .resizable(false)
        .show_separator_line(false)
        .frame(egui::Frame::NONE.fill(PANEL))
        .show(root, |ui| controls(ui, engine));
    actions.rects.push(panel.response.rect);

    let s = &mut engine.settings;
    if s.show_brush {
        let width = ctx.content_rect().width() - 16.0;
        let area = egui::Area::new(egui::Id::new("brush"))
            .fixed_pos(egui::pos2(8.0, panel.response.rect.top() - 72.0))
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(KEY)
                    .stroke(Stroke::new(1.0, RAISED))
                    .corner_radius(14)
                    .inner_margin(egui::Margin::symmetric(14, 10))
                    .show(ui, |ui| {
                        ui.set_width(width - 28.0);
                        ui.horizontal(|ui| {
                            ui.label(RichText::new("Brush").font(strong(14.0)));
                            ui.spacing_mut().slider_width = (ui.available_width() - 70.0).max(40.0);
                            ui.add(egui::Slider::new(&mut s.brush_2d, 1.0..=150.0).logarithmic(true).integer());
                        });
                    });
            });
        actions.rects.push(area.response.rect);
    }

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
                RichText::new(format!(
                    "{fps:.0} fps  {:.1} ms  {world}  {:.2} s",
                    engine.stats.frame_ms, engine.stats.simulated_seconds
                ))
                .monospace()
                .color(Color32::from_gray(200)),
            );
            if engine.stats.simulation_limited {
                ui.label(RichText::new("Simulation slowed to stay responsive").small().color(ACCENT));
            }
            if !engine.stats.message.is_empty() {
                ui.label(RichText::new(&engine.stats.message).color(Color32::from_rgb(255, 190, 90)));
            }
        });

    let s = &mut engine.settings;
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
                        ui.label("Thermal conductivity");
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
                        ui.label(RichText::new(&engine.stats.gpu_name).small().color(MUTED));
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

/// The control panel: the top strip, then the rail and the list side by side.
fn controls(ui: &mut egui::Ui, engine: &mut Engine) {
    let full = ui.max_rect();
    let Engine { table, settings: s, .. } = engine;
    let p = ui.painter().clone();
    p.hline(full.x_range(), full.top() + 0.5, Stroke::new(1.0, LINE));

    let strip = Rect::from_min_size(full.min, vec2(full.width(), STRIP_H));
    top_strip(ui, strip.shrink2(vec2(10.0, 0.0)), s);
    p.hline(full.x_range(), strip.bottom() - 0.5, Stroke::new(1.0, LINE));

    let body = Rect::from_min_max(pos2(full.left(), strip.bottom()), full.max);
    let rail_rect = Rect::from_min_size(body.min, vec2(RAIL_W, body.height()));
    p.vline(rail_rect.right() - 0.5, body.y_range(), Stroke::new(1.0, LINE));
    let cats = catalog::categories(table);
    ui.scope_builder(egui::UiBuilder::new().max_rect(rail_rect.shrink2(vec2(6.0, 0.0))), |ui| {
        rail(ui, &cats, s)
    });

    let list_rect = Rect::from_min_max(pos2(rail_rect.right(), body.top()), body.max).shrink2(vec2(8.0, 0.0));
    ui.scope_builder(egui::UiBuilder::new().max_rect(list_rect), |ui| list(ui, table, s));
}

/// A tappable key: paints its background (lighter while pressed) and returns
/// the response.
fn key(ui: &egui::Ui, rect: Rect, id: &str, enabled: bool, fill: Color32) -> Response {
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let response = ui.interact(rect, ui.id().with(id), sense);
    let fill = if response.is_pointer_button_down_on() {
        fill.lerp_to_gamma(Color32::WHITE, 0.08)
    } else {
        fill
    };
    if fill != Color32::TRANSPARENT {
        ui.painter().rect_filled(rect, 10, fill);
    }
    response
}

/// Play, step and speed on the left; brush size and the menu on the right.
fn top_strip(ui: &egui::Ui, rect: Rect, s: &mut Settings) {
    let p = ui.painter();
    let y = rect.center().y - KEY_H / 2.0;
    let at = |x: f32, w: f32| Rect::from_min_size(pos2(x, y), vec2(w, KEY_H));

    let play = at(rect.left(), KEY_H);
    let (fill, fg) = if s.paused { (ACCENT, PANEL) } else { (KEY, TEXT) };
    if key(ui, play, "play", true, fill).clicked() {
        s.paused = !s.paused;
    }
    let icon = if s.paused { Icon::Play } else { Icon::Pause };
    paint_icon(p, icon, play.center(), 18.0, fg);

    let step = at(play.right() + 6.0, KEY_H);
    if key(ui, step, "step", s.paused, KEY).clicked() {
        s.step_once = true;
    }
    paint_icon(
        p,
        Icon::Step,
        step.center(),
        18.0,
        if s.paused { TEXT } else { TEXT.gamma_multiply(0.4) },
    );

    let speed = at(step.right() + 6.0, 94.0);
    p.rect_filled(speed, 10, KEY);
    let level = SPEEDS.iter().position(|&n| n >= s.simulation_speed).unwrap_or(SPEEDS.len() - 1);
    let can_slow = level > 0;
    let can_speed = level + 1 < SPEEDS.len();
    let slower = at(speed.left(), 34.0);
    if key(ui, slower, "slower", can_slow, Color32::TRANSPARENT).clicked() && can_slow {
        s.simulation_speed = SPEEDS[level - 1];
    }
    let faster = at(speed.right() - 34.0, 34.0);
    if key(ui, faster, "faster", can_speed, Color32::TRANSPARENT).clicked() && can_speed {
        s.simulation_speed = SPEEDS[level + 1];
    }
    let dim = |on: bool| if on { TEXT } else { TEXT.gamma_multiply(0.4) };
    paint_icon(p, Icon::Minus, slower.center(), 14.0, dim(can_slow));
    paint_icon(p, Icon::Plus, faster.center(), 14.0, dim(can_speed));
    p.text(
        speed.center(),
        egui::Align2::CENTER_CENTER,
        SPEED_LABELS[level],
        FontId::monospace(13.0),
        TEXT,
    );

    let menu = at(rect.right() - KEY_H, KEY_H);
    if key(ui, menu, "menu", true, if s.show_menu { RAISED } else { KEY }).clicked() {
        s.show_menu = !s.show_menu;
    }
    paint_icon(p, Icon::Menu, menu.center(), 18.0, if s.show_menu { ACCENT } else { TEXT });

    let brush = at(menu.left() - 6.0 - 62.0, 62.0);
    if key(ui, brush, "brush", true, if s.show_brush { RAISED } else { KEY }).clicked() {
        s.show_brush = !s.show_brush;
    }
    let fg = if s.show_brush { ACCENT } else { TEXT };
    p.circle_stroke(pos2(brush.left() + 18.0, brush.center().y), 7.0, Stroke::new(1.6, fg));
    p.text(
        pos2(brush.left() + 31.0, brush.center().y),
        egui::Align2::LEFT_CENTER,
        format!("{:.0}", s.brush_2d),
        FontId::monospace(12.0),
        fg,
    );
}

/// The category rail. It scrolls once packs add more tags than fit.
fn rail(ui: &mut egui::Ui, cats: &[Category], s: &mut Settings) {
    let out = egui::ScrollArea::vertical()
        .id_salt("rail")
        .auto_shrink([false, false])
        .scroll_bar_visibility(ScrollBarVisibility::AlwaysHidden)
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            ui.add_space(4.0);
            for (i, cat) in cats.iter().enumerate() {
                if i == 2 {
                    // Recent and Tools above, the pack's own categories below.
                    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 5.0), Sense::hover());
                    ui.painter()
                        .hline(r.shrink2(vec2(8.0, 0.0)).x_range(), r.center().y, Stroke::new(1.0, LINE));
                }
                let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), KEY_H), Sense::click());
                let on = s.category == *cat;
                let p = ui.painter();
                if on {
                    p.rect_filled(rect, 10, RAISED);
                } else if response.is_pointer_button_down_on() {
                    p.rect_filled(rect, 10, KEY);
                }
                let color = if on { ACCENT } else { DIM };
                let icon = match cat {
                    Category::Recent => Icon::Recent,
                    Category::Tools => Icon::Tools,
                    Category::Kind(k) => Icon::Kind(*k),
                    Category::Tag(_) => Icon::Tag,
                };
                paint_icon(p, icon, rect.center() - vec2(0.0, 7.0), 20.0, color);
                let label = single_line(p, &cat.label(), strong(10.0), color, rect.width() - 4.0);
                p.galley(pos2(rect.center().x - label.size().x / 2.0, rect.center().y + 5.0), label, color);
                if response.clicked() {
                    s.category = cat.clone();
                }
            }
            ui.add_space(20.0);
        });
    // Fade the bottom edge while there is more rail below.
    if out.content_size.y > out.state.offset.y + out.inner_rect.height() + 1.0 {
        let r = out.inner_rect;
        fade_down(
            ui.painter(),
            Rect::from_min_max(pos2(r.left() - 6.0, r.bottom() - 24.0), pos2(r.right() + 5.0, r.bottom())),
        );
    }
}

/// The chosen category: tools, or materials with what they do and their density.
fn list(ui: &mut egui::Ui, table: &pt_core::MaterialTable, s: &mut Settings) {
    egui::ScrollArea::vertical()
        .id_salt(("list", s.category.label()))
        .auto_shrink([false, false])
        .scroll_bar_visibility(ScrollBarVisibility::AlwaysHidden)
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            ui.add_space(4.0);
            if s.category == Category::Tools {
                tool_rows(ui, s);
                return;
            }
            let ids = match &s.category {
                Category::Recent => s.recent.clone(),
                cat => catalog::members(table, cat),
            };
            if ids.is_empty() {
                ui.add_space(16.0);
                ui.label(RichText::new("Materials you paint with show up here").size(13.0).color(MUTED));
            }
            for id in ids {
                let def = table.def(id);
                let selected = s.tool == ToolKind::Material && s.material == id;
                let rgb = table.rgb(id);
                let color = Color32::from_rgb(rgb[0], rgb[1], rgb[2]);
                let note = catalog::describe(table, id);
                let stat = catalog::density_label(table, id);
                let response = row(ui, selected, &def.name, &note, &stat, |p, r| paint_swatch(p, r, color, def.kind));
                if response.clicked() {
                    s.material = id;
                    s.tool = ToolKind::Material;
                }
            }
        });
}

fn tool_rows(ui: &mut egui::Ui, s: &mut Settings) {
    // Heat, cool and erase replace painting until a material is picked (or
    // the tool is tapped again); Over and Temp view are toggles.
    let brushes = [
        (ToolKind::Heat, Icon::Heat, "Heat", "raises temperature under the brush"),
        (ToolKind::Cool, Icon::Cool, "Cool", "lowers temperature under the brush"),
        (ToolKind::Erase, Icon::Erase, "Erase", "turns cells back to air"),
    ];
    for (tool, icon, name, note) in brushes {
        let on = s.tool == tool;
        if tool_row(ui, on, icon, name, note).clicked() {
            s.tool = if on { ToolKind::Material } else { tool };
        }
    }
    if tool_row(ui, s.replace, Icon::Over, "Over", "toggle · draw over material").clicked() {
        s.replace = !s.replace;
    }
    if tool_row(ui, s.temperature_view, Icon::Temp, "Temp view", "toggle · show the heat map").clicked() {
        s.temperature_view = !s.temperature_view;
    }
}

fn tool_row(ui: &mut egui::Ui, on: bool, icon: Icon, name: &str, note: &str) -> Response {
    row(ui, on, name, note, if on { "on" } else { "" }, |p, r| {
        p.rect_filled(r, 8, if on { ACCENT } else { TOOL_SWATCH });
        paint_icon(p, icon, r.center(), 20.0, if on { PANEL } else { TEXT });
    })
}

/// One list row: a swatch, the name over a note, and a stat on the right.
fn row(ui: &mut egui::Ui, selected: bool, name: &str, note: &str, stat: &str, swatch: impl FnOnce(&Painter, Rect)) -> Response {
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_H), Sense::click());
    let p = ui.painter();
    if selected {
        p.rect_filled(rect, 10, RAISED);
    } else if response.is_pointer_button_down_on() {
        p.rect_filled(rect, 10, KEY);
    }
    let sw = Rect::from_center_size(pos2(rect.left() + 6.0 + 17.0, rect.center().y), Vec2::splat(34.0));
    swatch(p, sw);
    if selected {
        p.rect_stroke(sw.expand(3.0), 10, Stroke::new(2.0, ACCENT), StrokeKind::Middle);
    }
    let stat = p.layout_no_wrap(stat.to_string(), FontId::monospace(11.0), MUTED);
    let stat_w = stat.size().x;
    p.galley(
        pos2(rect.right() - 8.0 - stat_w, rect.center().y - stat.size().y / 2.0),
        stat,
        MUTED,
    );
    let x = sw.right() + 10.0;
    let width = (rect.right() - 8.0 - stat_w - 8.0 - x).max(20.0);
    let name = single_line(p, name, strong(14.0), TEXT, width);
    let note = single_line(p, note, FontId::proportional(11.0), MUTED, width);
    let top = rect.center().y - (name.size().y + note.size().y) / 2.0;
    let name_h = name.size().y;
    p.galley(pos2(x, top), name, TEXT);
    p.galley(pos2(x, top + name_h), note, MUTED);
    response
}

/// `text` on one line, cut short with `…` if it is wider than `width`.
fn single_line(p: &Painter, text: &str, font: FontId, color: Color32, width: f32) -> Arc<egui::Galley> {
    let mut job = LayoutJob::single_section(text.to_string(), TextFormat::simple(font, color));
    job.wrap = TextWrapping::truncate_at_width(width);
    p.layout_job(job)
}

/// A transparent-to-panel gradient over `rect`, for scroll edges.
fn fade_down(p: &Painter, rect: Rect) {
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(rect.left_top(), Color32::TRANSPARENT);
    mesh.colored_vertex(rect.right_top(), Color32::TRANSPARENT);
    mesh.colored_vertex(rect.left_bottom(), PANEL);
    mesh.colored_vertex(rect.right_bottom(), PANEL);
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(1, 3, 2);
    p.add(Shape::mesh(mesh));
}

/// A material swatch: its color with a texture that shows its kind, so a long
/// list reads at a glance and kinds differ by more than hue: grain for
/// powders, ripples for liquids, puffs for gases, bricks for solids.
fn paint_swatch(p: &Painter, rect: Rect, color: Color32, kind: Kind) {
    p.rect_filled(rect, 8, color);
    let tex = p.with_clip_rect(rect.shrink(2.0).intersect(p.clip_rect()));
    let (w, h) = (rect.width(), rect.height());
    match kind {
        Kind::Powder => {
            let dots = [
                (5.0, Vec2::ZERO, Color32::from_black_alpha(70)),
                (7.0, vec2(2.0, 3.0), Color32::from_white_alpha(50)),
            ];
            for (step, offset, c) in dots {
                let mut y = rect.top() + offset.y;
                while y < rect.bottom() {
                    let mut x = rect.left() + offset.x;
                    while x < rect.right() {
                        tex.circle_filled(pos2(x, y), 0.7, c);
                        x += step;
                    }
                    y += step;
                }
            }
        }
        Kind::Liquid => {
            let slant = h * 0.27;
            let mut x = rect.left() - slant;
            while x < rect.right() + slant {
                tex.line_segment(
                    [pos2(x + slant, rect.top()), pos2(x, rect.bottom())],
                    Stroke::new(2.0, Color32::from_white_alpha(40)),
                );
                x += 9.0;
            }
        }
        Kind::Gas => {
            let puff = |fx: f32, fy: f32, r: f32, a: u8| {
                let c = rect.left_top() + vec2(w * fx, h * fy);
                tex.circle_filled(c, r, Color32::from_white_alpha(a / 2));
                tex.circle_filled(c, r * 0.6, Color32::from_white_alpha(a / 2));
            };
            puff(0.28, 0.38, w * 0.3, 90);
            puff(0.72, 0.7, w * 0.24, 56);
        }
        Kind::Solid => {
            let mortar = Stroke::new(1.0, Color32::from_black_alpha(55));
            let mut y = rect.top();
            let mut course = 0;
            while y < rect.bottom() {
                let mut x = rect.left() + if course % 2 == 0 { 0.0 } else { 7.0 };
                while x < rect.right() {
                    tex.line_segment([pos2(x, y), pos2(x, (y + 9.0).min(rect.bottom()))], mortar);
                    x += 14.0;
                }
                y += 9.0;
                tex.hline(rect.x_range(), y, mortar);
                course += 1;
            }
        }
        Kind::Empty => {}
    }
}

#[derive(Debug, Clone, Copy)]
enum Icon {
    Play,
    Pause,
    Step,
    Minus,
    Plus,
    Menu,
    Recent,
    Tools,
    Tag,
    Kind(Kind),
    Heat,
    Cool,
    Erase,
    Over,
    Temp,
}

/// Points on a circle from `a0` to `a1` degrees (y down, so 90 is straight down).
fn arc(c: (f32, f32), r: f32, a0: f32, a1: f32) -> Vec<(f32, f32)> {
    let n = 10;
    (0..=n)
        .map(|i| {
            let a = (a0 + (a1 - a0) * i as f32 / n as f32).to_radians();
            (c.0 + r * a.cos(), c.1 + r * a.sin())
        })
        .collect()
}

/// Draws `icon` as strokes in a `size`-point square centred on `c`. The shapes
/// are drawn on an 18 x 18 grid.
fn paint_icon(p: &Painter, icon: Icon, c: Pos2, size: f32, color: Color32) {
    let k = size / 18.0;
    let at = move |(x, y): (f32, f32)| pos2(c.x + (x - 9.0) * k, c.y + (y - 9.0) * k);
    let stroke = Stroke::new(1.6, color);
    let open = |pts: &[(f32, f32)]| {
        p.add(Shape::line(pts.iter().copied().map(at).collect(), stroke));
    };
    let closed = |pts: &[(f32, f32)]| {
        p.add(Shape::closed_line(pts.iter().copied().map(at).collect(), stroke));
    };
    let circle = |center: (f32, f32), r: f32| {
        p.circle_stroke(at(center), r * k, stroke);
    };
    let dot = |center: (f32, f32)| {
        p.circle_filled(at(center), 1.0 * k, color);
    };
    let thermometer = |x: f32| {
        open(&[(x, 2.5), (x, 10.5)]);
        circle((x, 13.0), 2.5);
    };
    match icon {
        Icon::Play => closed(&[(5.0, 3.5), (5.0, 14.5), (14.0, 9.0)]),
        Icon::Pause => {
            open(&[(6.0, 4.0), (6.0, 14.0)]);
            open(&[(12.0, 4.0), (12.0, 14.0)]);
        }
        Icon::Step => {
            closed(&[(4.0, 3.5), (4.0, 14.5), (11.5, 9.0)]);
            open(&[(14.0, 3.5), (14.0, 14.5)]);
        }
        Icon::Minus => open(&[(3.0, 9.0), (15.0, 9.0)]),
        Icon::Plus => {
            open(&[(3.0, 9.0), (15.0, 9.0)]);
            open(&[(9.0, 3.0), (9.0, 15.0)]);
        }
        Icon::Menu => {
            for y in [5.0, 9.0, 13.0] {
                open(&[(3.0, y), (15.0, y)]);
            }
        }
        Icon::Recent => {
            circle((9.0, 9.0), 6.5);
            open(&[(9.0, 5.5), (9.0, 9.0), (11.5, 10.5)]);
        }
        Icon::Tools => {
            thermometer(6.0);
            for a in [90.0f32, 30.0, 150.0] {
                let (dx, dy) = (3.2 * a.to_radians().cos(), 3.2 * a.to_radians().sin());
                open(&[(13.0 - dx, 9.0 - dy), (13.0 + dx, 9.0 + dy)]);
            }
        }
        Icon::Heat => {
            thermometer(7.0);
            open(&[(13.5, 13.0), (13.5, 4.0)]);
            open(&[(11.5, 6.0), (13.5, 4.0), (15.5, 6.0)]);
        }
        Icon::Cool => {
            for a in [90.0f32, 30.0, 150.0] {
                let (dx, dy) = (7.0 * a.to_radians().cos(), 7.0 * a.to_radians().sin());
                open(&[(9.0 - dx, 9.0 - dy), (9.0 + dx, 9.0 + dy)]);
            }
        }
        Icon::Erase => {
            closed(&[(10.5, 2.5), (15.5, 7.5), (8.5, 14.5), (5.0, 14.5), (2.5, 12.0)]);
            open(&[(6.0, 6.5), (11.0, 11.5)]);
            open(&[(8.5, 15.5), (15.5, 15.5)]);
        }
        Icon::Over => {
            closed(&[(3.0, 7.0), (11.0, 7.0), (11.0, 15.0), (3.0, 15.0)]);
            closed(&[(7.0, 3.0), (15.0, 3.0), (15.0, 11.0), (7.0, 11.0)]);
        }
        Icon::Temp => {
            let mut lens = arc((9.0, 15.0), 9.0, 216.0, 324.0);
            lens.extend(arc((9.0, 3.0), 9.0, 36.0, 144.0));
            closed(&lens);
            circle((9.0, 9.0), 2.0);
        }
        Icon::Tag => {
            closed(&[(2.5, 9.5), (2.5, 2.5), (9.5, 2.5), (16.0, 9.0), (9.0, 16.0)]);
            circle((6.0, 6.0), 1.2);
        }
        Icon::Kind(Kind::Powder) => {
            open(&[(2.0, 15.5), (16.0, 15.5)]);
            open(
                &arc((9.0, 15.5), 6.0, 180.0, 360.0)
                    .iter()
                    .map(|&(x, y)| (x, 15.5 - (15.5 - y) * 1.05))
                    .collect::<Vec<_>>(),
            );
            dot((7.0, 5.5));
            dot((10.5, 3.5));
            dot((11.5, 7.0));
        }
        Icon::Kind(Kind::Liquid) => {
            let mut drop = vec![(9.0, 2.0)];
            drop.extend(arc((9.0, 11.5), 4.8, 205.0, -25.0));
            closed(&drop);
        }
        Icon::Kind(Kind::Gas) => {
            let mut cloud = arc((5.5, 11.5), 3.0, 90.0, 250.0);
            cloud.extend(arc((9.5, 8.5), 4.2, 200.0, 340.0));
            cloud.extend(arc((13.0, 11.5), 3.0, 290.0, 450.0));
            closed(&cloud);
        }
        Icon::Kind(Kind::Solid) => {
            closed(&[(2.5, 4.5), (15.5, 4.5), (15.5, 13.5), (2.5, 13.5)]);
            open(&[(2.5, 9.0), (15.5, 9.0)]);
            open(&[(7.5, 4.5), (7.5, 9.0)]);
            open(&[(11.0, 9.0), (11.0, 13.5)]);
        }
        Icon::Kind(Kind::Empty) => {}
    }
}
