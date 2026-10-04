//! Deterministic, locally synthesized demo. No downloaded or AI-generated media.
use anyhow::Result;
use std::path::PathBuf;
use velvet_core::{Command, Position, Project, Session, Source, SourceKind};

fn main() -> Result<()> {
    let root = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("examples/demo"));
    anyhow::ensure!(
        !root.join("project.yaml").exists(),
        "Demo already exists; choose an empty folder"
    );
    std::fs::create_dir_all(root.join("generated"))?;
    let mut s = Session::new(Project::new("Quiet hours"), root.clone());
    let rate = 48000u32;
    let colors = [
        [168, 147, 112],
        [130, 157, 155],
        [159, 140, 167],
        [131, 148, 175],
    ];
    for (index, name) in ["Drums", "Bass", "Keys", "Atmosphere"].iter().enumerate() {
        let filename = format!("{}.wav", name.to_lowercase());
        let path = root.join("generated").join(&filename);
        let mut w = hound::WavWriter::create(
            &path,
            hound::WavSpec {
                channels: 2,
                sample_rate: rate,
                bits_per_sample: 16,
                sample_format: hound::SampleFormat::Int,
            },
        )?;
        for frame in 0..rate as usize * 8 {
            let t = frame as f64 / rate as f64;
            let beat = t % 0.5;
            let tau = std::f64::consts::TAU;
            let wave = match index {
                0 => {
                    let kick = (tau * (52.0 * beat + 18.0 * (1.0 - (-beat * 35.0).exp()))).sin()
                        * (-beat * 17.0).exp();
                    let hat_time = t % 0.25;
                    let noise = (((frame as u64 * 1664525 + 1013904223) % 65536) as f64 / 32768.0
                        - 1.0)
                        * (-hat_time * 100.0).exp();
                    kick * 0.55 + noise * 0.1
                }
                1 => {
                    let frequency = [65.406, 65.406, 87.307, 77.782][(t / 2.0) as usize % 4];
                    (tau * frequency * t).sin() * (-beat * 3.0).exp() * 0.3
                }
                2 => {
                    let note = [261.626, 311.127, 391.995, 466.164][(t / 0.5) as usize % 4];
                    ((tau * note * t).sin() + (tau * note * 2.0 * t).sin() * 0.3)
                        * (-beat * 7.0).exp()
                        * 0.18
                }
                _ => {
                    let env = (t / 1.5).min(1.0) * ((8.0 - t) / 1.5).min(1.0);
                    ((tau * 130.813 * t).sin()
                        + (tau * 155.563 * t).sin()
                        + (tau * 195.998 * t).sin())
                        * env
                        * 0.055
                }
            };
            for channel in 0..2 {
                let value = wave
                    * if index == 3 && channel == 1 {
                        0.85
                    } else {
                        1.0
                    };
                w.write_sample((value * 32767.0).round() as i16)?;
            }
        }
        w.finalize()?;
        s.execute(Command::AddTrack {
            name: name.to_string(),
        })?;
        let tid = s.project.tracks.last().unwrap().id.clone();
        s.execute(Command::SetTrackColor {
            track_id: tid.clone(),
            color: colors[index],
        })?;
        s.execute(Command::SetTrackVolume {
            track_id: tid.clone(),
            volume_db: -3.0,
        })?;
        for start in match index {
            0 => vec![0.0, 16.0, 32.0],
            1 => vec![8.0, 24.0, 40.0],
            2 => vec![16.0, 32.0],
            _ => vec![0.0, 32.0],
        } {
            s.execute(Command::ImportAudioClip {
                track_id: tid.clone(),
                source: Source {
                    path: PathBuf::from("generated").join(&filename),
                    kind: SourceKind::Project,
                },
                position: Position {
                    start_beats: start,
                    offset_seconds: 0.0,
                    length_seconds: 8.0,
                },
            })?;
        }
        s.execute(Command::AddDevice {
            track_id: tid.clone(),
            kind: "builtin.gain".into(),
        })?;
        if index == 0 {
            s.execute(Command::AddDevice {
                track_id: tid,
                kind: "builtin.eq".into(),
            })?;
        }
    }
    s.execute(Command::SaveProject)?;
    println!("Created demo at {}", root.display());
    Ok(())
}
