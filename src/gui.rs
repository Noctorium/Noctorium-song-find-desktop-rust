//! The window: Listen, History and Settings.
//!
//! Immediate mode, as Noctorium Stats is: every frame draws whatever the state is now. Listening and asking
//! happen on a thread of their own, which posts what it hears and what each provider says, and asks for a
//! repaint, so the window never waits on the microphone or the network.

pub mod theme;

use crate::audio::{self, Progress};
use crate::providers::{self, Answer, Clip, Provider};
use crate::settings::{self, Found, Places, Settings, Source};
use crate::song::{self, Song};
use crate::update::{self, Newer};
use eframe::egui::{self, RichText};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use std::time::Instant;
use theme::*;

const MARK: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/mark.rgba"));
include!(concat!(env!("OUT_DIR"), "/mark.rs"));

pub const TITLE: &str = "Noctorium Song Find";

pub fn run() -> Result<(), String> {
    let places = Places::system();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(TITLE)
            .with_app_id("noctorium-song-find")
            .with_inner_size([980.0, 720.0])
            .with_min_inner_size([620.0, 520.0])
            .with_drag_and_drop(true)
            .with_icon(egui::IconData {
                rgba: MARK.to_vec(),
                width: MARK_SIDE as u32,
                height: MARK_SIDE as u32,
            }),
        ..Default::default()
    };
    eframe::run_native(
        TITLE,
        options,
        Box::new(move |cc| {
            theme::install(&cc.egui_ctx);
            egui_extras::install_image_loaders(&cc.egui_ctx);
            Ok(Box::new(App::new(&cc.egui_ctx, places)))
        }),
    )
    .map_err(|e| e.to_string())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Page {
    Listen,
    History,
    Settings,
}

/// What the threads send back.
enum Event {
    Progress(Progress),
    Heard,
    Asking(Provider),
    Answered(Provider, Answer),
    Done(Result<Vec<(Provider, Answer)>, String>, String),
    Newer(Option<Newer>),
    Installed(Result<(), String>),
    Microphones(Vec<String>),
}

#[derive(Debug, Clone, PartialEq)]
enum Activity {
    Idle,
    Listening(Source),
    Reading(String),
    Asking(Provider),
}

#[derive(Debug, Clone, PartialEq)]
enum Updating {
    Nothing,
    Installing,
    /// In place, and ready at the next start.
    Ready,
    /// The installer will run once the window has closed.
    Handed,
    Failed(String),
}

struct App {
    ctx: egui::Context,
    places: Places,
    settings: Settings,
    saved: Settings,
    history: Vec<Found>,
    page: Page,
    tx: Sender<Event>,
    rx: Receiver<Event>,
    activity: Activity,
    stop: Arc<AtomicBool>,
    progress: Progress,
    /// What each provider said last time, in the order they were asked.
    answers: Vec<(Provider, Answer)>,
    /// The song shown on the Listen page.
    shown: Option<Song>,
    problem: Option<String>,
    newer: Option<Newer>,
    updating: Updating,
    microphones: Vec<String>,
    filter: String,
    open_entry: Option<usize>,
    confirm_clear: bool,
    toast: Option<(String, Instant)>,
    started: Instant,
    capture: Option<Capture>,
}

/// NOCTORIUM_SONG_FIND_CAPTURE=<folder>: draws each page, saves it as a PNG there, and closes. For looking
/// at the window without a person at it.
struct Capture {
    folder: std::path::PathBuf,
    pages: Vec<(Page, &'static str)>,
    frames: u32,
    asked: bool,
}

impl App {
    fn new(ctx: &egui::Context, places: Places) -> App {
        let settings = Settings::load(&places);
        let history = settings::load_history(&places);
        let (tx, rx) = channel();
        let app = App {
            ctx: ctx.clone(),
            saved: settings.clone(),
            settings,
            history,
            places,
            page: Page::Listen,
            tx,
            rx,
            activity: Activity::Idle,
            stop: Arc::new(AtomicBool::new(false)),
            progress: Progress::default(),
            answers: Vec::new(),
            shown: None,
            problem: None,
            newer: None,
            updating: Updating::Nothing,
            microphones: Vec::new(),
            filter: String::new(),
            open_entry: None,
            confirm_clear: false,
            toast: None,
            started: Instant::now(),
            capture: std::env::var_os("NOCTORIUM_SONG_FIND_CAPTURE").map(|folder| Capture {
                folder: folder.into(),
                pages: vec![
                    (Page::Listen, "listen"),
                    (Page::History, "history"),
                    (Page::Settings, "settings"),
                ],
                frames: 0,
                asked: false,
            }),
        };
        if app.settings.check_for_updates {
            app.spawn(|tx| {
                let _ = tx.send(Event::Newer(update::check()));
            });
        }
        app.spawn(|tx| {
            let _ = tx.send(Event::Microphones(audio::microphones()));
        });
        app
    }

