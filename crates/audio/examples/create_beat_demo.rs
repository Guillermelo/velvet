//! Local demonstration of Beat, generated without external audio or plugins.
use anyhow::Result;
use std::path::PathBuf;
use velvet_core::{Command, Position, Project, Session, Source, SourceKind};

fn main() -> Result<()> {
    let root = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("examples/beat-demo"));
    anyhow::ensure!(
        !root.join("project.yaml").exists(),
        "Choose an empty demo folder"
    );
    std::fs::create_dir_all(&root)?;
    let path = root.join("beat.wav");
    let rate = 48000;
    let mut wav = hound::WavWriter::create(
        &path,
        hound::WavSpec {
            channels: 2,
            sample_rate: rate,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        },
    )?;
    for i in 0..rate * 8 {
        let t = i as f64 / rate as f64;
        let beat = t % 0.5;
        let kick = (std::f64::consts::TAU * (45.0 * beat + 4.0 * (1.0 - (-beat * 30.0).exp())))
            .sin()
            * (-beat * 18.0).exp();
        let key = [261.63, 329.63, 392.0, 523.25][(t * 4.0) as usize % 4];
        let note = (std::f64::consts::TAU * key * t).sin() * (-(t % 0.25) * 12.0).exp();
        let sample = ((kick * 0.35 + note * 0.2) * i16::MAX as f64) as i16;
        wav.write_sample(sample)?;
        wav.write_sample(sample)?;
    }
    wav.finalize()?;
    let mut session = Session::new(Project::new("Beat / Velvet"), root);
    session.execute(Command::AddTrack {
        name: "Beat playground".into(),
    })?;
    let track = session.project.tracks[0].id.clone();
    session.execute(Command::ImportAudioClip {
        track_id: track.clone(),
        source: Source {
            path: path.canonicalize()?,
            kind: SourceKind::External,
        },
        position: Position {
            start_beats: 0.0,
            offset_seconds: 0.0,
            length_seconds: 8.0,
        },
    })?;
    session.execute(Command::AddDevice {
        track_id: track.clone(),
        kind: "builtin.beat".into(),
    })?;
    let device = session.project.tracks[0].devices[0].id.clone();
    session.execute(Command::SetDeviceParameter {
        track_id: track,
        device_id: device,
        parameter: "time_slot".into(),
        value: 1.0,
    })?;
    session.execute(Command::SaveProject)?;
    let mut cache = velvet_audio::MediaCache::default();
    let mix = velvet_audio::mix(&session.project, &session.root, rate, &mut cache)?;
    velvet_audio::export(&mix, &session.root.join("half-speed.wav"))?;
    Ok(())
}
