//! Decoding and DSP happen outside the CPAL callback. The callback only reads
//! an immutable stereo mix and bounded atomic transport controls, and synthesizes
//! the short metronome click without allocations.
pub mod synth;
use anyhow::{bail, ensure, Context, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use rtrb::{Consumer, Producer, PushError, RingBuffer};
use std::{
    collections::HashMap,
    fs::File,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc,
    },
};
use symphonia::core::{
    audio::SampleBuffer, codecs::DecoderOptions, errors::Error, formats::FormatOptions,
    io::MediaSourceStream, meta::MetadataOptions, probe::Hint,
};
use velvet_core::{Command, Position, Project, Session, Source, SourceKind};
mod beat;
mod devices;
pub mod plugins;
#[cfg(test)]
use devices::Biquad;
pub use devices::{eq_response, EQ_FILTER_TYPES};

#[derive(Clone)]
pub struct AudioData {
    pub sample_rate: u32,
    pub frames: Vec<[f32; 2]>,
}
impl AudioData {
    pub fn duration(&self) -> f64 {
        self.frames.len() as f64 / self.sample_rate as f64
    }
}
pub fn decode(path: &Path) -> Result<AudioData> {
    let file = File::open(path).with_context(|| format!("Cannot open {}", path.display()))?;
    let stream = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|s| s.to_str()) {
        hint.with_extension(ext);
    }
    let mut format = symphonia::default::get_probe()
        .format(
            &hint,
            stream,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )?
        .format;
    let track = format.default_track().context("No audio track")?;
    let track_id = track.id;
    let mut decoder =
        symphonia::default::get_codecs().make(&track.codec_params, &DecoderOptions::default())?;
    let mut sample_rate = track.codec_params.sample_rate.unwrap_or(48000);
    let mut frames = Vec::new();
    loop {
        let packet = match format.next_packet() {
            Ok(p) => p,
            Err(Error::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(e) => return Err(e.into()),
        };
        if packet.track_id() != track_id {
            continue;
        }
        let decoded = decoder.decode(&packet)?;
        sample_rate = decoded.spec().rate;
        let channels = decoded.spec().channels.count();
        ensure!(
            channels == 1 || channels == 2,
            "Only mono and stereo media are supported"
        );
        let mut buf = SampleBuffer::<f32>::new(decoded.capacity() as u64, *decoded.spec());
        buf.copy_interleaved_ref(decoded);
        for f in buf.samples().chunks(channels) {
            frames.push([f[0], f[channels - 1]]);
        }
        ensure!(
            frames.len() <= (sample_rate as usize * 1800).min(64_000_000),
            "Media exceeds the pre-alpha 30-minute limit"
        );
    }
    ensure!(!frames.is_empty(), "Audio file is empty");
    Ok(AudioData {
        sample_rate,
        frames,
    })
}
pub fn import(
    session: &mut Session,
    track_id: &str,
    path: &Path,
    start_beats: f64,
) -> Result<AudioData> {
    let path = path.canonicalize()?;
    let data = decode(&path)?;
    session.execute(Command::ImportAudioClip {
        track_id: track_id.into(),
        source: Source {
            path,
            kind: SourceKind::External,
        },
        position: Position {
            start_beats,
            offset_seconds: 0.0,
            length_seconds: data.duration(),
        },
    })?;
    Ok(data)
}
#[derive(Clone)]
pub struct Mix {
    pub sample_rate: u32,
    pub frames: Vec<[f32; 2]>,
    pub missing: Vec<PathBuf>,
    pub sources: Vec<PathBuf>,
    pub peak: f32,
    pub device_signals: HashMap<String, Arc<ScopeSignal>>,
}
#[derive(Default)]
pub struct MediaCache {
    pub files: HashMap<PathBuf, Arc<AudioData>>,
}
impl MediaCache {
    pub fn get(&mut self, path: &Path) -> Result<Arc<AudioData>> {
        if let Some(d) = self.files.get(path) {
            return Ok(d.clone());
        }
        let data = Arc::new(decode(path)?);
        self.files.insert(path.into(), data.clone());
        Ok(data)
    }
}
pub fn mix(project: &Project, root: &Path, rate: u32, cache: &mut MediaCache) -> Result<Mix> {
    mix_with_scopes(
        project,
        root,
        rate,
        cache,
        &std::collections::HashSet::new(),
    )
}
pub fn mix_with_scopes(
    project: &Project,
    root: &Path,
    rate: u32,
    cache: &mut MediaCache,
    scopes: &std::collections::HashSet<String>,
) -> Result<Mix> {
    mix_internal(project, root, rate, cache, scopes, None)
}

struct CachedTrack {
    track: velvet_core::Track,
    frames: Vec<[f32; 2]>,
    signals: HashMap<String, Arc<ScopeSignal>>,
    missing: Vec<PathBuf>,
    sources: Vec<(PathBuf, std::sync::Weak<AudioData>)>,
}

/// Persistent playback renderer; use on a single worker thread for plugin thread affinity.
#[derive(Default)]
pub struct LiveMixer {
    plugins: plugins::RenderCache,
    tracks: HashMap<String, CachedTrack>,
    configuration: Option<(PathBuf, u32, f64, usize, std::collections::HashSet<String>)>,
}

impl LiveMixer {
    pub fn preview(&mut self, device: &velvet_core::Device, key: u8, velocity: u8,
        channel: u8, bpm: f64, rate: u32) -> Result<Mix> {
        ensure!((8000..=192000).contains(&rate) && bpm.is_finite() && bpm > 0.0,
            "Invalid preview sample rate or tempo");
        ensure!(key <= 127 && velocity <= 127 && (1..=16).contains(&channel), "Invalid preview MIDI note");
        let note = velvet_core::MidiNote { key, velocity, channel, start_beats: 0.0,
            length_beats: 0.4 * bpm / 60.0, muted: false };
        let release = device.parameters.get("release_ms").copied().unwrap_or(1100.0) / 1000.0;
        ensure!(release.is_finite() && (0.0..=10.0).contains(&release), "Invalid preview release");
        let mut frames = vec![[0.0; 2]; ((0.4 + release) * rate as f64).ceil() as usize];
        if device.plugin_path().is_some() {
            self.plugins.process(&mut frames, device, &[note], bpm, rate)?;
        } else {
            synth::render(&mut frames, &[note], device, bpm, rate);
        }
        let mut peak = 0.0_f32;
        for sample in frames.iter_mut().flatten() {
            ensure!(sample.is_finite(), "Preview produced invalid audio");
            *sample = sample.clamp(-1.0, 1.0);
            peak = peak.max(sample.abs());
        }
        Ok(Mix { sample_rate: rate, frames, missing: vec![], sources: vec![],
            peak, device_signals: HashMap::new() })
    }
    pub fn set_cancellation(&mut self, generation: Arc<std::sync::atomic::AtomicU64>, expected: u64) {
        self.plugins.set_cancellation(generation, expected);
    }
    pub fn mix(&mut self, project: &Project, root: &Path, rate: u32,
        cache: &mut MediaCache, scopes: &std::collections::HashSet<String>) -> Result<Mix> {
        let configuration = (root.to_owned(), rate, project.tempo.bpm,
            (project.duration_seconds() * rate as f64).ceil() as usize, scopes.clone());
        if self.configuration.as_ref() != Some(&configuration) {
            self.tracks.clear();
            self.configuration = Some(configuration);
        }
        self.tracks.retain(|id, _| project.tracks.iter().any(|t| t.id == *id));
        let ids = project.tracks.iter().flat_map(|t| t.synth.iter().chain(&t.devices))
            .chain(&project.master_devices).map(|d| d.id.clone()).collect();
        self.plugins.retain(&ids);
        mix_internal(project, root, rate, cache, scopes, Some(self))
    }
}

