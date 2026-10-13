//! Apple-style building blocks for egui: springs, an adaptive palette,
//! grouped lists, a segmented control, battery rings and EQ sliders.

use crate::proto::BatteryLevel;
use eframe::egui::{
    self, Color32, CursorIcon, FontData, FontDefinitions, FontFamily, FontId, Id, Pos2, Rect, RichText, Rounding, Sense,
    Shape, Stroke, Ui, Vec2, pos2, vec2,
};
use std::f32::consts::{PI, TAU};
use std::sync::atomic::{AtomicBool, Ordering};

// ---- motion ----------------------------------------------------------------

/// Set from the Windows "Animation effects" setting: springs jump straight to their target.
pub static REDUCE_MOTION: AtomicBool = AtomicBool::new(false);

/// (response seconds, damping ratio) — Apple's designer-facing spring parameters.
pub type SpringParams = (f32, f32);
/// Critically damped: the default for anything that wasn't thrown.
pub const SMOOTH: SpringParams = (0.35, 1.0);
/// A little overshoot, only after a gesture that carried momentum.
pub const MOMENTUM: SpringParams = (0.3, 0.8);
pub const SLOW: SpringParams = (0.8, 1.0);

#[derive(Clone, Copy)]
struct SpringState {
    x: f32,
    v: f32,
}

/// Advances a spring toward `target` and returns its presentation value. Because it
/// always integrates from the current value and velocity, a new target mid-flight
/// redirects the motion without a jump.
pub fn spring(ctx: &egui::Context, id: Id, target: f32, (response, damping): SpringParams) -> f32 {
    let mut s = ctx.data(|d| d.get_temp::<SpringState>(id)).unwrap_or(SpringState { x: target, v: 0.0 });
    if REDUCE_MOTION.load(Ordering::Relaxed) {
        s = SpringState { x: target, v: 0.0 };
    } else {
        let dt = ctx.input(|i| i.stable_dt).clamp(0.0, 1.0 / 20.0);
        let k = (TAU / response).powi(2);
        let c = 4.0 * PI * damping / response;
        const STEPS: usize = 8;
        let h = dt / STEPS as f32;
        for _ in 0..STEPS {
            let a = -k * (s.x - target) - c * s.v;
            s.v += a * h;
            s.x += s.v * h;
        }
        if (s.x - target).abs() < 1e-3 && s.v.abs() < 1e-2 {
            s = SpringState { x: target, v: 0.0 };
        } else {
            ctx.request_repaint();
        }
    }
    ctx.data_mut(|d| d.insert_temp(id, s));
    s.x
}

/// Pins a spring to a value (1:1 tracking) and seeds the velocity it continues with on release.
pub fn spring_set(ctx: &egui::Context, id: Id, x: f32, v: f32) {
    ctx.data_mut(|d| d.insert_temp(id, SpringState { x, v }));
}

/// Progressive resistance past a boundary instead of a hard stop.
fn rubberband(overshoot: f32, dimension: f32) -> f32 {
    const C: f32 = 0.55;
    overshoot * dimension * C / (dimension + C * overshoot.abs())
}

// ---- palette -----------------------------------------------------------------

#[derive(Clone, Copy)]
pub struct Palette {
    pub bg: Color32,
    pub card: Color32,
    pub fill: Color32,
    pub pressed: Color32,
    pub pill: Color32,
    pub separator: Color32,
    pub label: Color32,
    pub secondary: Color32,
    pub tint: Color32,
    pub tint_text: Color32,
    pub on_tint: Color32,
    pub green: Color32,
    pub red: Color32,
    pub knob: Color32,
    pub shadow: Color32,
}