    fn spawn(&self, work: impl FnOnce(&Sender<Event>) + Send + 'static) {
        let tx = self.tx.clone();
        let ctx = self.ctx.clone();
        std::thread::spawn(move || {
            work(&tx);
            ctx.request_repaint();
        });
    }

    fn busy(&self) -> bool {
        self.activity != Activity::Idle
    }

    fn listen(&mut self, source: Source) {
        if self.busy() {
            return;
        }
        self.settings.source = source;
        self.activity = Activity::Listening(source);
        self.answers.clear();
        self.problem = None;
        self.progress = Progress::default();
        self.stop = Arc::new(AtomicBool::new(false));
        let stop = self.stop.clone();
        let settings = self.settings.clone();
        let ctx = self.ctx.clone();
        self.spawn(move |tx| {
            let progress_tx = tx.clone();
            let recorded = audio::record(
                source,
                settings.microphone.as_deref(),
                settings.seconds,
                stop,
                |p| {
                    let _ = progress_tx.send(Event::Progress(p));
                    ctx.request_repaint();
                },
            );
            let _ = tx.send(Event::Heard);
            match recorded {
                Ok(clip) => ask(tx, &ctx, clip, &settings, source.name().to_string()),
                Err(why) => {
                    let _ = tx.send(Event::Done(Err(why), source.name().into()));
                }
            }
        });
    }

    fn identify_file(&mut self, path: std::path::PathBuf) {
        if self.busy() {
            return;
        }
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "the file".into());
        self.page = Page::Listen;
        self.activity = Activity::Reading(name.clone());
        self.answers.clear();
        self.problem = None;
        let settings = self.settings.clone();
        let ctx = self.ctx.clone();
        self.spawn(move |tx| match audio::decode_file(&path) {
            Ok(clip) => ask(tx, &ctx, clip, &settings, format!("File: {name}")),
            Err(why) => {
                let _ = tx.send(Event::Done(Err(why), name));
            }
        });
    }