fn mix_internal(project: &Project, root: &Path, rate: u32, cache: &mut MediaCache,
    scopes: &std::collections::HashSet<String>, mut live: Option<&mut LiveMixer>) -> Result<Mix> {
    let mut device_signals = HashMap::new();
    project.validate()?;
    ensure!(
        (8000..=192000).contains(&rate),
        "Invalid output sample rate"
    );
    let length = project.duration_seconds();
    ensure!(
        length <= 1800.0,
        "Arrangement exceeds the pre-alpha 30-minute limit"
    );
    let count = (length * rate as f64).ceil() as usize;
    ensure!(
        count <= 64_000_000,
        "Arrangement exceeds the pre-alpha render memory limit"
    );
    let mut output = vec![[0.0; 2]; count];
    let solo = project.tracks.iter().any(|t| t.mixer.solo);
    let mut missing = vec![];
    for track in &project.tracks {
        if track.mixer.mute || (solo && !track.mixer.solo) {
            continue;
        }
        let cached = live.as_ref().and_then(|l| l.tracks.get(&track.id))
            .filter(|c| c.track.synth == track.synth && c.track.notes == track.notes
                && c.track.midi_region == track.midi_region && c.track.clips == track.clips
                && c.track.devices == track.devices && c.track.kind == track.kind
                && c.sources.iter().all(|(path, version)| path.is_file()
                    && cache.files.get(path).is_some_and(|source| version.ptr_eq(&Arc::downgrade(source)))));
        let bus = if let Some(cached) = cached {
            missing.extend(cached.missing.iter().cloned());
            device_signals.extend(cached.signals.iter().map(|(id, s)| (id.clone(), s.clone())));
            cached.frames.clone()
        } else {
        let missing_start = missing.len();
        let mut track_signals = HashMap::new();
        let mut bus = vec![[0.0; 2]; count];
        for clip in &track.clips {
            let path = project.source_path(root, &clip.source);
            if !path.is_file() {
                missing.push(path);
                continue;
            }
            let data = cache.get(&path)?;
            let start = (clip.position.start_beats * 60.0 / project.tempo.bpm * rate as f64).round()
                as usize;
            let speed = clip.playback_rate(project.tempo.bpm);
            let n = (clip.duration_seconds(project.tempo.bpm) * rate as f64).round() as usize;
            for i in 0..n.min(count.saturating_sub(start)) {
                let source = (clip.position.offset_seconds + i as f64 / rate as f64 * speed)
                    * data.sample_rate as f64;
                let a = source.floor() as usize;
                if a >= data.frames.len() {
                    break;
                }
                let b = (a + 1).min(data.frames.len() - 1);
                let f = (source - a as f64) as f32;
                for (c, sample) in bus[start + i].iter_mut().enumerate() {
                    *sample += data.frames[a][c] * (1.0 - f) + data.frames[b][c] * f;
                }
            }
        }
        if let Some(instrument) = &track.synth {
            if instrument.plugin_path().is_some() {
                process_plugin(
                    &mut bus,
                    instrument,
                    &track.arranged_midi_notes(),
                    project.tempo.bpm,
                    rate,
                    live.as_mut().map(|l| &mut l.plugins),
                )?;
            } else {
                synth::render(
                    &mut bus,
                    &track.arranged_midi_notes(),
                    instrument,
                    project.tempo.bpm,
                    rate,
                );
            }
        }
        if let Some(instrument) = &track.synth {
            if scopes.contains(&instrument.id) {
                track_signals.insert(
                    instrument.id.clone(),
                    Arc::new(ScopeSignal::new(&bus, rate)),
                );
            }
        }
        process_chain(
            &mut bus,
            &track.devices,
            project.tempo.bpm,
            rate,
            scopes,
            &mut track_signals,
            live.as_mut().map(|l| &mut l.plugins),
        )?;
        if let Some(live) = live.as_mut() {
            // Bound retained audio to 128 MiB; longer projects still reuse plugin instances.
            let retained = live.tracks.values().map(|c| c.frames.len()).sum::<usize>();
            if retained + bus.len() > 16_000_000 {
                live.tracks.clear();
            }
            if bus.len() <= 16_000_000 && missing_start == missing.len() {
                live.tracks.insert(track.id.clone(), CachedTrack {
                    track: track.clone(), frames: bus.clone(), signals: track_signals.clone(),
                    missing: missing[missing_start..].to_vec(),
                    sources: track.clips.iter().filter_map(|clip| {
                        let path = project.source_path(root, &clip.source);
                        cache.files.get(&path).map(|source| (path, Arc::downgrade(source)))
                    }).collect(),
                });
            }
        }
        device_signals.extend(track_signals);
        bus
        };
        let gain = db(track.mixer.volume_db);
        // Stereo balance law: center preserves stereo; extremes silence one side.
        let left = gain * (1.0 - track.mixer.pan.max(0.0)) as f32;
        let right = gain * (1.0 + track.mixer.pan.min(0.0)) as f32;
        for (o, f) in output.iter_mut().zip(bus) {
            o[0] += f[0] * left;
            o[1] += f[1] * right;
        }
    }
    let gain = if project.master.mute {
        0.0
    } else {
        db(project.master.volume_db)
    };
    for frame in &mut output {
        for sample in frame {
            *sample *= gain;
        }
    }
    process_chain(
        &mut output,
        &project.master_devices,
        project.tempo.bpm,
        rate,
        scopes,
        &mut device_signals,
        live.as_mut().map(|l| &mut l.plugins),
    )?;
    let mut peak: f32 = 0.0;
    for f in &mut output {
        for s in f {
            peak = peak.max(s.abs());
            *s = s.clamp(-1.0, 1.0);
        }
    }
    Ok(Mix {
        device_signals,
        sample_rate: rate,
        frames: output,
        missing,
        sources: project
            .tracks
            .iter()
            .flat_map(|t| &t.clips)
            .map(|c| project.source_path(root, &c.source))
            .collect(),
        peak,
    })
}
fn db(n: f64) -> f32 {
    10.0_f32.powf(n as f32 / 20.0)
}
pub fn export(mix: &Mix, path: &Path) -> Result<()> {
    ensure!(
        path.extension()
            .and_then(|s| s.to_str())
            .map(|s| s.eq_ignore_ascii_case("wav"))
            .unwrap_or(false),
        "Export filename must end in .wav"
    );
    let target = path.canonicalize().or_else(|_| std::path::absolute(path))?;
    for source in &mix.sources {
        let original = source
            .canonicalize()
            .or_else(|_| std::path::absolute(source))?;
        ensure!(
            target != original,
            "Export cannot overwrite a project audio source"
        );
    }
    ensure!(
        mix.missing.is_empty(),
        "Cannot export: {} audio source(s) are missing",
        mix.missing.len()
    );
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    std::fs::create_dir_all(parent)?;
    let temp = tempfile::NamedTempFile::new_in(parent)?;
    let spec = hound::WavSpec {
        channels: 2,
        sample_rate: mix.sample_rate,
        bits_per_sample: 24,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer = hound::WavWriter::new(temp.reopen()?, spec)?;
    for f in &mix.frames {
        for s in f {
            writer.write_sample((s.clamp(-1.0, 1.0) * 8_388_607.0).round() as i32)?;
        }
    }
    writer.finalize()?;
    temp.as_file().sync_all()?;
    temp.persist(path).map_err(|e| e.error)?;
    Ok(())
}
pub struct Player {
    _stream: cpal::Stream,
    pub state: Arc<PlaybackState>,
    pub sample_rate: u32,
    updates: MixUpdates,
}
pub struct PlaybackState {
    pub playing: AtomicBool,
    pub frame: AtomicU64,
    pub failed: AtomicBool,
    // Pack both frame boundaries into one atomic so the callback sees a coherent range.
    // Zero disables looping; the renderer's 64M-frame limit fits in 32 bits.
    loop_frames: AtomicU64,
    metronome_bpm: AtomicU64,
    output_gain: AtomicU64,
    metronome_gain: AtomicU64,
}
impl PlaybackState {
    fn set_metronome(&self, bpm: Option<f64>) {
        self.metronome_bpm.store(
            bpm.filter(|bpm| bpm.is_finite() && (20.0..=400.0).contains(bpm))
                .map_or(0, f64::to_bits),
            Ordering::Relaxed,
        );
    }
    fn set_loop(&self, range: Option<(f64, f64)>, rate: u32) {
        let packed = range
            .filter(|(start, end)| {
                start.is_finite() && end.is_finite() && *start >= 0.0 && end > start
            })
            .map_or(0, |(start, end)| {
                let start = (start * rate as f64).round().min(u32::MAX as f64) as u64;
                let end = (end * rate as f64).round().min(u32::MAX as f64) as u64;
                if end > start {
                    (start << 32) | end
                } else {
                    0
                }
            });
        self.loop_frames.store(packed, Ordering::Relaxed);
    }
}
/// UI-side ownership: retired buffers are freed here, never in render().
struct MixUpdates {
    incoming: Producer<Arc<Mix>>,
    retired: Consumer<Arc<Mix>>,
    pending: Option<Arc<Mix>>,
}
impl MixUpdates {
    fn maintain(&mut self) {
        while self.retired.pop().is_ok() {}
        if let Some(mix) = self.pending.take() {
            if let Err(PushError::Full(mix)) = self.incoming.push(mix) {
                self.pending = Some(mix);
            }
        }
    }
}
struct PlaybackBuffer {
    mix: Arc<Mix>,
    previous: Option<Arc<Mix>>,
    fade: usize,
    incoming: Consumer<Arc<Mix>>,
    retired: Producer<Arc<Mix>>,
    state: Arc<PlaybackState>,
}
impl PlaybackBuffer {
    fn new(mix: Arc<Mix>, state: Arc<PlaybackState>) -> (MixUpdates, Self) {
        let (tx, rx) = RingBuffer::new(1);
        let (retire_tx, retire_rx) = RingBuffer::new(2);
        (
            MixUpdates {
                incoming: tx,
                retired: retire_rx,
                pending: None,
            },
            Self {
                mix,
                previous: None,
                fade: 0,
                incoming: rx,
                retired: retire_tx,
                state,
            },
        )
    }
    fn render<T: cpal::SizedSample + cpal::FromSample<f32>>(
        &mut self,
        output: &mut [T],
        channels: usize,
    ) {
        // One update at each callback boundary. Reserving a retirement slot
        // guarantees that an old buffer never needs to be dropped here.
        if self.previous.is_none() && !self.retired.is_full() {
            if let Ok(next) = self.incoming.pop() {
                self.previous = Some(std::mem::replace(&mut self.mix, next));
                self.fade = 0;
            }
        }
        let loop_frames = self.state.loop_frames.load(Ordering::Relaxed);
        let loop_start = (loop_frames >> 32) as usize;
        let loop_end = loop_frames as u32 as usize;
        let bpm = f64::from_bits(self.state.metronome_bpm.load(Ordering::Relaxed));
        let output_gain = f64::from_bits(self.state.output_gain.load(Ordering::Relaxed)) as f32;
        let metronome_gain = f64::from_bits(self.state.metronome_gain.load(Ordering::Relaxed)) as f32;
        let rate = self.mix.sample_rate;
        for frame in output.chunks_mut(channels) {
            let mut sample = [0.0; 2];
            if self.state.playing.load(Ordering::Relaxed) {
                let mut index = self.state.frame.fetch_add(1, Ordering::Relaxed) as usize;
                if loop_end > loop_start && (index >= loop_end || index < loop_start) {
                    index = loop_start;
                    self.state.frame.store(index as u64 + 1, Ordering::Relaxed);
                }
                if let Some(f) = self.mix.frames.get(index) {
                    sample = *f;
                    if let Some(old) = &self.previous {
                        let prior = old.frames.get(index).copied().unwrap_or([0.0; 2]);
                        let blend = (self.fade as f32 / 128.0).min(1.0);
                        for c in 0..2 {
                            sample[c] = prior[c] + (sample[c] - prior[c]) * blend;
                        }
                    }
                } else if loop_end <= loop_start {
                    self.state.playing.store(false, Ordering::Relaxed);
                }
                if bpm > 0.0 && self.state.playing.load(Ordering::Relaxed) {
                    let click = metronome_click(index as u64, rate, bpm) * metronome_gain;
                    for channel in &mut sample {
                        *channel = (*channel + click).clamp(-1.0, 1.0);
                    }
                }
            }
            self.fade = (self.fade + 1).min(128);
            for (c, s) in frame.iter_mut().enumerate() {
                *s = T::from_sample(output_gain * if channels == 1 {
                    (sample[0] + sample[1]) * 0.5
                } else {
                    sample[c % 2]
                });
            }
        }
        if self.fade == 128 {
            if let Some(old) = self.previous.take() {
                // Only this thread produces retired buffers; the slot reserved
                // above cannot be taken by another producer.
                let result = self.retired.push(old);
                if let Err(PushError::Full(old)) = result {
                    self.previous = Some(old);
                }
            }
        }
    }
}
fn metronome_click(frame: u64, rate: u32, bpm: f64) -> f32 {
    let frames_per_beat = rate as f64 * 60.0 / bpm;
    let beat = (frame as f64 / frames_per_beat).floor();
    let seconds = (frame as f64 - beat * frames_per_beat) / rate as f64;
    if seconds >= 0.035 {
        return 0.0;
    }
    let accented = (beat as u64).is_multiple_of(4);
    let frequency = if accented { 1760.0 } else { 1320.0 };
    let volume = if accented { 0.3 } else { 0.2 };
    (volume
        * (std::f64::consts::TAU * frequency * seconds).sin()
        * (-120.0 * seconds).exp()
        * (seconds / 0.001).min(1.0)) as f32
}
impl Player {
    pub fn output_rate() -> Result<u32> {
        Ok(cpal::default_host()
            .default_output_device()
            .context("No audio output device")?
            .default_output_config()?
            .sample_rate()
            .0)
    }
    pub fn new(mix: Arc<Mix>, seconds: f64) -> Result<Self> {
        let device = cpal::default_host()
            .default_output_device()
            .context("No audio output device")?;
        let supported = device.default_output_config()?;
        let config: cpal::StreamConfig = supported.clone().into();
        ensure!(
            config.sample_rate.0 == mix.sample_rate,
            "Audio device changed; prepare playback again"
        );
        let state = Arc::new(PlaybackState {
            playing: AtomicBool::new(false),
            frame: AtomicU64::new((seconds * mix.sample_rate as f64) as u64),
            failed: AtomicBool::new(false),
            loop_frames: AtomicU64::new(0),
            metronome_bpm: AtomicU64::new(0),
            output_gain: AtomicU64::new(1.0f64.to_bits()),
            metronome_gain: AtomicU64::new(1.0f64.to_bits()),
        });
        let (updates, buffer) = PlaybackBuffer::new(mix, state.clone());
        let stream = match supported.sample_format() {
            cpal::SampleFormat::F32 => build::<f32>(&device, &config, buffer)?,
            cpal::SampleFormat::I16 => build::<i16>(&device, &config, buffer)?,
            cpal::SampleFormat::U16 => build::<u16>(&device, &config, buffer)?,
            other => bail!("Unsupported audio output format: {other:?}"),
        };
        stream.play()?;
        Ok(Self {
            _stream: stream,
            state,
            sample_rate: config.sample_rate.0,
            updates,
        })
    }
    pub fn play(&self) {
        self.state.playing.store(true, Ordering::Relaxed);
    }
    pub fn pause(&self) {
        self.state.playing.store(false, Ordering::Relaxed);
    }
    pub fn seek(&self, seconds: f64) {
        self.state.frame.store(
            (seconds.max(0.0) * self.sample_rate as f64) as u64,
            Ordering::Relaxed,
        );
    }
    pub fn set_loop(&self, range: Option<(f64, f64)>) {
        self.state.set_loop(range, self.sample_rate);
    }
    pub fn set_metronome(&self, bpm: Option<f64>) {
        self.state.set_metronome(bpm);
    }
    /// Linear monitoring gain; independent of the rendered master mix.
    pub fn set_monitor_gain(&self, gain: f64, metronome_only: bool) {
        let gain = if gain.is_finite() { gain.clamp(0.0, 1.0) } else { 0.0 };
        let target = if metronome_only { &self.state.metronome_gain } else { &self.state.output_gain };
        target.store(gain.to_bits(), Ordering::Relaxed);
    }
    pub fn seconds(&self) -> f64 {
        self.state.frame.load(Ordering::Relaxed) as f64 / self.sample_rate as f64
    }
    pub fn playing(&self) -> bool {
        self.state.playing.load(Ordering::Relaxed)
    }
    pub fn replace_mix(&mut self, mix: Arc<Mix>) -> Result<()> {
        ensure!(
            mix.sample_rate == self.sample_rate,
            "Live mix must match the output sample rate"
        );
        self.updates.pending = Some(mix);
        self.maintain();
        Ok(())
    }
    pub fn maintain(&mut self) {
        self.updates.maintain();
    }
}
fn build<T: cpal::SizedSample + cpal::FromSample<f32>>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    mut buffer: PlaybackBuffer,
) -> Result<cpal::Stream> {
    let channels = config.channels as usize;
    let errors = buffer.state.clone();
    Ok(device.build_output_stream(
        config,
        move |output: &mut [T], _| {
            buffer.render(output, channels);
        },
        move |_| {
            errors.failed.store(true, Ordering::Relaxed);
        },
        None,
    )?)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn metronome_only_sounds_during_playback_and_follows_tempo_and_loops() {
        let state = Arc::new(PlaybackState {
            playing: AtomicBool::new(false),
            frame: AtomicU64::new(1500),
            failed: AtomicBool::new(false),
            loop_frames: AtomicU64::new(0),
            metronome_bpm: AtomicU64::new(0),
            output_gain: AtomicU64::new(1.0f64.to_bits()),
            metronome_gain: AtomicU64::new(1.0f64.to_bits()),
        });
        let mix = Arc::new(Mix {
            device_signals: Default::default(),
            sample_rate: 8000,
            frames: vec![[0.0; 2]; 20000],
            missing: vec![],
            sources: vec![],
            peak: 0.0,
        });
        let (_updates, mut buffer) = PlaybackBuffer::new(mix, state.clone());
        let mut output = vec![0.0_f32; 40000];
        buffer.render(&mut output, 2);
        assert!(output.iter().all(|s| *s == 0.0));
        state.set_metronome(Some(120.0));
        buffer.render(&mut output, 2);
        assert!(
            output.iter().all(|s| *s == 0.0),
            "Enabled metronome must be silent while stopped"
        );
        assert!(!state.playing.load(Ordering::Relaxed));
        assert_eq!(state.frame.load(Ordering::Relaxed), 1500);
        state.frame.store(0, Ordering::Relaxed);
        state.playing.store(true, Ordering::Relaxed);
        buffer.render(&mut output, 2);
        let energy = |samples: &[f32]| samples.iter().map(|s| s * s).sum::<f32>();
        for beat in 0..5 {
            let start = beat * 8000;
            assert!(energy(&output[start..start + 560]) > 0.1);
            assert!(output[start + 560..start + 8000].iter().all(|s| *s == 0.0));
        }
        assert!(energy(&output[..560]) > energy(&output[8000..8560]) * 1.5);
        state.frame.store(0, Ordering::Relaxed);
        state.metronome_gain.store(0.0f64.to_bits(), Ordering::Relaxed);
        buffer.render(&mut output[..560], 2);
        assert!(output[..560].iter().all(|s| *s == 0.0), "Monitoring mute must silence the metronome");
        state.metronome_gain.store(1.0f64.to_bits(), Ordering::Relaxed);

        state.playing.store(false, Ordering::Relaxed);
        let cursor = state.frame.load(Ordering::Relaxed);
        buffer.render(&mut output, 2);
        assert!(
            output.iter().all(|s| *s == 0.0),
            "Pause must silence the click"
        );
        assert_eq!(state.frame.load(Ordering::Relaxed), cursor);
        state.frame.store(0, Ordering::Relaxed);
        buffer.render(&mut output, 2);
        assert!(
            output.iter().all(|s| *s == 0.0),
            "Stop must silence the click at beat zero"
        );
        state.set_metronome(Some(240.0));
        state.playing.store(true, Ordering::Relaxed);
        buffer.render(&mut output[..8000], 2);
        assert!(
            energy(&output[4000..4560]) > 0.1,
            "Tempo change must shorten the beat interval"
        );
        state.set_metronome(Some(120.0));
        state.playing.store(true, Ordering::Relaxed);
        state.frame.store(3984, Ordering::Relaxed);
        buffer.render(&mut output[..128], 2);
        assert!(output[..32].iter().all(|s| *s == 0.0));
        assert!(
            energy(&output[32..128]) > 0.01,
            "Click must align with transport beat"
        );
        state.set_loop(Some((0.5, 1.0)), 8000);
        state.frame.store(7984, Ordering::Relaxed);
        buffer.render(&mut output[..128], 2);
        assert!(output[..32].iter().all(|s| *s == 0.0));
        assert!(
            energy(&output[32..128]) > 0.01,
            "Click must follow the loop boundary"
        );
        state.set_metronome(None);
        buffer.render(&mut output, 2);
        assert!(output.iter().all(|s| *s == 0.0));
        state.set_loop(None, 8000);
        state.set_metronome(Some(120.0));
        state.frame.store(19996, Ordering::Relaxed);
        buffer.render(&mut output, 2);
        assert!(!state.playing.load(Ordering::Relaxed));
        assert!(
            output.iter().all(|s| *s == 0.0),
            "End of song must also silence the click"
        );
        for bpm in [f64::NAN, -1.0, 0.0, 401.0] {
            state.set_metronome(Some(bpm));
            assert_eq!(state.metronome_bpm.load(Ordering::Relaxed), 0);
        }
    }
    #[test]
    fn loop_wraps_inside_the_audio_callback_without_silence_and_can_be_disabled() {
        let state = Arc::new(PlaybackState {
            playing: AtomicBool::new(true),
            frame: AtomicU64::new(5),
            failed: AtomicBool::new(false),
            loop_frames: AtomicU64::new(0),
            metronome_bpm: AtomicU64::new(0),
            output_gain: AtomicU64::new(1.0f64.to_bits()),
            metronome_gain: AtomicU64::new(1.0f64.to_bits()),
        });
        let mix = Arc::new(Mix {
            device_signals: Default::default(),
            sample_rate: 100,
            frames: (0..10)
                .map(|i| [i as f32 / 10.0, -(i as f32) / 10.0])
                .collect(),
            missing: vec![],
            sources: vec![],
            peak: 0.9,
        });
        let (_updates, mut buffer) = PlaybackBuffer::new(mix, state.clone());
        state.set_loop(Some((0.05, 0.08)), 100);
        let mut output = [0.0_f32; 28];
        buffer.render(&mut output, 2);
        for (i, frame) in output.as_chunks::<2>().0.iter().enumerate() {
            let sample = (5 + i % 3) as f32 / 10.0;
            assert_eq!(*frame, [sample, -sample]);
        }
        assert!(state.playing.load(Ordering::Relaxed));
        state.frame.store(1, Ordering::Relaxed);
        buffer.render(&mut output[..2], 2);
        assert_eq!(&output[..2], &[0.5, -0.5]);
        state.playing.store(false, Ordering::Relaxed);
        let cursor = state.frame.load(Ordering::Relaxed);
        buffer.render(&mut output, 2);
        assert!(output.iter().all(|s| *s == 0.0));
        assert_eq!(state.frame.load(Ordering::Relaxed), cursor);
        state.set_loop(None, 100);
        state.frame.store(7, Ordering::Relaxed);
        state.playing.store(true, Ordering::Relaxed);
        buffer.render(&mut output[..8], 2);
        assert_eq!(&output[..8], &[0.7, -0.7, 0.8, -0.8, 0.9, -0.9, 0.0, 0.0]);
        assert!(!state.playing.load(Ordering::Relaxed));
        // A clip move can briefly put the loop beyond the previous live mix.
        // Keep the transport alive until the updated mix arrives.
        state.set_loop(Some((0.12, 0.15)), 100);
        state.playing.store(true, Ordering::Relaxed);
        buffer.render(&mut output, 2);
        assert!(state.playing.load(Ordering::Relaxed));
        assert!(output.iter().all(|s| *s == 0.0));
        for range in [(f64::NAN, 1.0), (-1.0, 1.0), (1.0, 1.0), (2.0, 1.0)] {
            state.set_loop(Some(range), 100);
            assert_eq!(state.loop_frames.load(Ordering::Relaxed), 0);
        }
    }
    #[test]
    fn mix_swap_keeps_audio_running_at_the_current_frame() {
        let state = Arc::new(PlaybackState {
            playing: AtomicBool::new(true),
            frame: AtomicU64::new(400),
            failed: AtomicBool::new(false),
            loop_frames: AtomicU64::new(0),
            metronome_bpm: AtomicU64::new(0),
            output_gain: AtomicU64::new(1.0f64.to_bits()),
            metronome_gain: AtomicU64::new(1.0f64.to_bits()),
        });
        let mix = |sample| {
            Arc::new(Mix {
                device_signals: Default::default(),
                sample_rate: 8000,
                frames: vec![[sample; 2]; 8000],
                missing: vec![],
                sources: vec![],
                peak: sample,
            })
        };
        let old = mix(0.25);
        let weak = Arc::downgrade(&old);
        let (mut updates, mut buffer) = PlaybackBuffer::new(old, state.clone());
        let mut output = [0.0_f32; 512];
        buffer.render(&mut output, 2);
        assert_eq!(output[0], 0.25);
        updates.pending = Some(mix(0.125));
        updates.maintain();
        let frame = state.frame.load(Ordering::Relaxed);
        buffer.render(&mut output, 2);
        assert_eq!(state.frame.load(Ordering::Relaxed), frame + 256);
        assert!(state.playing.load(Ordering::Relaxed));
        assert_eq!(output[0], 0.25);
        assert_eq!(output[511], 0.125);
        assert!(
            weak.upgrade().is_some(),
            "Old mix was freed in the audio callback"
        );
        updates.maintain();
        assert!(weak.upgrade().is_none());
        // Backpressure postpones updates, never playback, and latest pending edit wins.
        for sample in [0.2, 0.3, 0.4, 0.5] {
            updates.pending = Some(mix(sample));
            updates.maintain();
            buffer.render(&mut output, 2);
            assert!(state.playing.load(Ordering::Relaxed));
        }
        updates.maintain();
        buffer.render(&mut output, 2);
        assert_eq!(output[511], 0.5);
    }
    #[test]
    #[ignore = "Requires a real default audio output device"]
    fn hardware_transport() {
        let rate = Player::output_rate().unwrap();
        let m = Arc::new(Mix {
            device_signals: Default::default(),
            sample_rate: rate,
            frames: vec![[0.0; 2]; rate as usize],
            missing: vec![],
            sources: vec![],
            peak: 0.0,
        });
        let mut p = Player::new(m.clone(), 0.0).unwrap();
        p.play();
        std::thread::sleep(std::time::Duration::from_millis(150));
        assert!(p.seconds() > 0.0);
        assert!(!p.state.failed.load(Ordering::Relaxed));
        let before = p.seconds();
        p.replace_mix(m).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(50));
        p.maintain();
        assert!(p.playing());
        assert!(p.seconds() > before);
        p.pause();
        std::thread::sleep(std::time::Duration::from_millis(30));
        let stopped = p.seconds();
        std::thread::sleep(std::time::Duration::from_millis(30));
        assert_eq!(p.seconds(), stopped);
        p.seek(0.5);
        assert!((p.seconds() - 0.5).abs() < 0.001);
        p.seek(0.0);
        assert_eq!(p.seconds(), 0.0);
    }
    #[test]
    fn eq_has_the_requested_center_gain_and_is_stable() {
        let mut filter = Biquad::peak(48000, 1000.0, -6.0);
        let mut input_energy = 0.0;
        let mut output_energy = 0.0;
        for frame in 0..48000 {
            let x = (std::f64::consts::TAU * 1000.0 * frame as f64 / 48000.0).sin() as f32;
            let y = filter.process(x);
            assert!(y.is_finite());
            if frame > 4800 {
                input_energy += x as f64 * x as f64;
                output_energy += y as f64 * y as f64;
            }
        }
        assert!((10.0_f64 * (output_energy / input_energy).log10() + 6.0).abs() < 0.01);
    }
    #[test]
    fn render_trim_pan_mute_solo_and_gain() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("tone.wav");
        let mut w = hound::WavWriter::create(
            &source,
            hound::WavSpec {
                channels: 1,
                sample_rate: 8000,
                bits_per_sample: 16,
                sample_format: hound::SampleFormat::Int,
            },
        )
        .unwrap();
        for _ in 0..8000 {
            w.write_sample(8192i16).unwrap();
        }
        w.finalize().unwrap();
        let original = std::fs::read(&source).unwrap();
        let mut s = Session::new(Project::new("Test"), root.path().into());
        s.execute(Command::AddTrack {
            name: "Tone".into(),
        })
        .unwrap();
        let tid = s.project.tracks[0].id.clone();
        import(&mut s, &tid, &source, 2.0).unwrap();
        let cid = s.project.tracks[0].clips[0].id.clone();
        s.execute(Command::TrimClip {
            track_id: tid.clone(),
            clip_id: cid,
            offset_seconds: 0.25,
            length_seconds: 0.5,
        })
        .unwrap();
        s.execute(Command::SetTrackPan {
            track_id: tid.clone(),
            pan: -1.0,
        })
        .unwrap();
        s.execute(Command::AddDevice {
            track_id: tid.clone(),
            kind: "builtin.gain".into(),
        })
        .unwrap();
        let did = s.project.tracks[0].devices[0].id.clone();
        s.execute(Command::SetDeviceParameter {
            track_id: tid.clone(),
            device_id: did,
            parameter: "gain_db".into(),
            value: -6.0,
        })
        .unwrap();
        let m = mix(&s.project, root.path(), 8000, &mut MediaCache::default()).unwrap();
        assert_eq!(m.frames.len(), 12000);
        assert_eq!(m.frames[0], [0.0, 0.0]);
        assert!((m.frames[9000][0] - 0.1253).abs() < 0.001);
        assert_eq!(m.frames[9000][1], 0.0);
        export(&m, &root.path().join("out.wav")).unwrap();
        assert!(export(&m, &source).is_err());
        assert_eq!(
            decode(&root.path().join("out.wav")).unwrap().frames.len(),
            12000
        );
        s.execute(Command::SetMasterVolume { volume_db: 12.0 })
            .unwrap();
        s.execute(Command::AddDevice {
            track_id: "master".into(),
            kind: "builtin.limiter".into(),
        })
        .unwrap();
        let limiter = s.project.master_devices[0].id.clone();
        s.execute(Command::SetDeviceParameter {
            track_id: "master".into(),
            device_id: limiter,
            parameter: "ceiling_db".into(),
            value: -12.0,
        })
        .unwrap();
        let limited = mix(&s.project, root.path(), 8000, &mut MediaCache::default()).unwrap();
        assert!(
            limited.peak <= db(-12.0) + 1e-6,
            "Master limiter must follow master volume"
        );
        assert!((limited.frames[9000][0] - db(-12.0)).abs() < 1e-5);
        assert_eq!(limited.frames[9000][1], 0.0);
        export(&limited, &root.path().join("limited.wav")).unwrap();
        assert!(decode(&root.path().join("limited.wav"))
            .unwrap()
            .frames
            .iter()
            .flatten()
            .all(|s| s.abs() <= db(-12.0) + 1e-6));
        s.execute(Command::SetMute {
            track_id: tid,
            mute: true,
        })
        .unwrap();
        assert_eq!(
            mix(&s.project, root.path(), 8000, &mut MediaCache::default())
                .unwrap()
                .peak,
            0.0
        );
        assert_eq!(std::fs::read(source).unwrap(), original);
    }
    #[test]
    fn missing_is_silent_but_export_fails() {
        let root = tempfile::tempdir().unwrap();
        let mut s = Session::new(Project::new("Test"), root.path().into());
        s.execute(Command::AddTrack {
            name: "Missing".into(),
        })
        .unwrap();
        s.execute(Command::ImportAudioClip {
            track_id: s.project.tracks[0].id.clone(),
            source: Source {
                path: root.path().join("absent.wav"),
                kind: SourceKind::External,
            },
            position: Position {
                start_beats: 0.0,
                offset_seconds: 0.0,
                length_seconds: 1.0,
            },
        })
        .unwrap();
        let m = mix(&s.project, root.path(), 8000, &mut MediaCache::default()).unwrap();
        assert_eq!(m.missing.len(), 1);
        assert!(export(&m, &root.path().join("out.wav")).is_err());
    }
}

