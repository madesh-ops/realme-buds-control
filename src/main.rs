#![windows_subsystem = "windows"]

mod device;
mod proto;
mod ui;
mod win;

use device::{Request, Snapshot};
use eframe::egui::{self, Stroke};
use proto::*;
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use ui::{Palette, RowPos, Trailing};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if !args.is_empty() {
        win::attach_parent_console();
        std::process::exit(run_cli(&args));
    }

    let (tx, rx) = mpsc::channel();
    let shared = Arc::new(Mutex::new(Snapshot { message: "Starting…".into(), ..Default::default() }));
    let ctx_slot: Arc<Mutex<Option<egui::Context>>> = Arc::default();
    {
        let shared = shared.clone();
        let ctx_slot = ctx_slot.clone();
        std::thread::spawn(move || {
            device::run_worker(rx, |s| {
                *shared.lock().unwrap() = s;
                if let Some(ctx) = ctx_slot.lock().unwrap().as_ref() {
                    ctx.request_repaint();
                }
            })
        });
    }

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("T310 Control")
            .with_inner_size([420.0, 860.0])
            .with_min_inner_size([380.0, 520.0])
            .with_icon(egui::IconData { rgba: include_bytes!("../assets/icon_64.rgba").to_vec(), width: 64, height: 64 }),
        ..Default::default()
    };
    let _ = eframe::run_native(
        "T310 Control",
        options,
        Box::new(move |cc| {
            *ctx_slot.lock().unwrap() = Some(cc.egui_ctx.clone());
            ui::REDUCE_MOTION.store(!win::animations_enabled(), std::sync::atomic::Ordering::Relaxed);
            ui::install_fonts(&cc.egui_ctx);
            cc.egui_ctx.set_theme(egui::ThemePreference::System);
            style_for(&cc.egui_ctx, egui::Theme::Light);
            style_for(&cc.egui_ctx, egui::Theme::Dark);
            Ok(Box::new(App::new(tx, shared)))
        }),
    );
}

fn run_cli(args: &[String]) -> i32 {
    let usage = "usage: t310_control status | mode anc|normal|transparency | eq <id> | custom add <name> [g1..g6] | custom set <id> <name> [g1..g6] | custom del <id>";
    let Some(addr) = device::find_paired() else {
        eprintln!("realme Buds T310 is not paired with this PC");
        return 1;
    };
    let mut b = match device::Earbuds::connect(addr) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("connect failed: {e}");
            return 1;
        }
    };
    let res = (|| -> std::io::Result<()> {
        match args[0].as_str() {
            "status" => {}
            "mode" => {
                let m = match args.get(1).map(String::as_str) {
                    Some("anc") => MODE_ANC,
                    Some("normal") | Some("off") => MODE_NORMAL,
                    Some("transparency") => MODE_TRANSPARENCY,
                    _ => {
                        eprintln!("{usage}");
                        return Ok(());
                    }
                };
                b.set_mode(m)?;
            }
            "eq" => {
                let id = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(0);
                b.select_eq(id)?;
            }
            "custom" => {
                let gains = |from: usize| -> Vec<(u16, i8)> {
                    DEFAULT_EQ_FREQS
                        .iter()
                        .enumerate()
                        .map(|(i, &f)| (f, args.get(from + i).and_then(|g| g.parse().ok()).unwrap_or(0)))
                        .collect()
                };
                match (args.get(1).map(String::as_str), args.get(2)) {
                    (Some("add"), Some(name)) => {
                        let id = b.custom_eq(EQ_ADD, 0, name, &gains(3))?;
                        println!("created custom EQ {id}");
                    }
                    (Some("set"), Some(id)) => {
                        let id: u8 = id.parse().unwrap_or(0);
                        let name = args.get(3).cloned().unwrap_or_default();
                        b.custom_eq(EQ_MODIFY, id, &name, &gains(4))?;
                        b.select_eq(id)?;
                    }
                    (Some("del"), Some(id)) => {
                        let id: u8 = id.parse().unwrap_or(0);
                        let name = b.custom_eqs()?.into_iter().find(|c| c.id == id).map(|c| c.name).unwrap_or_default();
                        let flat: Vec<(u16, i8)> = DEFAULT_EQ_FREQS.iter().map(|&f| (f, 0)).collect();
                        b.custom_eq(EQ_DELETE, id, &name, &flat)?;
                        b.select_eq(0)?;
                    }
                    _ => {
                        eprintln!("{usage}");
                        return Ok(());
                    }
                }
            }
            _ => {
                eprintln!("{usage}");
                return Ok(());
            }
        }
        let s = Snapshot {
            connected: true,
            address: Some(addr),
            message: format!("Connected to {}", win::format_addr(addr)),
            mode: Some(b.get_mode()?),
            battery: b.battery()?,
            firmware: b.firmware().ok(),
            eq: Some(b.current_eq()?),
            custom_eqs: b.custom_eqs()?,
            codec: win::a2dp_codec(addr),
            busy: false,
        };
        println!("{}", device::describe(&s));
        for c in &s.custom_eqs {
            println!("Custom EQ {} \"{}\": {:?}", c.id, c.name, c.bands);
        }
        Ok(())
    })();
    match res {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("error: {e}");
            1
        }
    }
}

