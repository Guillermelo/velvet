//! The only mutable project authority shared by GUI, CLI and AI.
use anyhow::{bail, ensure, Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashSet},
    fs,
    io::Write,
    path::{Component, Path, PathBuf},
};

pub mod beat;

pub fn id(prefix: &str) -> String {
    format!("{prefix}_{}", uuid::Uuid::new_v4().simple())
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Project {
    pub format: u32,
    pub project: Identity,
    pub audio: AudioSettings,
    pub tempo: Tempo,
    #[serde(default)]
    pub master: Mixer,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub master_devices: Vec<Device>,
    #[serde(default)]
    pub tracks: Vec<Track>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Identity {
    pub id: String,
    pub name: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct AudioSettings {
    pub sample_rate: u32,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Tempo {
    pub bpm: f64,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Track {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub midi_region: Option<MidiRegion>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub synth: Option<Device>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<MidiNote>,
    pub id: String,
    pub name: String,
    #[serde(rename = "type")]
    pub kind: TrackKind,
    #[serde(default = "default_color")]
    pub color: [u8; 3],
    #[serde(default)]
    pub mixer: Mixer,
    #[serde(default)]
    pub clips: Vec<Clip>,
    #[serde(default)]
    pub devices: Vec<Device>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct MidiNote {
    pub key: u8,
    pub velocity: u8,
    pub start_beats: f64,
    pub length_beats: f64,
    #[serde(default)]
    pub muted: bool,
    #[serde(default = "default_midi_channel")]
    pub channel: u8,
}
fn default_midi_channel() -> u8 {
    1
}
impl MidiNote {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.key <= 127
                && (1..=127).contains(&self.velocity)
                && (1..=16).contains(&self.channel)
                && self.start_beats.is_finite()
                && self.start_beats >= 0.0
                && self.length_beats.is_finite()
                && self.length_beats > 0.0,
            "Invalid MIDI note"
        );
        Ok(())
    }
}
impl Default for MidiNote {
    fn default() -> Self {
        Self {
            key: 60,
            velocity: 100,
            start_beats: 0.0,
            length_beats: 1.0,
            muted: false,
            channel: 1,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct MidiRegion {
    pub start_beats: f64,
    pub offset_beats: f64,
    pub length_beats: f64,
}
impl Track {
    pub fn midi_region(&self) -> Option<MidiRegion> {
        if !matches!(self.kind, TrackKind::Midi) {
            return None;
        }
        if let Some(region) = &self.midi_region {
            return Some(region.clone());
        }
        if self.notes.is_empty() {
            return None;
        }
        let start = (self
            .notes
            .iter()
            .map(|n| n.start_beats)
            .fold(f64::INFINITY, f64::min)
            / 4.0)
            .floor()
            * 4.0;
        let end = (self
            .notes
            .iter()
            .map(|n| n.start_beats + n.length_beats)
            .fold(0.0, f64::max)
            / 4.0)
            .ceil()
            * 4.0;
        Some(MidiRegion {
            start_beats: start,
            offset_beats: start,
            length_beats: end - start,
        })
    }
    pub fn arranged_midi_notes(&self) -> Vec<MidiNote> {
        let Some(region) = self.midi_region() else {
            return Vec::new();
        };
        self.notes
            .iter()
            .filter(|n| !n.muted)
            .filter_map(|n| {
                let start = n.start_beats.max(region.offset_beats);
                let end =
                    (n.start_beats + n.length_beats).min(region.offset_beats + region.length_beats);
                (end > start).then(|| MidiNote {
                    start_beats: start - region.offset_beats + region.start_beats,
                    length_beats: end - start,
                    ..n.clone()
                })
            })
            .collect()
    }
}
fn default_color() -> [u8; 3] {
    [120, 157, 156]
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum TrackKind {
    Audio,
    Midi,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct Mixer {
    pub volume_db: f64,
    pub pan: f64,
    pub mute: bool,
    pub solo: bool,
}
impl Default for Mixer {
    fn default() -> Self {
        Self {
            volume_db: 0.0,
            pan: 0.0,
            mute: false,
            solo: false,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Clip {
    pub id: String,
    pub source: Source,
    pub position: Position,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_bpm: Option<f64>,
}
impl Clip {
    pub fn playback_rate(&self, bpm: f64) -> f64 {
        self.source_bpm.map_or(1.0, |source| bpm / source)
    }
    pub fn duration_seconds(&self, bpm: f64) -> f64 {
        self.position.length_seconds / self.playback_rate(bpm)
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Source {
    pub path: PathBuf,
    pub kind: SourceKind,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    External,
    Project,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Position {
    pub start_beats: f64,
    pub offset_seconds: f64,
    pub length_seconds: f64,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Device {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub parameters: BTreeMap<String, f64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub plugin_state: Vec<u8>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub beat_envelopes: BTreeMap<String, Vec<beat::BeatPoint>>,
}
impl Device {
    pub fn plugin_path(&self) -> Option<&Path> {
        self.kind
            .strip_prefix("vst3.effect:")
            .or_else(|| self.kind.strip_prefix("vst3.instrument:"))
            .map(Path::new)
    }
    pub fn display_name(&self) -> String {
        if let Some(path) = self.plugin_path() {
            return path
                .file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();
        }
        if self.kind == "builtin.dot" {
            return "Dot".into();
        }
        BUILTIN_DEVICES
            .iter()
            .find(|(_, k, _)| *k == self.kind)
            .map_or(self.kind.clone(), |(name, _, _)| (*name).into())
    }
    pub fn new(kind: &str) -> Result<Self> {
        let defaults: &[(&str, f64)] = match kind {
            "builtin.dot" => &[
                ("wave_type", 0.0),
                ("gain_db", -12.0),
                ("cutoff_freq_hz", 6000.0),
                ("attack_ms", 10.0),
                ("decay_ms", 180.0),
                ("sustain", 0.65),
                ("release_ms", 250.0),
            ],
            "builtin.beat" => &[
                ("time_slot", 0.0),
                ("volume_slot", 0.0),
                ("time_mix", 1.0),
                ("volume_mix", 1.0),
                ("mix", 1.0),
                ("loop_beats", 4.0),
                ("attack_ms", 1.0),
                ("release_ms", 10.0),
                ("smooth_ms", 2.0),
                ("offset_beats", 0.0),
                ("hold_enabled", 0.0),
                ("link_enabled", 0.0),
                ("bypass_enabled", 0.0),
                ("tension", 0.0),
            ],
            "builtin.gain" => &[("gain_db", 0.0)],
            "builtin.eq" => &[
                ("low_gain_db", 0.0),
                ("mid_gain_db", 0.0),
                ("high_gain_db", 0.0),
            ],
            "builtin.eq8" => &[("output_gain_db", 0.0)],
            "builtin.compressor" => &[
                ("threshold_db", -18.0),
                ("ratio", 4.0),
                ("attack_ms", 10.0),
                ("release_ms", 120.0),
                ("knee_db", 6.0),
                ("makeup_db", 0.0),
            ],
            "builtin.limiter" => &[
                ("input_gain_db", 0.0),
                ("ceiling_db", -1.0),
                ("release_ms", 80.0),
            ],
            k if k.starts_with("vst3.effect:") || k.starts_with("vst3.instrument:") => &[],
            _ => bail!("Unknown device: {kind}"),
        };
        let mut parameters: BTreeMap<_, _> =
            defaults.iter().map(|(s, v)| (s.to_string(), *v)).collect();
        if kind == "builtin.eq8" {
            for (i, frequency) in [40.0, 150.0, 1000.0, 5000.0, 250.0, 2500.0, 10000.0, 16000.0]
                .into_iter()
                .enumerate()
            {
                for (suffix, value) in [
                    ("freq_hz", frequency),
                    ("gain_db", 0.0),
                    ("q", 0.707),
                    ("type", 0.0),
                    ("enabled", if i < 4 { 1.0 } else { 0.0 }),
                ] {
                    parameters.insert(format!("band{}_{suffix}", i + 1), value);
                }
            }
        }
        Ok(Self {
            id: id("device"),
            kind: kind.into(),
            parameters,
            plugin_state: vec![],
            beat_envelopes: BTreeMap::new(),
        })
    }
    pub fn parameter_range(&self, parameter: &str) -> Result<(f64, f64)> {
        ensure!(
            self.parameters.contains_key(parameter),
            "Unknown device parameter: {parameter}"
        );
        if self.kind == "builtin.beat" {
            return Ok(match parameter {
                "time_slot" | "volume_slot" => (0.0, 35.0),
                "loop_beats" => (0.25, 8.0),
                "attack_ms" => (0.0, 500.0),
                "release_ms" => (0.0, 1000.0),
                "smooth_ms" => (0.0, 50.0),
                "offset_beats" => (0.0, 8.0),
                "tension" => (-1.0, 1.0),
                _ => (0.0, 1.0),
            });
        }
        Ok(match parameter {
            "wave_type" => (0.0, 3.0),
            "sustain" => (0.0, 1.0),
            "decay_ms" => (1.0, 2000.0),
            "threshold_db" => (-60.0, 0.0),
            "ratio" => (1.0, 20.0),
            "attack_ms" => (0.1, 200.0),
            "release_ms" => (5.0, 2000.0),
            "knee_db" => (0.0, 24.0),
            "ceiling_db" => (-24.0, 0.0),
            p if p.ends_with("_freq_hz") => (20.0, 20000.0),
            p if p.ends_with("_q") => (0.1, 18.0),
            p if p.ends_with("_type") => (0.0, 5.0),
            p if p.ends_with("_enabled") => (0.0, 1.0),
            _ => (-24.0, 24.0),
        })
    }
    fn validate(&self) -> Result<()> {
        ensure!(
            self.kind == "builtin.beat" || self.beat_envelopes.is_empty(),
            "Envelopes require Beat"
        );
        ensure!(
            self.beat_envelopes.len() <= 72,
            "Beat has 72 envelope slots"
        );
        for (key, points) in &self.beat_envelopes {
            let (lane, slot) = key.split_once(':').context("Invalid envelope slot")?;
            ensure!(
                matches!(lane, "time" | "volume") && slot.parse::<usize>().is_ok_and(|s| s < 36),
                "Invalid envelope slot"
            );
            ensure!(
                key == &format!("{lane}:{}", slot.parse::<usize>()?),
                "Invalid envelope slot spelling"
            );
            ensure!(
                (2..=256).contains(&points.len()),
                "Envelope needs 2–256 points"
            );
            ensure!(
                points[0].x == 0.0 && points.last().unwrap().x == 1.0,
                "Envelope endpoints must be 0 and 1"
            );
            for point in points {
                ensure!(
                    point.x.is_finite()
                        && (0.0..=1.0).contains(&point.x)
                        && point.y.is_finite()
                        && (0.0..=if lane == "time" { 2.0 } else { 1.0 }).contains(&point.y)
                        && point.curve <= 2,
                    "Invalid envelope point"
                );
            }
            ensure!(
                points.windows(2).all(|p| p[0].x < p[1].x),
                "Envelope points must be ordered"
            );
        }
        if let Some(path) = self.plugin_path() {
            ensure!(
                path.is_absolute()
                    && path
                        .extension()
                        .is_some_and(|e| e.eq_ignore_ascii_case("vst3")),
                "VST3 path must be absolute and end in .vst3"
            );
            ensure!(
                self.parameters.is_empty(),
                "VST3 settings are stored in plugin state"
            );
            ensure!(
                self.plugin_state.len() <= 16 * 1024 * 1024,
                "Plugin state exceeds 16 MiB"
            );
            return Ok(());
        }
        ensure!(
            self.plugin_state.is_empty(),
            "Built-in devices cannot contain plugin state"
        );
        let prototype = Self::new(&self.kind)?;
        ensure!(
            self.parameters.keys().eq(prototype.parameters.keys()),
            "Invalid parameters for {}",
            self.kind
        );
        for (key, value) in &self.parameters {
            let (min, max) = self.parameter_range(key)?;
            ensure!(
                value.is_finite() && (min..=max).contains(value),
                "{key} must be {min}–{max}"
            );
            if key.ends_with("_type") || key.ends_with("_enabled") || key.ends_with("_slot") {
                ensure!(value.fract() == 0.0, "{key} must be an integer");
            }
        }
        Ok(())
    }
}
pub const BUILTIN_DEVICES: &[(&str, &str, &str)] = &[
    ("Beat", "builtin.beat", "Time / volume shaping · 72 slots"),
    ("Gain", "builtin.gain", "Level control"),
    ("EQ Eight", "builtin.eq8", "Eight parametric bands"),
    (
        "Compressor",
        "builtin.compressor",
        "Stereo dynamics · attack / release",
    ),
    (
        "Limiter",
        "builtin.limiter",
        "Peak ceiling · 5 ms lookahead",
    ),
    ("EQ Three", "builtin.eq", "Three broad bands"),
];
impl Project {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            format: 1,
            project: Identity {
                id: id("project"),
                name: name.into(),
            },
            audio: AudioSettings { sample_rate: 48000 },
            tempo: Tempo { bpm: 120.0 },
            master: Mixer::default(),
            master_devices: vec![],
            tracks: vec![],
        }
    }
    pub fn track(&self, track_id: &str) -> Result<&Track> {
        self.tracks
            .iter()
            .find(|t| t.id == track_id)
            .context("Track not found")
    }
    /// The reserved target "master" addresses the post-fader master chain.
    pub fn devices(&self, target: &str) -> Result<&[Device]> {
        if target == "master" {
            Ok(&self.master_devices)
        } else {
            Ok(&self.track(target)?.devices)
        }
    }
    fn devices_mut(&mut self, target: &str) -> Result<&mut Vec<Device>> {
        if target == "master" {
            Ok(&mut self.master_devices)
        } else {
            Ok(&mut self.track_mut(target)?.devices)
        }
    }
    fn track_mut(&mut self, track_id: &str) -> Result<&mut Track> {
        self.tracks
            .iter_mut()
            .find(|t| t.id == track_id)
            .context("Track not found")
    }
    pub fn source_path(&self, root: &Path, source: &Source) -> PathBuf {
        match source.kind {
            SourceKind::External => source.path.clone(),
            SourceKind::Project => root.join(&source.path),
        }
    }
    pub fn missing(&self, root: &Path) -> Vec<(String, PathBuf)> {
        self.tracks
            .iter()
            .flat_map(|t| &t.clips)
            .filter_map(|c| {
                let p = self.source_path(root, &c.source);
                (!p.is_file()).then(|| (c.id.clone(), p))
            })
            .collect()
    }
    pub fn duration_seconds(&self) -> f64 {
        let duration = self
            .tracks
            .iter()
            .flat_map(|t| &t.clips)
            .map(|c| {
                c.position.start_beats * 60.0 / self.tempo.bpm + c.duration_seconds(self.tempo.bpm)
            })
            .chain(self.tracks.iter().flat_map(|t| {
                t.arranged_midi_notes().into_iter().map(move |n| {
                    (n.start_beats + n.length_beats) * 60.0 / self.tempo.bpm
                        + t.synth.as_ref().map_or(0.0, |s| {
                            s.parameters.get("release_ms").copied().unwrap_or(2000.0) / 1000.0
                        })
                })
            }))
            .chain(
                self.tracks
                    .iter()
                    .filter_map(|t| t.midi_region.clone())
                    .map(|r| (r.start_beats + r.length_beats) * 60.0 / self.tempo.bpm),
            )
            .fold(0.0, f64::max);
        // ponytail: reserve a bounded effect tail; expose render-tail settings for long reverbs.
        if duration > 0.0
            && self
                .tracks
                .iter()
                .flat_map(|t| &t.devices)
                .chain(&self.master_devices)
                .any(|d| d.plugin_path().is_some())
        {
            duration + 2.0
        } else {
            duration
        }
    }
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.format == 1,
            "Unsupported project format {} (supported: 1)",
            self.format
        );
        ensure!(
            !self.project.name.trim().is_empty(),
            "Project name is empty"
        );
        ensure!(
            (8000..=192000).contains(&self.audio.sample_rate),
            "Sample rate must be 8000–192000 Hz"
        );
        ensure!(
            self.tempo.bpm.is_finite() && (20.0..=400.0).contains(&self.tempo.bpm),
            "Tempo must be 20–400 BPM"
        );
        let mut ids = HashSet::new();
        let mut check_id = |s: &str| -> Result<()> {
            ensure!(
                !s.trim().is_empty() && ids.insert(s.to_string()),
                "Empty or duplicate ID: {s}"
            );
            Ok(())
        };
        check_id(&self.project.id)?;
        check_mixer(&self.master)?;
        for t in &self.tracks {
            check_id(&t.id)?;
            ensure!(t.id != "master", "Track ID 'master' is reserved");
            ensure!(!t.name.trim().is_empty(), "Track name is empty");
            check_mixer(&t.mixer)?;
            ensure!(t.notes.len() <= 10000, "Too many MIDI notes");
            ensure!(
                (t.notes.is_empty() && t.synth.is_none()) || matches!(t.kind, TrackKind::Midi),
                "Only MIDI tracks can contain notes or instruments"
            );
            if let Some(r) = &t.midi_region {
                ensure!(
                    matches!(t.kind, TrackKind::Midi),
                    "Region requires MIDI track"
                );
                ensure!(
                    r.start_beats.is_finite()
                        && r.start_beats >= 0.0
                        && r.offset_beats.is_finite()
                        && r.offset_beats >= 0.0
                        && r.length_beats.is_finite()
                        && r.length_beats >= 0.01,
                    "Invalid MIDI region"
                );
            }
            if let Some(s) = &t.synth {
                check_id(&s.id)?;
                ensure!(
                    s.kind == "builtin.dot" || s.kind.starts_with("vst3.instrument:"),
                    "Invalid instrument"
                );
                s.validate()?;
            }
            for n in &t.notes {
                n.validate()?;
            }
            for c in &t.clips {
                ensure!(
                    c.source_bpm
                        .is_none_or(|bpm| bpm.is_finite() && (20.0..=400.0).contains(&bpm)),
                    "Invalid clip source tempo"
                );
                check_id(&c.id)?;
                for n in [
                    c.position.start_beats,
                    c.position.offset_seconds,
                    c.position.length_seconds,
                ] {
                    ensure!(n.is_finite() && n >= 0.0, "Invalid clip position");
                }
                ensure!(
                    c.position.length_seconds > 0.0,
                    "Clip length must be positive"
                );
                ensure!(!c.source.path.as_os_str().is_empty(), "Empty media path");
                if c.source.kind == SourceKind::Project {
                    ensure!(
                        c.source
                            .path
                            .components()
                            .all(|p| matches!(p, Component::Normal(_) | Component::CurDir)),
                        "Project media path must stay inside the project"
                    );
                } else {
                    // Recognize foreign absolute paths so a Windows project remains loadable on Linux.
                    let text = c.source.path.to_string_lossy();
                    ensure!(
                        c.source.path.is_absolute()
                            || text.starts_with('/')
                            || text.starts_with("\\\\")
                            || (text.len() > 2
                                && text.as_bytes()[1] == b':'
                                && matches!(text.as_bytes()[2], b'/' | b'\\')),
                        "External media must have an absolute path"
                    );
                }
            }
        }
        for d in self
            .tracks
            .iter()
            .flat_map(|t| &t.devices)
            .chain(&self.master_devices)
        {
            check_id(&d.id)?;
            ensure!(
                d.kind != "builtin.dot" && !d.kind.starts_with("vst3.instrument:"),
                "Instrument belongs in the instrument slot"
            );
            d.validate()?;
        }
        Ok(())
    }
}
fn check_mixer(m: &Mixer) -> Result<()> {
    ensure!(
        m.volume_db.is_finite() && (-90.0..=12.0).contains(&m.volume_db),
        "Volume must be -90–12 dB"
    );
    ensure!(
        m.pan.is_finite() && (-1.0..=1.0).contains(&m.pan),
        "Pan must be -1–1"
    );
    Ok(())
}

pub fn load(root: &Path) -> Result<Project> {
    let text = fs::read_to_string(root.join("project.yaml")).context("Cannot read project.yaml")?;
    let p: Project = serde_yaml::from_str(&text).context("Invalid project YAML")?;
    p.validate()?;
    Ok(p)
}
pub fn save(root: &Path, project: &Project) -> Result<()> {
    ensure!(
        !root.as_os_str().is_empty(),
        "Choose a project folder before saving"
    );
    project.validate()?;
    fs::create_dir_all(root)?;
    for dir in ["recordings", "generated", "cache", "renders"] {
        fs::create_dir_all(root.join(dir))?;
    }
    let yaml = serde_yaml::to_string(project)?;
    let check: Project = serde_yaml::from_str(&yaml)?;
    check.validate()?;
    atomic_write(&root.join("project.yaml"), yaml.as_bytes())
}
/// Same-directory temporary file + sync + atomic replacement on Windows and Unix.
pub fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(parent)?;
    let mut temp = tempfile::NamedTempFile::new_in(parent)?;
    temp.write_all(bytes)?;
    temp.as_file().sync_all()?;
    temp.persist(path)
        .map_err(|e| e.error)
        .context("Atomic file replacement failed")?;
    #[cfg(unix)]
    fs::File::open(parent)?.sync_all()?;
    Ok(())
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum Command {
    SetArrangementTracks { tracks: Vec<Track> },
    SetTrackInstrument {
        track_id: String,
        kind: Option<String>,
    },
    AddMidiTrack {
        name: String,
    },
    SetMidiRegion {
        track_id: String,
        region: MidiRegion,
    },
    SetMidiNotes {
        track_id: String,
        notes: Vec<MidiNote>,
    },
    SetMidiScore {
        track_id: String,
        notes: Vec<MidiNote>,
        region: Option<MidiRegion>,
    },
    SetPluginState {
        track_id: String,
        device_id: String,
        state: Vec<u8>,
    },
    SetSynthParameter {
        track_id: String,
        parameter: String,
        value: f64,
    },
    AddTrack {
        name: String,
    },
    RemoveTrack {
        track_id: String,
    },
    RenameTrack {
        track_id: String,
        name: String,
    },
    SetTrackVolume {
        track_id: String,
        volume_db: f64,
    },
    SetTrackPan {
        track_id: String,
        pan: f64,
    },
    SetMute {
        track_id: String,
        mute: bool,
    },
    SetSolo {
        track_id: String,
        solo: bool,
    },
    SetTrackColor {
        track_id: String,
        color: [u8; 3],
    },
    SetMasterVolume {
        volume_db: f64,
    },
    SetTempo {
        bpm: f64,
    },
    RenameProject {
        name: String,
    },
    ImportAudioClip {
        track_id: String,
        source: Source,
        position: Position,
    },
    RemoveClip {
        track_id: String,
        clip_id: String,
    },
    MoveClip {
        track_id: String,
        clip_id: String,
        start_beats: f64,
    },
    TrimClip {
        track_id: String,
        clip_id: String,
        offset_seconds: f64,
        length_seconds: f64,
    },
    SetClipPosition {
        track_id: String,
        clip_id: String,
        position: Position,
    },
    SetClipTempo {
        track_id: String,
        clip_id: String,
        source_bpm: Option<f64>,
    },
    RelinkClip {
        track_id: String,
        clip_id: String,
        source: Source,
    },
    AddDevice {
        track_id: String,
        kind: String,
    },
    RemoveDevice {
        track_id: String,
        device_id: String,
    },
    MoveDevice {
        track_id: String,
        device_id: String,
        index: usize,
    },
    SetBeatEnvelope {
        track_id: String,
        device_id: String,
        lane: String,
        slot: usize,
        /// None restores the factory envelope.
        points: Option<Vec<beat::BeatPoint>>,
    },
    SetDeviceParameter {
        track_id: String,
        device_id: String,
        parameter: String,
        value: f64,
    },
    Play,
    Pause,
    Stop,
    Seek {
        seconds: f64,
    },
    SaveProject,
    RenderProject {
        path: PathBuf,
    },
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Transport {
    pub playing: bool,
    pub seconds: f64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HistoryEntry {
    pub label: String,
    pub before: Project,
    pub after: Project,
}
#[derive(Clone, Debug)]
pub enum Effect {
    None,
    Transport,
    Saved,
    Render(PathBuf),
}
#[derive(Clone, Debug)]
pub struct Session {
    pub project: Project,
    pub root: PathBuf,
    pub transport: Transport,
    pub history: Vec<HistoryEntry>,
    pub redo_history: Vec<HistoryEntry>,
    pub revision: u64,
}
impl Session {
    pub fn new(project: Project, root: PathBuf) -> Self {
        Self {
            project,
            root,
            transport: Transport::default(),
            history: vec![],
            redo_history: vec![],
            revision: 0,
        }
    }
    pub fn open(root: &Path) -> Result<Self> {
        Ok(Self::new(load(root)?, root.to_path_buf()))
    }
    pub fn execute(&mut self, command: Command) -> Result<Effect> {
        match &command {
            Command::SaveProject => {
                save(&self.root, &self.project)?;
                return Ok(Effect::Saved);
            }
            Command::RenderProject { path } => return Ok(Effect::Render(path.clone())),
            Command::Play => {
                self.transport.playing = true;
                return Ok(Effect::Transport);
            }
            Command::Pause => {
                self.transport.playing = false;
                return Ok(Effect::Transport);
            }
            Command::Stop => {
                self.transport = Transport::default();
                return Ok(Effect::Transport);
            }
            Command::Seek { seconds } => {
                ensure!(
                    seconds.is_finite() && *seconds >= 0.0,
                    "Invalid seek position"
                );
                self.transport.seconds = *seconds;
                return Ok(Effect::Transport);
            }
            _ => {}
        }
        let before = self.project.clone();
        let mut p = before.clone();
        let label = match &command {
            Command::SetArrangementTracks { .. } => "Edit arrangement".into(),
            Command::SetPluginState { track_id, .. } => format!("{track_id}: update VST3 settings"),
            Command::SetTrackVolume {
                track_id,
                volume_db,
            } => {
                let t = before.track(track_id)?;
                format!(
                    "{}: volume {:.1} → {:.1} dB",
                    t.name, t.mixer.volume_db, volume_db
                )
            }
            Command::SetTrackPan { track_id, pan } => {
                let t = before.track(track_id)?;
                format!("{}: pan {:.2} → {:.2}", t.name, t.mixer.pan, pan)
            }
            Command::SetMute { track_id, mute } => format!(
                "{}: mute {} → {}",
                before.track(track_id)?.name,
                before.track(track_id)?.mixer.mute,
                mute
            ),
            Command::SetSolo { track_id, solo } => format!(
                "{}: solo {} → {}",
                before.track(track_id)?.name,
                before.track(track_id)?.mixer.solo,
                solo
            ),
            Command::SetDeviceParameter {
                track_id,
                device_id,
                parameter,
                value,
            } => {
                let name = if track_id == "master" {
                    "Master"
                } else {
                    &before.track(track_id)?.name
                };
                let d = before
                    .devices(track_id)?
                    .iter()
                    .find(|d| d.id == *device_id)
                    .context("Device not found")?;
                let previous = d
                    .parameters
                    .get(parameter)
                    .context("Unknown device parameter")?;
                format!(
                    "{} / {}: {} {:.2} → {:.2}",
                    name, d.kind, parameter, previous, value
                )
            }
            _ => serde_json::to_string(&command)?,
        };
        let clip_mut = |t: &mut Track, cid: &str| -> Result<usize> {
            t.clips
                .iter()
                .position(|c| c.id == cid)
                .context("Clip not found")
        };
        match command {
            Command::SetArrangementTracks { tracks } => p.tracks = tracks,
            Command::SetTrackInstrument { track_id, kind } => {
                let t = p.track_mut(&track_id)?;
                ensure!(
                    matches!(t.kind, TrackKind::Midi),
                    "Instruments require a MIDI track"
                );
                t.synth = kind.map(|k| Device::new(&k)).transpose()?;
            }
            Command::AddMidiTrack { name } => p.tracks.push(Track {
                id: id("track"),
                name,
                kind: TrackKind::Midi,
                synth: None,
                notes: vec![],
                midi_region: None,
                color: [160, 214, 230],
                mixer: Mixer::default(),
                clips: vec![],
                devices: vec![],
            }),
            Command::SetMidiRegion { track_id, region } => {
                let t = p.track_mut(&track_id)?;
                ensure!(matches!(t.kind, TrackKind::Midi), "Track is not MIDI");
                t.midi_region = Some(region);
            }
            Command::SetMidiNotes { track_id, notes } => {
                let t = p.track_mut(&track_id)?;
                ensure!(matches!(t.kind, TrackKind::Midi), "Track is not MIDI");
                if notes.is_empty() {
                    t.midi_region = None;
                }
                t.notes = notes;
            }
            Command::SetMidiScore {
                track_id,
                notes,
                region,
            } => {
                let t = p.track_mut(&track_id)?;
                ensure!(matches!(t.kind, TrackKind::Midi), "Track is not MIDI");
                t.midi_region = if notes.is_empty() { None } else { region };
                t.notes = notes;
            }
            Command::SetSynthParameter {
                track_id,
                parameter,
                value,
            } => {
                let s = p
                    .track_mut(&track_id)?
                    .synth
                    .as_mut()
                    .context("Track has no instrument")?;
                s.parameter_range(&parameter)?;
                s.parameters.insert(parameter, value);
            }
            Command::SetPluginState {
                track_id,
                device_id,
                state,
            } => {
                let d = if track_id != "master"
                    && p.track(&track_id)?
                        .synth
                        .as_ref()
                        .is_some_and(|d| d.id == device_id)
                {
                    p.track_mut(&track_id)?.synth.as_mut().unwrap()
                } else {
                    p.devices_mut(&track_id)?
                        .iter_mut()
                        .find(|d| d.id == device_id)
                        .context("Device not found")?
                };
                ensure!(d.plugin_path().is_some(), "Device is not a VST3 plugin");
                d.plugin_state = state;
            }
            Command::AddTrack { name } => p.tracks.push(Track {
                id: id("track"),
                name,
                kind: TrackKind::Audio,
                synth: None,
                notes: vec![],
                midi_region: None,
                color: default_color(),
                mixer: Mixer::default(),
                clips: vec![],
                devices: vec![],
            }),
            Command::RemoveTrack { track_id } => {
                p.track(&track_id)?;
                p.tracks.retain(|t| t.id != track_id);
            }
            Command::RenameTrack { track_id, name } => p.track_mut(&track_id)?.name = name,
            Command::SetTrackVolume {
                track_id,
                volume_db,
            } => p.track_mut(&track_id)?.mixer.volume_db = volume_db,
            Command::SetTrackPan { track_id, pan } => p.track_mut(&track_id)?.mixer.pan = pan,
            Command::SetMute { track_id, mute } => p.track_mut(&track_id)?.mixer.mute = mute,
            Command::SetSolo { track_id, solo } => p.track_mut(&track_id)?.mixer.solo = solo,
            Command::SetTrackColor { track_id, color } => p.track_mut(&track_id)?.color = color,
            Command::SetMasterVolume { volume_db } => p.master.volume_db = volume_db,
            Command::SetTempo { bpm } => p.tempo.bpm = bpm,
            Command::RenameProject { name } => p.project.name = name,
            Command::ImportAudioClip {
                track_id,
                source,
                position,
            } => p.track_mut(&track_id)?.clips.push(Clip {
                id: id("clip"),
                source,
                position,
                source_bpm: None,
            }),
            Command::RemoveClip { track_id, clip_id } => {
                let t = p.track_mut(&track_id)?;
                let i = clip_mut(t, &clip_id)?;
                t.clips.remove(i);
            }
            Command::MoveClip {
                track_id,
                clip_id,
                start_beats,
            } => {
                let t = p.track_mut(&track_id)?;
                let i = clip_mut(t, &clip_id)?;
                t.clips[i].position.start_beats = start_beats;
            }
            Command::TrimClip {
                track_id,
                clip_id,
                offset_seconds,
                length_seconds,
            } => {
                let t = p.track_mut(&track_id)?;
                let i = clip_mut(t, &clip_id)?;
                t.clips[i].position.offset_seconds = offset_seconds;
                t.clips[i].position.length_seconds = length_seconds;
            }
            Command::SetClipPosition {
                track_id,
                clip_id,
                position,
            } => {
                let t = p.track_mut(&track_id)?;
                let i = clip_mut(t, &clip_id)?;
                t.clips[i].position = position;
            }
            Command::SetClipTempo {
                track_id,
                clip_id,
                source_bpm,
            } => {
                let t = p.track_mut(&track_id)?;
                let i = clip_mut(t, &clip_id)?;
                t.clips[i].source_bpm = source_bpm;
            }
            Command::RelinkClip {
                track_id,
                clip_id,
                source,
            } => {
                let t = p.track_mut(&track_id)?;
                let i = clip_mut(t, &clip_id)?;
                t.clips[i].source = source;
            }
            Command::AddDevice { track_id, kind } => {
                p.devices_mut(&track_id)?.push(Device::new(&kind)?)
            }
            Command::RemoveDevice {
                track_id,
                device_id,
            } => {
                let devices = p.devices_mut(&track_id)?;
                ensure!(
                    devices.iter().any(|d| d.id == device_id),
                    "Device not found"
                );
                devices.retain(|d| d.id != device_id);
            }
            Command::MoveDevice {
                track_id,
                device_id,
                index,
            } => {
                let devices = p.devices_mut(&track_id)?;
                ensure!(index < devices.len(), "Invalid device position");
                let from = devices
                    .iter()
                    .position(|d| d.id == device_id)
                    .context("Device not found")?;
                let device = devices.remove(from);
                devices.insert(index, device);
            }
            Command::SetBeatEnvelope {
                track_id,
                device_id,
                lane,
                slot,
                points,
            } => {
                ensure!(
                    matches!(lane.as_str(), "time" | "volume") && slot < 36,
                    "Invalid envelope slot"
                );
                let d = p
                    .devices_mut(&track_id)?
                    .iter_mut()
                    .find(|d| d.id == device_id)
                    .context("Device not found")?;
                ensure!(d.kind == "builtin.beat", "Envelopes require Beat");
                let key = format!("{lane}:{slot}");
                if let Some(points) = points {
                    d.beat_envelopes.insert(key, points);
                } else {
                    d.beat_envelopes.remove(&key);
                }
            }
            Command::SetDeviceParameter {
                track_id,
                device_id,
                parameter,
                value,
            } => {
                let d = p
                    .devices_mut(&track_id)?
                    .iter_mut()
                    .find(|d| d.id == device_id)
                    .context("Device not found")?;
                *d.parameters
                    .get_mut(&parameter)
                    .context("Unknown device parameter")? = value;
            }
            _ => unreachable!(),
        }
        p.validate()?;
        if p != before {
            self.history.push(HistoryEntry {
                label,
                before,
                after: p.clone(),
            });
            // Bound snapshot history for this pre-alpha implementation.
            if self.history.len() > 100 {
                self.history.remove(0);
            }
            self.redo_history.clear();
            self.project = p;
            self.revision += 1;
        }
        Ok(Effect::None)
    }
    /// Collapse all updates of one pointer gesture into a single history entry.
    pub fn group_changes(&mut self, before: Project, revisions: u64) {
        let count = (revisions as usize).min(self.history.len());
        if count > 0 && self.project == before {
            self.history.truncate(self.history.len() - count);
        } else if count > 1 {
            let label = self.history.last().unwrap().label.clone();
            self.history.truncate(self.history.len() - count);
            self.history.push(HistoryEntry {
                label,
                before,
                after: self.project.clone(),
            });
        }
    }
    pub fn undo(&mut self) -> bool {
        if let Some(e) = self.history.pop() {
            self.project = e.before.clone();
            self.redo_history.push(e);
            self.revision += 1;
            true
        } else {
            false
        }
    }
    pub fn redo(&mut self) -> bool {
        if let Some(e) = self.redo_history.pop() {
            self.project = e.after.clone();
            self.history.push(e);
            self.revision += 1;
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reorder_devices_is_validated_undoable_and_persists_for_tracks_and_master() {
        let root = tempfile::tempdir().unwrap();
        let mut session = Session::new(Project::new("Order"), root.path().into());
        session
            .execute(Command::AddTrack {
                name: "Audio".into(),
            })
            .unwrap();
        let track = session.project.tracks[0].id.clone();
        for target in [track, "master".into()] {
            for kind in ["builtin.gain", "builtin.eq8", "builtin.limiter"] {
                session
                    .execute(Command::AddDevice {
                        track_id: target.clone(),
                        kind: kind.into(),
                    })
                    .unwrap();
            }
            let before = session.project.clone();
            let id = before.devices(&target).unwrap()[0].id.clone();
            session
                .execute(Command::MoveDevice {
                    track_id: target.clone(),
                    device_id: id.clone(),
                    index: 2,
                })
                .unwrap();
            assert_eq!(session.project.devices(&target).unwrap()[2].id, id);
            assert!(session.undo());
            assert_eq!(session.project, before);
            assert!(session.redo());
            let reordered = session.project.clone();
            let revision = session.revision;
            session
                .execute(Command::MoveDevice {
                    track_id: target.clone(),
                    device_id: id.clone(),
                    index: 2,
                })
                .unwrap();
            assert_eq!(
                session.revision, revision,
                "Dropping in place must not add history"
            );
            for (device_id, index) in [(id.clone(), 3), ("missing".into(), 0)] {
                assert!(session
                    .execute(Command::MoveDevice {
                        track_id: target.clone(),
                        device_id,
                        index
                    })
                    .is_err());
                assert_eq!(session.project, reordered);
                assert_eq!(session.revision, revision);
            }
            session
                .execute(Command::MoveDevice {
                    track_id: target.clone(),
                    device_id: id.clone(),
                    index: 0,
                })
                .unwrap();
            assert_eq!(session.project, before);
            session.execute(Command::SaveProject).unwrap();
            assert_eq!(
                Session::open(root.path())
                    .unwrap()
                    .project
                    .devices(&target)
                    .unwrap()[0]
                    .id,
                id
            );
        }
    }
    #[test]
    fn effects_validate_ranges_preserve_history_and_roundtrip_master_chain() {
        let root = tempfile::tempdir().unwrap();
        let legacy = Project::new("Effects");
        let yaml = serde_yaml::to_string(&legacy).unwrap();
        assert!(!yaml.contains("master_devices"));
        let mut s = Session::new(serde_yaml::from_str(&yaml).unwrap(), root.path().into());
        for &(_, kind, _) in BUILTIN_DEVICES {
            s.execute(Command::AddDevice {
                track_id: "master".into(),
                kind: kind.into(),
            })
            .unwrap();
        }
        let eq = s
            .project
            .master_devices
            .iter()
            .find(|d| d.kind == "builtin.eq8")
            .unwrap()
            .id
            .clone();
        for (parameter, value) in [
            ("band8_freq_hz", 20000.0),
            ("band8_enabled", 1.0),
            ("band8_q", 18.0),
            ("band8_type", 4.0),
        ] {
            s.execute(Command::SetDeviceParameter {
                track_id: "master".into(),
                device_id: eq.clone(),
                parameter: parameter.into(),
                value,
            })
            .unwrap();
        }
        let before = s.project.clone();
        let history = s.history.len();
        for (parameter, value) in [
            ("band8_freq_hz", 20001.0),
            ("band8_q", 0.0),
            ("band8_type", 0.5),
            ("band8_enabled", 0.5),
            ("band8_gain_db", f64::NAN),
            ("unknown", 0.0),
        ] {
            assert!(s
                .execute(Command::SetDeviceParameter {
                    track_id: "master".into(),
                    device_id: eq.clone(),
                    parameter: parameter.into(),
                    value
                })
                .is_err());
            assert_eq!(s.project, before);
            assert_eq!(s.history.len(), history);
        }
        let compressor = s
            .project
            .master_devices
            .iter()
            .find(|d| d.kind == "builtin.compressor")
            .unwrap()
            .id
            .clone();
        s.execute(Command::SetDeviceParameter {
            track_id: "master".into(),
            device_id: compressor,
            parameter: "release_ms".into(),
            value: 2000.0,
        })
        .unwrap();
        assert!(s.undo());
        assert_eq!(s.project, before);
        assert!(s.redo());
        save(root.path(), &s.project).unwrap();
        assert_eq!(load(root.path()).unwrap(), s.project);
    }
    #[test]
    fn transaction_and_history() {
        let mut s = Session::new(Project::new("Test"), PathBuf::from("."));
        s.execute(Command::AddTrack {
            name: "Vocals".into(),
        })
        .unwrap();
        let tid = s.project.tracks[0].id.clone();
        let p = s.project.clone();
        assert!(s
            .execute(Command::SetTrackPan {
                track_id: tid.clone(),
                pan: f64::NAN
            })
            .is_err());
        assert_eq!(s.project, p);
        assert_eq!(s.history.len(), 1);
        s.execute(Command::SetTrackVolume {
            track_id: tid,
            volume_db: -3.0,
        })
        .unwrap();
        assert!(s.undo());
        assert_eq!(s.project, p);
        assert!(s.redo());
        assert_eq!(s.project.tracks[0].mixer.volume_db, -3.0);
    }
    #[test]
    fn atomic_roundtrip_preserves_missing_media() {
        let root = tempfile::tempdir().unwrap();
        let mut s = Session::new(Project::new("Round trip"), root.path().to_path_buf());
        s.execute(Command::AddTrack {
            name: "Audio".into(),
        })
        .unwrap();
        s.execute(Command::ImportAudioClip {
            track_id: s.project.tracks[0].id.clone(),
            source: Source {
                path: root.path().join("absent.wav"),
                kind: SourceKind::External,
            },
            position: Position {
                start_beats: 4.0,
                offset_seconds: 1.0,
                length_seconds: 2.0,
            },
        })
        .unwrap();
        save(root.path(), &s.project).unwrap();
        save(root.path(), &s.project).unwrap();
        let p = load(root.path()).unwrap();
        assert_eq!(p, s.project);
        assert_eq!(p.missing(root.path()).len(), 1);
        assert!(!root.path().join("media").exists());
    }
    #[test]
    fn reject_versions_duplicates_and_escape_paths() {
        let mut p = Project::new("Test");
        p.format = 2;
        assert!(p.validate().is_err());
        let mut s = Session::new(Project::new("Test"), PathBuf::from("."));
        s.execute(Command::AddTrack {
            name: "Track".into(),
        })
        .unwrap();
        assert!(s
            .execute(Command::ImportAudioClip {
                track_id: s.project.tracks[0].id.clone(),
                source: Source {
                    path: PathBuf::from("../outside.wav"),
                    kind: SourceKind::Project
                },
                position: Position {
                    start_beats: 0.0,
                    offset_seconds: 0.0,
                    length_seconds: 1.0
                }
            })
            .is_err());
        s.project.tracks[0].id = s.project.project.id.clone();
        assert!(s.project.validate().is_err());
    }
}

#[cfg(test)]
mod midi_tests {
    use super::*;
    #[test]
    fn score_properties_are_compatible_atomic_and_undoable() {
        let legacy: MidiNote = serde_json::from_str(
            r#"{"key":60,"velocity":100,"start_beats":0.0,"length_beats":1.0}"#,
        )
        .unwrap();
        assert!(!legacy.muted);
        assert_eq!(legacy.channel, 1);
        let mut s = Session::new(Project::new("Score"), PathBuf::new());
        s.execute(Command::AddMidiTrack {
            name: "Keys".into(),
        })
        .unwrap();
        let id = s.project.tracks[0].id.clone();
        let region = Some(MidiRegion {
            start_beats: 8.0,
            offset_beats: 0.0,
            length_beats: 4.0,
        });
        let notes = vec![
            MidiNote {
                muted: true,
                channel: 16,
                ..legacy.clone()
            },
            legacy,
        ];
        s.execute(Command::SetMidiScore {
            track_id: id.clone(),
            notes: notes.clone(),
            region: region.clone(),
        })
        .unwrap();
        assert_eq!(s.project.tracks[0].arranged_midi_notes().len(), 1);
        let snapshot = s.project.clone();
        let mut invalid = notes.clone();
        invalid[0].channel = 17;
        assert!(s
            .execute(Command::SetMidiScore {
                track_id: id.clone(),
                notes: invalid,
                region: None
            })
            .is_err());
        assert_eq!(s.project, snapshot);
        let encoded = serde_yaml::to_string(&snapshot).unwrap();
        let restored: Project = serde_yaml::from_str(&encoded).unwrap();
        assert_eq!(restored, snapshot);
        s.execute(Command::SetMidiScore {
            track_id: id,
            notes: vec![],
            region: None,
        })
        .unwrap();
        assert!(s.project.tracks[0].notes.is_empty());
        assert!(s.undo());
        assert_eq!(s.project, snapshot);
    }
    #[test]
    fn midi_edits_validate_undo_and_roundtrip() {
        let root = tempfile::tempdir().unwrap();
        let mut session = Session::new(Project::new("MIDI"), root.path().to_path_buf());
        session
            .execute(Command::AddMidiTrack { name: "Dot".into() })
            .unwrap();
        let track_id = session.project.tracks[0].id.clone();
        session
            .execute(Command::SetMidiNotes {
                track_id: track_id.clone(),
                notes: vec![MidiNote {
                    key: 60,
                    velocity: 100,
                    start_beats: 0.0,
                    length_beats: 1.0,
                    ..MidiNote::default()
                }],
            })
            .unwrap();
        assert!(session.project.tracks[0].synth.is_none());
        session
            .execute(Command::SetTrackInstrument {
                track_id: track_id.clone(),
                kind: Some("builtin.dot".into()),
            })
            .unwrap();
        let snapshot = session.project.clone();
        session
            .execute(Command::SetTrackInstrument {
                track_id: track_id.clone(),
                kind: None,
            })
            .unwrap();
        assert_eq!(session.project.tracks[0].notes, snapshot.tracks[0].notes);
        assert!(session.undo());
        assert!(session
            .execute(Command::SetSynthParameter {
                track_id: track_id.clone(),
                parameter: "wave_type".into(),
                value: 1.5
            })
            .is_err());
        assert_eq!(session.project, snapshot);
        session
            .execute(Command::SetSynthParameter {
                track_id,
                parameter: "sustain".into(),
                value: 0.4,
            })
            .unwrap();
        assert!(session.undo());
        assert_eq!(session.project, snapshot);
        let loaded: Project =
            serde_json::from_str(&serde_json::to_string(&snapshot).unwrap()).unwrap();
        loaded.validate().unwrap();
        assert_eq!(loaded, snapshot);
        assert!(loaded.duration_seconds() > 0.5);
    }
}

#[cfg(test)]
mod plugin_tests {
    use super::*;
    #[test]
    fn plugin_roles_state_history_and_project_roundtrip() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("Test.vst3");
        let instrument = format!("vst3.instrument:{}", path.display());
        let effect = format!("vst3.effect:{}", path.display());
        let mut session = Session::new(Project::new("Plugins"), root.path().into());
        session
            .execute(Command::AddMidiTrack {
                name: "MIDI".into(),
            })
            .unwrap();
        let track_id = session.project.tracks[0].id.clone();
        session
            .execute(Command::SetTrackInstrument {
                track_id: track_id.clone(),
                kind: Some(instrument.clone()),
            })
            .unwrap();
        let id = session.project.tracks[0].synth.as_ref().unwrap().id.clone();
        session
            .execute(Command::SetPluginState {
                track_id: track_id.clone(),
                device_id: id,
                state: vec![0, 1, 255],
            })
            .unwrap();
        assert!(session.undo());
        assert!(session.project.tracks[0]
            .synth
            .as_ref()
            .unwrap()
            .plugin_state
            .is_empty());
        assert!(session.redo());
        let before = session.project.clone();
        assert!(session
            .execute(Command::AddDevice {
                track_id: track_id.clone(),
                kind: instrument
            })
            .is_err());
        assert_eq!(session.project, before);
        assert!(session
            .execute(Command::SetTrackInstrument {
                track_id: track_id.clone(),
                kind: Some(effect.clone())
            })
            .is_err());
        assert_eq!(session.project, before);
        session
            .execute(Command::AddDevice {
                track_id: "master".into(),
                kind: effect,
            })
            .unwrap();
        let id = session.project.master_devices[0].id.clone();
        session
            .execute(Command::SetPluginState {
                track_id: "master".into(),
                device_id: id,
                state: vec![33, 99],
            })
            .unwrap();
        save(root.path(), &session.project).unwrap();
        assert_eq!(load(root.path()).unwrap(), session.project);
        let mut invalid = Device::new("vst3.effect:relative.vst3").unwrap();
        assert!(invalid.validate().is_err());
        invalid = Device::new("builtin.gain").unwrap();
        invalid.plugin_state = vec![1];
        assert!(invalid.validate().is_err());
    }
}