    fn take_events(&mut self) {
        while let Ok(event) = self.rx.try_recv() {
            match event {
                Event::Progress(p) => self.progress = p,
                Event::Heard => self.progress.level = 0.0,
                Event::Asking(provider) => self.activity = Activity::Asking(provider),
                Event::Answered(provider, answer) => self.answers.push((provider, answer)),
                Event::Done(result, source) => {
                    self.activity = Activity::Idle;
                    match result {
                        Err(why) => self.problem = Some(why),
                        Ok(answers) => self.finish(answers, source),
                    }
                }
                Event::Newer(newer) => {
                    self.newer = newer;
                    if let Some(newer) = &self.newer {
                        if cfg!(windows) && self.settings.install_updates && newer.package.is_some()
                        {
                            self.install_update();
                        }
                    }
                }
                Event::Installed(result) => {
                    self.updating = match result {
                        Ok(()) if cfg!(windows) => Updating::Ready,
                        Ok(()) => Updating::Handed,
                        Err(why) => Updating::Failed(why),
                    };
                    if self.updating == Updating::Handed {
                        // The installer waits a few seconds for this window to close, then replaces it.
                        self.ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                }
                Event::Microphones(list) => self.microphones = list,
            }
        }
    }

    fn finish(&mut self, answers: Vec<(Provider, Answer)>, source: String) {
        self.answers = answers;
        let found: Vec<Song> = self
            .answers
            .iter()
            .filter_map(|(_, a)| match a {
                Answer::Found(song) => Some(song.clone()),
                _ => None,
            })
            .collect();
        self.shown = found.first().cloned();
        if found.is_empty() {
            let failures: Vec<&str> = self
                .answers
                .iter()
                .filter_map(|(_, a)| match a {
                    Answer::Failed(why) => Some(why.as_str()),
                    _ => None,
                })
                .collect();
            self.problem = Some(if self.answers.is_empty() {
                "No provider is switched on, or none has what it needs. See Settings › Providers."
                    .into()
            } else if failures.len() == self.answers.len() {
                failures.join("\n")
            } else {
                "No song was found. Try again closer to the sound, or for longer.".into()
            });
            return;
        }
        if self.settings.keep_history {
            let mut seen: Vec<(String, String)> = Vec::new();
            for song in found {
                let key = (song.title.to_lowercase(), song.artist.to_lowercase());
                if seen.contains(&key) {
                    continue;
                }
                seen.push(key);
                self.history.insert(
                    0,
                    Found {
                        song,
                        at: chrono::Utc::now(),
                        source: source.clone(),
                    },
                );
            }
            self.history.truncate(settings::HISTORY_MOST);
            self.save_history();
        }
    }

    fn save_history(&mut self) {
        if let Err(why) = settings::save_history(&self.places, &self.history) {
            self.toast(format!("The history could not be saved: {why}"));
        }
    }

    fn toast(&mut self, text: impl Into<String>) {
        self.toast = Some((text.into(), Instant::now()));
    }

    fn install_update(&mut self) {
        let Some(newer) = self.newer.clone() else {
            return;
        };
        self.updating = Updating::Installing;
        self.spawn(move |tx| {
            let _ = tx.send(Event::Installed(update::install(&newer)));
        });
    }
}

/// Asks the providers about a clip, telling the window as each is asked and answers.
fn ask(tx: &Sender<Event>, ctx: &egui::Context, clip: Clip, settings: &Settings, source: String) {
    if clip.samples.is_empty() || clip.level() < 0.002 {
        let why = if source == Source::System.name() {
            "Nothing was playing. Play the song on this computer, then listen again."
        } else {
            "Nothing was heard. Move closer to the sound, or check the microphone in Settings."
        };
        let _ = tx.send(Event::Done(Err(why.into()), source));
        return;
    }
    let answers = providers::identify(&clip, settings, |provider, answer| {
        let _ = match answer {
            None => tx.send(Event::Asking(provider)),
            Some(answer) => tx.send(Event::Answered(provider, answer.clone())),
        };
        ctx.request_repaint();
    });
    let _ = tx.send(Event::Done(Ok(answers), source));
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.take_events();

        let dropped: Vec<std::path::PathBuf> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .filter_map(|f| f.path.clone())
                .collect()
        });
        if let Some(path) = dropped.into_iter().next() {
            self.identify_file(path);
        }
        if self.page == Page::Listen
            && !ctx.wants_keyboard_input()
            && ctx.input(|i| i.key_pressed(egui::Key::Space))
        {
            if self.busy() {
                self.stop.store(true, Ordering::Relaxed);
            } else {
                self.listen(self.settings.source);
            }
        }

        egui::TopBottomPanel::top("bar")
            .frame(
                egui::Frame::new()
                    .fill(PANEL)
                    .inner_margin(egui::Margin::symmetric(18, 10)),
            )
            .show(ctx, |ui| self.bar(ui));

        if let Some(banner) = self.banner() {
            egui::TopBottomPanel::top("update")
                .frame(
                    egui::Frame::new()
                        .fill(CARD)
                        .inner_margin(egui::Margin::symmetric(18, 8)),
                )
                .show(ctx, |ui| self.update_banner(ui, banner));
        }

        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(PAGE)
                    .inner_margin(egui::Margin::same(22)),
            )
            .show(ctx, |ui| {
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| match self.page {
                        Page::Listen => self.listen_page(ui),
                        Page::History => self.history_page(ui),
                        Page::Settings => self.settings_page(ui),
                    });
            });

        if let Some((text, when)) = &self.toast {
            if when.elapsed().as_secs_f32() > 3.5 {
                self.toast = None;
            } else {
                egui::Area::new(egui::Id::new("toast"))
                    .anchor(egui::Align2::CENTER_BOTTOM, egui::vec2(0.0, -24.0))
                    .show(ctx, |ui| {
                        card()
                            .inner_margin(egui::Margin::symmetric(16, 10))
                            .show(ui, |ui| {
                                ui.label(RichText::new(text).color(TEXT));
                            });
                    });
                ctx.request_repaint_after(std::time::Duration::from_millis(250));
            }
        }

        if self.settings != self.saved {
            self.settings.tidy();
            if let Err(why) = self.settings.save(&self.places) {
                self.toast(format!("Settings could not be saved: {why}"));
            }
            self.saved = self.settings.clone();
        }
        if self.busy() {
            ctx.request_repaint_after(std::time::Duration::from_millis(33));
        }
        self.capture_step(ctx);
    }
}

impl App {
    fn capture_step(&mut self, ctx: &egui::Context) {
        let Some(capture) = &mut self.capture else {
            return;
        };
        let Some(&(page, name)) = capture.pages.first() else {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        };
        self.page = page;
        capture.frames += 1;
        ctx.request_repaint();
        if capture.frames < 30 {
            return;
        }
        if !capture.asked {
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
            capture.asked = true;
            return;
        }
        let shot = ctx.input(|i| {
            i.raw.events.iter().find_map(|e| match e {
                egui::Event::Screenshot { image, .. } => Some(image.clone()),
                _ => None,
            })
        });
        if let Some(image) = shot {
            let _ = std::fs::create_dir_all(&capture.folder);
            let path = capture.folder.join(format!("{name}.png"));
            let bytes: Vec<u8> = image.pixels.iter().flat_map(|p| p.to_array()).collect();
            let _ = image::save_buffer(
                &path,
                &bytes,
                image.size[0] as u32,
                image.size[1] as u32,
                image::ColorType::Rgba8,
            );
            capture.pages.remove(0);
            capture.frames = 0;
            capture.asked = false;
        }
    }
}