impl Palette {
    pub fn new(dark: bool) -> Self {
        if dark {
            Palette {
                bg: Color32::from_rgb(0, 0, 0),
                card: Color32::from_rgb(28, 28, 30),
                fill: Color32::from_rgb(44, 44, 46),
                pressed: Color32::from_rgb(58, 58, 60),
                pill: Color32::from_rgb(99, 99, 102),
                separator: Color32::from_rgb(56, 56, 58),
                label: Color32::from_rgb(255, 255, 255),
                secondary: Color32::from_rgb(152, 152, 159),
                tint: Color32::from_rgb(255, 214, 10),
                tint_text: Color32::from_rgb(255, 214, 10),
                on_tint: Color32::from_rgb(0, 0, 0),
                green: Color32::from_rgb(48, 209, 88),
                red: Color32::from_rgb(255, 69, 58),
                knob: Color32::from_rgb(255, 255, 255),
                shadow: Color32::from_black_alpha(90),
            }
        } else {
            Palette {
                bg: Color32::from_rgb(242, 242, 247),
                card: Color32::from_rgb(255, 255, 255),
                fill: Color32::from_rgb(233, 233, 235),
                pressed: Color32::from_rgb(222, 222, 226),
                pill: Color32::from_rgb(255, 255, 255),
                separator: Color32::from_rgb(216, 216, 220),
                label: Color32::from_rgb(0, 0, 0),
                secondary: Color32::from_rgb(138, 138, 142),
                tint: Color32::from_rgb(255, 204, 0),
                tint_text: Color32::from_rgb(160, 118, 0),
                on_tint: Color32::from_rgb(0, 0, 0),
                green: Color32::from_rgb(52, 199, 89),
                red: Color32::from_rgb(255, 59, 48),
                knob: Color32::from_rgb(255, 255, 255),
                shadow: Color32::from_black_alpha(40),
            }
        }
    }
}

// ---- type ----------------------------------------------------------------------

/// Segoe UI is the Windows system face; fall back to egui's font if it's missing.
pub fn install_fonts(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    let fallback = fonts.families[&FontFamily::Proportional].clone();
    let read = |f: &str| std::fs::read(format!(r"C:\Windows\Fonts\{f}")).ok();
    let mut family = |name: &str, file: &str| {
        let mut list = fallback.clone();
        if let Some(bytes) = read(file) {
            fonts.font_data.insert(name.into(), FontData::from_owned(bytes));
            list.insert(0, name.into());
        }
        fonts.families.insert(FontFamily::Name(name.into()), list);
    };
    family("regular", "segoeui.ttf");
    family("semibold", "seguisb.ttf");
    family("bold", "segoeuib.ttf");
    let regular = fonts.families[&FontFamily::Name("regular".into())].clone();
    fonts.families.insert(FontFamily::Proportional, regular);
    ctx.set_fonts(fonts);
}

pub fn regular(size: f32) -> FontId {
    FontId::new(size, FontFamily::Proportional)
}
pub fn semibold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("semibold".into()))
}
pub fn bold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("bold".into()))
}

/// Tracking scales with size: tight for display text, slightly open for small captions.
pub fn text(s: impl Into<String>, font: FontId, color: Color32) -> RichText {
    let tracking = if font.size >= 24.0 {
        -0.02 * font.size
    } else if font.size <= 12.5 {
        0.02 * font.size
    } else {
        0.0
    };
    RichText::new(s).font(font).color(color).extra_letter_spacing(tracking)
}

fn galley(ui: &Ui, s: &str, font: FontId, color: Color32) -> std::sync::Arc<egui::Galley> {
    ui.painter().layout_no_wrap(s.to_owned(), font, color)
}

// ---- layout ------------------------------------------------------------------------

pub const RADIUS: f32 = 12.0;
pub const ROW_H: f32 = 44.0;
pub const INSET: f32 = 16.0;

pub fn section_header(ui: &mut Ui, pal: &Palette, title: &str) {
    ui.add_space(22.0);
    ui.horizontal(|ui| {
        ui.add_space(INSET);
        ui.label(text(title.to_uppercase(), regular(12.0), pal.secondary));
    });
    ui.add_space(4.0);
}

pub fn section_footer(ui: &mut Ui, pal: &Palette, note: &str) {
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.add_space(INSET);
        // wrap inside the inset column so the footer never widens the layout
        let w = (ui.available_width() - INSET).max(0.0);
        ui.allocate_ui(vec2(w, 0.0), |ui| {
            ui.set_max_width(w);
            ui.add(egui::Label::new(text(note, regular(12.0), pal.secondary)).wrap());
        });
    });
}

