//! Screen layout: an upright 4:3 sim area sitting on a control panel about as
//! tall as an on-screen keyboard, both kept clear of the system bars.

/// Where things go on screen, in physical pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Layout {
    /// The sim area as x, y, width, height: 3 wide by 4 tall, one cell per
    /// screen pixel, directly above the control panel.
    pub sim: [u32; 4],
    /// Top edge and height of the control panel.
    pub panel_top: u32,
    pub panel_height: u32,
    /// Rows kept clear for the system bars: top (status bar) and bottom
    /// (gesture or navigation bar).
    pub insets: [u32; 2],
}

/// Control panel height in points (dp on Android): about what the on-screen
/// keyboard takes with its suggestion strip, so every control is in thumb reach.
pub const PANEL_POINTS: f32 = 300.0;

/// The panel gives up height down to this before the sim area narrows, so a
/// tall status bar costs a little panel rather than the full-width world.
pub const MIN_PANEL_POINTS: f32 = 240.0;

/// Bottom rows kept clear even when Android reports no inset, so no control
/// sits on the gesture bar.
pub const MIN_BOTTOM_POINTS: f32 = 24.0;

/// Lays out a screen of `size` pixels at `scale` pixels per point, with the
/// system bars covering `insets` rows at the top and bottom.
///
/// The sim area takes the full width when it fits, shrinking the panel toward
/// [`MIN_PANEL_POINTS`] if it must. On a screen too short even for that, it
/// keeps 4:3 and narrows, centred.
pub fn layout(size: [u32; 2], scale: f32, insets: [u32; 2]) -> Layout {
    let [w, h] = size;
    let top = insets[0].min(h);
    let bottom = insets[1].max((MIN_BOTTOM_POINTS * scale).round() as u32).min(h - top);
    let usable = h - top - bottom;
    // The panel never takes more than half the room, however large the scale.
    let max_panel = ((PANEL_POINTS * scale).round() as u32).min(usable / 2);
    let min_panel = ((MIN_PANEL_POINTS * scale).round() as u32).min(max_panel);

    // Even sizes keep the movement pass's 2 x 2 blocks whole at the edges.
    let mut sim_w = w & !1;
    let mut sim_h = (sim_w * 4 / 3) & !1;
    let panel = usable.saturating_sub(sim_h).clamp(min_panel, max_panel);
    let room = usable - panel;
    if sim_h > room {
        sim_h = room & !1;
        sim_w = (sim_h * 3 / 4) & !1;
    }
    let panel_top = h - bottom - panel;
    Layout {
        sim: [(w - sim_w) / 2, panel_top - sim_h, sim_w, sim_h],
        panel_top,
        panel_height: panel,
        insets: [top, bottom],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pixel_10_pro_xl_gets_a_full_width_4_by_3_area() {
        // 1080 x 2404 at 2.625 pixels per dp, gesture bar about 24 dp.
        let l = layout([1080, 2404], 2.625, [0, 63]);
        assert_eq!(l.sim[2..], [1080, 1440]);
        assert_eq!(l.sim[0], 0);
        assert_eq!(l.panel_height, 788);
        // The sim area sits right on the panel, and the panel on the gesture bar.
        assert_eq!(l.sim[1] + l.sim[3], l.panel_top);
        assert_eq!(l.panel_top + l.panel_height, 2404 - 63);
    }

    #[test]
    fn a_status_bar_takes_panel_height_not_world_width() {
        // A 136-pixel status bar and camera cutout on the same phone.
        let l = layout([1080, 2404], 2.625, [136, 63]);
        assert_eq!(l.sim, [0, 136, 1080, 1440]);
        assert_eq!(l.panel_height, 2404 - 136 - 63 - 1440);
        assert!(l.panel_height as f32 >= MIN_PANEL_POINTS * 2.625);
    }

    #[test]
    fn bottom_is_kept_clear_when_android_reports_no_inset() {
        let l = layout([1080, 2404], 2.625, [0, 0]);
        assert_eq!(l.insets[1], 63);
        assert_eq!(l.panel_top + l.panel_height, 2404 - 63);
    }

    #[test]
    fn short_screens_narrow_the_area_but_keep_4_by_3() {
        let l = layout([1080, 1600], 2.625, [60, 63]);
        let [x, y, w, h] = l.sim;
        assert!(w < 1080);
        assert_eq!(w % 2, 0);
        assert!((h as f32 / w as f32 - 4.0 / 3.0).abs() < 0.01);
        assert_eq!(x, (1080 - w) / 2);
        assert!(y >= 60);
        assert_eq!(y + h, l.panel_top);
    }

    #[test]
    fn tiny_windows_do_not_underflow() {
        let l = layout([1, 1], 3.0, [5, 5]);
        assert_eq!(l.sim[2..], [0, 0]);
        assert!(l.panel_top + l.panel_height <= 1);
    }
}
