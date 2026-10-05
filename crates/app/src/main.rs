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
mod arrangement_edit;
mod beat;
mod browser;
mod matrix;
mod piano_roll;
mod plugins;
mod rack;
mod synth;
mod theme;

use theme::{ACCENT, BG, CYAN, LINE, MUTED, PANEL, ROSE, TEXT};
enum Job {
    Warm(anyhow::Result<MediaCache>),
    Import {
        track: String,
        path: PathBuf,
        beats: f64,
        data: anyhow::Result<Arc<AudioData>>,
    },
    Mix(u64, anyhow::Result<Mix>),
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
type RenderTask = Box<dyn FnOnce(&mut velvet_audio::LiveMixer) + Send>;

struct Velvet {
    session: Session,
    saved: Project,
    selected_track: Option<String>,
    selected_clip: Option<String>,
    clip_clipboard: Option<arrangement_edit::ClipCopy>,
    selected_device: Option<(String, String)>,
    synth_popout: Option<(String, String)>,
    piano_roll: piano_roll::PianoRoll,
    note_preview: Option<Player>,
    note_preview_job: Option<Receiver<anyhow::Result<Mix>>>,
    plugin_paths: Vec<PathBuf>,
    plugin_roles: std::collections::BTreeMap<PathBuf, bool>,
    plugin_scan: Option<Receiver<(PathBuf, bool)>>,
    plugin_editor: Option<plugins::Editor>,
    scope_enabled: std::collections::HashSet<String>,
    scope_signals: std::collections::HashMap<String, Arc<velvet_audio::ScopeSignal>>,
    loop_clip: Option<(String, String)>,
    metronome: bool,
    monitor_volume: f64,
    sample_preview: Option<Player>,
    sample_preview_job: Option<Receiver<anyhow::Result<Mix>>>,
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
    render_worker: Option<mpsc::Sender<RenderTask>>,
    render_generation: Arc<std::sync::atomic::AtomicU64>,
    live_dirty: bool,
    edit_time: Instant,
    timeline_scroll: f32,
    track_heights: std::collections::BTreeMap<String, f32>,
    track_scroll: f32,
    timeline_pan: Option<(Pos2, f32, f32)>,
    search: String,
    browser_category: u8,
    browser_folder: Option<PathBuf>,
    browser_files: Vec<PathBuf>,
    browser_selected: Option<PathBuf>,
    browser_expanded: std::collections::BTreeSet<PathBuf>,
    browser_children: std::collections::BTreeMap<PathBuf, Vec<PathBuf>>,
    browser_history: Vec<PathBuf>,
    browser_history_index: usize,
    samples_root: Option<PathBuf>,
    grid_start_beats: f64,
    matrix: matrix::ParticleMatrix,
    matrix_enabled: bool,
}
fn track_toggle(
    ui: &mut egui::Ui,
    label: &str,
    active: bool,
    color: Color32,
    name: &str,
) -> egui::Response {
    let mut button = egui::Button::new(egui::RichText::new(label).strong().color(if active {
        BG
    } else {
        MUTED
    }))
    .min_size(Vec2::new(24.0, 22.0))
    .selected(active);
    if active {
        button = button.fill(color).stroke(Stroke::new(1.5_f32, color));
    } else {
        button = button.fill(BG).stroke(Stroke::new(1.0_f32, LINE));
    }
    ui.add(button)
        .on_hover_text(format!("{name}: {}", if active { "ON" } else { "OFF" }))
}
fn main() -> eframe::Result<()> {
    let _plugin_ui_thread = velvet_audio::plugins::initialize_ui_thread()
        .map_err(|error| eframe::Error::AppCreation(error.into()))?;
    if std::env::args_os().nth(1).as_deref() == Some(std::ffi::OsStr::new("--velvet-plugin-info")) {
        let result = std::env::args_os()
            .nth(2)
            .map(PathBuf::from)
            .ok_or_else(|| anyhow::anyhow!("Missing plugin path"))
            .and_then(|path| velvet_audio::plugins::is_instrument(&path));
        match result {
            Ok(instrument) => {
                println!("VELVET_PLUGIN_ROLE={instrument}");
                return Ok(());
            }
            Err(error) => {
                eprintln!("{error:#}");
                std::process::exit(1);
            }
        }
    }
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1440.0, 900.0])
            .with_min_inner_size([1000.0, 650.0])
            .with_title("Velvet DAW"),
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
        theme::apply(ctx);
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
            clip_clipboard: None,
            selected_device: None,
            synth_popout: None,
            piano_roll: piano_roll::PianoRoll::default(),
            note_preview: None,
            note_preview_job: None,
            plugin_paths: velvet_audio::plugins::scan(),
            plugin_roles: std::collections::BTreeMap::new(),
            plugin_scan: None,
            plugin_editor: None,
            scope_enabled: Default::default(),
            scope_signals: Default::default(),
            loop_clip: None,
            metronome: false,
            monitor_volume: 1.0,
            sample_preview: None,
            sample_preview_job: None,
            cache: MediaCache::default(),
            player: None,
            job: None,
            busy: String::new(),
            ai_open: false,
            prompt: String::new(),
            ai_log: vec![],
            status,
            error,
            zoom: 22.0,
            snap: true,
            drag: None,
            pending_new: None,
            closing: false,
            screenshot: std::env::var_os("VELVET_SCREENSHOT").map(PathBuf::from),
            capture_requested: false,
            gesture: None,
            live_job: None,
            render_worker: None,
            render_generation: Arc::new(std::sync::atomic::AtomicU64::new(0)),
            live_dirty: false,
            edit_time: Instant::now(),
            timeline_scroll: 0.0,
            track_heights: Default::default(),
            track_scroll: 0.0,
            timeline_pan: None,
            search: String::new(),
            browser_category: 1,
            browser_folder: None,
            browser_files: vec![],
            browser_selected: None,
            browser_expanded: Default::default(),
            browser_children: Default::default(),
            browser_history: vec![],
            browser_history_index: 0,
            samples_root: None,
            grid_start_beats: 0.0,
            matrix: matrix::ParticleMatrix::default(),
            matrix_enabled: true,
        };
        app.restore_samples_folder();
        app.scan_plugin_roles();
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
        let previous_bpm = self.session.project.tempo.bpm;
        let selected_index = self
            .session
            .project
            .tracks
            .iter()
            .position(|t| Some(&t.id) == self.selected_track.as_ref())
            .unwrap_or(0);
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
                    self.synchronize_tempo(previous_bpm);
                    self.status = "Edit applied · Undo available".into();
                    self.error = false;
                }
            }
            Err(e) => {
                self.status = e.to_string();
                self.error = true;
            }
        }
        self.ensure_track_selection(selected_index);
    }
    fn ensure_track_selection(&mut self, previous_index: usize) {
        if self.selected_track.as_deref() == Some("master")
            || self
                .session
                .project
                .tracks
                .iter()
                .any(|t| Some(&t.id) == self.selected_track.as_ref())
        {
            return;
        }
        let tracks = &self.session.project.tracks;
        self.selected_track = tracks
            .get(previous_index.min(tracks.len().saturating_sub(1)))
            .map(|t| t.id.clone());
        self.selected_clip = None;
        self.selected_device = None;
    }
    fn invalidate(&mut self) {
        self.render_generation.fetch_add(1, std::sync::atomic::Ordering::AcqRel);
        if !self.live_dirty {
            self.edit_time = Instant::now();
        }
        self.live_dirty =
            self.player.is_some() || self.job.is_some() || !self.scope_enabled.is_empty();
    }
    fn reset_playback(&mut self) {
        self.render_generation.fetch_add(1, std::sync::atomic::Ordering::AcqRel);
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
                            Ok(mut mix) => {
                                self.scope_signals = std::mem::take(&mut mix.device_signals);
                                self.scope_signals
                                    .retain(|id, _| self.scope_enabled.contains(id));
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
            && self.job.is_none()
            && (self.player.is_some() || !self.scope_enabled.is_empty())
            && self.live_job.is_none()
            && self.edit_time.elapsed() >= Duration::from_millis(30)
        {
            let rate = self
                .player
                .as_ref()
                .map_or(self.session.project.audio.sample_rate, |p| p.sample_rate);
            let scopes = self.scope_enabled.clone();
            let revision = self.session.revision;
            let project = self.session.project.clone();
            let root = self.session.root.clone();
            let mut cache = MediaCache {
                files: self.cache.files.clone(),
            };
            let (tx, rx) = mpsc::channel();
            self.live_job = Some(rx);
            self.live_dirty = false;
            self.render(move |renderer| {
                let _ = tx.send((
                    revision,
                    renderer.mix(&project, &root, rate, &mut cache, &scopes),
                ));
            });
        }
    }
    fn history(&mut self, redo: bool) {
        let previous_bpm = self.session.project.tempo.bpm;
        let selected_index = self
            .session
            .project
            .tracks
            .iter()
            .position(|t| Some(&t.id) == self.selected_track.as_ref())
            .unwrap_or(0);
        let changed = if redo {
            self.session.redo()
        } else {
            self.session.undo()
        };
        if changed {
            self.ensure_track_selection(selected_index);
            self.invalidate();
            self.synchronize_tempo(previous_bpm);
            self.status = if redo { "Edit restored" } else { "Edit undone" }.into();
        }
    }
    fn transport(&mut self) {
        if self.session.transport.playing {
            if let Some(p) = &self.player {
                p.seek(self.session.transport.seconds);
                p.play();
            } else if self.job.is_none() {
                self.prepare_audio();
            }
        } else if let Some(p) = &self.player {
            p.pause();
            p.seek(self.session.transport.seconds);
        }
    }
    fn synchronize_tempo(&mut self, previous_bpm: f64) {
        let bpm = self.session.project.tempo.bpm;
        if bpm == previous_bpm {
            return;
        }
        // The old rendered MIDI uses the old tempo. Never play it under the new grid.
        if let Some(player) = &self.player {
            player.pause();
            self.session.transport.seconds = player.seconds();
        }
        self.session.transport.seconds *= previous_bpm / bpm;
        self.player = None;
        self.live_job = None;
        if self.session.transport.playing && self.job.is_none() {
            self.prepare_audio();
        }
    }
    fn prepare_audio(&mut self) {
        let rate = match Player::output_rate() {
            Ok(r) => r,
            Err(e) => {
                self.report(Err(e), "");
                self.session.transport.playing = false;
                self.metronome = false;
                return;
            }
        };
        let project = self.session.project.clone();
        let root = self.session.root.clone();
        let mut cache = MediaCache::default();
        cache.files = self.cache.files.clone();
        let scopes = self.scope_enabled.clone();
        let revision = self.session.revision;
        let (tx, rx) = mpsc::channel();
        self.job = Some(rx);
        self.busy = "Preparing audio".into();
        self.render(move |renderer| {
            let _ = tx.send(Job::Mix(
                revision,
                renderer.mix(&project, &root, rate, &mut cache, &scopes),
            ));
        });
    }
    fn toggle_metronome(&mut self) {
        self.metronome = !self.metronome;
        if let Some(player) = &self.player {
            player.set_metronome(self.metronome.then_some(self.session.project.tempo.bpm));
            player.set_monitor_gain(self.monitor_volume, true);
        }
    }
    fn toggle_playback(&mut self) {
        if self.session.transport.playing {
            self.execute(Command::Pause);
            self.execute(Command::Seek {
                seconds: self.grid_start_beats * 60.0 / self.session.project.tempo.bpm,
            });
        } else {
            let start = self.loop_range().map_or(
                self.grid_start_beats * 60.0 / self.session.project.tempo.bpm,
                |(start, _)| start,
            );
            self.execute(Command::Seek { seconds: start });
            self.execute(Command::Play);
        }
    }
    fn loop_range(&self) -> Option<(f64, f64)> {
        if let Some((a, b)) = self.piano_roll.loop_beats(&self.session.project) {
            let seconds = 60.0 / self.session.project.tempo.bpm;
            return (b > a).then_some((a * seconds, b * seconds));
        }
        let (track, clip) = self.loop_clip.as_ref()?;
        if track == clip {
            let (start, end) = synth::midi_region(self.session.project.track(track).ok()?)?;
            let seconds = 60.0 / self.session.project.tempo.bpm;
            return Some((start * seconds, end * seconds));
        }
        let clip = self
            .session
            .project
            .track(track)
            .ok()?
            .clips
            .iter()
            .find(|c| c.id == *clip)?;
        let start = clip.position.start_beats * 60.0 / self.session.project.tempo.bpm;
        Some((
            start,
            start + clip.duration_seconds(self.session.project.tempo.bpm),
        ))
    }
    fn toggle_clip_loop(&mut self) {
        self.piano_roll.loop_enabled = false;
        if let (Some(track), Some(clip)) = (&self.selected_track, &self.selected_clip) {
            let selected = (track.clone(), clip.clone());
            self.loop_clip = if self.loop_clip.as_ref() == Some(&selected) {
                None
            } else {
                Some(selected)
            };
        } else {
            self.loop_clip = None;
        }
        let range = self.loop_range();
        if let Some(player) = &self.player {
            player.set_loop(range);
        }
        if let Some((start, _)) = range {
            self.execute(Command::Seek { seconds: start });
        }
    }
    fn remove_selection(&mut self) {
        if let Some((track_id, device_id)) = self.selected_device.take() {
            if self.selected_track.as_ref() == Some(&track_id) {
                self.execute(Command::RemoveDevice {
                    track_id,
                    device_id,
                });
                return;
            }
        }
        if let (Some(track_id), Some(clip_id)) =
            (self.selected_track.clone(), self.selected_clip.take())
        {
            if track_id == clip_id
                && self
                    .session
                    .project
                    .track(&track_id)
                    .is_ok_and(|t| matches!(t.kind, velvet_core::TrackKind::Midi))
            {
                self.execute(Command::SetMidiNotes {
                    track_id,
                    notes: Vec::new(),
                });
            } else {
                self.execute(Command::RemoveClip { track_id, clip_id });
            }
        } else if let Some(track_id) = self.selected_track.clone() {
            if self.session.project.tracks.iter().any(|t| t.id == track_id) {
                self.execute(Command::RemoveTrack { track_id });
            }
        }
    }
    fn shortcuts(&mut self, ctx: &egui::Context) {
        if !ctx.wants_keyboard_input() && ctx.input(|i| i.key_pressed(egui::Key::Space)) {
            self.toggle_playback();
        }
        if self.job.is_none() {
            if ctx.input_mut(|i| i.consume_key(egui::Modifiers::SHIFT, egui::Key::Tab)) {
                if self.piano_roll.open {
                    self.piano_roll.open = false;
                    self.piano_roll.keyboard_focus = false;
                } else if self.selected_track.as_ref().is_some_and(|id| {
                    self.session.project.track(id)
                        .is_ok_and(|t| matches!(t.kind, velvet_core::TrackKind::Midi))
                }) {
                    self.piano_roll.open = true;
                    self.piano_roll.keyboard_focus = true;
                }
            }
        }
        if self.job.is_none() && !ctx.wants_keyboard_input() {
            if ctx.input(|i| i.key_pressed(egui::Key::F7)) {
                if let Some(target) = self.selected_track.as_ref().filter(|id| {
                    self.session
                        .project
                        .track(id)
                        .is_ok_and(|t| matches!(t.kind, velvet_core::TrackKind::Midi))
                }) {
                    ctx.data_mut(|d| {
                        d.insert_temp(egui::Id::new("midi_edit_target"), target.clone())
                    });
                }
            }
            let (undo, redo, save, open, new, delete, loop_clip) = ctx.input(|i| {
                (
                    i.modifiers.ctrl && !i.modifiers.shift && i.key_pressed(egui::Key::Z),
                    i.modifiers.ctrl && i.modifiers.shift && i.key_pressed(egui::Key::Z),
                    i.modifiers.ctrl && i.key_pressed(egui::Key::S),
                    i.modifiers.ctrl && i.key_pressed(egui::Key::O),
                    i.modifiers.ctrl && i.key_pressed(egui::Key::N),
                    i.key_pressed(egui::Key::Delete) || i.key_pressed(egui::Key::Backspace),
                    i.modifiers.ctrl && i.key_pressed(egui::Key::L),
                )
            });
            if !self.piano_roll.keyboard_focus {
                self.arrangement_shortcuts(ctx);
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
            if loop_clip && !self.piano_roll.keyboard_focus {
                self.toggle_clip_loop();
            }
            if delete && !self.piano_roll.keyboard_focus {
                self.remove_selection();
            }
        }
    }
    fn navigate_arrangement(&mut self, ctx: &egui::Context, viewport: Rect, total: f32) {
        let (pointer, modifiers, wheel, horizontal, pressed, down) = ctx.input(|i| {
            (
                i.pointer.hover_pos(),
                i.modifiers,
                i.raw_scroll_delta.y,
                i.smooth_scroll_delta.x,
                i.pointer.primary_pressed(),
                i.pointer.primary_down(),
            )
        });
        let hovered = pointer.is_some_and(|p| viewport.contains(p));
        if hovered && self.job.is_none() && modifiers.ctrl && modifiers.alt && pressed {
            self.timeline_pan = pointer.map(|p| (p, self.timeline_scroll, self.track_scroll));
        }
        if let Some((origin, horizontal, vertical)) = self.timeline_pan {
            if let Some(pointer) = pointer {
                self.timeline_scroll = horizontal - (pointer.x - origin.x) / self.zoom;
                self.track_scroll = (vertical - (pointer.y - origin.y)).max(0.0);
                ctx.set_cursor_icon(egui::CursorIcon::Grabbing);
            }
            if !down {
                self.timeline_pan = None;
            }
        }
        if hovered && self.job.is_none() {
            if modifiers.ctrl && !modifiers.alt {
                if wheel != 0.0 {
                    self.zoom_at(viewport, pointer.unwrap().x, (wheel * 0.005).exp());
                }
            } else if !modifiers.alt {
                self.timeline_scroll -= horizontal / self.zoom;
                ctx.input_mut(|i| i.smooth_scroll_delta.x = 0.0);
            }
            if modifiers.alt || modifiers.ctrl {
                ctx.input_mut(|i| i.smooth_scroll_delta = Vec2::ZERO);
            }
        }
        if !ctx.wants_keyboard_input() && self.job.is_none() {
            let (plus, minus) = ctx.input(|i| {
                (
                    i.key_pressed(egui::Key::Plus) || i.key_pressed(egui::Key::Equals),
                    i.key_pressed(egui::Key::Minus),
                )
            });
            if plus || minus {
                let x = self
                    .selected_track
                    .as_deref()
                    .and_then(|track| self.session.project.track(track).ok())
                    .and_then(|track| {
                        track
                            .clips
                            .iter()
                            .find(|c| Some(&c.id) == self.selected_clip.as_ref())
                    })
                    .map_or(viewport.center().x, |clip| {
                        viewport.left()
                            + (clip.position.start_beats as f32 - self.timeline_scroll) * self.zoom
                    });
                self.zoom_at(
                    viewport,
                    x.clamp(viewport.left(), viewport.right()),
                    if plus { 1.2 } else { 1.0 / 1.2 },
                );
            }
        }
        let visible = viewport.width().max(1.0) / self.zoom;
        self.timeline_scroll = self.timeline_scroll.clamp(0.0, (total - visible).max(0.0));
    }
    fn zoom_at(&mut self, viewport: Rect, x: f32, factor: f32) {
        let offset = x - viewport.left();
        let beat = self.timeline_scroll + offset / self.zoom;
        self.zoom = (self.zoom * factor).clamp(8.0, 90.0);
        self.timeline_scroll = (beat - offset / self.zoom).max(0.0);
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

    fn render(&mut self, work: impl FnOnce(&mut velvet_audio::LiveMixer) + Send + 'static) {
        let generation = self.render_generation.clone();
        let expected = generation.load(std::sync::atomic::Ordering::Acquire);
        let mut task: RenderTask = Box::new(move |renderer| {
            renderer.set_cancellation(generation, expected);
            work(renderer);
        });
        loop {
            let sender = self.render_worker.get_or_insert_with(|| {
                let (tx, rx) = mpsc::channel::<RenderTask>();
                std::thread::spawn(move || {
                    let _ui = velvet_audio::plugins::initialize_ui_thread().ok();
                    let mut renderer = velvet_audio::LiveMixer::default();
                    while let Ok(task) = rx.recv() {
                        task(&mut renderer);
                    }
                });
                tx
            });
            match sender.send(task) {
                Ok(()) => break,
                Err(error) => {
                    task = error.0;
                    self.render_worker = None;
                }
            }
        }
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
        let preload_audio = matches!(&job, Job::Warm(Ok(_)) | Job::Import { data: Ok(_), .. });
        if matches!(&job, Job::Mix(revision, _) if *revision != self.session.revision) {
            self.prepare_audio();
            return;
        }
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
                    self.selected_device = None;
                    self.selected_clip = self
                        .session
                        .project
                        .track(&track)
                        .ok()
                        .and_then(|t| t.clips.last().map(|c| c.id.clone()));
                    if let Some(clip_id) = self.selected_clip.clone() {
                        self.execute(Command::SetClipTempo {
                            track_id: track,
                            clip_id,
                            source_bpm: Some(self.session.project.tempo.bpm),
                        });
                    }
                }
                Err(e) => self.report(Err(e), ""),
            },
            Job::Mix(_, r) => match r.and_then(|mut m| {
                self.scope_signals = std::mem::take(&mut m.device_signals);
                if !m.missing.is_empty() {
                    self.status = format!("{} missing source(s) are silent", m.missing.len());
                }
                Player::new(Arc::new(m), self.session.transport.seconds)
            }) {
                Ok(p) => {
                    p.set_loop(self.loop_range());
                    p.set_metronome(self.metronome.then_some(self.session.project.tempo.bpm));
                    if self.session.transport.playing {
                        p.play();
                    }
                    self.player = Some(p);
                    self.live_dirty = false;
                }
                Err(e) => {
                    self.session.transport.playing = false;
                    self.metronome = false;
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
                let previous_bpm = self.session.project.tempo.bpm;
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
                    if !transport_requested {
                        self.synchronize_tempo(previous_bpm);
                    } else if previous_bpm != self.session.project.tempo.bpm {
                        self.player = None;
                        self.live_job = None;
                    }
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
        if (self.session.transport.playing || preload_audio)
            && self.player.is_none()
            && self.job.is_none()
        {
            self.prepare_audio();
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
        if !self.capture_plugin_state() {
            return;
        }
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
        if !self.capture_plugin_state() {
            return;
        }
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
        if !self.capture_plugin_state() {
            return;
        }
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
                self.piano_roll = piano_roll::PianoRoll::default();
                self.note_preview = None;
                self.note_preview_job = None;
                self.plugin_editor = None;
                self.scope_enabled.clear();
                self.scope_signals.clear();
                self.reset_playback();
                self.session = s;
                self.saved = self.session.project.clone();
                self.selected_track = self.session.project.tracks.first().map(|t| t.id.clone());
                self.selected_clip = None;
                self.clip_clipboard = None;
                self.selected_device = None;
                self.loop_clip = None;
                self.metronome = false;
                self.track_heights.clear();
                self.track_scroll = 0.0;
                self.timeline_pan = None;
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
            .exact_height(44.0)
            .frame(egui::Frame::new().fill(PANEL).inner_margin(9.0))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new("VELVET")
                            .size(17.0)
                            .strong()
                            .color(TEXT),
                    );
                    let (dot, _) = ui.allocate_exact_size(Vec2::new(6.0, 6.0), Sense::hover());
                    ui.painter().circle_filled(dot.center(), 2.5, ACCENT);
                    ui.add_space(10.0);
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
                                egui::Button::new("Undo").frame(false),
                            )
                            .on_hover_text("Undo · Ctrl+Z")
                            .clicked()
                        {
                            self.history(false);
                        }
                        if ui
                            .add_enabled(
                                !self.session.redo_history.is_empty(),
                                egui::Button::new("Redo").frame(false),
                            )
                            .on_hover_text("Redo · Ctrl+Shift+Z")
                            .clicked()
                        {
                            self.history(true);
                        }
                        ui.add_space(10.0);
                    });
                    if ui
                        .selectable_label(
                            self.session.transport.playing,
                            egui::RichText::new("▶").color(ACCENT),
                        )
                        .on_hover_text("Play / return to grid marker · Space")
                        .clicked()
                    {
                        self.toggle_playback();
                    }
                    if ui.button("■").on_hover_text("Stop").clicked() {
                        self.execute(Command::Stop);
                    }
                    ui.add_enabled_ui(self.job.is_none(), |ui| {
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
                        if ui
                            .selectable_label(self.metronome, "Metronome")
                            .on_hover_text("Metronome during playback · Click to enable / disable")
                            .clicked()
                        {
                            self.toggle_metronome();
                        }
                        ui.label(egui::RichText::new("4/4").monospace().color(MUTED));
                        ui.add_space(10.0);
                        ui.label(
                            egui::RichText::new(format!(
                                "{:02}:{:06.3}",
                                (self.session.transport.seconds / 60.0) as u32,
                                self.session.transport.seconds % 60.0
                            ))
                            .monospace()
                            .color(ACCENT),
                        );
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.toggle_value(
                            &mut self.ai_open,
                            egui::RichText::new("Velvet AI").color(CYAN),
                        );
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
                    let mut synced = c.source_bpm.is_some();
                    let mut source_bpm = c.source_bpm.unwrap_or(self.session.project.tempo.bpm);
                    let sync_changed = ui.checkbox(&mut synced, "Sync to BPM").changed();
                    let bpm_changed = ui.add_enabled(synced,
                        egui::DragValue::new(&mut source_bpm).range(20.0..=400.0).speed(0.1).prefix("Source BPM ")
                    ).on_hover_text("Original audio tempo. Playback speed and pitch follow the project BPM.").changed();
                    if sync_changed || bpm_changed {
                        self.execute(Command::SetClipTempo { track_id: tid.clone(), clip_id: cid.clone(), source_bpm: synced.then_some(source_bpm) });
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
    fn track_name_ui(
        &mut self,
        ui: &mut egui::Ui,
        track: &velvet_core::Track,
        selected: bool,
        color: Color32,
    ) -> egui::Response {
        let key = egui::Id::new(("track_name_edit", &track.id));
        let draft = ui.data_mut(|d| d.get_temp::<(String, bool)>(key));
        if let Some((mut name, first_frame)) = draft {
            let response = ui.add(
                egui::TextEdit::singleline(&mut name)
                    .id(key)
                    .desired_width(ui.available_width()),
            );
            if first_frame {
                response.request_focus();
            }
            let cancel = ui.input(|i| i.key_pressed(egui::Key::Escape));
            let commit = !first_frame
                && (response.lost_focus() || ui.input(|i| i.key_pressed(egui::Key::Enter)));
            if cancel || commit {
                ui.data_mut(|d| d.remove::<(String, bool)>(key));
                response.surrender_focus();
                let name = name.trim();
                if !cancel && !name.is_empty() && name != track.name {
                    self.execute(Command::RenameTrack {
                        track_id: track.id.clone(),
                        name: name.into(),
                    });
                }
            } else {
                ui.data_mut(|d| d.insert_temp(key, (name, false)));
            }
            response
        } else {
            let response = ui
                .selectable_label(selected, egui::RichText::new(&track.name).color(color))
                .on_hover_text("Click to rename track");
            if response.clicked() {
                self.selected_track = Some(track.id.clone());
                self.selected_clip = None;
                self.selected_device = None;
                ui.data_mut(|d| d.insert_temp(key, (track.name.clone(), true)));
            }
            response
        }
    }
    fn arrangement(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(BG).inner_margin(0.0))
            .show(ctx, |ui| {
                // Reserve the background before controls/clips, then fill it after navigation/layout.
                let matrix_slot = ui.painter().add(egui::Shape::Noop);
                let mut matrix_canvas = Rect::NOTHING;
                ui.add_enabled_ui(self.job.is_none(), |ui| {
                    egui::Frame::new()
                        .fill(PANEL)
                        .inner_margin(9.0)
                        .show(ui, |ui| {
                            let total = (self.session.project.duration_seconds()
                                * self.session.project.tempo.bpm
                                / 60.0
                                + 8.0)
                                .clamp(32.0, 20000.0)
                                as f32;
                            let visible = ((ui.available_width() - 186.0) / self.zoom).max(1.0);
                            let limit = (total - visible).max(0.0);
                            self.timeline_scroll = self.timeline_scroll.min(limit);
                            let narrow = ui.available_width() < 700.0;
                            let name = format!(
                                "/  {}{}",
                                self.session.project.project.name,
                                if self.session.project != self.saved {
                                    " *"
                                } else {
                                    ""
                                }
                            );
                            if narrow {
                                ui.horizontal(|ui| {
                                    eyebrow(ui, "ARRANGEMENT");
                                    ui.add(
                                        egui::Label::new(egui::RichText::new(&name).color(MUTED))
                                            .truncate(),
                                    );
                                });
                            }
                            ui.horizontal(|ui| {
                                if !narrow {
                                    eyebrow(ui, "ARRANGEMENT");
                                    ui.label(egui::RichText::new(&name).color(MUTED));
                                }
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        if ui.small_button("+ MIDI Track").clicked() {
                                            self.execute(Command::AddMidiTrack {
                                                name: format!(
                                                    "MIDI {}",
                                                    self.session.project.tracks.len() + 1
                                                ),
                                            });
                                            self.selected_track = self
                                                .session
                                                .project
                                                .tracks
                                                .last()
                                                .map(|t| t.id.clone());
                                        }
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
                                        if ui
                                            .selectable_label(self.loop_range().is_some(), "Loop")
                                            .on_hover_text("Loop selected clip · Ctrl+L")
                                            .clicked()
                                        {
                                            self.toggle_clip_loop();
                                        }
                                        ui.menu_button("Edit", |ui| self.arrangement_menu(ui));
                                        ui.checkbox(&mut self.snap, "Snap");
                                        ui.menu_button("Matrix", |ui| {
                                            ui.checkbox(&mut self.matrix_enabled, "Enabled");
                                            ui.separator();
                                            for preset in [
                                                matrix::MatrixPreset::FluidGrid,
                                                matrix::MatrixPreset::CrossingWaves,
                                            ] {
                                                if ui
                                                    .selectable_value(
                                                        &mut self.matrix.preset,
                                                        preset,
                                                        preset.label(),
                                                    )
                                                    .clicked()
                                                {
                                                    self.matrix_enabled = true;
                                                    ui.close_menu();
                                                }
                                            }
                                        })
                                        .response
                                        .on_hover_text(
                                            format!(
                                                "{} · {}",
                                                self.matrix.preset.label(),
                                                if self.matrix_enabled {
                                                    "Enabled"
                                                } else {
                                                    "Disabled"
                                                }
                                            ),
                                        );
                                        ui.add(
                                            egui::Slider::new(&mut self.zoom, 8.0..=90.0)
                                                .show_value(false)
                                                .text("Zoom"),
                                        );
                                        if limit > 0.0 {
                                            ui.spacing_mut().slider_width = 44.0;
                                            ui.add(
                                                egui::Slider::new(
                                                    &mut self.timeline_scroll,
                                                    0.0..=limit,
                                                )
                                                .show_value(false)
                                                .text("View"),
                                            )
                                            .on_hover_text(
                                                "Scroll the timeline; track controls stay fixed",
                                            );
                                        }
                                    },
                                );
                            });
                            self.inspector(ui);
                        });
                    ui.separator();
                    let tracks = self.session.project.tracks.clone();
                    let beat_seconds = 60.0 / self.session.project.tempo.bpm;
                    let total_beats = (self.session.project.duration_seconds() / beat_seconds
                        + 8.0)
                        .clamp(32.0, 20000.0);
                    let header = 186.0;
                    let mut viewport = ui.available_rect_before_wrap();
                    viewport.max.x -= header;
                    self.navigate_arrangement(ctx, viewport, total_beats as f32);
                    let navigating = self.timeline_pan.is_some()
                        || ctx.input(|i| i.modifiers.ctrl && i.modifiers.alt);
                    let scroll = egui::ScrollArea::vertical()
                        .auto_shrink([false, false])
                        .drag_to_scroll(false)
                        .vertical_scroll_offset(self.track_scroll)
                        .show(ui, |ui| {
                            let width = ui.available_width();
                            let (ruler, r) =
                                ui.allocate_exact_size(Vec2::new(width, 30.0), Sense::click());
                            let controls_x = ruler.right() - header;
                            let timeline_x = ruler.left() - self.timeline_scroll * self.zoom;
                            let spacing = ui.spacing().item_spacing.y;
                            let mut matrix_view = matrix::MatrixView {
                                origin: Pos2::new(timeline_x, ruler.bottom() + spacing),
                                scale_x: self.zoom / 22.0,
                                row_height: 76.0 + spacing,
                                track_heights: Vec::with_capacity(tracks.len()),
                            };
                            ui.painter().rect_filled(ruler, 0.0, BG);
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
                            if r.clicked() && !navigating {
                                if let Some(p) = r.interact_pointer_pos() {
                                    if p.x < controls_x {
                                        self.selected_device = None;
                                        self.mark_grid(((p.x - timeline_x) / self.zoom) as f64);
                                    }
                                }
                            }
                            let mut drop_target = None;
                            let hover = ctx.input(|i| i.pointer.hover_pos());
                            for (index, t) in tracks.iter().enumerate() {
                                let mut height =
                                    self.track_heights.get(&t.id).copied().unwrap_or(76.0);
                                let row_area =
                                    Rect::from_min_size(ui.cursor().min, Vec2::new(width, height));
                                if hover
                                    .is_some_and(|p| row_area.intersect(ui.clip_rect()).contains(p))
                                {
                                    let (alt, ctrl, delta) = ui.input(|i| {
                                        (i.modifiers.alt, i.modifiers.ctrl, i.raw_scroll_delta.y)
                                    });
                                    if alt && !ctrl && delta != 0.0 {
                                        height =
                                            (height * (delta * 0.005).exp()).clamp(76.0, 320.0);
                                        self.track_heights.insert(t.id.clone(), height);
                                    }
                                }
                                let (row, _) = ui
                                    .allocate_exact_size(Vec2::new(width, height), Sense::hover());
                                matrix_view.track_heights.push(height + spacing);
                                let timeline = Rect::from_min_max(
                                    row.min,
                                    Pos2::new(controls_x, row.bottom()),
                                );
                                let controls =
                                    Rect::from_min_max(Pos2::new(controls_x, row.top()), row.max);
                                let header = ui.interact(
                                    controls,
                                    egui::Id::new(("track_selection", &t.id)),
                                    Sense::click(),
                                );
                                if header.clicked() {
                                    self.selected_track = Some(t.id.clone());
                                    self.selected_clip = None;
                                    self.selected_device = None;
                                }
                                let selected = self.selected_track.as_deref() == Some(&t.id);
                                let p = ui.painter();
                                p.rect_filled(timeline, 0.0, Color32::from_black_alpha(50));
                                p.rect_filled(
                                    Rect::from_min_max(Pos2::new(controls_x, row.top()), row.max),
                                    0.0,
                                    if selected || header.hovered() {
                                        Color32::from_rgb(30, 38, 45)
                                    } else {
                                        PANEL
                                    },
                                );
                                p.line_segment(
                                    [Pos2::new(row.left(), row.bottom()), row.right_bottom()],
                                    Stroke::new(1.0_f32, LINE),
                                );
                                let color = theme::track_color(t.color);
                                p.rect_filled(
                                    Rect::from_min_size(
                                        Pos2::new(controls_x, row.top()),
                                        Vec2::new(1.0, row.height()),
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
                                                Color32::from_rgb(17, 20, 25)
                                            },
                                        ),
                                    );
                                }
                                let header_rect = Rect::from_min_max(
                                    Pos2::new(controls_x + 10.0, row.top() + 3.0),
                                    row.max - Vec2::new(10.0, 6.0),
                                );
                                ui.scope_builder(
                                    egui::UiBuilder::new().max_rect(header_rect),
                                    |ui| {
                                        ui.spacing_mut().button_padding.x = 4.0;
                                        ui.spacing_mut().item_spacing.x = 4.0;
                                        ui.horizontal(|ui| {
                                            ui.label(
                                                egui::RichText::new(format!("{:02}", index + 1))
                                                    .small()
                                                    .color(MUTED),
                                            );
                                            self.track_name_ui(ui, t, selected, color);
                                        });
                                        ui.horizontal(|ui| {
                                            if track_toggle(
                                                ui,
                                                "M",
                                                t.mixer.mute,
                                                Color32::from_rgb(255, 190, 65),
                                                "Mute",
                                            )
                                            .clicked()
                                            {
                                                self.execute(Command::SetMute {
                                                    track_id: t.id.clone(),
                                                    mute: !t.mixer.mute,
                                                });
                                            }
                                            if track_toggle(ui, "S", t.mixer.solo, CYAN, "Solo")
                                                .clicked()
                                            {
                                                self.execute(Command::SetSolo {
                                                    track_id: t.id.clone(),
                                                    solo: !t.mixer.solo,
                                                });
                                            }
                                            let mut volume = t.mixer.volume_db;
                                            ui.spacing_mut().slider_width = 48.0;
                                            let r = ui.add(
                                                egui::Slider::new(&mut volume, -90.0..=12.0)
                                                    .show_value(false),
                                            );
                                            if r.changed() {
                                                self.execute(Command::SetTrackVolume {
                                                    track_id: t.id.clone(),
                                                    volume_db: volume,
                                                });
                                            }
                                            let mut volume = t.mixer.volume_db;
                                            if ui
                                                .add(
                                                    egui::DragValue::new(&mut volume)
                                                        .range(-90.0..=12.0)
                                                        .speed(0.2)
                                                        .fixed_decimals(1)
                                                        .suffix(" dB"),
                                                )
                                                .changed()
                                            {
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
                                    if navigating {
                                        Sense::hover()
                                    } else {
                                        Sense::click()
                                    },
                                );
                                if grid.clicked() {
                                    if let Some(pos) = grid.interact_pointer_pos() {
                                        self.mark_grid(((pos.x - timeline_x) / self.zoom) as f64);
                                        self.selected_track = Some(t.id.clone());
                                        self.selected_clip = None;
                                        self.selected_device = None;
                                    }
                                }
                                if self.job.is_none()
                                    && matches!(t.kind, velvet_core::TrackKind::Audio)
                                    && hover.is_some_and(|p| {
                                        timeline.intersect(ui.clip_rect()).contains(p)
                                    })
                                    && egui::DragAndDrop::payload::<browser::SampleDrag>(ctx)
                                        .is_some()
                                {
                                    let pos = hover.unwrap();
                                    let beats = ((pos.x - timeline_x) / self.zoom).max(0.0) as f64;
                                    let beats = if self.snap {
                                        (beats * 4.0).round() / 4.0
                                    } else {
                                        beats
                                    };
                                    let x = timeline_x + beats as f32 * self.zoom;
                                    ui.painter().line_segment(
                                        [
                                            Pos2::new(x, timeline.top()),
                                            Pos2::new(x, timeline.bottom()),
                                        ],
                                        Stroke::new(2.0_f32, CYAN),
                                    );
                                    if ui.input(|i| i.pointer.any_released()) {
                                        let payload = egui::DragAndDrop::take_payload::<
                                            browser::SampleDrag,
                                        >(ctx)
                                        .unwrap();
                                        self.selected_track = Some(t.id.clone());
                                        self.import(
                                            payload.path.clone(),
                                            Some(t.id.clone()),
                                            beats,
                                        );
                                    }
                                }
                                self.instrument_drop(ui, row, &t.id);
                                self.midi_clip_ui(
                                    ui, ctx, t, timeline, row, timeline_x, color, navigating,
                                );
                                for c in &t.clips {
                                    let playback_rate =
                                        c.playback_rate(self.session.project.tempo.bpm);
                                    let clip_beat_seconds = beat_seconds * playback_rate;
                                    let mut position = c.position.clone();
                                    let x = timeline_x + position.start_beats as f32 * self.zoom;
                                    let rect = Rect::from_min_size(
                                        Pos2::new(x, row.top() + 5.0),
                                        Vec2::new(
                                            (position.length_seconds / clip_beat_seconds) as f32
                                                * self.zoom,
                                            height - 14.0,
                                        ),
                                    );
                                    let response = ui.interact(
                                        rect.intersect(timeline),
                                        egui::Id::new(&c.id),
                                        if navigating {
                                            Sense::hover()
                                        } else {
                                            Sense::click_and_drag()
                                        },
                                    );
                                    if response.clicked() {
                                        self.selected_track = Some(t.id.clone());
                                        self.selected_clip = Some(c.id.clone());
                                        self.selected_device = None;
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
                                                    if ctx.input(|i| i.modifiers.shift) { 3 } else { 2 }
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
                                        self.selected_device = None;
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
                                                        let seconds = (delta * clip_beat_seconds)
                                                            .clamp(
                                                                -position.offset_seconds,
                                                                position.length_seconds - 0.01,
                                                            )
                                                            .max(
                                                                -position.start_beats
                                                                    * clip_beat_seconds,
                                                            );
                                                        position.start_beats +=
                                                            seconds / clip_beat_seconds;
                                                        position.offset_seconds += seconds;
                                                        position.length_seconds -= seconds;
                                                    }
                                                    _ => {
                                                        position.length_seconds = (position
                                                            .length_seconds
                                                            + delta * clip_beat_seconds)
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
                                            } else if d.mode == 3 {
                                                self.execute(Command::SetClipTempo {
                                                    track_id: d.track, clip_id: d.clip,
                                                    source_bpm: Some(c.source_bpm.unwrap_or(self.session.project.tempo.bpm) * position.length_seconds / d.position.length_seconds),
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
                                            row.top() + 5.0,
                                        ),
                                        Vec2::new(
                                            ((position.length_seconds / clip_beat_seconds) as f32
                                                * self.zoom)
                                                .max(3.0),
                                            height - 14.0,
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
                                                0.24
                                            } else {
                                                0.14
                                            })
                                        },
                                    );
                                    p.rect_stroke(
                                        visible,
                                        3.0,
                                        Stroke::new(
                                            if selected_clip { 1.0_f32 } else { 0.5_f32 },
                                            color.gamma_multiply(if selected_clip {
                                                0.9
                                            } else {
                                                0.35
                                            }),
                                        ),
                                        egui::StrokeKind::Inside,
                                    );
                                    let label =
                                        path.file_stem().unwrap_or_default().to_string_lossy();
                                    let p =
                                        p.with_clip_rect(visible.shrink(3.0).intersect(timeline));
                                    p.text(
                                        visible.min + Vec2::new(7.0, 12.0),
                                        egui::Align2::LEFT_CENTER,
                                        if missing {
                                            format!("Missing · {label}")
                                        } else {
                                            label.into()
                                        },
                                        FontId::monospace(10.0),
                                        color,
                                    );
                                    if let Some(data) = self.cache.files.get(&path) {
                                        waveform(&p, visible, data, &position, color);
                                    }
                                    response.context_menu(|ui| {
                                        self.selected_track = Some(t.id.clone());
                                        self.selected_clip = Some(c.id.clone());
                                        self.selected_device = None;
                                        self.arrangement_menu(ui);
                                    });
                                    response.on_hover_text(format!(
                                        "{}\nDrag to move · Drag edges to trim · Shift+right edge to resample\n{}",
                                        path.display(),
                                        if missing {
                                            "Missing source — locate in clip controls"
                                        } else {
                                            "Original file is preserved"
                                        }
                                    ));
                                }
                            }
                            self.matrix.view = Some(matrix_view);
                            matrix_canvas = ui.clip_rect();
                            matrix_canvas.max.x = matrix_canvas.right().min(controls_x);
                            let end_y = ui.max_rect().bottom().max(ui.cursor().top());
                            ui.painter().rect_filled(
                                Rect::from_min_max(
                                    Pos2::new(controls_x, ui.cursor().top()),
                                    Pos2::new(ruler.right(), end_y),
                                ),
                                0.0,
                                PANEL,
                            );
                            let grid_painter = ui.painter().with_clip_rect(Rect::from_min_max(
                                ruler.min,
                                Pos2::new(controls_x, end_y),
                            ));
                            paint_musical_grid(
                                &grid_painter,
                                Rect::from_min_max(
                                    ruler.left_bottom(),
                                    Pos2::new(controls_x, end_y),
                                ),
                                timeline_x,
                                self.zoom,
                            );
                            let marker_x = timeline_x + self.grid_start_beats as f32 * self.zoom;
                            let marker = ui.painter().with_clip_rect(Rect::from_min_max(
                                ruler.min,
                                Pos2::new(controls_x, end_y),
                            ));
                            if let Some((start, end)) = self.loop_range() {
                                let left = timeline_x + (start / beat_seconds) as f32 * self.zoom;
                                let right = timeline_x + (end / beat_seconds) as f32 * self.zoom;
                                marker.line_segment(
                                    [
                                        Pos2::new(left, ruler.bottom() - 2.0),
                                        Pos2::new(right, ruler.bottom() - 2.0),
                                    ],
                                    Stroke::new(1.5_f32, CYAN.gamma_multiply(0.45)),
                                );
                            }
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
                                    Stroke::new(1.0_f32, ROSE),
                                );
                            grid_painter.line_segment(
                                [Pos2::new(x, ruler.top()), Pos2::new(x, end_y)],
                                Stroke::new(
                                    5.0_f32,
                                    Color32::from_rgba_unmultiplied(243, 134, 161, 15),
                                ),
                            );
                            grid_painter.add(egui::Shape::convex_polygon(
                                vec![
                                    Pos2::new(x - 3.5, ruler.top() + 1.0),
                                    Pos2::new(x + 3.5, ruler.top() + 1.0),
                                    Pos2::new(x, ruler.top() + 7.0),
                                ],
                                ROSE,
                                Stroke::NONE,
                            ));
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
                            if self.job.is_none()
                                && drop_target.is_none()
                                && hover.is_some_and(|p| {
                                    ui.clip_rect().contains(p)
                                        && p.x >= ruler.left()
                                        && p.x < controls_x
                                        && p.y > ruler.bottom()
                                })
                                && egui::DragAndDrop::payload::<browser::SampleDrag>(ctx).is_some()
                                && ui.input(|i| i.pointer.any_released())
                            {
                                let payload =
                                    egui::DragAndDrop::take_payload::<browser::SampleDrag>(ctx)
                                        .unwrap();
                                let beats =
                                    ((hover.unwrap().x - timeline_x) / self.zoom).max(0.0) as f64;
                                let beats = if self.snap {
                                    (beats * 4.0).round() / 4.0
                                } else {
                                    beats
                                };
                                self.execute(Command::AddTrack {
                                    name: payload
                                        .path
                                        .file_stem()
                                        .unwrap_or_default()
                                        .to_string_lossy()
                                        .into_owned(),
                                });
                                let target = self.session.project.tracks.last().unwrap().id.clone();
                                self.selected_track = Some(target.clone());
                                self.import(payload.path.clone(), Some(target), beats);
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
                    self.track_scroll = scroll.state.offset.y;
                });
                if self.matrix_enabled && matrix_canvas.is_positive() {
                    let shapes = self.matrix.paint(
                        ctx,
                        matrix_canvas,
                        self.session.project.tempo.bpm,
                        self.session
                            .transport
                            .playing
                            .then_some(self.session.transport.seconds),
                    );
                    ui.painter()
                        .with_clip_rect(matrix_canvas)
                        .set(matrix_slot, egui::Shape::Vec(shapes));
                }
            });
    }
}
fn eyebrow(ui: &mut egui::Ui, text: &str) {
    ui.label(egui::RichText::new(text).monospace().size(9.0).color(MUTED));
}
fn musical_grid_divisions(zoom: f32) -> usize {
    if zoom >= 32.0 {
        4
    } else if zoom >= 16.0 {
        2
    } else {
        1
    }
}
fn paint_musical_grid(p: &egui::Painter, rect: Rect, origin: f32, zoom: f32) {
    let divisions = musical_grid_divisions(zoom);
    let step = zoom / divisions as f32;
    let first = ((rect.left() - origin) / step).ceil().max(0.0) as usize;
    let last = ((rect.right() - origin) / step).floor().max(0.0) as usize;
    for tick in first..=last {
        let x = origin + tick as f32 * step;
        let (width, alpha): (f32, u8) = if tick % (4 * divisions) == 0 {
            (1.0, 65)
        } else if tick % divisions == 0 {
            (1.0, 8)
        } else {
            (0.5, 3)
        };
        p.line_segment(
            [Pos2::new(x, rect.top()), Pos2::new(x, rect.bottom())],
            Stroke::new(width, Color32::from_white_alpha(alpha)),
        );
    }
}
fn waveform(p: &egui::Painter, r: Rect, data: &Arc<AudioData>, pos: &Position, color: Color32) {
    let width = r.width().max(1.0);
    let bins = (width / 2.0).ceil() as usize;
    let key = egui::Id::new(("waveform_peaks", data.frames.as_ptr() as usize));
    let signature = (
        data.frames.len(),
        data.sample_rate,
        pos.offset_seconds.to_bits(),
        pos.length_seconds.to_bits(),
        width.to_bits(),
    );
    type CachedPeaks = (
        (usize, u32, u64, u64, u32),
        std::sync::Weak<AudioData>,
        Arc<Vec<f32>>,
    );
    let cached = p
        .ctx()
        .data_mut(|memory| memory.get_temp::<CachedPeaks>(key));
    let peaks = cached
        .filter(|(previous, source, _)| {
            *previous == signature
                && source
                    .upgrade()
                    .is_some_and(|source| Arc::ptr_eq(&source, data))
        })
        .map(|(_, _, peaks)| peaks)
        .unwrap_or_else(|| {
            let peaks = Arc::new(
                (0..bins)
                    .map(|bin| {
                        let sample = |pixel: f64| {
                            (pos.offset_seconds + pixel / width as f64 * pos.length_seconds)
                                * data.sample_rate as f64
                        };
                        let begin = sample((bin * 2) as f64).floor() as usize;
                        let end = sample(((bin + 1) * 2) as f64).ceil() as usize;
                        data.frames[begin.min(data.frames.len())..end.min(data.frames.len())]
                            .iter()
                            .fold(0.0_f32, |peak, frame| {
                                peak.max(frame[0].abs()).max(frame[1].abs())
                            })
                    })
                    .collect::<Vec<_>>(),
            );
            p.ctx().data_mut(|memory| {
                memory.insert_temp(key, (signature, Arc::downgrade(data), peaks.clone()))
            });
            peaks
        });
    let center = r.center().y + 11.0;
    let first = ((p.clip_rect().left() - r.left()).max(0.0) / 2.0).floor() as usize;
    let last = (((p.clip_rect().right() - r.left()).max(0.0) / 2.0).ceil() as usize).min(bins);
    for (bin, &peak) in peaks.iter().enumerate().take(last).skip(first) {
        let amp = (peak.min(1.0) * (r.height() - 26.0).max(1.0) * 0.5).max(0.5);
        let x = r.left() + bin as f32 * 2.0;
        p.line_segment(
            [Pos2::new(x, center - amp), Pos2::new(x, center + amp)],
            Stroke::new(1.0_f32, color.gamma_multiply(0.8)),
        );
    }
}
impl eframe::App for Velvet {
    fn update(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        self.poll_sample_preview();
        self.poll_note_preview();
        self.poll();
        self.poll_plugin_editor(ctx);
        self.update_live_mix();
        let loop_range = self.loop_range();
        if loop_range.is_none() {
            self.loop_clip = None;
        }
        if let Some(player) = &self.player {
            player.set_loop(loop_range);
            player.set_metronome(self.metronome.then_some(self.session.project.tempo.bpm));
            player.set_monitor_gain(self.monitor_volume, true);
        }
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
        self.shortcuts(ctx);
        self.toolbar(ctx);
        egui::TopBottomPanel::bottom("status")
            .exact_height(24.0)
            .frame(egui::Frame::new().fill(BG).inner_margin(4.0))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    let (dot, _) = ui.allocate_exact_size(Vec2::new(6.0, 6.0), Sense::hover());
                    ui.painter().circle_filled(
                        dot.center(),
                        2.0,
                        if self.error { ROSE } else { ACCENT },
                    );
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
                            ui.spacing_mut().slider_width = 74.0;
                            if ui
                                .add(egui::Slider::new(&mut volume, -90.0..=12.0).show_value(false))
                                .changed()
                            {
                                self.execute(Command::SetMasterVolume { volume_db: volume });
                            }
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
                                self.selected_device = None;
                            }
                        });
                        ui.separator();
                        ui.spacing_mut().slider_width = 74.0;
                        if ui.add(egui::Slider::new(&mut self.monitor_volume, 0.0..=1.0).show_value(false))
                            .on_hover_text("Sample preview and metronome volume").changed() {
                            if let Some(player) = &self.player { player.set_monitor_gain(self.monitor_volume, true); }
                            for player in [&self.sample_preview, &self.note_preview].into_iter().flatten() {
                                player.set_monitor_gain(self.monitor_volume, false);
                            }
                        }
                        ui.label("Preview / Metro");

                    });
                });
            });
        self.rack(ctx);
        self.browser(ctx);
        self.ai(ctx);
        self.arrangement(ctx);
        self.midi_editor(ctx);
        self.synth_editor(ctx);
        ctx.layer_painter(egui::LayerId::new(
            egui::Order::Foreground,
            egui::Id::new("monolith_border"),
        ))
        .rect_stroke(
            ctx.screen_rect().shrink(1.0),
            7.0,
            Stroke::new(1.5_f32, Color32::from_rgb(102, 86, 207)),
            egui::StrokeKind::Inside,
        );
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
        if ctx.input(|i| i.viewport().close_requested()) {
            if !self.capture_plugin_state() {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            } else if self.session.project != self.saved {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                self.closing = true;
            }
        }
        if self.closing {
            egui::Window::new("Save before closing?")
                .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
                .show(ctx, |ui| {
                    ui.label("Your project has unsaved edits.");
                    if ui.button("Save and close").clicked() {
                        self.save(false);
                        if self.session.project == self.saved && !self.error {
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
            if self.matrix_enabled || self.session.transport.playing || self.job.is_some() {
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
    fn synced_clip_loop_and_waveform_share_the_musical_grid() {
        let ctx = egui::Context::default();
        let mut app = Velvet::new(&ctx);
        app.session = Session::new(Project::new("Grid sync"), PathBuf::new());
        app.session
            .execute(Command::AddTrack {
                name: "Audio".into(),
            })
            .unwrap();
        let track = app.session.project.tracks[0].id.clone();
        app.session
            .execute(Command::ImportAudioClip {
                track_id: track.clone(),
                source: Source {
                    path: std::path::absolute("grid.wav").unwrap(),
                    kind: SourceKind::External,
                },
                position: Position {
                    start_beats: 4.0,
                    offset_seconds: 0.5,
                    length_seconds: 2.0,
                },
            })
            .unwrap();
        let clip = app.session.project.tracks[0].clips[0].id.clone();
        app.session
            .execute(Command::SetClipTempo {
                track_id: track.clone(),
                clip_id: clip.clone(),
                source_bpm: Some(120.0),
            })
            .unwrap();
        app.loop_clip = Some((track, clip));
        let mut frames = vec![[0.0; 2]; 3000];
        frames[1000] = [1.0; 2];
        let data = Arc::new(AudioData {
            sample_rate: 1000,
            frames,
        });
        for bpm in [60.0, 120.0, 240.0] {
            app.execute(Command::SetTempo { bpm });
            assert_eq!(app.loop_range(), Some((4.0 * 60.0 / bpm, 8.0 * 60.0 / bpm)));
            let clip = &app.session.project.tracks[0].clips[0];
            let zoom = 40.0;
            let rect = Rect::from_min_size(
                Pos2::new(160.0, 0.0),
                Vec2::new(
                    (clip.duration_seconds(bpm) * bpm / 60.0) as f32 * zoom,
                    80.0,
                ),
            );
            let output = ctx.run(egui::RawInput::default(), |ctx| {
                let painter = ctx.layer_painter(egui::LayerId::background());
                paint_musical_grid(&painter, rect, 0.0, zoom);
                waveform(&painter, rect, &data, &clip.position, CYAN);
            });
            assert!(output.shapes.iter().any(|shape| matches!(&shape.shape,
                egui::Shape::LineSegment { points, stroke } if stroke.color == CYAN.gamma_multiply(0.8) && points[0].x == 200.0 && (points[1].y - points[0].y).abs() > 40.0
            )), "Waveform attack missed beat 5 at {bpm} BPM");
            assert!(output.shapes.iter().any(|shape| matches!(&shape.shape,
                egui::Shape::LineSegment { points, stroke } if points[0].x == 170.0 && stroke.color == Color32::from_white_alpha(14)
            )), "Missing sixteenth-note grid");
        }
    }
    #[test]
    fn tempo_edits_and_history_keep_the_playhead_on_the_same_beat() {
        let mut app = Velvet::new(&egui::Context::default());
        app.session = Session::new(Project::new("Tempo sync"), PathBuf::new());
        app.session.transport.seconds = 4.0;
        app.execute(Command::SetTempo { bpm: 60.0 });
        assert_eq!(app.session.transport.seconds, 8.0);
        app.history(false);
        assert_eq!(app.session.transport.seconds, 4.0);
        app.history(true);
        assert_eq!(app.session.transport.seconds, 8.0);
    }
    #[test]
    fn waveform_keeps_short_audio_attacks_on_the_timeline() {
        let ctx = egui::Context::default();
        let mut frames = vec![[0.0; 2]; 4000];
        frames[31] = [1.0; 2];
        let data = Arc::new(AudioData {
            sample_rate: 1000,
            frames,
        });
        let position = Position {
            start_beats: 0.0,
            offset_seconds: 0.0,
            length_seconds: 4.0,
        };
        let output = ctx.run(egui::RawInput::default(), |ctx| {
            let painter = ctx.layer_painter(egui::LayerId::background());
            waveform(
                &painter,
                Rect::from_min_size(Pos2::ZERO, Vec2::new(40.0, 80.0)),
                &data,
                &position,
                CYAN,
            );
        });
        assert!(
            output.shapes.iter().any(|shape| matches!(&shape.shape,
                egui::Shape::LineSegment { points, .. } if (points[1].y - points[0].y).abs() > 40.0
            )),
            "A short audible attack disappeared from the audio waveform"
        );
    }
    #[test]
    #[ignore = "Requires a real default audio output device"]
    fn hardware_stale_initial_mix_is_rebuilt_after_mixer_edits() {
        let ctx = egui::Context::default();
        let mut app = Velvet::new(&ctx);
        app.session = Session::new(Project::new("Stale mix"), PathBuf::new());
        app.job = None;
        app.execute(Command::AddMidiTrack { name: "Dot".into() });
        let track_id = app.session.project.tracks[0].id.clone();
        app.execute(Command::SetTrackInstrument {
            track_id: track_id.clone(),
            kind: Some("builtin.dot".into()),
        });
        app.execute(Command::SetMidiNotes {
            track_id: track_id.clone(),
            notes: vec![velvet_core::MidiNote::default()],
        });
        let revision = app.session.revision;
        let rate = Player::output_rate().unwrap();
        let mix = velvet_audio::mix(
            &app.session.project,
            &app.session.root,
            rate,
            &mut app.cache,
        )
        .unwrap();
        assert!(mix.peak > 0.0);
        let (tx, rx) = mpsc::channel();
        app.job = Some(rx);
        tx.send(Job::Mix(revision, Ok(mix))).unwrap();
        app.execute(Command::SetMute {
            track_id,
            mute: true,
        });
        app.poll();
        assert!(
            app.player.is_none(),
            "Stale unmuted audio reached the player"
        );
        let pending = app.job.take().expect("Current mix was not scheduled");
        let Job::Mix(current, result) = pending.recv_timeout(Duration::from_secs(2)).unwrap()
        else {
            panic!("Expected a rebuilt audio mix");
        };
        assert_eq!(current, app.session.revision);
        assert_eq!(result.unwrap().peak, 0.0);
    }
    #[test]
    fn mixer_edits_during_audio_preparation_are_not_lost() {
        let ctx = egui::Context::default();
        let mut app = Velvet::new(&ctx);
        app.session = Session::new(Project::new("Pending mix"), PathBuf::new());
        app.execute(Command::AddTrack {
            name: "Audio".into(),
        });
        let (_tx, rx) = mpsc::channel();
        app.job = Some(rx);
        app.busy = "Preparing audio".into();
        let track_id = app.session.project.tracks[0].id.clone();
        for command in [
            Command::SetMute {
                track_id: track_id.clone(),
                mute: true,
            },
            Command::SetSolo {
                track_id,
                solo: true,
            },
        ] {
            app.execute(command);
            assert!(
                app.live_dirty,
                "Mixer edit was lost while preparing the initial mix"
            );
            app.edit_time = Instant::now() - Duration::from_millis(50);
            app.update_live_mix();
            assert!(
                app.live_dirty,
                "Pending edit was consumed before a player existed"
            );
            assert!(app.live_job.is_none());
        }
    }
    #[test]
    fn arrangement_mute_and_solo_clicks_change_the_audio_mix() {
        let ctx = egui::Context::default();
        let mut app = Velvet::new(&ctx);
        app.session = Session::new(Project::new("Mixer buttons"), PathBuf::new());
        app.job = None;
        for name in ["First", "Second"] {
            app.execute(Command::AddMidiTrack { name: name.into() });
        }
        for (i, track) in app.session.project.tracks.iter_mut().enumerate() {
            track.synth = Some(velvet_core::Device::new("builtin.dot").unwrap());
            track.notes.push(velvet_core::MidiNote {
                key: 60 + i as u8 * 7,
                start_beats: 0.0,
                length_beats: 1.0,
                velocity: 100,
                ..Default::default()
            });
        }
        let frame = |app: &mut Velvet, events| {
            ctx.run(input(events, egui::Modifiers::NONE), |ctx| {
                app.arrangement(ctx)
            })
        };
        frame(&mut app, vec![]);
        let output = frame(&mut app, vec![]);
        let buttons: Vec<_> = output
            .shapes
            .iter()
            .filter_map(|shape| {
                if let egui::Shape::Text(t) = &shape.shape {
                    if t.galley.job.text == "M" || t.galley.job.text == "S" {
                        return Some(t.pos + t.galley.size() * 0.5);
                    }
                }
                None
            })
            .collect();
        assert_eq!(buttons.len(), 4);
        let render = |app: &mut Velvet| {
            velvet_audio::mix(
                &app.session.project,
                &app.session.root,
                8000,
                &mut app.cache,
            )
            .unwrap()
            .frames
        };
        let full = render(&mut app);
        assert!(full.iter().flatten().any(|s| s.abs() > 0.01));
        for (button, track, solo, active) in [
            (0, 0, false, true),
            (0, 0, false, false),
            (3, 1, true, true),
            (3, 1, true, false),
            (2, 1, false, true),
            (2, 1, false, false),
            (1, 0, true, true),
            (1, 0, true, false),
        ] {
            let pos = buttons[button];
            for pressed in [true, false] {
                frame(
                    &mut app,
                    vec![
                        egui::Event::PointerMoved(pos),
                        egui::Event::PointerButton {
                            pos,
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: egui::Modifiers::NONE,
                        },
                    ],
                );
            }
            let mixer = &app.session.project.tracks[track].mixer;
            assert_eq!(
                if solo { mixer.solo } else { mixer.mute },
                active,
                "Button {button} did not toggle its track"
            );
            let audio = render(&mut app);
            assert!(
                if active { audio != full } else { audio == full },
                "Button {button} did not update the audio"
            );
        }
    }
    #[test]
    fn track_toggle_keeps_active_color_on_selected_and_hovered_headers() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        for color in [Color32::from_rgb(255, 190, 65), CYAN] {
            for hovered in [false, true] {
                let events = if hovered {
                    vec![egui::Event::PointerMoved(Pos2::new(20.0, 20.0))]
                } else {
                    vec![]
                };
                let output = ctx.run(input(events, egui::Modifiers::NONE), |ctx| {
                    egui::CentralPanel::default()
                        .frame(egui::Frame::NONE.fill(Color32::from_rgb(30, 38, 45)))
                        .show(ctx, |ui| {
                            track_toggle(ui, "M", true, color, "Mute");
                        });
                });
                assert!(output
                    .shapes
                    .iter()
                    .any(|s| matches!(&s.shape, egui::Shape::Rect(r) if r.fill == color)));
            }
        }
    }
    #[test]
    fn track_name_click_edit_commit_cancel_and_undo() {
        let ctx = egui::Context::default();
        let mut app = Velvet::new(&ctx);
        app.session = Session::new(Project::new("Rename"), PathBuf::new());
        app.job = None;
        app.execute(Command::AddTrack {
            name: "MIDI 4".into(),
        });
        let track = app.session.project.tracks[0].clone();
        let edit_id = egui::Id::new(("track_name_edit", &track.id));
        let frame = |app: &mut Velvet, events| {
            let mut rect = Rect::NOTHING;
            let _ = ctx.run(input(events, egui::Modifiers::NONE), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    rect = app.track_name_ui(ui, &track, true, CYAN).rect;
                });
            });
            rect
        };
        let pos = frame(&mut app, vec![]).center();
        for pressed in [true, false] {
            frame(
                &mut app,
                vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
        }
        assert!(ctx
            .data_mut(|d| d.get_temp::<(String, bool)>(edit_id))
            .is_some());
        frame(&mut app, vec![]);
        assert!(ctx.memory(|m| m.has_focus(edit_id)));
        ctx.data_mut(|d| d.insert_temp(edit_id, ("  Piano  ".to_string(), false)));
        frame(&mut app, vec![key(egui::Key::Enter, egui::Modifiers::NONE)]);
        assert_eq!(app.session.project.tracks[0].name, "Piano");
        app.history(false);
        assert_eq!(app.session.project.tracks[0].name, "MIDI 4");
        for (draft, end_key) in [("Cancelled", egui::Key::Escape), ("   ", egui::Key::Enter)] {
            ctx.data_mut(|d| d.insert_temp(edit_id, (draft.to_string(), true)));
            frame(&mut app, vec![]);
            frame(&mut app, vec![key(end_key, egui::Modifiers::NONE)]);
            assert_eq!(app.session.project.tracks[0].name, "MIDI 4");
            assert!(ctx
                .data_mut(|d| d.get_temp::<(String, bool)>(edit_id))
                .is_none());
        }
        ctx.data_mut(|d| d.insert_temp(edit_id, ("Bass".to_string(), true)));
        frame(&mut app, vec![]);
        let pos = Pos2::new(800.0, 200.0);
        frame(
            &mut app,
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
        assert_eq!(app.session.project.tracks[0].name, "Bass");
    }
    fn input(events: Vec<egui::Event>, modifiers: egui::Modifiers) -> egui::RawInput {
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1440.0, 900.0))),
            events,
            modifiers,
            ..Default::default()
        }
    }
    fn key(key: egui::Key, modifiers: egui::Modifiers) -> egui::Event {
        egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        }
    }
    #[test]
    fn first_play_during_startup_is_remembered_and_can_be_cancelled() {
        let ctx = egui::Context::default();
        let mut app = Velvet::new(&ctx);
        let (_tx, rx) = mpsc::channel();
        app.job = Some(rx);
        app.busy = "Reading waveforms".into();
        let _ = ctx.run(
            input(
                vec![key(egui::Key::Space, egui::Modifiers::NONE)],
                egui::Modifiers::NONE,
            ),
            |ctx| app.shortcuts(ctx),
        );
        assert!(
            app.session.transport.playing,
            "First Play was discarded during startup"
        );
        assert_eq!(
            app.busy, "Reading waveforms",
            "Play replaced the startup job"
        );
        app.toggle_playback();
        assert!(!app.session.transport.playing);
        assert!(app.job.is_some());
    }
    #[test]
    #[ignore = "Requires a real default audio output device"]
    fn startup_prepares_audio_before_first_play() {
        let mut app = Velvet::new(&egui::Context::default());
        app.session =
            Session::open(&PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/demo"))
                .unwrap();
        let (tx, rx) = mpsc::channel();
        app.job = Some(rx);
        tx.send(Job::Warm(Ok(MediaCache::default()))).unwrap();
        app.poll();
        let deadline = Instant::now() + Duration::from_secs(10);
        while app.player.is_none() && Instant::now() < deadline {
            app.poll();
            std::thread::sleep(Duration::from_millis(5));
        }
        let state = app
            .player
            .as_ref()
            .expect("Audio was not prepared before Play")
            .state
            .clone();
        assert!(!app.session.transport.playing);
        let start = Instant::now();
        app.toggle_playback();
        assert!(start.elapsed() < Duration::from_millis(50));
        assert!(app.job.is_none(), "First Play started a loading job");
        assert!(app.player.as_ref().unwrap().playing());
        assert!(Arc::ptr_eq(&state, &app.player.as_ref().unwrap().state));
    }
    #[test]
    #[ignore = "Requires a real default audio output device"]
    fn first_play_starts_after_startup_finishes() {
        let mut app = Velvet::new(&egui::Context::default());
        app.session =
            Session::open(&PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/demo"))
                .unwrap();
        let (tx, rx) = mpsc::channel();
        app.job = Some(rx);
        app.busy = "Reading waveforms".into();
        app.toggle_playback();
        assert!(app.session.transport.playing);
        assert_eq!(app.busy, "Reading waveforms");
        tx.send(Job::Warm(Ok(MediaCache::default()))).unwrap();
        app.poll();
        assert_eq!(app.busy, "Preparing audio");
        let deadline = Instant::now() + Duration::from_secs(10);
        while app.player.is_none() && Instant::now() < deadline {
            app.poll();
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(app
            .player
            .as_ref()
            .expect("First Play never created the player")
            .playing());
    }
    #[test]
    fn backspace_removes_selected_track_and_undo_restores_it() {
        let ctx = egui::Context::default();
        let mut app = Velvet::new(&ctx);
        app.session = Session::new(Project::new("Selection"), PathBuf::new());
        app.job = None;
        app.execute(Command::AddTrack {
            name: "Audio".into(),
        });
        let track = app.session.project.tracks[0].id.clone();
        app.selected_track = Some(track.clone());
        let _ = ctx.run(
            input(
                vec![key(egui::Key::Backspace, egui::Modifiers::NONE)],
                egui::Modifiers::NONE,
            ),
            |ctx| app.shortcuts(ctx),
        );
        assert!(app.session.project.tracks.is_empty());
        assert!(app.selected_track.is_none());
        app.history(false);
        assert_eq!(app.session.project.tracks[0].id, track);
        assert_eq!(app.selected_track.as_ref(), Some(&track));
        app.execute(Command::AddTrack {
            name: "Middle".into(),
        });
        app.execute(Command::AddTrack {
            name: "Last".into(),
        });
        let middle = app.session.project.tracks[1].id.clone();
        let last = app.session.project.tracks[2].id.clone();
        app.selected_track = Some(middle);
        app.remove_selection();
        assert_eq!(app.selected_track.as_ref(), Some(&last));
        app.remove_selection();
        assert_eq!(app.selected_track.as_ref(), Some(&track));
        app.selected_track = Some("master".into());
        app.remove_selection();
        assert_eq!(app.session.project.tracks.len(), 1);
    }
    #[test]
    fn loop_shortcut_follows_clip_edits_and_backspace_removes_only_the_selected_effect() {
        let ctx = egui::Context::default();
        let mut app = Velvet::new(&ctx);
        app.session = Session::new(Project::new("QoL"), PathBuf::new());
        app.job = None;
        app.execute(Command::AddTrack {
            name: "Audio".into(),
        });
        let track = app.session.project.tracks[0].id.clone();
        app.execute(Command::ImportAudioClip {
            track_id: track.clone(),
            source: Source {
                path: std::path::absolute("qol.wav").unwrap(),
                kind: SourceKind::External,
            },
            position: Position {
                start_beats: 8.0,
                offset_seconds: 1.0,
                length_seconds: 2.0,
            },
        });
        let clip = app.session.project.tracks[0].clips[0].id.clone();
        app.selected_track = Some(track.clone());
        app.selected_clip = Some(clip.clone());
        let ctrl = egui::Modifiers::CTRL;
        let _ = ctx.run(input(vec![key(egui::Key::L, ctrl)], ctrl), |ctx| {
            app.shortcuts(ctx)
        });
        let start = 8.0 * 60.0 / app.session.project.tempo.bpm;
        assert_eq!(app.loop_range(), Some((start, start + 2.0)));
        assert_eq!(app.session.transport.seconds, start);
        app.execute(Command::SetTempo { bpm: 60.0 });
        app.execute(Command::MoveClip {
            track_id: track.clone(),
            clip_id: clip.clone(),
            start_beats: 12.0,
        });
        assert_eq!(app.loop_range(), Some((12.0, 14.0)));
        let _ = ctx.run(input(vec![key(egui::Key::L, ctrl)], ctrl), |ctx| {
            app.shortcuts(ctx)
        });
        assert_eq!(app.loop_range(), None);
        app.execute(Command::AddDevice {
            track_id: track.clone(),
            kind: "builtin.gain".into(),
        });
        let device = app.session.project.tracks[0].devices[0].id.clone();
        app.selected_device = Some((track.clone(), device.clone()));
        let _ = ctx.run(
            input(
                vec![key(egui::Key::Backspace, egui::Modifiers::NONE)],
                egui::Modifiers::NONE,
            ),
            |ctx| app.shortcuts(ctx),
        );
        assert!(app.session.project.tracks[0].devices.is_empty());
        assert_eq!(app.session.project.tracks[0].clips.len(), 1);
        app.history(false);
        assert_eq!(app.session.project.tracks[0].devices[0].id, device);
        app.selected_device = Some((track, device));
        let mut text = "value".to_owned();
        let _ = ctx.run(input(vec![], egui::Modifiers::NONE), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.text_edit_singleline(&mut text).request_focus();
            });
        });
        assert!(ctx.wants_keyboard_input());
        let _ = ctx.run(
            input(
                vec![key(egui::Key::Backspace, egui::Modifiers::NONE)],
                egui::Modifiers::NONE,
            ),
            |ctx| {
                app.shortcuts(ctx);
                egui::CentralPanel::default().show(ctx, |ui| {
                    ui.text_edit_singleline(&mut text);
                });
            },
        );
        assert_eq!(app.session.project.tracks[0].devices.len(), 1);
    }
    #[test]
    fn matrix_follows_arrangement_zoom_pan_and_individual_track_heights() {
        fn frame(app: &mut Velvet, ctx: &egui::Context) -> Vec<Pos2> {
            let output = ctx.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1000.0, 650.0))),
                    time: Some(0.0),
                    ..Default::default()
                },
                |ctx| app.arrangement(ctx),
            );
            output
                .shapes
                .into_iter()
                .flat_map(|shape| match shape.shape {
                    egui::Shape::Vec(shapes) => shapes,
                    shape => vec![shape],
                })
                .filter_map(|shape| match shape {
                    egui::Shape::Mesh(mesh) => Some(
                        mesh.vertices
                            .as_chunks::<4>()
                            .0
                            .iter()
                            .map(|quad| quad[0].pos.lerp(quad[3].pos, 0.5))
                            .collect::<Vec<_>>(),
                    ),
                    _ => None,
                })
                .max_by_key(Vec::len)
                .expect("The arrangement must paint a dot matrix")
        }
        let ctx = egui::Context::default();
        let mut app = Velvet::new(&ctx);
        app.session = Session::new(Project::new("Matrix navigation"), PathBuf::new());
        app.job = None;
        for index in 0..8 {
            app.execute(Command::AddTrack {
                name: format!("Track {index}"),
            });
        }
        let track = app.session.project.tracks[0].id.clone();
        app.zoom = 44.0;
        let original = frame(&mut app, &ctx);
        assert!(
            (original[1].x - original[0].x - 11.0).abs() < 0.001,
            "The matrix must use arrangement zoom, not screen coordinates"
        );
        app.zoom = 88.0;
        let zoomed = frame(&mut app, &ctx);
        assert!((zoomed[1].x - zoomed[0].x - 22.0).abs() < 0.001);
        app.timeline_scroll = 0.125;
        let panned = frame(&mut app, &ctx);
        assert!(
            panned
                .iter()
                .any(|point| point.distance(zoomed[3] - Vec2::new(11.0, 0.0)) < 0.001),
            "Horizontal pan must move the same world grid"
        );
        let origin = app.matrix.view.as_ref().unwrap().origin;
        let point = Pos2::new(panned[0].x, origin.y + 35.0);
        assert!(panned.iter().any(|p| p.distance(point) < 0.001));
        app.track_scroll = 7.0;
        let scrolled = frame(&mut app, &ctx);
        assert!(
            scrolled
                .iter()
                .any(|p| p.distance(point - Vec2::new(0.0, 7.0)) < 0.001),
            "Vertical scroll must translate the matrix with tracks"
        );
        app.track_heights.insert(track, 152.0);
        let resized = frame(&mut app, &ctx);
        let view = app.matrix.view.as_ref().unwrap();
        let expected = Pos2::new(point.x, view.origin.y + 35.0 * 157.0 / 81.0);
        assert!(
            resized.iter().any(|p| p.distance(expected) < 0.001),
            "Vertical zoom must stretch the resized track's portion of the matrix"
        );
        assert_eq!(
            view.track_heights[1], 81.0,
            "Other tracks keep their own scale"
        );
    }
    #[test]
    fn arrangement_navigation_zooms_pans_and_resizes_tracks_without_editing_clips() {
        let ctx = egui::Context::default();
        let mut app = Velvet::new(&ctx);
        app.session = Session::new(Project::new("Navigation"), PathBuf::new());
        app.job = None;
        app.execute(Command::AddTrack {
            name: "Audio".into(),
        });
        let track = app.session.project.tracks[0].id.clone();
        app.selected_track = Some(track.clone());
        let viewport = Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 500.0));
        let pointer = Pos2::new(400.0, 200.0);
        app.timeline_scroll = 20.0;
        let beat = app.timeline_scroll + pointer.x / app.zoom;
        let wheel = |delta, modifiers| egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta,
            modifiers,
        };
        let _ = ctx.run(
            input(
                vec![
                    egui::Event::PointerMoved(pointer),
                    wheel(Vec2::new(0.0, 30.0), egui::Modifiers::CTRL),
                ],
                egui::Modifiers::CTRL,
            ),
            |ctx| app.navigate_arrangement(ctx, viewport, 1000.0),
        );
        assert!(app.zoom > 22.0);
        assert!((app.timeline_scroll + pointer.x / app.zoom - beat).abs() < 0.0001);
        let scroll = app.timeline_scroll;
        let _ = ctx.run(
            input(
                vec![wheel(Vec2::new(-4.0, 0.0), egui::Modifiers::NONE)],
                egui::Modifiers::NONE,
            ),
            |ctx| app.navigate_arrangement(ctx, viewport, 1000.0),
        );
        assert!(
            app.timeline_scroll > scroll,
            "Two-finger horizontal gesture must pan"
        );
        let scroll = app.timeline_scroll;
        let shift = egui::Modifiers::SHIFT;
        let _ = ctx.run(
            input(vec![wheel(Vec2::new(0.0, -4.0), shift)], shift),
            |ctx| app.navigate_arrangement(ctx, viewport, 1000.0),
        );
        assert!(
            app.timeline_scroll > scroll,
            "Shift+wheel must pan horizontally"
        );
        let zoom = app.zoom;
        let _ = ctx.run(
            input(
                vec![key(egui::Key::Plus, egui::Modifiers::NONE)],
                egui::Modifiers::NONE,
            ),
            |ctx| app.navigate_arrangement(ctx, viewport, 1000.0),
        );
        assert!(app.zoom > zoom);
        let pan = egui::Modifiers {
            ctrl: true,
            alt: true,
            ..Default::default()
        };
        let button = |pos, pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: pan,
        };
        let scroll = app.timeline_scroll;
        let _ = ctx.run(
            input(
                vec![egui::Event::PointerMoved(pointer), button(pointer, true)],
                pan,
            ),
            |ctx| app.navigate_arrangement(ctx, viewport, 1000.0),
        );
        let end = pointer - Vec2::new(50.0, 40.0);
        let _ = ctx.run(input(vec![egui::Event::PointerMoved(end)], pan), |ctx| {
            app.navigate_arrangement(ctx, viewport, 1000.0)
        });
        assert!(app.timeline_scroll > scroll);
        assert_eq!(app.track_scroll, 40.0);
        let _ = ctx.run(input(vec![button(end, false)], pan), |ctx| {
            app.navigate_arrangement(ctx, viewport, 1000.0)
        });
        assert!(app.timeline_pan.is_none());
        let _ = ctx.run(input(vec![], egui::Modifiers::NONE), |ctx| {
            app.arrangement(ctx)
        });
        let output = ctx.run(input(vec![], egui::Modifiers::NONE), |ctx| {
            app.arrangement(ctx)
        });
        let mixer = output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Text(t) if t.galley.job.text == "TRACK / MIXER" => Some(t.pos),
                _ => None,
            })
            .unwrap();
        let lane = Pos2::new(200.0, mixer.y + 65.0);
        let before = app.session.project.clone();
        let alt = egui::Modifiers::ALT;
        let _ = ctx.run(
            input(
                vec![
                    egui::Event::PointerMoved(lane),
                    wheel(Vec2::new(0.0, 30.0), alt),
                ],
                alt,
            ),
            |ctx| app.arrangement(ctx),
        );
        assert!(app.track_heights[&track] > 76.0);
        assert_eq!(
            app.track_scroll, 0.0,
            "Alt+wheel must resize without scrolling"
        );
        assert_eq!(app.session.project, before);
    }
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
                    device_signals: Default::default(),
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
