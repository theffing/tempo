#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use eframe::egui::{self, Color32, RichText, Rounding};
use rodio::{Decoder, OutputStream, OutputStreamHandle, Sink, Source};
use std::{
    fs::File,
    io::BufReader,
    path::PathBuf,
    time::{Duration, Instant},
};

const ACCENT: Color32 = Color32::from_rgb(29, 185, 84);
const EXTS: [&str; 4] = ["mp3", "flac", "wav", "ogg"];
const W: f32 = 300.0;
const MIN_W: f32 = 220.0;
const H_COMPACT: f32 = 105.0;
const H_LIST: f32 = 320.0;

fn is_audio(p: &PathBuf) -> bool {
    p.extension()
        .map(|e| EXTS.iter().any(|x| e.eq_ignore_ascii_case(x)))
        .unwrap_or(false)
}

fn name_of(p: &PathBuf) -> String {
    p.file_stem().unwrap_or_default().to_string_lossy().into()
}

struct Player {
    _stream: OutputStream,
    handle: OutputStreamHandle,
    sink: Sink,
    playlist: Vec<PathBuf>,
    index: Option<usize>,
    volume: f32,
    total: Duration,
    offset: Duration,
    started: Option<Instant>,
    paused: bool,
    title_t0: f64,
    title_reset: bool,
    show_list: bool,
}

impl Player {
    fn new() -> Self {
        let (_stream, handle) = OutputStream::try_default().expect("no audio device");
        let sink = Sink::try_new(&handle).unwrap();
        Self {
            _stream,
            handle,
            sink,
            playlist: vec![],
            index: None,
            volume: 0.8,
            total: Duration::ZERO,
            offset: Duration::ZERO,
            started: None,
            paused: false,
            title_t0: 0.0,
            title_reset: true,
            show_list: false,
        }
    }

    fn pos(&self) -> Duration {
        let p = self.offset + self.started.map(|s| s.elapsed()).unwrap_or_default();
        p.min(self.total)
    }

    fn add_files(&mut self, paths: Vec<PathBuf>) {
        let start = self.playlist.len();
        self.playlist.extend(paths.into_iter().filter(is_audio));
        // Start playing the first new track if nothing is loaded
        if self.index.is_none() && start < self.playlist.len() {
            self.load(start);
        }
    }

    fn load(&mut self, i: usize) {
        let Some(path) = self.playlist.get(i).cloned() else { return };
        self.sink.stop();
        self.sink = Sink::try_new(&self.handle).unwrap();
        self.sink.set_volume(self.volume);

        let Ok(file) = File::open(&path) else { return };
        let src = match Decoder::new(BufReader::new(file)) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("decode error: {e}");
                return;
            }
        };

        self.total = mp3_duration::from_path(&path)
            .ok()
            .or_else(|| src.total_duration())
            .unwrap_or_default();
        self.sink.append(src);

        self.offset = Duration::ZERO;
        self.started = Some(Instant::now());
        self.paused = false;
        self.index = Some(i);
        self.title_reset = true;
    }

    fn stop(&mut self) {
        self.sink.stop();
        self.index = None;
        self.started = None;
        self.offset = Duration::ZERO;
        self.total = Duration::ZERO;
        self.paused = false;
        self.title_reset = true;
    }

    fn clear(&mut self) {
        self.stop();
        self.playlist.clear();
    }

    fn remove(&mut self, i: usize) {
        if i >= self.playlist.len() {
            return;
        }
        self.playlist.remove(i);
        match self.index {
            Some(c) if c == i => {
                if i < self.playlist.len() {
                    self.load(i);
                } else {
                    self.stop();
                }
            }
            Some(c) if c > i => self.index = Some(c - 1),
            _ => {}
        }
    }

    fn save_m3u(&self) {
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("Playlist", &["m3u"])
            .set_file_name("playlist.m3u")
            .save_file()
        {
            let mut s = String::from("#EXTM3U\n");
            for p in &self.playlist {
                s.push_str(&p.to_string_lossy());
                s.push('\n');
            }
            let _ = std::fs::write(path, s);
        }
    }

    fn load_m3u(&mut self) {
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("Playlist", &["m3u", "m3u8"])
            .pick_file()
        {
            let Ok(text) = std::fs::read_to_string(&path) else { return };
            let base = path.parent().map(|p| p.to_path_buf()).unwrap_or_default();
            let files: Vec<PathBuf> = text
                .lines()
                .map(str::trim)
                .filter(|l| !l.is_empty() && !l.starts_with('#'))
                .map(|l| {
                    let p = PathBuf::from(l);
                    if p.is_absolute() { p } else { base.join(p) }
                })
                .collect();
            self.clear();
            self.add_files(files);
        }
    }

    fn toggle(&mut self) {
        if self.sink.empty() {
            if let Some(i) = self.index {
                self.load(i);
            }
        } else if self.paused {
            self.sink.play();
            self.started = Some(Instant::now());
            self.paused = false;
        } else {
            self.offset = self.pos();
            self.sink.pause();
            self.started = None;
            self.paused = true;
        }
    }

    fn next(&mut self) {
        if let Some(i) = self.index {
            if i + 1 < self.playlist.len() {
                self.load(i + 1);
            }
        }
    }

    fn prev(&mut self) {
        if let Some(i) = self.index {
            // Restart the track if more than 3s in, otherwise go back
            if self.pos() > Duration::from_secs(3) || i == 0 {
                self.load(i);
            } else {
                self.load(i - 1);
            }
        }
    }

    fn seek(&mut self, to: Duration) {
        if self.sink.try_seek(to).is_ok() {
            self.offset = to;
            self.started = if self.paused { None } else { Some(Instant::now()) };
        }
    }

    fn title(&self) -> String {
        match self.index.and_then(|i| self.playlist.get(i)) {
            Some(p) => name_of(p),
            None => "Drop music here".into(),
        }
    }
}