// ------------------------------------------------------------------------------------------------ the bar

impl App {
    fn bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let texture = ui
                .ctx()
                .data_mut(|d| d.get_temp::<egui::TextureHandle>(egui::Id::new("mark")));
            let texture = texture.unwrap_or_else(|| {
                let image = egui::ColorImage::from_rgba_unmultiplied([MARK_SIDE, MARK_SIDE], MARK);
                let handle = ui
                    .ctx()
                    .load_texture("mark", image, egui::TextureOptions::LINEAR);
                ui.ctx()
                    .data_mut(|d| d.insert_temp(egui::Id::new("mark"), handle.clone()));
                handle
            });
            ui.add(egui::Image::new(&texture).fit_to_exact_size(egui::vec2(30.0, 30.0)));
            ui.add_space(4.0);
            ui.label(RichText::new("Song Find").font(bold(18.0)).color(TEXT));
            ui.add_space(18.0);
            let pages = [Page::Listen, Page::History, Page::Settings];
            let chosen = pages.iter().position(|p| *p == self.page).unwrap_or(0);
            if let Some(i) = segmented(ui, &["Listen", "History", "Settings"], chosen, 92.0) {
                self.page = pages[i];
            }
        });
    }

    fn banner(&self) -> Option<String> {
        let newer = self.newer.as_ref()?;
        Some(match &self.updating {
            Updating::Nothing => format!("Noctorium Song Find {} is out.", newer.version),
            Updating::Installing => format!("Installing {}…", newer.version),
            Updating::Ready => format!(
                "Version {} is installed. It starts the next time Song Find does.",
                newer.version
            ),
            Updating::Handed => format!("Installing {} once this window closes…", newer.version),
            Updating::Failed(why) => format!("{} could not be installed: {why}", newer.version),
        })
    }

    fn update_banner(&mut self, ui: &mut egui::Ui, text: String) {
        ui.horizontal(|ui| {
            ui.label(RichText::new(text).color(TEXT));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if let Some(newer) = self.newer.clone() {
                    link(ui, "What's new", &newer.url);
                    match self.updating {
                        Updating::Nothing | Updating::Failed(_) => {
                            let can = (cfg!(windows) && newer.package.is_some())
                                || (!cfg!(windows) && newer.installer.is_some());
                            if can && secondary(ui, "Install").clicked() {
                                self.install_update();
                            }
                        }
                        Updating::Ready if secondary(ui, "Restart now").clicked() => {
                            match update::relaunch() {
                                Ok(()) => ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close),
                                Err(why) => self.toast(why),
                            }
                        }
                        _ => {}
                    }
                }
            });
        });
    }
}

// --------------------------------------------------------------------------------------------- Listen