pub fn card<R>(ui: &mut Ui, pal: &Palette, padded: bool, add: impl FnOnce(&mut Ui) -> R) -> R {
    let margin = if padded { egui::Margin::symmetric(INSET, 14.0) } else { egui::Margin::ZERO };
    egui::Frame::none()
        .fill(pal.card)
        .rounding(Rounding::same(RADIUS))
        .inner_margin(margin)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 0.0;
            add(ui)
        })
        .inner
}

#[derive(Clone, Copy, PartialEq)]
pub enum RowPos {
    First,
    Middle,
    Last,
    Only,
}

impl RowPos {
    pub fn of(i: usize, n: usize) -> Self {
        match (i, n) {
            (_, 1) => RowPos::Only,
            (0, _) => RowPos::First,
            (i, n) if i + 1 == n => RowPos::Last,
            _ => RowPos::Middle,
        }
    }
    fn rounding(self) -> Rounding {
        let (top, bottom) = match self {
            RowPos::First => (RADIUS, 0.0),
            RowPos::Middle => (0.0, 0.0),
            RowPos::Last => (0.0, RADIUS),
            RowPos::Only => (RADIUS, RADIUS),
        };
        Rounding { nw: top, ne: top, sw: bottom, se: bottom }
    }
}

pub enum Trailing<'a> {
    None,
    Check(bool),
    CheckAndChevron(bool),
    Value(&'a str),
    Pill(&'a str),
}

fn checkmark(painter: &egui::Painter, center: Pos2, color: Color32) {
    let p = |x: f32, y: f32| center + vec2(x, y);
    painter.add(Shape::line(vec![p(-5.5, 0.5), p(-1.5, 4.5), p(6.0, -4.5)], Stroke::new(2.2_f32, color)));
}

fn chevron(painter: &egui::Painter, center: Pos2, color: Color32) {
    let p = |x: f32, y: f32| center + vec2(x, y);
    painter.add(Shape::line(vec![p(-2.5, -5.0), p(2.5, 0.0), p(-2.5, 5.0)], Stroke::new(1.8_f32, color)));
}

/// One row of an inset grouped list. Highlights on press, acts on release.
pub fn list_row(ui: &mut Ui, pal: &Palette, pos: RowPos, title: &str, title_color: Color32, trailing: Trailing, enabled: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_H), if enabled { Sense::click() } else { Sense::hover() });
    let resp = if enabled { resp.on_hover_cursor(CursorIcon::PointingHand) } else { resp };
    let painter = ui.painter_at(rect);
    let fade = |c: Color32| if enabled { c } else { c.gamma_multiply(0.4) };

    if enabled && resp.is_pointer_button_down_on() {
        painter.rect_filled(rect, pos.rounding(), pal.pressed);
    }
    if matches!(pos, RowPos::Middle | RowPos::Last) {
        painter.hline(rect.left() + INSET..=rect.right(), rect.top(), Stroke::new(1.0_f32, pal.separator));
    }
    let g = galley(ui, title, regular(15.0), fade(title_color));
    painter.galley(pos2(rect.left() + INSET, rect.center().y - g.size().y / 2.0), g, fade(title_color));

    let right = rect.right() - INSET;
    match trailing {
        Trailing::None => {}
        Trailing::Check(on) => {
            if on {
                checkmark(&painter, pos2(right - 6.0, rect.center().y), fade(pal.tint_text));
            }
        }
        Trailing::CheckAndChevron(on) => {
            chevron(&painter, pos2(right - 3.0, rect.center().y), fade(pal.secondary));
            if on {
                checkmark(&painter, pos2(right - 26.0, rect.center().y), fade(pal.tint_text));
            }
        }
        Trailing::Value(v) => {
            let g = galley(ui, v, regular(15.0), fade(pal.secondary));
            painter.galley(pos2(right - g.size().x, rect.center().y - g.size().y / 2.0), g, fade(pal.secondary));
        }
        Trailing::Pill(v) => {
            let g = galley(ui, v, semibold(13.0), pal.on_tint);
            let pill = Rect::from_min_size(pos2(right - g.size().x - 20.0, rect.center().y - 12.0), vec2(g.size().x + 20.0, 24.0));
            painter.rect_filled(pill, Rounding::same(12.0), fade(pal.tint));
            painter.galley(pill.center() - g.size() / 2.0, g, pal.on_tint);
        }
    }
    resp
}