/// A choice made in the UI that the earbuds haven't confirmed yet. It is shown
/// immediately and dropped once the earbuds agree, or after a timeout if they never do.
struct Pending {
    value: u8,
    since: Instant,
}

const MAX_CONTENT_WIDTH: f32 = 520.0;
const PENDING_TIMEOUT: Duration = Duration::from_secs(4);

fn optimistic(pending: &mut Option<Pending>, confirmed: Option<u8>) -> Option<u8> {
    if let Some(p) = pending {
        if confirmed == Some(p.value) || p.since.elapsed() > PENDING_TIMEOUT {
            *pending = None;
        }
    }
    pending.as_ref().map(|p| p.value).or(confirmed)
}

struct Editor {
    /// None while creating a new profile.
    id: Option<u8>,
    name: String,
    saved_name: String,
    bands: Vec<(u16, i8)>,
    confirm_delete: Option<Instant>,
}

struct App {
    tx: Sender<Request>,
    shared: Arc<Mutex<Snapshot>>,
    pending_mode: Option<Pending>,
    pending_eq: Option<Pending>,
    editor: Option<Editor>,
}

const MODES: [(u8, &str, &str); 3] = [
    (MODE_ANC, "Noise Cancellation", "Reduces outside noise so you can focus on your audio."),
    (MODE_TRANSPARENCY, "Transparency", "Lets outside sound in so you stay aware of your surroundings."),
    (MODE_NORMAL, "Off", "Noise control is off."),
];

fn codec_long_name(id: u8) -> &'static str {
    match id {
        0x00 => "Subband Coding",
        0x02 => "Advanced Audio Coding",
        _ => "",
    }
}

impl App {
    fn new(tx: Sender<Request>, shared: Arc<Mutex<Snapshot>>) -> Self {
        App { tx, shared, pending_mode: None, pending_eq: None, editor: None }
    }

    fn send(&self, r: Request) {
        let _ = self.tx.send(r);
    }

