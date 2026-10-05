//! A self-contained piano-roll demo using only Velvet's native instrument.
use anyhow::Result;
use std::path::PathBuf;
use velvet_core::{Command, MidiNote, MidiRegion, Project, Session};
fn main() -> Result<()> {
    let root = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("examples/piano-roll-demo"));
    anyhow::ensure!(
        !root.join("project.yaml").exists(),
        "Demo already exists; choose an empty folder"
    );
    std::fs::create_dir_all(&root)?;
    let mut s = Session::new(Project::new("Glass notes"), root);
    s.execute(Command::SetTempo { bpm: 108.0 })?;
    for name in ["Glass keys", "Low tide"] {
        s.execute(Command::AddMidiTrack { name: name.into() })?;
        let id = s.project.tracks.last().unwrap().id.clone();
        s.execute(Command::SetTrackInstrument {
            track_id: id.clone(),
            kind: Some("builtin.dot".into()),
        })?;
        s.execute(Command::SetSynthParameter {
            track_id: id.clone(),
            parameter: "gain_db".into(),
            value: -15.0,
        })?;
        s.execute(Command::SetSynthParameter {
            track_id: id.clone(),
            parameter: "release_ms".into(),
            value: 350.0,
        })?;
        let mut notes = vec![];
        for (bar, chord) in [
            [60, 64, 67, 71],
            [57, 60, 64, 67],
            [53, 57, 60, 64],
            [55, 59, 62, 65],
        ]
        .iter()
        .enumerate()
        {
            if name == "Glass keys" {
                for (j, key) in chord.iter().enumerate() {
                    notes.push(MidiNote {
                        key: *key,
                        velocity: 72 + j as u8 * 7,
                        start_beats: bar as f64 * 4.0 + j as f64 * 0.03,
                        length_beats: 3.0 - j as f64 * 0.03,
                        channel: 1,
                        ..MidiNote::default()
                    });
                }
                for (step, key) in [
                    (0, chord[3] + 12),
                    (2, chord[2] + 12),
                    (5, chord[1] + 12),
                    (7, chord[2] + 12),
                ] {
                    notes.push(MidiNote {
                        key,
                        velocity: 98,
                        start_beats: bar as f64 * 4.0 + step as f64 * 0.5,
                        length_beats: 0.4,
                        channel: 2,
                        ..MidiNote::default()
                    });
                }
            } else {
                notes.push(MidiNote {
                    key: chord[0] - 12,
                    velocity: 90,
                    start_beats: bar as f64 * 4.0,
                    length_beats: 3.5,
                    channel: 1,
                    ..MidiNote::default()
                });
            }
        }
        s.execute(Command::SetMidiScore {
            track_id: id,
            notes,
            region: Some(MidiRegion {
                start_beats: 0.0,
                offset_beats: 0.0,
                length_beats: 16.0,
            }),
        })?;
    }
    s.execute(Command::SaveProject)?;
    println!("{}", s.root.display());
    Ok(())
}
