//! Beat-synchronized offset and gain envelopes shared by the editor and renderer.
use serde::{Deserialize, Serialize};

pub const TIME_PRESETS: [&str; 36] = [
    "Bypass",
    "Half speed",
    "Quarter speed",
    "Double speed",
    "Reverse",
    "Freeze",
    "Repeat 1/2",
    "Repeat 1/4",
    "Repeat 1/8",
    "Repeat 1/16",
    "Repeat 1/32",
    "Triplet repeat",
    "Back 1 beat",
    "Back 2 beats",
    "Back 4 beats",
    "Back 8 beats",
    "Scratch up",
    "Scratch down",
    "Scratch wave",
    "Vinyl stop",
    "Vinyl start",
    "Reverse 1/2",
    "Reverse 1/4",
    "Reverse 1/8",
    "Slow then fast",
    "Fast then slow",
    "Pitch rise",
    "Pitch fall",
    "Shuffle",
    "Shuffle triplet",
    "Glitch 1",
    "Glitch 2",
    "Glitch 3",
    "Stutter build",
    "Tape wobble",
    "Turnaround",
];
pub const VOLUME_PRESETS: [&str; 36] = [
    "Bypass",
    "Gate 1/2",
    "Gate 1/4",
    "Gate 1/8",
    "Gate 1/16",
    "Gate 1/32",
    "Gate triplet",
    "Gate offbeat",
    "Sidechain",
    "Pump 1/2",
    "Pump 1/4",
    "Pump 1/8",
    "Fade in",
    "Fade out",
    "Triangle",
    "Tremolo",
    "Pulse 25%",
    "Pulse 75%",
    "Trance gate",
    "Trance gate 2",
    "Trance gate 3",
    "Break gate",
    "Break gate 2",
    "Syncopation",
    "Chop build",
    "Chop decay",
    "Swell",
    "Swell 1/2",
    "Duck 1/4",
    "Duck 1/8",
    "Stutter",
    "Stutter triplet",
    "Double pulse",
    "Heartbeat",
    "Silence",
    "Soft tremolo",
];

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BeatPoint {
    pub x: f64,
    /// Time: delay in loop lengths (0..2). Volume: gain (0..1).
    pub y: f64,
    /// 0 = linear, 1 = hold, 2 = smooth. Applied to the outgoing segment.
    pub curve: u8,
}

pub fn value(points: &[BeatPoint], phase: f64) -> f64 {
    let i = points.partition_point(|p| p.x <= phase).saturating_sub(1);
    let a = &points[i];
    let Some(b) = points.get(i + 1) else {
        return a.y;
    };
    let t = ((phase - a.x) / (b.x - a.x)).clamp(0.0, 1.0);
    let t = match a.curve {
        1 => 0.0,
        2 => t * t * (3.0 - 2.0 * t),
        _ => t,
    };
    a.y + (b.y - a.y) * t
}