impl App {
    fn listen_page(&mut self, ui: &mut egui::Ui) {
        ui.vertical_centered(|ui| {
            ui.add_space(6.0);
            let sources = [Source::Microphone, Source::System];
            let chosen = sources
                .iter()
                .position(|s| *s == self.settings.source)
                .unwrap_or(0);
            ui.add_enabled_ui(!self.busy(), |ui| {
                if let Some(i) = segmented(ui, &["Microphone", "System audio"], chosen, 150.0) {
                    self.settings.source = sources[i];
                }
            });
            ui.add_space(8.0);
            ui.label(
                RichText::new(match self.settings.source {
                    Source::Microphone => {
                        "Listens through the microphone, to a song playing in the room."
                    }
                    Source::System => {
                        "Listens to what this computer is playing: a video, a stream, another app."
                    }
                })
                .color(FAINT),
            );
            ui.add_space(18.0);
            self.big_button(ui);
            ui.add_space(14.0);
            ui.label(
                RichText::new(self.status())
                    .font(semibold(16.0))
                    .color(TEXT),
            );
            if !self.busy() {
                ui.label(
                    RichText::new("Space to listen · or drop an audio file onto the window")
                        .font(regular(12.5))
                        .color(FAINT),
                );
            }
            ui.add_space(18.0);
        });

        if let Some(problem) = self.problem.clone() {
            card().show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(RichText::new(problem).color(WARNING));
            });
            ui.add_space(GAP);
        }
        if !self.busy() {
            if let Some(song) = self.shown.clone() {
                song_card(ui, &song, &mut self.toast);
                ui.add_space(GAP);
            }
        }
        if !self.answers.is_empty() {
            card().show(ui, |ui| {
                ui.set_width(ui.available_width());
                card_title(ui, "What each provider said", None);
                let answers = self.answers.clone();
                for (provider, answer) in answers {
                    ui.horizontal(|ui| {
                        let (colour, text) = match &answer {
                            Answer::Found(song) => {
                                (ACCENT, format!("{} — {}", song.artist, song.title))
                            }
                            Answer::Unknown => (FAINT, "did not know it".into()),
                            Answer::Failed(why) => (ERROR, why.clone()),
                        };
                        dot(ui, colour, 14.0);
                        ui.label(
                            RichText::new(provider.name())
                                .font(semibold(14.0))
                                .color(TEXT),
                        );
                        let response =
                            ui.add(egui::Label::new(RichText::new(text).color(SUBTEXT)).wrap());
                        if let Answer::Found(song) = &answer {
                            if response
                                .interact(egui::Sense::click())
                                .on_hover_text("Show this one")
                                .clicked()
                            {
                                self.shown = Some(song.clone());
                            }
                        }
                    });
                }
            });
        }
    }

    fn status(&self) -> String {
        match &self.activity {
            Activity::Idle if self.shown.is_some() && self.problem.is_none() => "Found it".into(),
            Activity::Idle => "Tap to find the song".into(),
            Activity::Listening(source) => {
                let left = (self.settings.seconds as f32 * (1.0 - self.progress.fraction)).ceil();
                format!(
                    "Listening to the {}… {left:.0}s",
                    if *source == Source::System {
                        "computer"
                    } else {
                        "room"
                    }
                )
            }
            Activity::Reading(name) => format!("Reading {name}…"),
            Activity::Asking(provider) => format!("Asking {}…", provider.name()),
        }
    }

    fn big_button(&mut self, ui: &mut egui::Ui) {
        let size = 210.0;
        let (rect, response) = ui.allocate_exact_size(egui::vec2(size, size), egui::Sense::click());
        let painter = ui.painter_at(rect.expand(30.0));
        let centre = rect.center();
        let t = self.started.elapsed().as_secs_f32();
        let busy = self.busy();
        glow(
            &painter,
            centre,
            egui::vec2(size * 0.62, size * 0.62),
            if busy { 0.55 } else { 0.32 },
        );
        if busy {
            let level = self.progress.level.sqrt();
            for ring in 0..3 {
                let phase = (t * 0.7 + ring as f32 / 3.0).fract();
                let radius = 70.0 + phase * (30.0 + 25.0 * level);
                painter.circle_stroke(
                    centre,
                    radius,
                    stroke(2.0, ACCENT.gamma_multiply((1.0 - phase) * 0.8)),
                );
            }
            if let Activity::Listening(_) = self.activity {
                let arc = self.progress.fraction * std::f32::consts::TAU;
                let points: Vec<egui::Pos2> = (0..=64)
                    .map(|i| {
                        let a = -std::f32::consts::FRAC_PI_2 + arc * i as f32 / 64.0;
                        centre + egui::vec2(a.cos(), a.sin()) * 76.0
                    })
                    .collect();
                painter.add(egui::Shape::line(points, stroke(4.0, ACCENT_BRIGHT)));
            }
        }
        let hovered = response.hovered();
        let fill = if hovered { ACCENT_BRIGHT } else { ACCENT };
        painter.circle_filled(centre, 66.0, fill);
        // The mark of listening: a few bars, moving with the sound while it is heard.
        let bars = 5;
        for i in 0..bars {
            let x = centre.x + (i as f32 - 2.0) * 14.0;
            let wave = if busy {
                let l = self.progress.level.sqrt().max(0.15);
                (0.35 + 0.65 * ((t * 7.0 + i as f32 * 1.3).sin() * 0.5 + 0.5)) * l
            } else {
                [0.45, 0.75, 1.0, 0.75, 0.45][i]
            };
            let h = 12.0 + 34.0 * wave;
            painter.rect_filled(
                egui::Rect::from_center_size(egui::pos2(x, centre.y), egui::vec2(8.0, h)),
                egui::CornerRadius::same(4),
                ON_ACCENT,
            );
        }
        let response = response
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .on_hover_text(if busy {
                "Stop listening and ask now"
            } else {
                "Listen"
            });
        if response.clicked() {
            if busy {
                self.stop.store(true, Ordering::Relaxed);
            } else {
                self.listen(self.settings.source);
            }
        }
    }
}