// ---- segmented control ---------------------------------------------------------------

/// Returns the index tapped this frame. The selection pill slides on a spring, so a
/// second tap mid-slide simply redirects it.
pub fn segmented(ui: &mut Ui, pal: &Palette, id: Id, labels: &[&str], selected: Option<usize>, enabled: bool) -> Option<usize> {
    let n = labels.len();
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 36.0), Sense::hover());
    let painter = ui.painter_at(rect.expand(4.0));
    let alpha = if enabled { 1.0 } else { 0.4 };
    painter.rect_filled(rect, Rounding::same(9.0), pal.fill.gamma_multiply(alpha));

    let seg_w = rect.width() / n as f32;
    let shown = spring(ui.ctx(), id.with("visible"), if selected.is_some() { 1.0 } else { 0.0 }, SMOOTH);
    let pos = spring(ui.ctx(), id.with("pos"), selected.unwrap_or(0) as f32, SMOOTH);

    // separators fade out as the pill passes over them
    for b in 1..n {
        let bf = b as f32;
        let d = if bf < pos { pos - bf } else if bf > pos + 1.0 { bf - pos - 1.0 } else { 0.0 };
        let a = ((d * 3.0).min(1.0) * shown + (1.0 - shown)).min(1.0) * alpha;
        let x = rect.left() + bf * seg_w;
        painter.vline(x, rect.top() + 9.0..=rect.bottom() - 9.0, Stroke::new(1.0_f32, pal.separator.gamma_multiply(a)));
    }

    if shown > 0.01 {
        let pill = Rect::from_min_size(pos2(rect.left() + pos * seg_w, rect.top()), vec2(seg_w, rect.height())).shrink(2.5);
        painter.rect_filled(pill.translate(vec2(0.0, 1.5)), Rounding::same(7.0), pal.shadow.gamma_multiply(shown * alpha * 0.6));
        painter.rect_filled(pill, Rounding::same(7.0), pal.pill.gamma_multiply(shown * alpha));
    }

    let mut clicked = None;
    for (i, label) in labels.iter().enumerate() {
        let seg = Rect::from_min_size(pos2(rect.left() + i as f32 * seg_w, rect.top()), vec2(seg_w, rect.height()));
        let resp = ui.interact(seg, id.with(i), if enabled { Sense::click() } else { Sense::hover() });
        let pressed = enabled && resp.is_pointer_button_down_on();
        let color = pal.label.gamma_multiply(alpha * if pressed { 0.45 } else { 1.0 });
        let font = if selected == Some(i) { semibold(13.0) } else { regular(13.0) };
        let g = galley(ui, label, font, color);
        painter.galley(seg.center() - g.size() / 2.0, g, color);
        if enabled && resp.on_hover_cursor(CursorIcon::PointingHand).clicked() {
            clicked = Some(i);
        }
    }
    clicked
}

// ---- battery ring ----------------------------------------------------------------------

