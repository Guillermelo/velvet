//! The only mutable project authority shared by GUI, CLI and AI.
use anyhow::{bail, ensure, Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashSet},
    fs,
    io::Write,
    path::{Component, Path, PathBuf},
};

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
fn default_color() -> [u8; 3] {
    [120, 157, 156]
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum TrackKind {
    Audio,
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
}
impl Device {
    pub fn new(kind: &str) -> Result<Self> {
        let defaults: &[(&str, f64)] = match kind {
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
        })
    }
    pub fn parameter_range(&self, parameter: &str) -> Result<(f64, f64)> {
        ensure!(
            self.parameters.contains_key(parameter),
            "Unknown device parameter: {parameter}"
        );
        Ok(match parameter {
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
            if key.ends_with("_type") || key.ends_with("_enabled") {
                ensure!(value.fract() == 0.0, "{key} must be an integer");
            }
        }
        Ok(())
    }
}
pub const BUILTIN_DEVICES: &[(&str, &str, &str)] = &[
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
        self.tracks
            .iter()
            .flat_map(|t| &t.clips)
            .map(|c| c.position.start_beats * 60.0 / self.tempo.bpm + c.position.length_seconds)
            .fold(0.0, f64::max)
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
            for c in &t.clips {
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
            Command::AddTrack { name } => p.tracks.push(Track {
                id: id("track"),
                name,
                kind: TrackKind::Audio,
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
        if count > 1 {
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
