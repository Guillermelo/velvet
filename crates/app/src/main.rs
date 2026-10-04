use eframe::egui::{self, Color32, FontId, Pos2, Rect, Sense, Stroke, Vec2};
use std::{
    path::PathBuf,
    sync::{
        mpsc::{self, Receiver},
        Arc,
    },
    time::{Duration, Instant},
};
use velvet_audio::{AudioData, MediaCache, Mix, Player};
use velvet_core::{Command, Effect, Position, Project, Session, Source, SourceKind};
mod browser;
mod rack;

const BG: Color32 = Color32::from_rgb(23, 25, 28);
const PANEL: Color32 = Color32::from_rgb(30, 32, 36);
const LINE: Color32 = Color32::from_rgb(47, 50, 55);
const TEXT: Color32 = Color32::from_rgb(218, 221, 225);
const MUTED: Color32 = Color32::from_rgb(133, 140, 149);
const ACCENT: Color32 = Color32::from_rgb(165, 182, 154);
enum Job {
    Warm(anyhow::Result<MediaCache>),
    Import {
        track: String,
        path: PathBuf,
        beats: f64,
        data: anyhow::Result<Arc<AudioData>>,
    },
    Mix(anyhow::Result<Mix>),
    Export(anyhow::Result<PathBuf>),
    Ai {
        session: Box<Session>,
        result: anyhow::Result<velvet_ai::Answer>,
    },
}
#[derive(Clone)]
struct Drag {
    track: String,
    clip: String,
    position: Position,
    mode: u8,
    origin: Pos2,
}
struct Velvet {
    session: Session,
    saved: Project,
    selected_track: Option<String>,
    selected_clip: Option<String>,
    cache: MediaCache,
    player: Option<Player>,
    job: Option<Receiver<Job>>,
    busy: String,
    ai_open: bool,
    prompt: String,
    ai_log: Vec<(bool, String)>,
    status: String,
    error: bool,
    zoom: f32,
    snap: bool,
    drag: Option<Drag>,
    pending_new: Option<Option<PathBuf>>,
    closing: bool,
    screenshot: Option<PathBuf>,
    capture_requested: bool,
    gesture: Option<(Project, u64)>,
    live_job: Option<Receiver<(u64, anyhow::Result<Mix>)>>,
    live_dirty: bool,
    edit_time: Instant,
    timeline_scroll: f32,
    search: String,
    browser_category: u8,
    browser_folder: Option<PathBuf>,
    browser_files: Vec<PathBuf>,
    samples_root: Option<PathBuf>,
    grid_start_beats: f64,
}
fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1440.0, 900.0])
            .with_min_inner_size([1000.0, 650.0])
            .with_title("Velvet · Audio workstation"),
        ..Default::default()
    };
    eframe::run_native(
        "Velvet",
        options,
        Box::new(|cc| Ok(Box::new(Velvet::new(&cc.egui_ctx)))),
    )
}
impl Velvet {
    fn new(ctx: &egui::Context) -> Self {
        let mut style = (*ctx.style()).clone();
        style.visuals = egui::Visuals::dark();
        style.visuals.panel_fill = PANEL;
        style.visuals.window_fill = PANEL;
        style.visuals.extreme_bg_color = BG;
        style.visuals.override_text_color = Some(TEXT);
        style.visuals.selection.bg_fill = Color32::from_rgb(65, 76, 68);
        style.visuals.selection.stroke = Stroke::new(1.0_f32, ACCENT);
        style.visuals.widgets.inactive.bg_fill = Color32::from_rgb(40, 43, 48);
        style.visuals.widgets.inactive.bg_stroke = Stroke::new(1.0_f32, LINE);
        style.visuals.widgets.hovered.bg_fill = Color32::from_rgb(53, 57, 61);
        style.visuals.widgets.active.bg_fill = Color32::from_rgb(67, 76, 66);
        style.spacing.item_spacing = Vec2::new(8.0, 6.0);
        style.spacing.button_padding = Vec2::new(9.0, 5.0);
        style
            .text_styles
            .insert(egui::TextStyle::Body, FontId::proportional(12.0));
        style
            .text_styles
            .insert(egui::TextStyle::Button, FontId::proportional(12.0));
        style
            .text_styles
            .insert(egui::TextStyle::Small, FontId::proportional(10.0));
        ctx.set_style(style);
        let loaded = std::env::args_os()
            .nth(1)
            .map(PathBuf::from)
            .map(|p| Session::open(&p));
        let (session, status, error) = match loaded {
            Some(Ok(s)) => (s, "Project opened".into(), false),
            Some(Err(e)) => (
                Session::new(Project::new("Untitled"), PathBuf::new()),
                e.to_string(),
                true,
            ),
            None => (
                Session::new(Project::new("Untitled"), PathBuf::new()),
                "Ready · Drop WAV or FLAC files to begin".into(),
                false,
            ),
        };
        let selected_track = session.project.tracks.first().map(|t| t.id.clone());
        let mut app = Self {
            saved: session.project.clone(),
            session,
            selected_track,
            selected_clip: None,
            cache: MediaCache::default(),
            player: None,
            job: None,
            busy: String::new(),
            ai_open: false,
            prompt: String::new(),
            ai_log: vec![],
            status,
            error,
            zoom: 30.0,
            snap: true,
            drag: None,
            pending_new: None,
            closing: false,
            screenshot: std::env::var_os("VELVET_SCREENSHOT").map(PathBuf::from),
            capture_requested: false,
            gesture: None,
            live_job: None,
            live_dirty: false,
            edit_time: Instant::now(),
            timeline_scroll: 0.0,
            search: String::new(),
            browser_category: 0,
            browser_folder: None,
            browser_files: vec![],
            samples_root: None,
            grid_start_beats: 0.0,
        };
        app.restore_samples_folder();
        if !app.session.project.tracks.is_empty() {
            app.warm();
        }
        app
    }
    fn report(&mut self, result: anyhow::Result<()>, success: &str) {
        match result {
            Ok(()) => {
                self.status = success.into();
                self.error = false;
            }
            Err(e) => {
                self.status = format!("{e:#}");
                self.error = true;
            }
        }
    }
    fn execute(&mut self, command: Command) {
        let revision = self.session.revision;
        let seek = match &command {
            Command::Seek { seconds } => Some(*seconds),
            Command::Stop => Some(0.0),
            _ => None,
        };
        match self.session.execute(command) {
            Ok(Effect::Transport) => {
                if let Some(seconds) = seek {
                    self.grid_start_beats = seconds * self.session.project.tempo.bpm / 60.0;
                }
                self.transport();
            }
            Ok(Effect::Saved) => {
                self.saved = self.session.project.clone();
                self.status = "Project saved".into();
                self.error = false;
            }
            Ok(Effect::Render(path)) => self.export(path),
            Ok(Effect::None) => {
                if self.session.revision != revision {
                    self.invalidate();
                    self.status = "Edit applied · Undo available".into();
                    self.error = false;
                }
            }
            Err(e) => {
                self.status = e.to_string();
                self.error = true;
            }
        }
    }
    fn invalidate(&mut self) {
        self.live_dirty = self.player.is_some();
        self.edit_time = Instant::now();
    }
    fn reset_playback(&mut self) {
        self.player = None;
        self.live_job = None;
        self.live_dirty = false;
        self.session.transport.playing = false;
    }
    fn update_live_mix(&mut self) {
        if let Some(p) = &mut self.player {
            p.maintain();
        }
        if let Some(rx) = &self.live_job {
            match rx.try_recv() {
                Ok((revision, result)) => {
                    self.live_job = None;
                    if revision == self.session.revision {
                        match result {
                            Ok(mix) => {
                                if let Some(p) = &mut self.player {
                                    if let Err(e) = p.replace_mix(Arc::new(mix)) {
                                        self.report(Err(e), "");
                                    }
                                }
                            }
                            Err(e) => self.report(Err(e), ""),
                        }
                    }
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.live_job = None;
                    self.report(
                        Err(anyhow::anyhow!(
                            "Live mix update failed; playback continues with the last mix"
                        )),
                        "",
                    );
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
        if self.live_dirty
            && self.live_job.is_none()
            && self.edit_time.elapsed() >= Duration::from_millis(30)
        {
            if let Some(player) = &self.player {
                let rate = player.sample_rate;
                let revision = self.session.revision;
                let project = self.session.project.clone();
                let root = self.session.root.clone();
                let mut cache = MediaCache {
                    files: self.cache.files.clone(),
                };
                let (tx, rx) = mpsc::channel();
                self.live_job = Some(rx);
                self.live_dirty = false;
                // ponytail: whole-mix rebuild; use incremental track buses
                // when larger projects make edit latency noticeable.
                std::thread::spawn(move || {
                    let _ = tx.send((
                        revision,
                        velvet_audio::mix(&project, &root, rate, &mut cache),
                    ));
                });
            }
        }
    }
    fn history(&mut self, redo: bool) {
        let changed = if redo {
            self.session.redo()
        } else {
            self.session.undo()
        };
        if changed {
            self.invalidate();
            self.status = if redo { "Edit restored" } else { "Edit undone" }.into();
        }
    }
    fn transport(&mut self) {
        if self.session.transport.playing {
            if let Some(p) = &self.player {
                p.seek(self.session.transport.seconds);
                p.play();
            } else {
                let rate = match Player::output_rate() {
                    Ok(r) => r,
                    Err(e) => {
                        self.report(Err(e), "");
                        self.session.transport.playing = false;
                        return;
                    }
                };
                let project = self.session.project.clone();
                let root = self.session.root.clone();
                let mut cache = MediaCache::default();
                cache.files = self.cache.files.clone();
                self.start("Preparing audio", move || {
                    Job::Mix(velvet_audio::mix(&project, &root, rate, &mut cache))
                });
            }
        } else if let Some(p) = &self.player {
            p.pause();
            p.seek(self.session.transport.seconds);
        }
    }
    fn toggle_playback(&mut self) {
        if self.session.transport.playing {
            self.execute(Command::Pause);
            self.execute(Command::Seek {
                seconds: self.grid_start_beats * 60.0 / self.session.project.tempo.bpm,
            });
        } else {
            self.execute(Command::Seek {
                seconds: self.grid_start_beats * 60.0 / self.session.project.tempo.bpm,
            });
            self.execute(Command::Play);
        }
    }
    fn mark_grid(&mut self, beats: f64) {
        let beats = if self.snap {
            (beats * 4.0).round() / 4.0
        } else {
            beats
        };
        self.execute(Command::Seek {
            seconds: beats.max(0.0) * 60.0 / self.session.project.tempo.bpm,
        });
    }
    fn start(&mut self, label: &str, work: impl FnOnce() -> Job + Send + 'static) {
        let (tx, rx) = mpsc::channel();
        self.job = Some(rx);
        self.busy = label.into();
        std::thread::spawn(move || {
            let _ = tx.send(work());
        });
    }
    fn warm(&mut self) {
        let project = self.session.project.clone();
        let root = self.session.root.clone();
        self.start("Reading waveforms", move || {
            let mut cache = MediaCache::default();
            for c in project.tracks.iter().flat_map(|t| &t.clips) {
                let p = project.source_path(&root, &c.source);
                if p.is_file() {
                    if let Err(e) = cache.get(&p) {
                        return Job::Warm(Err(e));
                    }
                }
            }
            Job::Warm(Ok(cache))
        });
    }
    fn poll(&mut self) {
        let message = self.job.as_ref().map(|r| r.try_recv());
        let job = match message {
            Some(Ok(j)) => j,
            Some(Err(mpsc::TryRecvError::Disconnected)) => {
                self.job = None;
                self.report(
                    Err(anyhow::anyhow!("Background task ended unexpectedly")),
                    "",
                );
                return;
            }
            _ => return,
        };
        self.job = None;
        self.busy.clear();
        match job {
            Job::Warm(r) => match r {
                Ok(cache) => self.cache = cache,
                Err(e) => self.report(Err(e), ""),
            },
            Job::Import {
                track,
                path,
                beats,
                data,
            } => match data {
                Ok(data) => {
                    let length = data.duration();
                    self.cache.files.insert(path.clone(), data);
                    self.execute(Command::ImportAudioClip {
                        track_id: track.clone(),
                        source: Source {
                            path,
                            kind: SourceKind::External,
                        },
                        position: Position {
                            start_beats: beats,
                            offset_seconds: 0.0,
                            length_seconds: length,
                        },
                    });
                    self.selected_track = Some(track.clone());
                    self.selected_clip = self
                        .session
                        .project
                        .track(&track)
                        .ok()
                        .and_then(|t| t.clips.last().map(|c| c.id.clone()));
                }
                Err(e) => self.report(Err(e), ""),
            },
            Job::Mix(r) => match r.and_then(|m| {
                if !m.missing.is_empty() {
                    self.status = format!("{} missing source(s) are silent", m.missing.len());
                }
                Player::new(Arc::new(m), self.session.transport.seconds)
            }) {
                Ok(p) => {
                    if self.session.transport.playing {
                        p.play();
                    }
                    self.player = Some(p);
                }
                Err(e) => {
                    self.session.transport.playing = false;
                    self.report(Err(e), "");
                }
            },
            Job::Export(r) => match r {
                Ok(p) => {
                    self.status = format!("Exported {}", p.display());
                    self.error = false;
                }
                Err(e) => self.report(Err(e), ""),
            },
            Job::Ai { session, result } => {
                let changed = session.project != self.session.project;
                let runtime = self
                    .player
                    .as_ref()
                    .map(|p| velvet_core::Transport {
                        playing: p.playing(),
                        seconds: p.seconds(),
                    })
                    .unwrap_or_else(|| self.session.transport.clone());
                let transport_requested = result
                    .as_ref()
                    .map(|a| a.actions.iter().any(|s| s.starts_with("transport_")))
                    .unwrap_or(false);
                self.session = *session;
                if !transport_requested {
                    self.session.transport = runtime;
                }
                if changed {
                    self.invalidate();
                }
                match result {
                    Ok(a) => {
                        self.ai_log.push((false, a.text));
                        for action in a.actions {
                            self.ai_log.push((false, format!("↳ {action}")));
                        }
                    }
                    Err(e) => self.ai_log.push((false, e.to_string())),
                }
                if transport_requested {
                    self.transport();
                }
            }
        }
    }
    fn import(&mut self, path: PathBuf, track: Option<String>, beats: f64) {
        if self.job.is_some() {
            return;
        }
        let track = track
            .or_else(|| self.selected_track.clone())
            .filter(|id| self.session.project.track(id).is_ok())
            .unwrap_or_else(|| {
                let name = path
                    .file_stem()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into();
                self.execute(Command::AddTrack { name });
                self.session.project.tracks.last().unwrap().id.clone()
            });
        self.start("Importing audio", move || {
            let result = (|| -> anyhow::Result<(PathBuf, Arc<AudioData>)> {
                let path = path.canonicalize()?;
                let data = Arc::new(velvet_audio::decode(&path)?);
                Ok((path, data))
            })();
            match result {
                Ok((path, data)) => Job::Import {
                    track,
                    path,
                    beats,
                    data: Ok(data),
                },
                Err(e) => Job::Import {
                    track,
                    path,
                    beats,
                    data: Err(e),
                },
            }
        });
    }
    fn export(&mut self, path: PathBuf) {
        let project = self.session.project.clone();
        let root = self.session.root.clone();
        let mut cache = MediaCache::default();
        cache.files = self.cache.files.clone();
        self.start("Rendering WAV", move || {
            Job::Export((|| {
                let m = velvet_audio::mix(&project, &root, project.audio.sample_rate, &mut cache)?;
                velvet_audio::export(&m, &path)?;
                Ok(path)
            })())
        });
    }
    fn save(&mut self, choose: bool) {
        if choose
            || self.session.root.as_os_str().is_empty()
            || !self.session.root.join("project.yaml").exists()
        {
            if let Some(root) = rfd::FileDialog::new()
                .set_title("Choose project folder")
                .pick_folder()
            {
                if root != self.session.root && root.join("project.yaml").exists() {
                    self.report(
                        Err(anyhow::anyhow!(
                            "That folder already contains a project. Choose a new folder."
                        )),
                        "",
                    );
                    return;
                }
                let old = self.session.root.clone();
                let mut project = self.session.project.clone();
                for c in project.tracks.iter_mut().flat_map(|t| &mut t.clips) {
                    if c.source.kind == SourceKind::Project && root != old {
                        c.source = Source {
                            path: std::path::absolute(old.join(&c.source.path))
                                .unwrap_or_else(|_| old.join(&c.source.path)),
                            kind: SourceKind::External,
                        };
                    }
                }
                match velvet_core::save(&root, &project) {
                    Ok(()) => {
                        self.reset_playback();
                        self.session = Session::new(project, root);
                        self.saved = self.session.project.clone();
                        self.status = "Project saved".into();
                        self.error = false;
                    }
                    Err(e) => self.report(Err(e), ""),
                }
            }
        } else {
            self.execute(Command::SaveProject);
        }
    }
    fn request_open(&mut self, path: Option<PathBuf>) {
        if self.session.project != self.saved {
            self.pending_new = Some(path);
        } else {
            self.replace(path);
        }
    }
    fn replace(&mut self, path: Option<PathBuf>) {
        let result = match path {
            Some(p) => Session::open(&p),
            None => Ok(Session::new(Project::new("Untitled"), PathBuf::new())),
        };
        match result {
            Ok(s) => {
                self.reset_playback();
                self.session = s;
                self.saved = self.session.project.clone();
                self.selected_track = self.session.project.tracks.first().map(|t| t.id.clone());
                self.selected_clip = None;
                self.timeline_scroll = 0.0;
                self.grid_start_beats = 0.0;
                self.ai_log.clear();
                self.cache = MediaCache::default();
                self.status = "Ready".into();
                self.error = false;
                if !self.session.project.tracks.is_empty() {
                    self.warm();
                }
            }
            Err(e) => self.report(Err(e), ""),
        }
    }
    fn ask(&mut self) {
        if self.prompt.trim().is_empty() || self.job.is_some() {
            return;
        }
        let prompt = std::mem::take(&mut self.prompt);
        self.ai_log.push((true, prompt.clone()));
        let mut session = self.session.clone();
        self.start("Velvet AI is working", move || {
            let result = velvet_ai::Agent::from_env().and_then(|a| a.ask(&mut session, &prompt));
            Job::Ai {
                session: Box::new(session),
                result,
            }
        });
    }
    fn toolbar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("toolbar")
            .exact_height(54.0)
            .frame(egui::Frame::new().fill(PANEL).inner_margin(12.0))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("velvet").size(21.0).color(TEXT));
                    ui.add_space(15.0);
                    ui.add_enabled_ui(self.job.is_none(), |ui| {
                        ui.menu_button("Project", |ui| {
                            let key = egui::Id::new("project_name");
                            let mut name = ui
                                .data_mut(|d| d.get_temp::<String>(key))
                                .unwrap_or_else(|| self.session.project.project.name.clone());
                            ui.text_edit_singleline(&mut name);
                            ui.data_mut(|d| d.insert_temp(key, name.clone()));
                            if ui.button("Rename project").clicked() {
                                self.execute(Command::RenameProject { name });
                                ui.close_menu();
                            }
                            ui.separator();
                            if ui.button("New       Ctrl+N").clicked() {
                                self.request_open(None);
                                ui.close_menu();
                            }
                            if ui.button("Open…    Ctrl+O").clicked() {
                                if let Some(p) = rfd::FileDialog::new().pick_folder() {
                                    self.request_open(Some(p));
                                }
                                ui.close_menu();
                            }
                            if ui.button("Save       Ctrl+S").clicked() {
                                self.save(false);
                                ui.close_menu();
                            }
                            if ui.button("Save to folder…").clicked() {
                                self.save(true);
                                ui.close_menu();
                            }
                            ui.separator();
                            if ui.button("Export WAV…").clicked() {
                                if let Some(path) = rfd::FileDialog::new()
                                    .set_directory(self.session.root.join("renders"))
                                    .set_file_name("mix.wav")
                                    .add_filter("WAV", &["wav"])
                                    .save_file()
                                {
                                    self.execute(Command::RenderProject { path });
                                }
                                ui.close_menu();
                            }
                        });
                        if ui
                            .add_enabled(
                                !self.session.history.is_empty(),
                                egui::Button::new("Undo"),
                            )
                            .on_hover_text("Undo · Ctrl+Z")
                            .clicked()
                        {
                            self.history(false);
                        }
                        if ui
                            .add_enabled(
                                !self.session.redo_history.is_empty(),
                                egui::Button::new("Redo"),
                            )
                            .on_hover_text("Redo · Ctrl+Shift+Z")
                            .clicked()
                        {
                            self.history(true);
                        }
                        ui.add_space(10.0);
                        if ui
                            .selectable_label(self.session.transport.playing, "▶")
                            .on_hover_text("Play / return to grid marker · Space")
                            .clicked()
                        {
                            self.toggle_playback();
                        }
                        if ui.button("■").on_hover_text("Stop").clicked() {
                            self.execute(Command::Stop);
                        }
                        ui.add_space(10.0);
                        let mut bpm = self.session.project.tempo.bpm;
                        let r = ui.add(
                            egui::DragValue::new(&mut bpm)
                                .speed(0.1)
                                .range(20.0..=400.0)
                                .fixed_decimals(2)
                                .suffix(" BPM"),
                        );
                        if r.changed() {
                            self.execute(Command::SetTempo { bpm });
                        }
                        ui.label(egui::RichText::new("4 / 4").color(MUTED));
                        ui.add_space(10.0);
                        ui.monospace(format!(
                            "{:02}:{:06.3}",
                            (self.session.transport.seconds / 60.0) as u32,
                            self.session.transport.seconds % 60.0
                        ));
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.toggle_value(&mut self.ai_open, "Velvet AI");
                        ui.label(
                            egui::RichText::new(format!(
                                "{} kHz",
                                self.session.project.audio.sample_rate / 1000
                            ))
                            .color(MUTED),
                        );
                    });
                });
            });
    }
    fn ai(&mut self, ctx: &egui::Context) {
        if !self.ai_open {
            return;
        }
        egui::SidePanel::right("ai").default_width(280.0).width_range(240.0..=420.0).frame(egui::Frame::new().fill(PANEL).inner_margin(16.0)).show(ctx,|ui|{
            ui.add_space(6.0);eyebrow(ui,"VELVET AI");ui.add_space(5.0);ui.label(egui::RichText::new("A quieter way to work.").color(MUTED));ui.add_space(16.0);ui.separator();
            let configured=std::env::var("OPENAI_API_KEY").map(|k|!k.trim().is_empty()).unwrap_or(false);
            if !configured{ui.add_space(10.0);ui.label("Connect your OpenAI key");ui.label(egui::RichText::new("Set OPENAI_API_KEY before launching Velvet. All manual editing remains available.").small().color(MUTED));}
            let h=(ui.available_height()-120.0).max(60.0);
            egui::ScrollArea::vertical().max_height(h).stick_to_bottom(true).show(ui,|ui|{
                if self.ai_log.is_empty(){ui.add_space(30.0);for text in ["“Lower the vocals by 3 dB”","“Add an EQ to the bass”","“Which tracks are muted?”"]{ui.label(egui::RichText::new(text).color(MUTED));ui.add_space(15.0);}}
                for (user,text) in &self.ai_log {ui.add_space(12.0);ui.label(egui::RichText::new(if *user{"YOU"}else{"VELVET"}).small().color(if *user{MUTED}else{ACCENT}));ui.label(text);ui.add_space(8.0);ui.separator();}
            });
            ui.with_layout(egui::Layout::bottom_up(egui::Align::LEFT),|ui|{
                ui.label(egui::RichText::new("Changes are visible and undoable.").small().color(MUTED));
                ui.add_enabled_ui(configured&&self.job.is_none(),|ui|{
                    if ui.add_sized([ui.available_width(),28.0],egui::Button::new("Send  ↗")).clicked(){self.ask();}
                    let r=ui.add(egui::TextEdit::multiline(&mut self.prompt).hint_text("Ask Velvet…").desired_rows(2).desired_width(f32::INFINITY));
                    if r.has_focus()&&ui.input(|i|i.modifiers.ctrl&&i.key_pressed(egui::Key::Enter)){self.ask();}
                });
            });
        });
    }
    fn inspector(&mut self, ui: &mut egui::Ui) {
        if let (Some(tid), Some(cid)) = (self.selected_track.clone(), self.selected_clip.clone()) {
            if let Some(c) = self
                .session
                .project
                .track(&tid)
                .ok()
                .and_then(|t| t.clips.iter().find(|c| c.id == cid))
                .cloned()
            {
                ui.horizontal_wrapped(|ui| {
                    ui.label(egui::RichText::new("CLIP").small().color(MUTED));
                    let mut beats = c.position.start_beats;
                    let mut offset = c.position.offset_seconds;
                    let mut length = c.position.length_seconds;
                    let rb = ui.add(
                        egui::DragValue::new(&mut beats)
                            .range(0.0..=14400.0)
                            .speed(0.25)
                            .prefix("Beat ")
                            .fixed_decimals(2),
                    );
                    let ro = ui.add(
                        egui::DragValue::new(&mut offset)
                            .range(0.0..=1800.0)
                            .speed(0.01)
                            .prefix("Offset ")
                            .suffix(" s"),
                    );
                    let rl = ui.add(
                        egui::DragValue::new(&mut length)
                            .range(0.01..=1800.0)
                            .speed(0.01)
                            .prefix("Length ")
                            .suffix(" s"),
                    );
                    if rb.changed() {
                        self.execute(Command::MoveClip {
                            track_id: tid.clone(),
                            clip_id: cid.clone(),
                            start_beats: beats,
                        });
                    }
                    if ro.changed() || rl.changed() {
                        self.execute(Command::TrimClip {
                            track_id: tid.clone(),
                            clip_id: cid.clone(),
                            offset_seconds: offset,
                            length_seconds: length,
                        });
                    }
                    let missing = !self
                        .session
                        .project
                        .source_path(&self.session.root, &c.source)
                        .is_file();
                    if ui
                        .button(if missing {
                            "Locate missing…"
                        } else {
                            "Relink…"
                        })
                        .on_hover_text(c.source.path.display().to_string())
                        .clicked()
                    {
                        if let Some(path) = rfd::FileDialog::new()
                            .add_filter("Audio", &["wav", "flac"])
                            .pick_file()
                        {
                            let r = (|| {
                                let path = path.canonicalize()?;
                                velvet_audio::decode(&path)?;
                                Ok(path)
                            })();
                            match r {
                                Ok(path) => {
                                    self.execute(Command::RelinkClip {
                                        track_id: tid.clone(),
                                        clip_id: cid.clone(),
                                        source: Source {
                                            path,
                                            kind: SourceKind::External,
                                        },
                                    });
                                    self.warm();
                                }
                                Err(e) => self.report(Err(e), ""),
                            }
                        }
                    }
                    if ui.button("Delete").clicked() {
                        self.execute(Command::RemoveClip {
                            track_id: tid,
                            clip_id: cid,
                        });
                        self.selected_clip = None;
                    }
                });
            }
        }
    }
    fn arrangement(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(BG).inner_margin(0.0))
            .show(ctx, |ui| {
                ui.add_enabled_ui(self.job.is_none(), |ui| {
                    egui::Frame::new().inner_margin(12.0).show(ui, |ui| {
                        ui.horizontal(|ui| {
                            eyebrow(ui, "ARRANGEMENT");
                            ui.label(
                                egui::RichText::new(format!(
                                    "/  {}{}",
                                    self.session.project.project.name,
                                    if self.session.project != self.saved {
                                        " *"
                                    } else {
                                        ""
                                    }
                                ))
                                .color(MUTED),
                            );
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    if ui.small_button("+ Track").clicked() {
                                        let n = self.session.project.tracks.len() + 1;
                                        self.execute(Command::AddTrack {
                                            name: format!("Audio {n}"),
                                        });
                                        self.selected_track = self
                                            .session
                                            .project
                                            .tracks
                                            .last()
                                            .map(|t| t.id.clone());
                                    }
                                    ui.checkbox(&mut self.snap, "Snap");
                                    ui.add(
                                        egui::Slider::new(&mut self.zoom, 8.0..=90.0)
                                            .show_value(false)
                                            .text("Zoom"),
                                    );
                                },
                            );
                        });
                        self.inspector(ui);
                        let total = (self.session.project.duration_seconds()
                            * self.session.project.tempo.bpm
                            / 60.0
                            + 8.0)
                            .clamp(32.0, 20000.0) as f32;
                        let visible = ((ui.available_width() - 194.0) / self.zoom).max(1.0);
                        let limit = (total - visible).max(0.0);
                        self.timeline_scroll = self.timeline_scroll.min(limit);
                        if limit > 0.0 {
                            ui.horizontal(|ui| {
                                ui.label(egui::RichText::new("View").small().color(MUTED));
                                ui.spacing_mut().slider_width =
                                    (ui.available_width() - 30.0).max(50.0);
                                ui.add(
                                    egui::Slider::new(&mut self.timeline_scroll, 0.0..=limit)
                                        .show_value(false),
                                )
                                .on_hover_text("Scroll the timeline; track controls stay fixed");
                            });
                        }
                    });
                    ui.separator();
                    let tracks = self.session.project.tracks.clone();
                    let beat_seconds = 60.0 / self.session.project.tempo.bpm;
                    let total_beats = (self.session.project.duration_seconds() / beat_seconds
                        + 8.0)
                        .clamp(32.0, 20000.0);
                    let header = 194.0;
                    egui::ScrollArea::vertical()
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            let width = ui.available_width();
                            let (ruler, r) =
                                ui.allocate_exact_size(Vec2::new(width, 30.0), Sense::click());
                            let controls_x = ruler.right() - header;
                            let timeline_x = ruler.left() - self.timeline_scroll * self.zoom;
                            ui.painter().rect_filled(ruler, 0.0, PANEL);
                            ui.painter().text(
                                Pos2::new(controls_x + 12.0, ruler.center().y),
                                egui::Align2::LEFT_CENTER,
                                "TRACK / MIXER",
                                FontId::proportional(10.0),
                                MUTED,
                            );
                            let painter = ui.painter().with_clip_rect(Rect::from_min_max(
                                ruler.min,
                                Pos2::new(controls_x, ruler.bottom()),
                            ));
                            for beat in 0..=(total_beats as usize) {
                                let x = timeline_x + beat as f32 * self.zoom;
                                if beat % 4 == 0 {
                                    painter.text(
                                        Pos2::new(x + 5.0, ruler.center().y),
                                        egui::Align2::LEFT_CENTER,
                                        format!("{}", beat / 4 + 1),
                                        FontId::monospace(10.0),
                                        MUTED,
                                    );
                                }
                            }
                            if r.clicked() {
                                if let Some(p) = r.interact_pointer_pos() {
                                    if p.x < controls_x {
                                        self.mark_grid(((p.x - timeline_x) / self.zoom) as f64);
                                    }
                                }
                            }
                            let mut drop_target = None;
                            let hover = ctx.input(|i| i.pointer.hover_pos());
                            for (index, t) in tracks.iter().enumerate() {
                                let (row, _) =
                                    ui.allocate_exact_size(Vec2::new(width, 108.0), Sense::hover());
                                let timeline = Rect::from_min_max(
                                    row.min,
                                    Pos2::new(controls_x, row.bottom()),
                                );
                                let selected = self.selected_track.as_deref() == Some(&t.id);
                                let p = ui.painter();
                                p.rect_filled(
                                    row,
                                    0.0,
                                    if index % 2 == 0 {
                                        Color32::from_rgb(26, 28, 31)
                                    } else {
                                        BG
                                    },
                                );
                                p.rect_filled(
                                    Rect::from_min_max(Pos2::new(controls_x, row.top()), row.max),
                                    0.0,
                                    if selected {
                                        Color32::from_rgb(40, 45, 43)
                                    } else {
                                        PANEL
                                    },
                                );
                                p.line_segment(
                                    [Pos2::new(row.left(), row.bottom()), row.right_bottom()],
                                    Stroke::new(1.0_f32, LINE),
                                );
                                let color = Color32::from_rgb(t.color[0], t.color[1], t.color[2]);
                                p.rect_filled(
                                    Rect::from_min_size(
                                        Pos2::new(controls_x, row.top()),
                                        Vec2::new(3.0, row.height()),
                                    ),
                                    0.0,
                                    color,
                                );
                                for beat in 0..=(total_beats as usize) {
                                    let x = timeline_x + beat as f32 * self.zoom;
                                    p.with_clip_rect(timeline).line_segment(
                                        [Pos2::new(x, row.top()), Pos2::new(x, row.bottom())],
                                        Stroke::new(
                                            1.0_f32,
                                            if beat % 4 == 0 {
                                                LINE
                                            } else {
                                                Color32::from_rgb(33, 35, 39)
                                            },
                                        ),
                                    );
                                }
                                let header_rect = Rect::from_min_max(
                                    Pos2::new(controls_x + 12.0, row.top() + 8.0),
                                    row.max - Vec2::new(10.0, 6.0),
                                );
                                ui.scope_builder(
                                    egui::UiBuilder::new().max_rect(header_rect),
                                    |ui| {
                                        ui.horizontal(|ui| {
                                            ui.label(
                                                egui::RichText::new(format!("{:02}", index + 1))
                                                    .small()
                                                    .color(MUTED),
                                            );
                                            if ui.selectable_label(selected, &t.name).clicked() {
                                                self.selected_track = Some(t.id.clone());
                                                self.selected_clip = None;
                                            }
                                        });
                                        ui.horizontal(|ui| {
                                            if ui.selectable_label(t.mixer.mute, "M").clicked() {
                                                self.execute(Command::SetMute {
                                                    track_id: t.id.clone(),
                                                    mute: !t.mixer.mute,
                                                });
                                            }
                                            if ui.selectable_label(t.mixer.solo, "S").clicked() {
                                                self.execute(Command::SetSolo {
                                                    track_id: t.id.clone(),
                                                    solo: !t.mixer.solo,
                                                });
                                            }
                                            let mut volume = t.mixer.volume_db;
                                            let r = ui.add(
                                                egui::DragValue::new(&mut volume)
                                                    .range(-90.0..=12.0)
                                                    .speed(0.2)
                                                    .suffix(" dB")
                                                    .fixed_decimals(1),
                                            );
                                            if r.changed() {
                                                self.execute(Command::SetTrackVolume {
                                                    track_id: t.id.clone(),
                                                    volume_db: volume,
                                                });
                                            }
                                        });
                                        ui.horizontal(|ui| {
                                            let mut pan = t.mixer.pan;
                                            let r = ui.add(
                                                egui::DragValue::new(&mut pan)
                                                    .range(-1.0..=1.0)
                                                    .speed(0.01)
                                                    .prefix("Pan ")
                                                    .fixed_decimals(2),
                                            );
                                            if r.changed() {
                                                self.execute(Command::SetTrackPan {
                                                    track_id: t.id.clone(),
                                                    pan,
                                                });
                                            }
                                            ui.menu_button("···", |ui| {
                                                let key = egui::Id::new(("rename", &t.id));
                                                let mut name = ui
                                                    .data_mut(|d| d.get_temp::<String>(key))
                                                    .unwrap_or_else(|| t.name.clone());
                                                ui.text_edit_singleline(&mut name);
                                                ui.data_mut(|d| d.insert_temp(key, name.clone()));
                                                if ui.button("Rename").clicked() {
                                                    self.execute(Command::RenameTrack {
                                                        track_id: t.id.clone(),
                                                        name,
                                                    });
                                                    ui.close_menu();
                                                }
                                                let mut color = t.color;
                                                if ui.color_edit_button_srgb(&mut color).changed() {
                                                    self.execute(Command::SetTrackColor {
                                                        track_id: t.id.clone(),
                                                        color,
                                                    });
                                                }
                                                if ui.button("Delete track").clicked() {
                                                    self.execute(Command::RemoveTrack {
                                                        track_id: t.id.clone(),
                                                    });
                                                    if selected {
                                                        self.selected_track = None;
                                                        self.selected_clip = None;
                                                    }
                                                    ui.close_menu();
                                                }
                                            });
                                        });
                                    },
                                );
                                if let Some(pos) = hover {
                                    if timeline.contains(pos) {
                                        drop_target = Some((
                                            t.id.clone(),
                                            ((pos.x - timeline_x) / self.zoom).max(0.0) as f64,
                                        ));
                                    }
                                }
                                let grid = ui.interact(
                                    timeline,
                                    egui::Id::new(("grid", &t.id)),
                                    Sense::click(),
                                );
                                if grid.clicked() {
                                    if let Some(pos) = grid.interact_pointer_pos() {
                                        self.mark_grid(((pos.x - timeline_x) / self.zoom) as f64);
                                        self.selected_track = Some(t.id.clone());
                                        self.selected_clip = None;
                                    }
                                }
                                for c in &t.clips {
                                    let mut position = c.position.clone();
                                    let x = timeline_x + position.start_beats as f32 * self.zoom;
                                    let rect = Rect::from_min_size(
                                        Pos2::new(x, row.top() + 9.0),
                                        Vec2::new(
                                            (position.length_seconds / beat_seconds) as f32
                                                * self.zoom,
                                            88.0,
                                        ),
                                    );
                                    let response = ui.interact(
                                        rect.intersect(timeline),
                                        egui::Id::new(&c.id),
                                        Sense::click_and_drag(),
                                    );
                                    if response.clicked() {
                                        self.selected_track = Some(t.id.clone());
                                        self.selected_clip = Some(c.id.clone());
                                        if let Some(pos) = response.interact_pointer_pos() {
                                            self.mark_grid(
                                                ((pos.x - timeline_x) / self.zoom) as f64,
                                            );
                                        }
                                    }
                                    if response.drag_started() {
                                        let mode = response
                                            .interact_pointer_pos()
                                            .map(|p| {
                                                if p.x < rect.left() + 8.0 {
                                                    1
                                                } else if p.x > rect.right() - 8.0 {
                                                    2
                                                } else {
                                                    0
                                                }
                                            })
                                            .unwrap_or(0);
                                        self.drag = Some(Drag {
                                            track: t.id.clone(),
                                            clip: c.id.clone(),
                                            position: c.position.clone(),
                                            mode,
                                            origin: ctx
                                                .input(|i| i.pointer.press_origin())
                                                .unwrap_or(rect.min),
                                        });
                                        self.selected_track = Some(t.id.clone());
                                        self.selected_clip = Some(c.id.clone());
                                    }
                                    if response.dragged() || response.drag_stopped() {
                                        if let Some(d) = &self.drag {
                                            if d.clip == c.id {
                                                let delta = (response
                                                    .interact_pointer_pos()
                                                    .unwrap_or(d.origin)
                                                    .x
                                                    - d.origin.x)
                                                    as f64
                                                    / self.zoom as f64;
                                                position = d.position.clone();
                                                match d.mode {
                                                    0 => {
                                                        let start =
                                                            (position.start_beats + delta).max(0.0);
                                                        position.start_beats = if self.snap {
                                                            (start * 4.0).round() / 4.0
                                                        } else {
                                                            start
                                                        };
                                                    }
                                                    1 => {
                                                        let seconds = (delta * beat_seconds)
                                                            .clamp(
                                                                -position.offset_seconds,
                                                                position.length_seconds - 0.01,
                                                            )
                                                            .max(
                                                                -position.start_beats
                                                                    * beat_seconds,
                                                            );
                                                        position.start_beats +=
                                                            seconds / beat_seconds;
                                                        position.offset_seconds += seconds;
                                                        position.length_seconds -= seconds;
                                                    }
                                                    _ => {
                                                        position.length_seconds = (position
                                                            .length_seconds
                                                            + delta * beat_seconds)
                                                            .max(0.01);
                                                    }
                                                }
                                            }
                                        }
                                    }
                                    if response.drag_stopped() {
                                        if let Some(d) = self.drag.take() {
                                            if d.mode == 0 {
                                                self.execute(Command::MoveClip {
                                                    track_id: d.track,
                                                    clip_id: d.clip,
                                                    start_beats: position.start_beats,
                                                });
                                            } else {
                                                self.execute(Command::SetClipPosition {
                                                    track_id: d.track,
                                                    clip_id: d.clip,
                                                    position: position.clone(),
                                                });
                                            }
                                        }
                                    }
                                    let visible = Rect::from_min_size(
                                        Pos2::new(
                                            timeline_x + position.start_beats as f32 * self.zoom,
                                            row.top() + 9.0,
                                        ),
                                        Vec2::new(
                                            ((position.length_seconds / beat_seconds) as f32
                                                * self.zoom)
                                                .max(3.0),
                                            88.0,
                                        ),
                                    );
                                    let path = self
                                        .session
                                        .project
                                        .source_path(&self.session.root, &c.source);
                                    let missing = !path.is_file();
                                    let selected_clip =
                                        self.selected_clip.as_deref() == Some(&c.id);
                                    let p = ui.painter().with_clip_rect(timeline);
                                    p.rect_filled(
                                        visible,
                                        3.0,
                                        if missing {
                                            Color32::from_rgb(106, 67, 57)
                                        } else {
                                            color.gamma_multiply(if selected_clip {
                                                0.85
                                            } else {
                                                0.60
                                            })
                                        },
                                    );
                                    p.rect_stroke(
                                        visible,
                                        3.0,
                                        Stroke::new(
                                            if selected_clip { 1.5_f32 } else { 0.5_f32 },
                                            if selected_clip { TEXT } else { color },
                                        ),
                                        egui::StrokeKind::Inside,
                                    );
                                    let label =
                                        path.file_stem().unwrap_or_default().to_string_lossy();
                                    let p = p.with_clip_rect(visible.shrink(3.0));
                                    p.text(
                                        visible.min + Vec2::new(7.0, 12.0),
                                        egui::Align2::LEFT_CENTER,
                                        if missing {
                                            format!("Missing · {label}")
                                        } else {
                                            label.into()
                                        },
                                        FontId::proportional(11.0),
                                        TEXT,
                                    );
                                    if let Some(data) = self.cache.files.get(&path) {
                                        waveform(&p, visible, data, &position);
                                    }
                                    response.on_hover_text(format!(
                                        "{}\nDrag to move · Drag edges to trim\n{}",
                                        path.display(),
                                        if missing {
                                            "Missing source — locate in clip controls"
                                        } else {
                                            "Original file is preserved"
                                        }
                                    ));
                                }
                            }
                            let end_y = ui.cursor().top();
                            let marker_x = timeline_x + self.grid_start_beats as f32 * self.zoom;
                            let marker = ui.painter().with_clip_rect(Rect::from_min_max(
                                ruler.min,
                                Pos2::new(controls_x, end_y),
                            ));
                            marker.line_segment(
                                [
                                    Pos2::new(marker_x, ruler.bottom()),
                                    Pos2::new(marker_x, end_y),
                                ],
                                Stroke::new(1.0_f32, MUTED.gamma_multiply(0.6)),
                            );
                            marker.add(egui::Shape::convex_polygon(
                                vec![
                                    Pos2::new(marker_x - 5.0, ruler.top() + 2.0),
                                    Pos2::new(marker_x + 5.0, ruler.top() + 2.0),
                                    Pos2::new(marker_x, ruler.top() + 9.0),
                                ],
                                MUTED,
                                Stroke::NONE,
                            ));
                            let x = timeline_x
                                + (self.session.transport.seconds / beat_seconds) as f32
                                    * self.zoom;
                            ui.painter()
                                .with_clip_rect(Rect::from_min_max(
                                    ruler.min,
                                    Pos2::new(controls_x, end_y),
                                ))
                                .line_segment(
                                    [Pos2::new(x, ruler.top()), Pos2::new(x, end_y)],
                                    Stroke::new(1.2_f32, ACCENT),
                                );
                            if tracks.is_empty() {
                                ui.add_space(100.0);
                                ui.vertical_centered(|ui| {
                                    ui.label(
                                        egui::RichText::new("Room for your next idea.").size(20.0),
                                    );
                                    ui.add_space(10.0);
                                    ui.label(
                                        egui::RichText::new(
                                            "Drop a WAV or FLAC file here, or add an audio track.",
                                        )
                                        .color(MUTED),
                                    );
                                });
                            }
                            if self.job.is_none() {
                                let files = ctx.input(|i| i.raw.dropped_files.clone());
                                if let Some(path) = files.first().and_then(|f| f.path.clone()) {
                                    let (track, beats) = drop_target
                                        .map(|(t, b)| {
                                            (
                                                Some(t),
                                                if self.snap {
                                                    (b * 4.0).round() / 4.0
                                                } else {
                                                    b
                                                },
                                            )
                                        })
                                        .unwrap_or((None, 0.0));
                                    self.import(path, track, beats);
                                    if files.len() > 1 {
                                        self.status =
                                            "Import one file at a time in this pre-alpha".into();
                                    }
                                }
                            }
                        });
                });
            });
    }
}
fn eyebrow(ui: &mut egui::Ui, text: &str) {
    ui.label(egui::RichText::new(text).size(10.0).color(MUTED).strong());
}
fn waveform(p: &egui::Painter, r: Rect, data: &AudioData, pos: &Position) {
    let width = r.width().max(1.0) as usize;
    let center = r.center().y + 9.0;
    for pixel in (0..width).step_by(2) {
        let begin = ((pos.offset_seconds + pixel as f64 / width as f64 * pos.length_seconds)
            * data.sample_rate as f64) as usize;
        let end = ((pos.offset_seconds + (pixel + 2) as f64 / width as f64 * pos.length_seconds)
            * data.sample_rate as f64) as usize;
        let mut peak = 0.0_f32;
        let step = ((end.saturating_sub(begin)) / 32).max(1);
        for i in (begin.min(data.frames.len())..end.min(data.frames.len())).step_by(step) {
            peak = peak
                .max(data.frames[i][0].abs())
                .max(data.frames[i][1].abs());
        }
        let amp = (peak.min(1.0) * 24.0).max(0.5);
        let x = r.left() + pixel as f32;
        p.line_segment(
            [Pos2::new(x, center - amp), Pos2::new(x, center + amp)],
            Stroke::new(1.0_f32, Color32::from_white_alpha(160)),
        );
    }
}
impl eframe::App for Velvet {
    fn update(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        self.poll();
        self.update_live_mix();
        if self.job.is_none() && ctx.input(|i| i.pointer.any_pressed()) {
            self.gesture = Some((self.session.project.clone(), self.session.revision));
        }
        if let Some(p) = &self.player {
            self.session.transport.seconds = p.seconds();
            self.session.transport.playing = p.playing();
            if p.state.failed.load(std::sync::atomic::Ordering::Relaxed) {
                self.status = "Audio output failed. Stop and reconnect your device.".into();
                self.error = true;
            }
        }
        if self.job.is_none() && !ctx.wants_keyboard_input() {
            let (space, undo, redo, save, open, new, delete) = ctx.input(|i| {
                (
                    i.key_pressed(egui::Key::Space),
                    i.modifiers.ctrl && !i.modifiers.shift && i.key_pressed(egui::Key::Z),
                    i.modifiers.ctrl && i.modifiers.shift && i.key_pressed(egui::Key::Z),
                    i.modifiers.ctrl && i.key_pressed(egui::Key::S),
                    i.modifiers.ctrl && i.key_pressed(egui::Key::O),
                    i.modifiers.ctrl && i.key_pressed(egui::Key::N),
                    i.key_pressed(egui::Key::Delete),
                )
            });
            if space {
                self.toggle_playback();
            }
            if undo {
                self.history(false);
            }
            if redo {
                self.history(true);
            }
            if save {
                self.save(false);
            }
            if open {
                if let Some(p) = rfd::FileDialog::new().pick_folder() {
                    self.request_open(Some(p));
                }
            }
            if new {
                self.request_open(None);
            }
            if delete {
                if let (Some(track_id), Some(clip_id)) =
                    (self.selected_track.clone(), self.selected_clip.take())
                {
                    self.execute(Command::RemoveClip { track_id, clip_id });
                }
            }
        }
        self.toolbar(ctx);
        egui::TopBottomPanel::bottom("status")
            .exact_height(30.0)
            .frame(egui::Frame::new().fill(BG).inner_margin(8.0))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    if self.job.is_some() {
                        ui.spinner();
                        ui.label(&self.busy);
                    } else {
                        ui.colored_label(
                            if self.error {
                                Color32::from_rgb(222, 160, 131)
                            } else {
                                MUTED
                            },
                            &self.status,
                        );
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let mut volume = self.session.project.master.volume_db;
                        ui.add_enabled_ui(self.job.is_none(), |ui| {
                            let r = ui.add(
                                egui::DragValue::new(&mut volume)
                                    .range(-90.0..=12.0)
                                    .speed(0.2)
                                    .suffix(" dB")
                                    .fixed_decimals(1),
                            );
                            if r.changed() {
                                self.execute(Command::SetMasterVolume { volume_db: volume });
                            }
                            if ui
                                .selectable_label(
                                    self.selected_track.as_deref() == Some("master"),
                                    "Master",
                                )
                                .on_hover_text("Select the master effect chain")
                                .clicked()
                            {
                                self.selected_track = Some("master".into());
                                self.selected_clip = None;
                            }
                        });
                    });
                });
            });
        self.rack(ctx);
        self.browser(ctx);
        self.ai(ctx);
        self.arrangement(ctx);
        if ctx.input(|i| i.pointer.any_released()) {
            if let Some((before, revision)) = self.gesture.take() {
                self.session
                    .group_changes(before, self.session.revision.saturating_sub(revision));
            }
        }
        if self.pending_new.is_some() {
            egui::Window::new("Unsaved project")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
                .show(ctx, |ui| {
                    ui.label("Save your edits before opening another project.");
                    ui.horizontal(|ui| {
                        if ui.button("Save").clicked() {
                            self.save(false);
                            if self.session.project == self.saved {
                                let path = self.pending_new.take().unwrap();
                                self.replace(path);
                            }
                        }
                        if ui.button("Discard edits").clicked() {
                            let path = self.pending_new.take().unwrap();
                            self.replace(path);
                        }
                        if ui.button("Cancel").clicked() {
                            self.pending_new = None;
                        }
                    });
                });
        }
        if ctx.input(|i| i.viewport().close_requested()) && self.session.project != self.saved {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.closing = true;
        }
        if self.closing {
            egui::Window::new("Save before closing?")
                .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
                .show(ctx, |ui| {
                    ui.label("Your project has unsaved edits.");
                    if ui.button("Save and close").clicked() {
                        self.save(false);
                        if self.session.project == self.saved {
                            self.closing = false;
                            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                        }
                    }
                    if ui.button("Discard and close").clicked() {
                        self.saved = self.session.project.clone();
                        self.closing = false;
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                    if ui.button("Cancel").clicked() {
                        self.closing = false;
                    }
                });
        }
        if self.screenshot.is_some() && self.job.is_none() {
            if !self.capture_requested {
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
                self.capture_requested = true;
            }
            let events = ctx.input(|i| i.events.clone());
            for e in events {
                if let egui::Event::Screenshot { image, .. } = e {
                    let bytes: Vec<u8> = image.pixels.iter().flat_map(|c| c.to_array()).collect();
                    if let Some(path) = self.screenshot.take() {
                        let _ = image::save_buffer(
                            path,
                            &bytes,
                            image.size[0] as u32,
                            image.size[1] as u32,
                            image::ColorType::Rgba8,
                        );
                    }
                    self.saved = self.session.project.clone();
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }
        }
        ctx.request_repaint_after(Duration::from_millis(
            if self.session.transport.playing || self.job.is_some() {
                16
            } else {
                100
            },
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn space_stop_returns_to_the_last_grid_marker_after_edits_and_tempo_changes() {
        let mut app = Velvet::new(&egui::Context::default());
        app.session = Session::new(Project::new("Test"), PathBuf::new());
        app.mark_grid(8.13);
        assert_eq!(app.grid_start_beats, 8.25);
        app.execute(Command::SetTempo { bpm: 60.0 });
        app.session.transport.playing = true;
        app.session.transport.seconds = 17.0;
        app.execute(Command::AddTrack {
            name: "Audio".into(),
        });
        app.history(false);
        app.toggle_playback();
        assert!(!app.session.transport.playing);
        assert_eq!(app.session.transport.seconds, 8.25);
        app.snap = false;
        app.mark_grid(11.37);
        app.session.transport.playing = true;
        app.session.transport.seconds = 25.0;
        app.toggle_playback();
        assert_eq!(app.session.transport.seconds, 11.37);
        app.execute(Command::Seek { seconds: -1.0 });
        assert_eq!(
            app.grid_start_beats, 11.37,
            "Rejected seek moved the grid marker"
        );
        app.execute(Command::Stop);
        assert_eq!(app.grid_start_beats, 0.0);
    }
    #[test]
    #[ignore = "Requires a real default audio output device"]
    fn hardware_gui_edit_keeps_the_same_stream_and_cursor() {
        let mut app = Velvet::new(&egui::Context::default());
        app.session = Session::new(Project::new("Test"), PathBuf::new());
        app.execute(Command::AddTrack {
            name: "Audio".into(),
        });
        let track_id = app.session.project.tracks[0].id.clone();
        app.execute(Command::ImportAudioClip {
            track_id: track_id.clone(),
            source: Source {
                path: std::path::absolute("missing-test.wav").unwrap(),
                kind: SourceKind::External,
            },
            position: Position {
                start_beats: 0.0,
                offset_seconds: 0.0,
                length_seconds: 2.0,
            },
        });
        let rate = Player::output_rate().unwrap();
        app.player = Some(
            Player::new(
                Arc::new(Mix {
                    sample_rate: rate,
                    frames: vec![[0.0; 2]; rate as usize * 2],
                    missing: vec![],
                    sources: vec![],
                    peak: 0.0,
                }),
                0.5,
            )
            .unwrap(),
        );
        app.player.as_ref().unwrap().play();
        app.session.transport.playing = true;
        app.session.transport.seconds = 0.5;
        let state = app.player.as_ref().unwrap().state.clone();
        app.execute(Command::SetTrackVolume {
            track_id: track_id.clone(),
            volume_db: -6.0,
        });
        app.execute(Command::SetMute {
            track_id,
            mute: true,
        });
        for kind in ["builtin.eq8", "builtin.compressor", "builtin.limiter"] {
            app.execute(Command::AddDevice {
                track_id: "master".into(),
                kind: kind.into(),
            });
        }
        let eq = app.session.project.master_devices[0].id.clone();
        app.execute(Command::SetDeviceParameter {
            track_id: "master".into(),
            device_id: eq,
            parameter: "band3_gain_db".into(),
            value: -9.0,
        });
        let deadline = Instant::now() + Duration::from_secs(1);
        while Instant::now() < deadline {
            app.update_live_mix();
            if !app.live_dirty && app.live_job.is_none() {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(
            !app.live_dirty && app.live_job.is_none(),
            "Live update did not finish"
        );
        std::thread::sleep(Duration::from_millis(50));
        app.update_live_mix();
        let player = app.player.as_ref().unwrap();
        assert!(
            Arc::ptr_eq(&state, &player.state),
            "Editing recreated the audio stream"
        );
        assert!(player.playing());
        assert!(player.seconds() > 0.5);
        app.history(false);
        app.history(true);
        assert!(state.playing.load(std::sync::atomic::Ordering::Relaxed));
        app.mark_grid(0.75);
        app.toggle_playback();
        assert!(!app.player.as_ref().unwrap().playing());
        assert!((app.player.as_ref().unwrap().seconds() - 0.375).abs() < 0.001);
        app.toggle_playback();
        std::thread::sleep(Duration::from_millis(60));
        assert!(app.player.as_ref().unwrap().playing());
        assert!(app.player.as_ref().unwrap().seconds() > 0.375);
        app.toggle_playback();
        assert!((app.player.as_ref().unwrap().seconds() - 0.375).abs() < 0.001);
        assert!(Arc::ptr_eq(&state, &app.player.as_ref().unwrap().state));
    }
    #[test]
    fn editing_and_history_keep_transport_running() {
        let mut app = Velvet::new(&egui::Context::default());
        app.session = Session::new(Project::new("Test"), PathBuf::new());
        app.session.transport.playing = true;
        app.session.transport.seconds = 4.0;
        app.execute(Command::AddTrack {
            name: "Audio".into(),
        });
        assert!(app.session.transport.playing, "Editing paused playback");
        assert_eq!(app.session.transport.seconds, 4.0);
        let track_id = app.session.project.tracks[0].id.clone();
        for command in [
            Command::SetTrackVolume {
                track_id: track_id.clone(),
                volume_db: -3.0,
            },
            Command::SetMute {
                track_id,
                mute: true,
            },
        ] {
            app.execute(command);
            assert!(app.session.transport.playing, "Mixer edit paused playback");
        }
        app.history(false);
        assert!(app.session.transport.playing, "Undo paused playback");
        app.history(true);
        assert!(app.session.transport.playing, "Redo paused playback");
    }
}