pub fn battery_ring(ui: &mut Ui, pal: &Palette, id: Id, label: &str, level: Option<BatteryLevel>, width: f32) {
    let (rect, _) = ui.allocate_exact_size(vec2(width, 98.0), Sense::hover());
    let painter = ui.painter_at(rect);
    let r = 29.0;
    let stroke_w = 5.5;
    let center = pos2(rect.center().x, rect.top() + r + 6.0);
    painter.circle_stroke(center, r, Stroke::new(stroke_w, pal.fill));

    let frac = spring(ui.ctx(), id, level.map_or(0.0, |l| l.percent as f32 / 100.0), SLOW);
    if let Some(l) = level {
        let color = if l.charging || l.percent > 20 { pal.green } else { pal.red };
        if frac > 0.002 {
            let steps = ((frac * 90.0) as usize).max(2);
            let pts: Vec<Pos2> = (0..=steps)
                .map(|i| {
                    let a = -PI / 2.0 + TAU * frac * i as f32 / steps as f32;
                    center + vec2(a.cos(), a.sin()) * r
                })
                .collect();
            // round caps
            painter.circle_filled(pts[0], stroke_w / 2.0, color);
            painter.circle_filled(*pts.last().unwrap(), stroke_w / 2.0, color);
            painter.add(Shape::line(pts, Stroke::new(stroke_w, color)));
        }
        let g = galley(ui, &format!("{}%", l.percent), semibold(15.0), pal.label);
        painter.galley(center - g.size() / 2.0, g, pal.label);
    } else {
        let g = galley(ui, "–", semibold(15.0), pal.secondary);
        painter.galley(center - g.size() / 2.0, g, pal.secondary);
    }

    let charging = level.is_some_and(|l| l.charging);
    let g = galley(ui, label, regular(13.0), pal.secondary);
    let label_w = g.size().x + if charging { 12.0 } else { 0.0 };
    let label_pos = pos2(rect.center().x - label_w / 2.0, center.y + r + 10.0);
    if charging {
        // lightning bolt drawn as a stroke so it stays crisp at any scale
        let o = label_pos + vec2(0.0, 2.5);
        let p = |x: f32, y: f32| o + vec2(x, y) * 11.0;
        painter.add(Shape::line(vec![p(0.62, 0.0), p(0.15, 0.55), p(0.7, 0.45), p(0.25, 1.0)], Stroke::new(1.8_f32, pal.green)));
    }
    painter.galley(label_pos + vec2(if charging { 12.0 } else { 0.0 }, 0.0), g, pal.secondary);
}

// ---- EQ slider ------------------------------------------------------------------------------

#[derive(Clone, Copy, Default)]
struct DragState {
    grab_offset: f32,
    thrown: bool,
}

/// Vertical gain slider. The knob tracks the pointer 1:1 from wherever it was grabbed,
/// rubber-bands past the limits, and on release springs to the nearest whole dB,
/// carrying the release velocity. Returns true when a drag ends with a new value.
pub fn eq_slider(ui: &mut Ui, pal: &Palette, id: Id, gain: &mut i8, freq: u16, (min, max): (i8, i8), width: f32) -> bool {
    let (rect, _) = ui.allocate_exact_size(vec2(width, 200.0), Sense::hover());
    let track = Rect::from_center_size(pos2(rect.center().x, rect.top() + 100.0), vec2(4.0, 140.0));
    let span = (max - min) as f32;
    let px_per_db = track.height() / span;
    let y_of = |db: f32| track.bottom() - (db - min as f32) * px_per_db;
    let db_of = |y: f32| min as f32 + (track.bottom() - y) / px_per_db;

    let ctx = ui.ctx().clone();
    let spring_id = id.with("knob");
    let drag_id = id.with("drag");
    let mut drag: DragState = ctx.data(|d| d.get_temp(drag_id)).unwrap_or_default();

    let hit = Rect::from_center_size(track.center(), vec2(width, track.height() + 28.0));
    let resp = ui.interact(hit, id, Sense::drag()).on_hover_cursor(CursorIcon::ResizeVertical);
    let mut changed = false;

    let presentation = if resp.dragged() {
        let pointer_y = resp.interact_pointer_pos().map_or(y_of(*gain as f32), |p| p.y);
        let current = ctx.data(|d| d.get_temp::<SpringState>(spring_id)).map_or(*gain as f32, |s| s.x);
        if resp.drag_started() {
            drag.grab_offset = pointer_y - y_of(current);
        }
        let raw = db_of(pointer_y - drag.grab_offset);
        let shown = if raw > max as f32 {
            max as f32 + rubberband(raw - max as f32, span)
        } else if raw < min as f32 {
            min as f32 + rubberband(raw - min as f32, span)
        } else {
            raw
        };
        spring_set(&ctx, spring_id, shown, 0.0);
        *gain = raw.round().clamp(min as f32, max as f32) as i8;
        shown
    } else {
        if resp.drag_stopped() {
            let vy = ctx.input(|i| i.pointer.velocity().y);
            let current = ctx.data(|d| d.get_temp::<SpringState>(spring_id)).map_or(*gain as f32, |s| s.x);
            spring_set(&ctx, spring_id, current, -vy / px_per_db);
            drag.thrown = true;
            changed = true;
        }
        let x = spring(&ctx, spring_id, *gain as f32, if drag.thrown { MOMENTUM } else { SMOOTH });
        if (x - *gain as f32).abs() < 1e-3 {
            drag.thrown = false;
        }
        x
    };
    ctx.data_mut(|d| d.insert_temp(drag_id, drag));

    let painter = ui.painter_at(rect);
    painter.rect_filled(track, Rounding::same(2.0), pal.fill);
    let zero = y_of(0.0);
    painter.hline(track.center().x - 7.0..=track.center().x + 7.0, zero, Stroke::new(1.0_f32, pal.separator));
    let knob_y = y_of(presentation);
    let fill = Rect::from_x_y_ranges(track.x_range(), zero.min(knob_y)..=zero.max(knob_y));
    painter.rect_filled(fill, Rounding::same(2.0), pal.tint);

    let pressed = resp.is_pointer_button_down_on() || resp.dragged();
    let knob_r = spring(&ctx, id.with("press"), if pressed { 12.5 } else { 10.5 }, SMOOTH);
    let knob = pos2(track.center().x, knob_y);
    painter.circle_filled(knob + vec2(0.0, 1.5), knob_r + 0.5, pal.shadow);
    painter.circle_filled(knob, knob_r, pal.knob);

    let value = if *gain == 0 { "0".to_string() } else { format!("{:+}", gain) };
    let color = if *gain == 0 { pal.secondary } else { pal.tint_text };
    let g = galley(ui, &value, semibold(13.0), color);
    painter.galley(pos2(rect.center().x - g.size().x / 2.0, rect.top()), g, color);
    let f = if freq >= 1000 { format!("{}k", freq as f32 / 1000.0) } else { freq.to_string() };
    let g = galley(ui, &f, regular(12.0), pal.secondary);
    painter.galley(pos2(rect.center().x - g.size().x / 2.0, rect.bottom() - g.size().y), g, pal.secondary);
    changed
}