    fn header(&mut self, ui: &mut egui::Ui, pal: &Palette, s: &Snapshot) {
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.add_space(ui::INSET);
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                ui.label(ui::text("realme Buds", ui::semibold(13.0), pal.secondary));
                ui.label(ui::text("T310", ui::bold(34.0), pal.label));
                ui.add_space(2.0);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    let (color, msg) = if s.busy {
                        (pal.secondary, "Connecting…".to_string())
                    } else if s.connected {
                        (pal.green, "Connected".to_string())
                    } else {
                        (pal.red, s.message.clone())
                    };
                    ui::status_dot(ui, color);
                    ui.add(egui::Label::new(ui::text(msg, ui::regular(13.0), pal.secondary)).truncate());
                });
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                ui.add_space(ui::INSET - 4.0);
                let label = if s.connected { "Refresh" } else { "Connect" };
                if ui::text_button(ui, label, ui::semibold(15.0), pal.tint_text, !s.busy) {
                    self.send(Request::Connect);
                }
            });
        });
    }

    fn battery(&self, ui: &mut egui::Ui, pal: &Palette, s: &Snapshot) {
        ui::section_header(ui, pal, "Battery");
        // The case only reports a level while a bud is docked; 0% without charging means unknown.
        let case = s.battery.case.filter(|c| c.percent > 0 || c.charging);
        ui::card(ui, pal, true, |ui| {
            let w = ui.available_width() / 3.0;
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                ui::battery_ring(ui, pal, egui::Id::new("bat-l"), "Left", s.battery.left, w);
                ui::battery_ring(ui, pal, egui::Id::new("bat-r"), "Right", s.battery.right, w);
                ui::battery_ring(ui, pal, egui::Id::new("bat-c"), "Case", case, w);
            });
        });
        if s.connected && case.is_none() {
            ui::section_footer(ui, pal, "The case reports its level while a bud is inside and the lid is open.");
        }
    }

    fn noise_control(&mut self, ui: &mut egui::Ui, pal: &Palette, s: &Snapshot, live: bool) {
        ui::section_header(ui, pal, "Noise Control");
        let mode = optimistic(&mut self.pending_mode, s.mode);
        let selected = mode.and_then(|m| MODES.iter().position(|(v, _, _)| *v == m));
        let labels: Vec<&str> = MODES.iter().map(|(_, l, _)| *l).collect();
        let tapped = ui::card(ui, pal, true, |ui| ui::segmented(ui, pal, egui::Id::new("noise"), &labels, selected, live));
        if let Some(i) = tapped {
            let value = MODES[i].0;
            self.pending_mode = Some(Pending { value, since: Instant::now() });
            self.send(Request::SetMode(value));
        }
        let note = selected.map_or("Connect the earbuds to change noise control.", |i| MODES[i].2);
        ui::section_footer(ui, pal, note);
    }

    fn equalizer(&mut self, ui: &mut egui::Ui, pal: &Palette, s: &Snapshot, live: bool) {
        let eq = optimistic(&mut self.pending_eq, s.eq);
        let mut choose: Option<u8> = None;

        ui::section_header(ui, pal, "Equalizer");
        ui::card(ui, pal, false, |ui| {
            for (i, (id, name)) in PRESETS.iter().enumerate() {
                let row = ui::list_row(ui, pal, RowPos::of(i, PRESETS.len()), name, pal.label, Trailing::Check(eq == Some(*id)), live);
                if row.clicked() {
                    choose = Some(*id);
                }
            }
        });

        ui::section_header(ui, pal, "Custom EQ");
        let can_add = s.custom_eqs.len() < MAX_CUSTOM_EQS;
        let rows = s.custom_eqs.len() + 1;
        let mut open: Option<&proto::EqProfile> = None;
        let mut add = false;
        ui::card(ui, pal, false, |ui| {
            for (i, c) in s.custom_eqs.iter().enumerate() {
                let row = ui::list_row(ui, pal, RowPos::of(i, rows), &c.name, pal.label, Trailing::CheckAndChevron(eq == Some(c.id)), live);
                if row.clicked() {
                    choose = Some(c.id);
                    open = Some(c);
                }
            }
            let label = if can_add { "Add Custom EQ…" } else { "Add Custom EQ (3 of 3 used)" };
            if ui::list_row(ui, pal, RowPos::of(rows - 1, rows), label, pal.tint_text, Trailing::None, live && can_add).clicked() {
                add = true;
            }
        });

        if let Some(c) = open {
            self.editor = Some(Editor { id: Some(c.id), name: c.name.clone(), saved_name: c.name.clone(), bands: c.bands.clone(), confirm_delete: None });
        }
        if add {
            let n = (1..=MAX_CUSTOM_EQS).find(|i| !s.custom_eqs.iter().any(|c| c.name == format!("Custom {i}"))).unwrap_or(1);
            let name = format!("Custom {n}");
            let bands = DEFAULT_EQ_FREQS.iter().map(|&f| (f, 0)).collect();
            self.editor = Some(Editor { id: None, name: name.clone(), saved_name: name, bands, confirm_delete: None });
        }
        if let Some(id) = choose {
            self.pending_eq = Some(Pending { value: id, since: Instant::now() });
            self.send(Request::SelectEq(id));
        }

        // a profile deleted elsewhere closes its editor
        if let Some(Editor { id: Some(id), .. }) = &self.editor {
            if s.connected && !s.custom_eqs.iter().any(|c| c.id == *id) {
                self.editor = None;
            }
        }
        if self.editor.is_some() {
            self.eq_editor(ui, pal, live);
        } else {
            ui::section_footer(ui, pal, "Up to three custom profiles are stored on the earbuds. Select one to edit it.");
        }
    }

    fn eq_editor(&mut self, ui: &mut egui::Ui, pal: &Palette, live: bool) {
        let Some(ed) = self.editor.as_mut() else { return };
        let mut requests = Vec::new();
        let mut close = false;
        let save = |ed: &Editor| Request::SaveCustomEq { id: ed.id.unwrap_or(0), name: ed.name.trim().to_string(), bands: ed.bands.clone() };

        ui.add_space(12.0);
        ui::card(ui, pal, true, |ui| {
            ui.horizontal(|ui| {
                let name = egui::TextEdit::singleline(&mut ed.name)
                    .font(ui::semibold(17.0))
                    .text_color(pal.label)
                    .frame(false)
                    .char_limit(16)
                    .hint_text("Profile name")
                    .desired_width(ui.available_width() - 70.0);
                let r = ui.add(name);
                // rename an existing profile when editing finishes
                if ed.id.is_some() && r.lost_focus() && ed.name.trim() != ed.saved_name && !ed.name.trim().is_empty() {
                    ed.saved_name = ed.name.trim().to_string();
                    requests.push(save(ed));
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui::text_button(ui, if ed.id.is_some() { "Done" } else { "Cancel" }, ui::semibold(15.0), pal.tint_text, true) {
                        close = true;
                    }
                });
            });
            ui.add_space(10.0);

            let col = ui.available_width() / ed.bands.len() as f32;
            let mut released = false;
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                for (i, (freq, gain)) in ed.bands.iter_mut().enumerate() {
                    let id = egui::Id::new(("eq-band", ed.id, i));
                    released |= ui::eq_slider(ui, pal, id, gain, *freq, (EQ_MIN_DB, EQ_MAX_DB), col);
                }
            });
            // existing profiles apply as soon as a slider is released
            if released && ed.id.is_some() && live {
                requests.push(save(ed));
            }

            ui.add_space(12.0);
            ui.horizontal(|ui| {
                if ui::text_button(ui, "Reset to Flat", ui::regular(15.0), pal.tint_text, true) {
                    ed.bands.iter_mut().for_each(|(_, g)| *g = 0);
                    if ed.id.is_some() && live {
                        requests.push(save(ed));
                    }
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| match ed.id {
                    Some(id) => {
                        let armed = ed.confirm_delete.is_some_and(|t| t.elapsed() < Duration::from_secs(3));
                        let label = if armed { "Tap Again to Delete" } else { "Delete" };
                        if ui::text_button(ui, label, ui::semibold(15.0), pal.red, live) {
                            if armed {
                                requests.push(Request::DeleteCustomEq { id, name: ed.saved_name.clone() });
                                close = true;
                            } else {
                                ed.confirm_delete = Some(Instant::now());
                            }
                        }
                        if armed {
                            ui.ctx().request_repaint_after(Duration::from_millis(500));
                        }
                    }
                    None => {
                        if ui::filled_button(ui, pal, "Create", 110.0, live && !ed.name.trim().is_empty()) {
                            requests.push(Request::AddCustomEq { name: ed.name.trim().to_string(), bands: ed.bands.clone() });
                            close = true;
                        }
                    }
                });
            });
        });
        let note = if ed.id.is_some() { "Changes apply when you release a slider." } else { "Shape the sound, then create the profile." };
        ui::section_footer(ui, pal, note);

        for r in requests {
            self.send(r);
        }
        if close {
            self.editor = None;
        }
    }

    fn device_info(&self, ui: &mut egui::Ui, pal: &Palette, s: &Snapshot) {
        ui::section_header(ui, pal, "Connection");
        let codec = s.codec.active.map(win::codec_name);
        let supported: Vec<&str> = s.codec.available.iter().map(|&c| win::codec_name(c)).collect();
        let supported = supported.join(", ");
        let address = s.address.map(win::format_addr);
        let mut rows: Vec<(&str, Trailing)> = vec![(
            "Audio Codec",
            match codec {
                Some(c) => Trailing::Pill(c),
                None => Trailing::Value("Not streaming"),
            },
        )];
        if !supported.is_empty() {
            rows.push(("Supported Codecs", Trailing::Value(&supported)));
        }
        if let Some(fw) = &s.firmware {
            rows.push(("Firmware", Trailing::Value(fw)));
        }
        if let Some(a) = &address {
            rows.push(("Bluetooth Address", Trailing::Value(a)));
        }
        let n = rows.len();
        ui::card(ui, pal, false, |ui| {
            for (i, (title, trailing)) in rows.into_iter().enumerate() {
                ui::list_row(ui, pal, RowPos::of(i, n), title, pal.label, trailing, false);
            }
        });
        let note = match s.codec.active {
            Some(c) => format!(
                "{} is what Windows negotiated for this PC's audio. Your phone may use a different codec.",
                match codec_long_name(c) {
                    "" => win::codec_name(c).to_string(),
                    long => format!("{} ({long})", win::codec_name(c)),
                }
            ),
            None => "Choose the T310 as the Windows output device to see its codec.".to_string(),
        };
        ui::section_footer(ui, pal, &note);
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let s = self.shared.lock().unwrap().clone();
        let live = s.connected && !s.busy;
        let pal = Palette::new(ctx.style().visuals.dark_mode);

        egui::CentralPanel::default().frame(egui::Frame::none().fill(pal.bg)).show(ctx, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                // keep a readable column on wide windows
                let side = 16.0 + ((ui.available_width() - 32.0 - MAX_CONTENT_WIDTH) / 2.0).max(0.0);
                egui::Frame::none().inner_margin(egui::Margin { left: side, right: side, top: 12.0, bottom: 28.0 }).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.spacing_mut().item_spacing.y = 0.0;
                    self.header(ui, &pal, &s);
                    self.battery(ui, &pal, &s);
                    self.noise_control(ui, &pal, &s, live);
                    self.equalizer(ui, &pal, &s, live);
                    self.device_info(ui, &pal, &s);
                });
            });
        });
        // keep the "pending" state honest even when nothing else repaints
        if self.pending_mode.is_some() || self.pending_eq.is_some() {
            ctx.request_repaint_after(Duration::from_millis(250));
        }
    }
}

fn style_for(ctx: &egui::Context, theme: egui::Theme) {
    let pal = Palette::new(theme == egui::Theme::Dark);
    ctx.style_mut_of(theme, |style| {
        style.visuals.panel_fill = pal.bg;
        style.visuals.extreme_bg_color = pal.fill;
        style.visuals.selection.bg_fill = pal.tint.gamma_multiply(0.45);
        style.visuals.selection.stroke = Stroke::new(1.5_f32, pal.tint_text);
        style.visuals.text_cursor.stroke = Stroke::new(2.0_f32, pal.tint_text);
        style.spacing.scroll.floating = true;
    });
}