pub fn preset(time: bool, slot: usize) -> Vec<BeatPoint> {
    let point = |x, y, curve| BeatPoint { x, y, curve };
    let line = |a, b| vec![point(0.0, a, 0), point(1.0, b, 0)];
    let repeat = |count: usize, slope: f64, offset: f64| {
        let mut points = Vec::new();
        for i in 0..count {
            let x = i as f64 / count as f64;
            let delay_slope = if slope == 1.0 { 0.0 } else { slope };
            points.push(point(x, offset + x, 0));
            points.push(point(
                (i + 1) as f64 / count as f64 - 1e-7,
                offset + x + delay_slope / count as f64,
                1,
            ));
        }
        points.push(point(1.0, points.last().unwrap().y, 0));
        points
    };
    if time {
        return match slot {
            0 => line(0.0, 0.0),
            1 => line(0.0, 0.5),
            2 => line(0.0, 0.75),
            3 => line(1.0, 0.0),
            4 => line(0.0, 2.0),
            5 => line(0.0, 1.0),
            6..=11 => repeat([2, 4, 8, 16, 32, 12][slot - 6], 1.0, 0.0),
            12..=15 => line(
                [0.25, 0.5, 1.0, 2.0][slot - 12],
                [0.25, 0.5, 1.0, 2.0][slot - 12],
            ),
            16 => vec![point(0.0, 0.5, 2), point(0.5, 0.0, 2), point(1.0, 0.5, 0)],
            17 => vec![point(0.0, 0.0, 2), point(0.5, 1.0, 2), point(1.0, 0.0, 0)],
            18 => (0..=32)
                .map(|i| {
                    let x = i as f64 / 32.0;
                    point(x, 0.3 + 0.25 * (x * std::f64::consts::TAU * 2.0).sin(), 0)
                })
                .collect(),
            19 => (0..=32)
                .map(|i| {
                    let x = i as f64 / 32.0;
                    point(x, x * x / 2.0, 0)
                })
                .collect(),
            20 => (0..=32)
                .map(|i| {
                    let x = i as f64 / 32.0;
                    point(x, x - x * x / 2.0, 0)
                })
                .collect(),
            21..=23 => repeat([2, 4, 8][slot - 21], 2.0, 0.0),
            24 => vec![point(0.0, 0.0, 0), point(0.5, 0.375, 0), point(1.0, 0.0, 0)],
            25 => vec![point(0.0, 0.5, 0), point(0.5, 0.0, 0), point(1.0, 0.375, 0)],
            26 => (0..=32)
                .map(|i| {
                    let x = i as f64 / 32.0;
                    point(x, 0.5 + x - x * x * 1.5, 0)
                })
                .collect(),
            27 => (0..=32)
                .map(|i| {
                    let x = i as f64 / 32.0;
                    point(x, x * x * 0.75, 0)
                })
                .collect(),
            28..=29 => repeat(if slot == 28 { 8 } else { 12 }, 0.5, 0.0),
            30..=32 => (0..=16)
                .map(|i| point(i as f64 / 16.0, ((i * (slot - 27)) % 7) as f64 / 8.0, 1))
                .collect(),
            33 => {
                let mut p = repeat(16, 1.0, 0.0);
                for a in &mut p {
                    a.y *= a.x;
                }
                p
            }
            34 => (0..=64)
                .map(|i| {
                    let x = i as f64 / 64.0;
                    point(x, 0.02 + 0.015 * (x * std::f64::consts::TAU * 3.0).sin(), 0)
                })
                .collect(),
            _ => vec![point(0.0, 0.0, 0), point(0.5, 1.0, 0), point(1.0, 0.0, 0)],
        };
    }
    let pulse = |count: usize, duty: f64, offbeat: bool| {
        let mut p = Vec::new();
        for i in 0..count {
            p.push(point(
                i as f64 / count as f64,
                if offbeat { 0.0 } else { 1.0 },
                1,
            ));
            p.push(point(
                (i as f64 + duty) / count as f64,
                if offbeat { 1.0 } else { 0.0 },
                1,
            ));
        }
        p.push(point(1.0, 1.0, 1));
        p
    };
    let pump = |count: usize| {
        let mut p = Vec::new();
        for i in 0..count {
            p.push(point(i as f64 / count as f64, 0.0, 2));
            p.push(point((i as f64 + 0.75) / count as f64, 1.0, 1));
        }
        p.push(point(1.0, 1.0, 1));
        p
    };
    match slot {
        0 => line(1.0, 1.0),
        1..=6 => pulse([2, 4, 8, 16, 32, 12][slot - 1], 0.5, false),
        7 => pulse(8, 0.5, true),
        8..=11 => pump([4, 2, 4, 8][slot - 8]),
        12 => line(0.0, 1.0),
        13 => line(1.0, 0.0),
        14 => vec![point(0.0, 0.0, 0), point(0.5, 1.0, 0), point(1.0, 0.0, 0)],
        15 | 35 => (0..=64)
            .map(|i| {
                let x = i as f64 / 64.0;
                let v = 0.5 + 0.5 * (x * std::f64::consts::TAU * 4.0).cos();
                point(x, if slot == 35 { 0.5 + v * 0.5 } else { v }, 0)
            })
            .collect(),
        16 => pulse(8, 0.25, false),
        17 => pulse(8, 0.75, false),
        18..=23 => (0..=32)
            .map(|i| {
                point(
                    i as f64 / 32.0,
                    if (i * (slot - 15) + i / 4) % 7 < 3 {
                        1.0
                    } else {
                        0.0
                    },
                    1,
                )
            })
            .collect(),
        24..=25 => {
            let mut p = pulse(16, 0.5, false);
            for a in &mut p {
                a.y *= if slot == 24 { a.x } else { 1.0 - a.x };
            }
            p
        }
        26 => vec![point(0.0, 0.0, 2), point(0.5, 1.0, 2), point(1.0, 0.0, 0)],
        27 => pump(2),
        28 => pump(4),
        29 => pump(8),
        30 => pulse(16, 0.25, false),
        31 => pulse(12, 0.25, false),
        32 => pulse(2, 0.25, false),
        33 => pulse(4, 0.2, false),
        _ => line(0.0, 0.0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Command, Device, Project, Session};
    #[test]
    fn all_factory_slots_validate_and_edits_roundtrip_with_history() {
        let root = tempfile::tempdir().unwrap();
        let mut session = Session::new(Project::new("Beat"), root.path().into());
        session
            .execute(Command::AddDevice {
                track_id: "master".into(),
                kind: "builtin.beat".into(),
            })
            .unwrap();
        let id = session.project.master_devices[0].id.clone();
        for time in [true, false] {
            for slot in 0..36 {
                let points = preset(time, slot);
                session
                    .execute(Command::SetBeatEnvelope {
                        track_id: "master".into(),
                        device_id: id.clone(),
                        lane: if time { "time" } else { "volume" }.into(),
                        slot,
                        points: Some(points),
                    })
                    .unwrap();
            }
        }
        let before = session.project.clone();
        for points in [
            vec![],
            vec![
                BeatPoint {
                    x: 0.0,
                    y: f64::NAN,
                    curve: 0,
                },
                BeatPoint {
                    x: 1.0,
                    y: 1.0,
                    curve: 0,
                },
            ],
            vec![
                BeatPoint {
                    x: 0.0,
                    y: 0.0,
                    curve: 0,
                },
                BeatPoint {
                    x: 0.0,
                    y: 1.0,
                    curve: 0,
                },
            ],
        ] {
            assert!(session
                .execute(Command::SetBeatEnvelope {
                    track_id: "master".into(),
                    device_id: id.clone(),
                    lane: "time".into(),
                    slot: 0,
                    points: Some(points)
                })
                .is_err());
            assert_eq!(session.project, before);
        }
        for bad in [-1.0, 36.0, 1.5, f64::NAN] {
            assert!(session
                .execute(Command::SetDeviceParameter {
                    track_id: "master".into(),
                    device_id: id.clone(),
                    parameter: "time_slot".into(),
                    value: bad
                })
                .is_err());
        }
        session
            .execute(Command::SetBeatEnvelope {
                track_id: "master".into(),
                device_id: id,
                lane: "time".into(),
                slot: 0,
                points: None,
            })
            .unwrap();
        assert!(session.undo());
        assert_eq!(session.project, before);
        session.execute(Command::SaveProject).unwrap();
        assert_eq!(Session::open(root.path()).unwrap().project, before);
        let mut old = serde_json::to_value(Device::new("builtin.gain").unwrap()).unwrap();
        old.as_object_mut().unwrap().remove("beat_envelopes");
        assert!(serde_json::from_value::<Device>(old)
            .unwrap()
            .beat_envelopes
            .is_empty());
    }
}