// ---- buttons ------------------------------------------------------------------------------

pub fn filled_button(ui: &mut Ui, pal: &Palette, label: &str, width: f32, enabled: bool) -> bool {
    let (rect, resp) = ui.allocate_exact_size(vec2(width, 40.0), if enabled { Sense::click() } else { Sense::hover() });
    let pressed = enabled && resp.is_pointer_button_down_on();
    let scale = spring(ui.ctx(), resp.id.with("scale"), if pressed { 0.97 } else { 1.0 }, (0.2, 1.0));
    let r = Rect::from_center_size(rect.center(), rect.size() * scale);
    let a = if enabled { 1.0 } else { 0.4 };
    let fill = if pressed { pal.tint.linear_multiply(0.85) } else { pal.tint };
    ui.painter().rect_filled(r, Rounding::same(10.0), fill.gamma_multiply(a));
    let g = galley(ui, label, semibold(15.0), pal.on_tint);
    ui.painter().galley(r.center() - g.size() / 2.0, g, pal.on_tint.gamma_multiply(a));
    enabled && resp.on_hover_cursor(CursorIcon::PointingHand).clicked()
}

pub fn text_button(ui: &mut Ui, label: &str, font: FontId, color: Color32, enabled: bool) -> bool {
    let g = galley(ui, label, font, color);
    let (rect, resp) = ui.allocate_exact_size(g.size() + vec2(8.0, 12.0), if enabled { Sense::click() } else { Sense::hover() });
    let pressed = enabled && resp.is_pointer_button_down_on();
    let c = color.gamma_multiply(if !enabled { 0.35 } else if pressed { 0.4 } else { 1.0 });
    ui.painter().galley(rect.center() - g.size() / 2.0, g, c);
    enabled && resp.on_hover_cursor(CursorIcon::PointingHand).clicked()
}

pub fn status_dot(ui: &mut Ui, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(8.0), Sense::hover());
    ui.painter().circle_filled(rect.center(), 3.5, color);
}