/// A song, with its cover, what is known of it, and where to hear it.
fn song_card(ui: &mut egui::Ui, song: &Song, toast: &mut Option<(String, Instant)>) {
    card().show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal_top(|ui| {
            let side = 150.0;
            match &song.cover {
                Some(url) => {
                    ui.add(
                        egui::Image::new(url.as_str())
                            .fit_to_exact_size(egui::vec2(side, side))
                            .corner_radius(egui::CornerRadius::same(12)),
                    );
                }
                None => {
                    let (rect, _) =
                        ui.allocate_exact_size(egui::vec2(side, side), egui::Sense::hover());
                    ui.painter()
                        .rect_filled(rect, egui::CornerRadius::same(12), WELL);
                    ui.painter().text(
                        rect.center(),
                        egui::Align2::CENTER_CENTER,
                        "♪",
                        regular(52.0),
                        FAINT,
                    );
                }
            }
            ui.add_space(14.0);
            ui.vertical(|ui| {
                ui.add(
                    egui::Label::new(RichText::new(&song.title).font(bold(24.0)).color(TEXT))
                        .wrap(),
                );
                ui.label(
                    RichText::new(&song.artist)
                        .font(semibold(17.0))
                        .color(ACCENT_BRIGHT),
                );
                ui.add_space(6.0);
                let mut facts = Vec::new();
                if let Some(album) = &song.album {
                    facts.push(album.clone());
                }
                if let Some(year) = &song.year {
                    facts.push(year.clone());
                }
                if let Some(genre) = &song.genre {
                    facts.push(genre.clone());
                }
                if let Some(label) = &song.label {
                    facts.push(label.clone());
                }
                if !facts.is_empty() {
                    ui.add(
                        egui::Label::new(RichText::new(facts.join(" · ")).color(SUBTEXT)).wrap(),
                    );
                }
                if !song.provider.is_empty() {
                    ui.label(
                        RichText::new(format!("Found by {}", song.provider))
                            .font(regular(12.5))
                            .color(FAINT),
                    );
                }
                ui.add_space(10.0);
                ui.horizontal_wrapped(|ui| {
                    for (name, url) in song::links(song) {
                        if secondary(ui, name).on_hover_text(&url).clicked() {
                            ui.ctx().open_url(egui::OpenUrl::new_tab(url));
                        }
                    }
                    if secondary(ui, "Copy")
                        .on_hover_text("Copy “artist – title”")
                        .clicked()
                    {
                        ui.ctx()
                            .copy_text(format!("{} – {}", song.artist, song.title));
                        *toast = Some(("Copied".into(), Instant::now()));
                    }
                });
            });
        });
    });
}

// -------------------------------------------------------------------------------------------- History

impl App {
    fn history_page(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(RichText::new("History").font(bold(24.0)).color(TEXT));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if !self.history.is_empty() {
                    if self.confirm_clear {
                        if secondary(ui, "Keep it").clicked() {
                            self.confirm_clear = false;
                        }
                        if primary(ui, "Clear everything", 150.0, true).clicked() {
                            self.history.clear();
                            self.confirm_clear = false;
                            self.open_entry = None;
                            self.save_history();
                        }
                    } else if secondary(ui, "Clear history").clicked() {
                        self.confirm_clear = true;
                    }
                }
            });
        });
        ui.label(
            RichText::new(
                "Every song found, on this computer only. Nothing here is sent anywhere.",
            )
            .color(FAINT),
        );
        ui.add_space(10.0);
        if self.history.is_empty() {
            card().show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(
                    RichText::new("Nothing found yet. The songs you find show up here.")
                        .color(SUBTEXT),
                );
            });
            return;
        }
        ui.add(
            egui::TextEdit::singleline(&mut self.filter)
                .hint_text("Search the history")
                .desired_width(f32::INFINITY),
        );
        ui.add_space(10.0);
        let needle = self.filter.to_lowercase();
        let mut remove = None;
        for index in 0..self.history.len() {
            let entry = self.history[index].clone();
            let song = &entry.song;
            if !needle.is_empty()
                && !format!(
                    "{} {} {}",
                    song.title,
                    song.artist,
                    song.album.clone().unwrap_or_default()
                )
                .to_lowercase()
                .contains(&needle)
            {
                continue;
            }
            let open = self.open_entry == Some(index);
            if open {
                song_card(ui, song, &mut self.toast);
                ui.horizontal(|ui| {
                    if secondary(ui, "Close").clicked() {
                        self.open_entry = None;
                    }
                    if secondary(ui, "Remove from history").clicked() {
                        remove = Some(index);
                    }
                });
                ui.add_space(8.0);
                continue;
            }
            let response = card()
                .inner_margin(egui::Margin::symmetric(14, 10))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        match &song.cover {
                            Some(url) => {
                                ui.add(
                                    egui::Image::new(url.as_str())
                                        .fit_to_exact_size(egui::vec2(48.0, 48.0))
                                        .corner_radius(egui::CornerRadius::same(8)),
                                );
                            }
                            None => {
                                let (rect, _) = ui.allocate_exact_size(
                                    egui::vec2(48.0, 48.0),
                                    egui::Sense::hover(),
                                );
                                ui.painter()
                                    .rect_filled(rect, egui::CornerRadius::same(8), WELL);
                                ui.painter().text(
                                    rect.center(),
                                    egui::Align2::CENTER_CENTER,
                                    "♪",
                                    regular(20.0),
                                    FAINT,
                                );
                            }
                        }
                        ui.vertical(|ui| {
                            ui.label(RichText::new(&song.title).font(semibold(15.0)).color(TEXT));
                            ui.label(RichText::new(&song.artist).color(SUBTEXT));
                        });
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let when = entry
                                .at
                                .with_timezone(&chrono::Local)
                                .format("%e %b %Y, %H:%M");
                            ui.label(
                                RichText::new(format!(
                                    "{when}\n{} · {}",
                                    entry.source, song.provider
                                ))
                                .font(regular(12.0))
                                .color(FAINT),
                            );
                        });
                    });
                })
                .response
                .interact(egui::Sense::click())
                .on_hover_cursor(egui::CursorIcon::PointingHand);
            if response.clicked() {
                self.open_entry = Some(index);
            }
            ui.add_space(6.0);
        }
        if let Some(index) = remove {
            self.history.remove(index);
            self.open_entry = None;
            self.save_history();
        }
    }
}

