//! Decoding and DSP happen outside the CPAL callback. The callback only reads
//! an immutable stereo mix and bounded atomic transport controls.
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
mod devices;
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
            let n = (clip.position.length_seconds * rate as f64).round() as usize;
            for i in 0..n.min(count.saturating_sub(start)) {
                let source = (clip.position.offset_seconds + i as f64 / rate as f64)
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
        devices::process_devices(&mut bus, &track.devices, rate);
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
    devices::process_devices(&mut output, &project.master_devices, rate);
    let mut peak: f32 = 0.0;
    for f in &mut output {
        for s in f {
            peak = peak.max(s.abs());
            *s = s.clamp(-1.0, 1.0);
        }
    }
    Ok(Mix {
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
        for frame in output.chunks_mut(channels) {
            let mut sample = [0.0; 2];
            if self.state.playing.load(Ordering::Relaxed) {
                let index = self.state.frame.fetch_add(1, Ordering::Relaxed) as usize;
                if let Some(f) = self.mix.frames.get(index) {
                    sample = *f;
                    if let Some(old) = &self.previous {
                        let prior = old.frames.get(index).copied().unwrap_or([0.0; 2]);
                        let blend = (self.fade as f32 / 128.0).min(1.0);
                        for c in 0..2 {
                            sample[c] = prior[c] + (sample[c] - prior[c]) * blend;
                        }
                    }
                } else {
                    self.state.playing.store(false, Ordering::Relaxed);
                }
            }
            self.fade = (self.fade + 1).min(128);
            for (c, s) in frame.iter_mut().enumerate() {
                *s = T::from_sample(if channels == 1 {
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
    fn mix_swap_keeps_audio_running_at_the_current_frame() {
        let state = Arc::new(PlaybackState {
            playing: AtomicBool::new(true),
            frame: AtomicU64::new(400),
            failed: AtomicBool::new(false),
        });
        let mix = |sample| {
            Arc::new(Mix {
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