#[cfg(test)]
mod midi_integration_tests {
    use super::*;
    use velvet_core::{Command, MidiNote, Project, Session};
    #[test]
    fn synced_audio_hits_the_same_beats_as_midi_after_tempo_changes() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("pulses.wav");
        let mut wav = hound::WavWriter::create(
            &path,
            hound::WavSpec {
                channels: 2,
                sample_rate: 8000,
                bits_per_sample: 32,
                sample_format: hound::SampleFormat::Float,
            },
        )
        .unwrap();
        for frame in 0..24000 {
            let pulse = if [4000, 8000, 12000, 16000, 20000].contains(&frame) {
                0.5_f32
            } else {
                0.0
            };
            wav.write_sample(pulse).unwrap();
            wav.write_sample(-pulse).unwrap();
        }
        wav.finalize().unwrap();
        let mut session = Session::new(Project::new("Sync"), root.path().into());
        session
            .execute(Command::AddTrack {
                name: "Audio".into(),
            })
            .unwrap();
        let track_id = session.project.tracks[0].id.clone();
        session
            .execute(Command::ImportAudioClip {
                track_id: track_id.clone(),
                source: Source {
                    path,
                    kind: SourceKind::External,
                },
                position: Position {
                    start_beats: 4.0,
                    offset_seconds: 0.5,
                    length_seconds: 2.0,
                },
            })
            .unwrap();
        let clip_id = session.project.tracks[0].clips[0].id.clone();
        session
            .execute(Command::SetClipTempo {
                track_id: track_id.clone(),
                clip_id: clip_id.clone(),
                source_bpm: Some(120.0),
            })
            .unwrap();
        session
            .execute(Command::AddMidiTrack {
                name: "MIDI".into(),
            })
            .unwrap();
        let midi_id = session.project.tracks[1].id.clone();
        let notes: Vec<_> = (4..8)
            .map(|beat| MidiNote {
                start_beats: beat as f64,
                length_beats: 0.25,
                ..Default::default()
            })
            .collect();
        session
            .execute(Command::SetMidiNotes {
                track_id: midi_id,
                notes: notes.clone(),
            })
            .unwrap();
        for bpm in [60.0, 120.0, 240.0] {
            session.execute(Command::SetTempo { bpm }).unwrap();
            let clip = &session.project.tracks[0].clips[0];
            assert_eq!(clip.duration_seconds(bpm) * bpm / 60.0, 4.0);
            let rendered = mix(
                &session.project,
                root.path(),
                8000,
                &mut MediaCache::default(),
            )
            .unwrap();
            for note in &notes {
                let frame = (note.start_beats * 60.0 / bpm * 8000.0).round() as usize;
                assert_eq!(
                    rendered.frames[frame],
                    [0.5, -0.5],
                    "Audio attack missed MIDI beat at {bpm} BPM"
                );
            }
            assert_eq!(rendered.frames.len(), (8.0 * 60.0 / bpm * 8000.0) as usize);
        }
        session.execute(Command::SaveProject).unwrap();
        let reopened = Session::open(root.path()).unwrap();
        assert_eq!(reopened.project.tracks[0].clips[0].source_bpm, Some(120.0));
        for source_bpm in [f64::NAN, 0.0, 401.0] {
            let before = session.project.clone();
            assert!(session
                .execute(Command::SetClipTempo {
                    track_id: track_id.clone(),
                    clip_id: clip_id.clone(),
                    source_bpm: Some(source_bpm)
                })
                .is_err());
            assert_eq!(session.project, before);
        }
        session
            .execute(Command::SetClipTempo {
                track_id,
                clip_id,
                source_bpm: None,
            })
            .unwrap();
        assert_eq!(
            session.project.tracks[0].clips[0].duration_seconds(240.0),
            2.0
        );
        assert!(session.undo());
        assert_eq!(session.project.tracks[0].clips[0].source_bpm, Some(120.0));
    }
    #[test]
    fn empty_instrument_slot_is_silent_and_assignment_preserves_notes() {
        let root = tempfile::tempdir().unwrap();
        let mut s = Session::new(Project::new("MIDI"), root.path().into());
        s.execute(Command::AddMidiTrack {
            name: "MIDI".into(),
        })
        .unwrap();
        let track_id = s.project.tracks[0].id.clone();
        s.execute(Command::SetMidiNotes {
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
        let mut cache = MediaCache::default();
        let silent = mix(&s.project, root.path(), 8000, &mut cache).unwrap();
        assert_eq!(silent.frames.len(), 4000);
        assert!(silent.frames.iter().all(|f| *f == [0.0; 2]));
        s.execute(Command::SetTrackInstrument {
            track_id: track_id.clone(),
            kind: Some("builtin.dot".into()),
        })
        .unwrap();
        assert!(mix(&s.project, root.path(), 8000, &mut cache).unwrap().peak > 0.01);
        s.execute(Command::SetTrackInstrument {
            track_id,
            kind: None,
        })
        .unwrap();
        assert_eq!(s.project.tracks[0].notes.len(), 1);
        assert_eq!(
            mix(&s.project, root.path(), 8000, &mut cache).unwrap().peak,
            0.0
        );
    }
}

fn process_chain(
    frames: &mut [[f32; 2]],
    chain: &[velvet_core::Device],
    bpm: f64,
    rate: u32,
    scopes: &std::collections::HashSet<String>,
    signals: &mut HashMap<String, Arc<ScopeSignal>>,
    mut plugins: Option<&mut plugins::RenderCache>,
) -> anyhow::Result<()> {
    for device in chain {
        if device.plugin_path().is_some() {
            process_plugin(frames, device, &[], bpm, rate, plugins.as_deref_mut())?;
        } else {
            if device.kind == "builtin.beat" {
                beat::process(frames, device, bpm, rate);
            } else {
                devices::process_devices(frames, std::slice::from_ref(device), rate);
            }
        }
        if scopes.contains(&device.id) {
            signals.insert(device.id.clone(), Arc::new(ScopeSignal::new(frames, rate)));
        }
    }
    Ok(())
}

fn process_plugin(frames: &mut [[f32; 2]], device: &velvet_core::Device,
    notes: &[velvet_core::MidiNote], bpm: f64, rate: u32,
    cache: Option<&mut plugins::RenderCache>) -> Result<()> {
    match cache {
        Some(cache) => cache.process(frames, device, notes, bpm, rate),
        None => plugins::process(frames, device, notes, bpm, rate),
    }
}

/// Bounded stereo output preview. Min/max buckets preserve transients in long arrangements.
pub struct ScopeSignal {
    pub sample_rate: u32,
    pub stride: usize,
    pub buckets: Vec<[[f32; 2]; 2]>,
}
impl ScopeSignal {
    pub fn new(frames: &[[f32; 2]], sample_rate: u32) -> Self {
        const MAX_BUCKETS: usize = 262_144;
        let stride = frames.len().div_ceil(MAX_BUCKETS).max(1);
        let buckets = frames
            .chunks(stride)
            .map(|chunk| {
                let mut bounds = [[f32::INFINITY; 2], [f32::NEG_INFINITY; 2]];
                for frame in chunk {
                    for (channel, sample) in frame.iter().enumerate() {
                        bounds[0][channel] = bounds[0][channel].min(*sample);
                        bounds[1][channel] = bounds[1][channel].max(*sample);
                    }
                }
                bounds
            })
            .collect();
        Self {
            sample_rate,
            stride,
            buckets,
        }
    }
}

#[cfg(test)]
mod scope_tests {
    use super::*;
    #[test]
    fn cached_preview_obeys_pitch_and_velocity_and_releases_the_note() {
        let mut renderer = LiveMixer::default();
        let device = velvet_core::Device::new("builtin.dot").unwrap();
        let low = renderer.preview(&device, 60, 30, 1, 120.0, 8000).unwrap();
        let high = renderer.preview(&device, 72, 110, 1, 120.0, 8000).unwrap();
        assert!(high.peak > low.peak);
        assert_ne!(high.frames, low.frames);
        assert!(high.frames.last().unwrap().iter().all(|sample| sample.abs() < 0.001));
    }
    #[test]
    fn live_mix_reuses_unchanged_tracks_and_matches_fresh_audio_after_edits() {
        let root = tempfile::tempdir().unwrap();
        let mut session = Session::new(Project::new("Live cache"), root.path().into());
        for key in [60, 67] {
            session.execute(Command::AddMidiTrack { name: format!("Note {key}") }).unwrap();
            let id = session.project.tracks.last().unwrap().id.clone();
            session.execute(Command::SetTrackInstrument {
                track_id: id.clone(), kind: Some("builtin.dot".into()),
            }).unwrap();
            session.execute(Command::SetMidiNotes {
                track_id: id, notes: vec![velvet_core::MidiNote { key, velocity: 100, length_beats: 2.0, ..Default::default() }],
            }).unwrap();
        }
        let mut renderer = LiveMixer::default();
        let mut media = MediaCache::default();
        let scopes = std::collections::HashSet::from([session.project.tracks[1].synth.as_ref().unwrap().id.clone()]);
        let first = renderer.mix(&session.project, root.path(), 8000, &mut media, &scopes).unwrap();
        let unaffected = session.project.tracks[1].id.clone();
        let retained = renderer.tracks[&unaffected].frames.as_ptr();
        session.project.tracks[0].synth.as_mut().unwrap().parameters.insert("gain_db".into(), -24.0);
        let updated = renderer.mix(&session.project, root.path(), 8000, &mut media, &scopes).unwrap();
        assert_eq!(retained, renderer.tracks[&unaffected].frames.as_ptr());
        let fresh = mix_with_scopes(&session.project, root.path(), 8000, &mut media, &scopes).unwrap();
        assert_eq!(updated.frames, fresh.frames);
        assert_eq!(updated.device_signals.keys().collect::<std::collections::HashSet<_>>(), fresh.device_signals.keys().collect());
        assert!(Arc::ptr_eq(&first.device_signals.values().next().unwrap(), &updated.device_signals.values().next().unwrap()));
        session.project.tracks[1].mixer.volume_db = -12.0;
        session.project.tracks[1].mixer.pan = 0.6;
        let updated = renderer.mix(&session.project, root.path(), 8000, &mut media, &scopes).unwrap();
        assert_eq!(retained, renderer.tracks[&unaffected].frames.as_ptr());
        let fresh = mix_with_scopes(&session.project, root.path(), 8000, &mut media, &scopes).unwrap();
        assert_eq!(updated.frames, fresh.frames);
        session.project.tempo.bpm = 90.0;
        let updated = renderer.mix(&session.project, root.path(), 16000, &mut media, &scopes).unwrap();
        let fresh = mix_with_scopes(&session.project, root.path(), 16000, &mut media, &scopes).unwrap();
        assert_eq!(updated.frames, fresh.frames);
        session.project.tracks.pop();
        renderer.mix(&session.project, root.path(), 16000, &mut media, &scopes).unwrap();
        assert!(!renderer.tracks.contains_key(&unaffected));
    }
    #[test]
    fn live_mix_invalidates_replaced_decoded_audio_and_deleted_sources() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("source.wav");
        // Decoded data is supplied directly to isolate cache invalidation from decoding.
        std::fs::write(&path, []).unwrap();
        let mut session = Session::new(Project::new("Source versions"), root.path().into());
        session.execute(Command::AddTrack { name: "Audio".into() }).unwrap();
        session.execute(Command::ImportAudioClip {
            track_id: session.project.tracks[0].id.clone(),
            source: Source { path: path.clone(), kind: SourceKind::External },
            position: Position { start_beats: 0.0, offset_seconds: 0.0, length_seconds: 1.0 },
        }).unwrap();
        let mut media = MediaCache::default();
        media.files.insert(path.clone(), Arc::new(AudioData { sample_rate: 8000, frames: vec![[0.2; 2]; 8000] }));
        let mut renderer = LiveMixer::default();
        let scopes = std::collections::HashSet::new();
        let before = renderer.mix(&session.project, root.path(), 8000, &mut media, &scopes).unwrap();
        media.files.insert(path.clone(), Arc::new(AudioData { sample_rate: 8000, frames: vec![[0.6; 2]; 8000] }));
        let after = renderer.mix(&session.project, root.path(), 8000, &mut media, &scopes).unwrap();
        assert_ne!(before.frames, after.frames);
        assert_eq!(after.frames, mix(&session.project, root.path(), 8000, &mut media).unwrap().frames);
        std::fs::remove_file(&path).unwrap();
        let missing = renderer.mix(&session.project, root.path(), 8000, &mut media, &scopes).unwrap();
        assert_eq!(missing.peak, 0.0);
        assert_eq!(missing.missing, vec![path]);
    }
    #[test]
    fn scopes_capture_device_output_before_following_devices_and_preserve_peaks() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("input.wav");
        let mut wav = hound::WavWriter::create(
            &source,
            hound::WavSpec {
                channels: 2,
                sample_rate: 8000,
                bits_per_sample: 32,
                sample_format: hound::SampleFormat::Float,
            },
        )
        .unwrap();
        for _ in 0..8000 {
            wav.write_sample(0.4_f32).unwrap();
            wav.write_sample(-0.2_f32).unwrap();
        }
        wav.finalize().unwrap();
        let mut session = Session::new(Project::new("Scope"), root.path().into());
        session
            .execute(Command::AddTrack {
                name: "Track".into(),
            })
            .unwrap();
        let track_id = session.project.tracks[0].id.clone();
        session
            .execute(Command::ImportAudioClip {
                track_id: track_id.clone(),
                source: Source {
                    path: source,
                    kind: SourceKind::External,
                },
                position: Position {
                    start_beats: 0.0,
                    offset_seconds: 0.0,
                    length_seconds: 1.0,
                },
            })
            .unwrap();
        for _ in 0..2 {
            session
                .execute(Command::AddDevice {
                    track_id: track_id.clone(),
                    kind: "builtin.gain".into(),
                })
                .unwrap();
        }
        let id = session.project.tracks[0].devices[0].id.clone();
        session
            .execute(Command::SetDeviceParameter {
                track_id: track_id.clone(),
                device_id: id.clone(),
                parameter: "gain_db".into(),
                value: -6.0,
            })
            .unwrap();
        let last = session.project.tracks[0].devices[1].id.clone();
        session
            .execute(Command::SetDeviceParameter {
                track_id,
                device_id: last,
                parameter: "gain_db".into(),
                value: -12.0,
            })
            .unwrap();
        let mut cache = MediaCache::default();
        let enabled = std::collections::HashSet::from([id.clone()]);
        let mix =
            mix_with_scopes(&session.project, root.path(), 8000, &mut cache, &enabled).unwrap();
        let signal = &mix.device_signals[&id];
        assert!((signal.buckets[0][0][0] - 0.4 * db(-6.0)).abs() < 1e-6);
        assert!((signal.buckets[0][0][1] + 0.2 * db(-6.0)).abs() < 1e-6);
        assert!(mix.frames[0][0] < signal.buckets[0][0][0] * 0.3);
        assert!(super::mix(&session.project, root.path(), 8000, &mut cache)
            .unwrap()
            .device_signals
            .is_empty());
        let mut long = vec![[0.0; 2]; 600_000];
        long[1] = [0.9, -0.8];
        let preview = ScopeSignal::new(&long, 48000);
        assert!(preview.buckets.len() <= 262_144);
        assert_eq!(preview.buckets[0][1][0], 0.9);
        assert_eq!(preview.buckets[0][0][1], -0.8);
    }
}