// ------------------------------------------------------------------------------------------- Settings

impl App {
    fn settings_page(&mut self, ui: &mut egui::Ui) {
        ui.label(RichText::new("Settings").font(bold(24.0)).color(TEXT));
        ui.add_space(10.0);

        card().show(ui, |ui| {
            ui.set_width(ui.available_width());
            card_title(ui, "Listening", None);
            ui.horizontal(|ui| {
                ui.label(RichText::new("Listen for").color(SUBTEXT));
                let chosen = settings::SECONDS
                    .iter()
                    .position(|s| *s == self.settings.seconds)
                    .unwrap_or(2);
                let labels: Vec<String> = settings::SECONDS.iter().map(|s| format!("{s} s")).collect();
                let labels: Vec<&str> = labels.iter().map(String::as_str).collect();
                if let Some(i) = segmented(ui, &labels, chosen, 60.0) {
                    self.settings.seconds = settings::SECONDS[i];
                }
            });
            ui.label(RichText::new("Longer is surer in a noisy room; ten seconds is plenty for most songs.").font(regular(12.5)).color(FAINT));
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new("Microphone").color(SUBTEXT));
                let current = self.settings.microphone.clone().unwrap_or_else(|| "The system's default".into());
                egui::ComboBox::from_id_salt("microphone")
                    .selected_text(current)
                    .width(320.0)
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut self.settings.microphone, None, "The system's default");
                        for name in self.microphones.clone() {
                            ui.selectable_value(&mut self.settings.microphone, Some(name.clone()), name);
                        }
                    });
                if secondary(ui, "Look again").clicked() {
                    self.spawn(|tx| {
                        let _ = tx.send(Event::Microphones(audio::microphones()));
                    });
                }
            });
            ui.add_space(6.0);
            ui.label(
                RichText::new(if cfg!(windows) {
                    "System audio records whatever plays through the default output, by Windows' own loopback."
                } else if cfg!(target_os = "macos") {
                    "System audio needs macOS 14.2 or later, and asks for permission the first time."
                } else {
                    "System audio records the monitor of the default output, through PulseAudio or PipeWire."
                })
                .font(regular(12.5))
                .color(FAINT),
            );
        });
        ui.add_space(GAP);

        card().show(ui, |ui| {
            ui.set_width(ui.available_width());
            card_title(ui, "Providers", Some("asked from the top down"));
            ui.checkbox(
                &mut self.settings.ask_everyone,
                "Ask every provider, not only until one knows the song",
            );
            ui.add_space(8.0);
            let count = self.settings.providers.len();
            let mut swap = None;
            for index in 0..count {
                let (provider, _) = self.settings.providers[index];
                egui::Frame::new()
                    .fill(WELL)
                    .corner_radius(egui::CornerRadius::same(12))
                    .inner_margin(egui::Margin::same(12))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.horizontal(|ui| {
                            ui.checkbox(&mut self.settings.providers[index].1, "");
                            ui.label(RichText::new(provider.name()).font(semibold(15.0)).color(TEXT));
                            if let Err(why) = provider.ready(&self.settings) {
                                ui.label(RichText::new(why).font(regular(12.5)).color(WARNING));
                            }
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if ui.add_enabled(index + 1 < count, egui::Button::new("↓")).on_hover_text("Ask later").clicked() {
                                    swap = Some((index, index + 1));
                                }
                                if ui.add_enabled(index > 0, egui::Button::new("↑")).on_hover_text("Ask sooner").clicked() {
                                    swap = Some((index, index - 1));
                                }
                            });
                        });
                        ui.add(egui::Label::new(RichText::new(provider.about()).font(regular(12.5)).color(FAINT)).wrap());
                        let field = |ui: &mut egui::Ui, label: &str, value: &mut String, secret: bool, hint: &str| {
                            ui.horizontal(|ui| {
                                ui.add_sized([120.0, 24.0], egui::Label::new(RichText::new(label).color(SUBTEXT)));
                                ui.add(
                                    egui::TextEdit::singleline(value)
                                        .password(secret)
                                        .hint_text(hint)
                                        .desired_width(f32::INFINITY),
                                );
                            });
                        };
                        match provider {
                            Provider::Shazam => {}
                            Provider::AudD => {
                                ui.add_space(4.0);
                                field(ui, "API token", &mut self.settings.audd_token, true, "optional");
                                link(ui, "Get a token at audd.io", "https://dashboard.audd.io/");
                            }
                            Provider::AcrCloud => {
                                ui.add_space(4.0);
                                field(ui, "Host", &mut self.settings.acrcloud_host, false, "identify-eu-west-1.acrcloud.com");
                                field(ui, "Access key", &mut self.settings.acrcloud_key, false, "");
                                field(ui, "Access secret", &mut self.settings.acrcloud_secret, true, "");
                                link(ui, "Make a project at console.acrcloud.com", "https://console.acrcloud.com/");
                            }
                            Provider::AcoustId => {
                                ui.add_space(4.0);
                                field(ui, "Application key", &mut self.settings.acoustid_key, true, "");
                                link(ui, "Register an application at acoustid.org", "https://acoustid.org/new-application");
                            }
                        }
                    });
                ui.add_space(6.0);
            }
            if let Some((a, b)) = swap {
                self.settings.providers.swap(a, b);
            }
            ui.label(
                RichText::new("Keys are kept in this computer's settings file, in plain text, and sent only to their own provider.")
                    .font(regular(12.5))
                    .color(FAINT),
            );
        });
        ui.add_space(GAP);

        card().show(ui, |ui| {
            ui.set_width(ui.available_width());
            card_title(
                ui,
                "History",
                Some(&format!("{} songs", self.history.len())),
            );
            ui.checkbox(
                &mut self.settings.keep_history,
                "Keep a history of the songs found",
            );
            ui.label(
                RichText::new(format!(
                    "Kept on this computer only, in {}",
                    self.places.history().display()
                ))
                .font(regular(12.5))
                .color(FAINT),
            );
        });
        ui.add_space(GAP);

        card().show(ui, |ui| {
            ui.set_width(ui.available_width());
            card_title(
                ui,
                "Updates",
                Some(&format!("this is version {}", crate::version())),
            );
            ui.checkbox(
                &mut self.settings.check_for_updates,
                "Check for a newer release when Song Find starts",
            );
            ui.add_enabled_ui(self.settings.check_for_updates, |ui| {
                ui.checkbox(
                    &mut self.settings.install_updates,
                    if cfg!(windows) {
                        "Install it by itself, ready the next time Song Find starts"
                    } else {
                        "Offer to install it"
                    },
                );
            });
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if secondary(ui, "Check now").clicked() {
                    self.newer = None;
                    self.updating = Updating::Nothing;
                    let ctx = self.ctx.clone();
                    self.spawn(move |tx| {
                        let newer = update::check();
                        if newer.is_none() {
                            ctx.data_mut(|d| {
                                d.insert_temp(egui::Id::new("up-to-date"), Instant::now())
                            });
                        }
                        let _ = tx.send(Event::Newer(newer));
                    });
                }
                let told = ui
                    .ctx()
                    .data(|d| d.get_temp::<Instant>(egui::Id::new("up-to-date")));
                if told.is_some_and(|t| t.elapsed().as_secs() < 8) && self.newer.is_none() {
                    ui.label(RichText::new("This is the newest.").color(SUBTEXT));
                }
                link(ui, "Releases", update::RELEASES);
            });
        });
        ui.add_space(GAP);

        card().show(ui, |ui| {
            ui.set_width(ui.available_width());
            card_title(ui, "About", None);
            ui.label(RichText::new(format!("Noctorium Song Find {}", crate::version())).color(TEXT));
            ui.label(
                RichText::new("Part of Noctorium. Free software under the GNU GPL, version 3. Shazam's fingerprint is made as SongRec worked it out.")
                    .color(SUBTEXT),
            );
            ui.horizontal(|ui| {
                link(ui, "Source", "https://github.com/Noctorium/Noctorium-song-find-desktop-rust");
                link(ui, "Website", "https://noctorium.vercel.app");
            });
        });
    }
}
