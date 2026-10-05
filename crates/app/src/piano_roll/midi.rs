//! Note-only Standard MIDI File interchange. Musical ticks preserve timing across tempos.
use anyhow::{bail, ensure, Context, Result};
use midly::{Format, Header, MetaMessage, MidiMessage, Smf, Timing, TrackEvent, TrackEventKind};
use std::collections::{BTreeMap, VecDeque};
use velvet_core::MidiNote;

pub(super) fn decode(bytes: &[u8]) -> Result<Vec<MidiNote>> {
    ensure!(bytes.len() <= 16 * 1024 * 1024, "MIDI file exceeds 16 MB");
    let smf = Smf::parse(bytes).context("Invalid MIDI file")?;
    ensure!(
        smf.header.format != Format::Sequential,
        "MIDI format 2 contains independent songs; export as format 0 or 1 first"
    );
    let Timing::Metrical(ppq) = smf.header.timing else {
        bail!("SMPTE MIDI timing is not supported; use a beat-based MIDI file");
    };
    ensure!(ppq.as_int() > 0, "MIDI resolution must be positive");
    let resolution = ppq.as_int() as f64;
    let mut notes = vec![];
    for events in smf.tracks {
        let mut tick = 0_u64;
        let mut active: BTreeMap<(u8, u8), VecDeque<(u64, u8)>> = BTreeMap::new();
        let mut note_count = 0;
        for event in events {
            tick = tick
                .checked_add(event.delta.as_int() as u64)
                .context("MIDI time overflow")?;
            if let TrackEventKind::Midi { channel, message } = event.kind {
                let channel = channel.as_int() + 1;
                match message {
                    MidiMessage::NoteOn { key, vel } if vel.as_int() > 0 => {
                        note_count += 1;
                        ensure!(
                            note_count + notes.len() <= super::MAX_NOTES,
                            "MIDI score exceeds 10,000 notes"
                        );
                        active
                            .entry((channel, key.as_int()))
                            .or_default()
                            .push_back((tick, vel.as_int()));
                    }
                    MidiMessage::NoteOff { key, .. } | MidiMessage::NoteOn { key, .. } => {
                        if let Some((start, velocity)) = active
                            .get_mut(&(channel, key.as_int()))
                            .and_then(VecDeque::pop_front)
                        {
                            if tick > start {
                                notes.push(MidiNote {
                                    key: key.as_int(),
                                    channel,
                                    velocity,
                                    start_beats: start as f64 / resolution,
                                    length_beats: (tick - start) as f64 / resolution,
                                    muted: false,
                                });
                            }
                            note_count -= 1;
                        }
                    }
                    _ => {}
                }
            }
        }
        ensure!(
            active.values().all(VecDeque::is_empty),
            "MIDI contains notes without a matching note-off"
        );
    }
    ensure!(
        notes.len() <= super::MAX_NOTES,
        "MIDI score exceeds 10,000 notes"
    );
    notes.sort_by(|a, b| {
        a.start_beats
            .total_cmp(&b.start_beats)
            .then(a.key.cmp(&b.key))
    });
    Ok(notes)
}

pub(super) fn encode(notes: &[MidiNote], bpm: f64) -> Result<Vec<u8>> {
    ensure!(
        bpm.is_finite() && (20.0..=300.0).contains(&bpm),
        "Invalid MIDI tempo"
    );
    let mut events = vec![];
    for note in notes.iter().filter(|n| !n.muted) {
        ensure!(
            note.start_beats.is_finite()
                && note.start_beats >= 0.0
                && note.length_beats.is_finite()
                && note.length_beats > 0.0
                && note.key <= 127
                && (1..=16).contains(&note.channel)
                && (1..=127).contains(&note.velocity),
            "Invalid MIDI note"
        );
        let end = ((note.start_beats + note.length_beats) * 960.0).round();
        ensure!(end < u32::MAX as f64, "MIDI score is too long");
        let start = (note.start_beats * 960.0).round() as u32;
        let end = (end as u32).max(start + 1);
        let channel = (note.channel - 1).into();
        events.push((
            start,
            1,
            TrackEventKind::Midi {
                channel,
                message: MidiMessage::NoteOn {
                    key: note.key.into(),
                    vel: note.velocity.into(),
                },
            },
        ));
        events.push((
            end,
            0,
            TrackEventKind::Midi {
                channel,
                message: MidiMessage::NoteOff {
                    key: note.key.into(),
                    vel: 0.into(),
                },
            },
        ));
    }
    events.sort_by_key(|(tick, priority, _)| (*tick, *priority));
    let mut track = vec![TrackEvent {
        delta: 0.into(),
        kind: TrackEventKind::Meta(MetaMessage::Tempo(
            ((60_000_000.0 / bpm).round() as u32).into(),
        )),
    }];
    let mut previous = 0;
    for (tick, _, kind) in events {
        ensure!(
            tick - previous <= 0x0fff_ffff,
            "MIDI event interval exceeds SMF limits"
        );
        track.push(TrackEvent {
            delta: (tick - previous).into(),
            kind,
        });
        previous = tick;
    }
    track.push(TrackEvent {
        delta: 0.into(),
        kind: TrackEventKind::Meta(MetaMessage::EndOfTrack),
    });
    let smf = Smf {
        header: Header::new(Format::SingleTrack, Timing::Metrical(960.into())),
        tracks: vec![track],
    };
    let mut bytes = vec![];
    smf.write_std(&mut bytes)?;
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn roundtrip_preserves_overlapping_notes_channels_and_note_off_order() {
        let notes = vec![
            MidiNote {
                channel: 16,
                start_beats: 0.25,
                length_beats: 0.5,
                ..MidiNote::default()
            },
            MidiNote {
                channel: 16,
                start_beats: 0.5,
                length_beats: 0.5,
                velocity: 45,
                ..MidiNote::default()
            },
            MidiNote {
                key: 64,
                start_beats: 0.75,
                length_beats: 0.25,
                ..MidiNote::default()
            },
            MidiNote {
                muted: true,
                ..MidiNote::default()
            },
        ];
        let bytes = encode(&notes, 123.0).unwrap();
        assert_eq!(decode(&bytes).unwrap(), notes[..3]);
        assert!(decode(b"not midi").is_err());
        let mut invalid = notes[0].clone();
        invalid.start_beats = f64::NAN;
        assert!(encode(&[invalid], 120.0).is_err());
    }
}