fn fmt(d: Duration) -> String {
    let s = d.as_secs();
    format!("{}:{:02}", s / 60, s % 60)
}

fn small_button(text: &str) -> egui::Button<'static> {
    egui::Button::new(RichText::new(text.to_owned()).size(11.0).color(Color32::GRAY)).frame(false)
}

impl eframe::App for Player {
    fn clear_color(&self, _: &egui::Visuals) -> [f32; 4] {
        egui::Rgba::TRANSPARENT.to_array()
    }

    fn update(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        // Restart the title scroll whenever a new track loads
        if self.title_reset {
            self.title_t0 = ctx.input(|i| i.time);
            self.title_reset = false;
        }

        // Drag-and-drop: add all dropped audio files to the playlist
        let dropped: Vec<PathBuf> = ctx.input(|i| {
            i.raw.dropped_files.iter().filter_map(|f| f.path.clone()).collect()
        });
        if !dropped.is_empty() {
            self.add_files(dropped);
        }

        // Auto-advance when a track finishes
        if self.index.is_some() && self.sink.empty() && self.started.is_some() {
            if self.index.unwrap() + 1 < self.playlist.len() {
                self.next();
            } else {
                self.started = None;
                self.offset = Duration::ZERO;
                self.paused = true;
            }
        }

        let (mut do_prev, mut do_toggle, mut do_next) = (false, false, false);
        let mut seek_to: Option<Duration> = None;
        let mut vol_changed = false;
        let mut open_dialog = false;
        let mut scrolling = false;
        let mut toggle_list = false;
        let (mut do_save, mut do_load, mut do_clear) = (false, false, false);
        let mut play_idx: Option<usize> = None;
        let mut remove_idx: Option<usize> = None;

        let frame = egui::Frame::none()
            .fill(Color32::from_rgb(18, 18, 18))
            .rounding(Rounding::same(12.0))
            .inner_margin(12.0);

        egui::CentralPanel::default().frame(frame).show(ctx, |ui| {
            // Background drag area: added first, so every widget drawn after it
            // takes priority and only empty space moves the window
            let bg = ui.interact(
                ui.max_rect().expand(12.0),
                ui.id().with("bg_drag"),
                egui::Sense::drag(),
            );
            if bg.drag_started() {
                ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
            }

            // Title row: scrolling title, then List, 📂, - and X
            ui.horizontal(|ui| {
                let w = (ui.available_width() - 120.0).max(20.0);
                let (rect, r) =
                    ui.allocate_exact_size(egui::vec2(w, 18.0), egui::Sense::drag());
                if r.drag_started() {
                    ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
                }

                let galley = ui.painter().layout_no_wrap(
                    self.title(),
                    egui::FontId::proportional(13.0),
                    Color32::WHITE,
                );
                let painter = ui.painter().with_clip_rect(rect);
                let y = rect.center().y - galley.size().y / 2.0;
                let text_w = galley.size().x;

                if text_w <= rect.width() {
                    painter.galley(egui::pos2(rect.min.x, y), galley, Color32::WHITE);
                } else {
                    scrolling = true;
                    let gap = 40.0;
                    let cycle = text_w + gap;
                    let speed = 35.0; // pixels per second
                    let pause = 1.5; // seconds
                    let t = (ui.input(|i| i.time) - self.title_t0) as f32;
                    let period = pause + cycle / speed;
                    let phase = t % period;
                    let off = if phase < pause { 0.0 } else { (phase - pause) * speed };
                    painter.galley(
                        egui::pos2(rect.min.x - off, y),
                        galley.clone(),
                        Color32::WHITE,
                    );
                    painter.galley(
                        egui::pos2(rect.min.x - off + cycle, y),
                        galley,
                        Color32::WHITE,
                    );
                }

                let list_color = if self.show_list { ACCENT } else { Color32::GRAY };
                if ui
                    .add(
                        egui::Button::new(RichText::new("List").size(12.0).color(list_color))
                            .frame(false),
                    )
                    .clicked()
                {
                    toggle_list = true;
                }
                if ui.add(egui::Button::new("📂").frame(false)).clicked() {
                    open_dialog = true;
                }
                if ui.add(egui::Button::new("-").frame(false)).clicked() {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
                }
                if ui.add(egui::Button::new("X").frame(false)).clicked() {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            });

            // Seek bar
            let total = self.total.as_secs_f32().max(0.1);
            let mut secs = self.pos().as_secs_f32();
            ui.spacing_mut().slider_width = (ui.available_width() - 70.0).max(20.0);
            ui.horizontal(|ui| {
                let r = ui.add(egui::Slider::new(&mut secs, 0.0..=total).show_value(false));
                if r.drag_stopped() || (r.changed() && !r.dragged()) {
                    seek_to = Some(Duration::from_secs_f32(secs));
                }
                ui.label(
                    RichText::new(format!("{} / {}", fmt(self.pos()), fmt(self.total)))
                        .size(10.0)
                        .color(Color32::GRAY),
                );
            });

            // Controls
            ui.horizontal(|ui| {
                let playing = !self.paused && !self.sink.empty();
                let icon = if playing { "⏸" } else { "▶" };
                do_prev = ui.add(egui::Button::new("⏮").frame(false)).clicked();
                do_toggle = ui.add(egui::Button::new(icon).frame(false)).clicked();
                do_next = ui.add(egui::Button::new("⏭").frame(false)).clicked();

                ui.spacing_mut().slider_width = 80.0;
                vol_changed = ui
                    .add(egui::Slider::new(&mut self.volume, 0.0..=1.0).show_value(false))
                    .changed();
            });

            // Playlist panel
            if self.show_list {
                ui.separator();
                ui.horizontal(|ui| {
                    do_save = ui.add(small_button("Save")).clicked();
                    do_load = ui.add(small_button("Load")).clicked();
                    do_clear = ui.add(small_button("Clear")).clicked();
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            RichText::new(format!("{} tracks", self.playlist.len()))
                                .size(10.0)
                                .color(Color32::DARK_GRAY),
                        );
                    });
                });

                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        for (i, p) in self.playlist.iter().enumerate() {
                            ui.horizontal(|ui| {
                                let w = (ui.available_width() - 24.0).max(20.0);
                                let (rect, r) = ui
                                    .allocate_exact_size(egui::vec2(w, 16.0), egui::Sense::click());
                                let color = if self.index == Some(i) {
                                    ACCENT
                                } else if r.hovered() {
                                    Color32::WHITE
                                } else {
                                    Color32::LIGHT_GRAY
                                };
                                let galley = ui.painter().layout_no_wrap(
                                    name_of(p),
                                    egui::FontId::proportional(12.0),
                                    color,
                                );
                                let y = rect.center().y - galley.size().y / 2.0;
                                ui.painter()
                                    .with_clip_rect(rect)
                                    .galley(egui::pos2(rect.min.x, y), galley, color);
                                if r.clicked() {
                                    play_idx = Some(i);
                                }
                                if ui.add(small_button("x")).clicked() {
                                    remove_idx = Some(i);
                                }
                            });
                        }
                    });
            }
        });

        if do_prev { self.prev(); }
        if do_toggle { self.toggle(); }
        if do_next { self.next(); }
        if let Some(t) = seek_to { self.seek(t); }
        if vol_changed { self.sink.set_volume(self.volume); }

        if let Some(i) = play_idx { self.load(i); }
        if let Some(i) = remove_idx { self.remove(i); }
        if do_clear { self.clear(); }
        if do_save { self.save_m3u(); }
        if do_load { self.load_m3u(); }

        if toggle_list {
            self.show_list = !self.show_list;
            let h = if self.show_list { H_LIST } else { H_COMPACT };
            let w = ctx.screen_rect().width().max(MIN_W);
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(w, h)));
        }

        if open_dialog {
            if let Some(files) = rfd::FileDialog::new()
                .add_filter("Audio", &EXTS)
                .pick_files()
            {
                self.add_files(files);
            }
        }

        // Repaint faster only while the title is scrolling
        let ms = if scrolling { 33 } else { 250 };
        ctx.request_repaint_after(Duration::from_millis(ms));
    }
}

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([W, H_COMPACT])
            .with_min_inner_size([MIN_W, H_COMPACT])
            .with_decorations(false)
            .with_transparent(true)
            .with_resizable(true)
            .with_always_on_top(),
        ..Default::default()
    };

    eframe::run_native(
        "Tempo",
        options,
        Box::new(|cc| {
            let mut v = egui::Visuals::dark();
            v.selection.bg_fill = ACCENT;
            v.widgets.hovered.fg_stroke.color = ACCENT;
            cc.egui_ctx.set_visuals(v);
            Ok(Box::new(Player::new()))
        }),
    )
}