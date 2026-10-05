use std::f64::consts::TAU;
use velvet_core::{Device, MidiNote};

/// Raw C4 oscillator (MIDI 60, 261.63 Hz), before filter, envelope and output gain.
pub fn oscillator_c4(phase: f64, wave: u8) -> f64 {
    oscillator(
        phase.rem_euclid(1.0),
        wave,
        440.0 * 2.0_f64.powf(-9.0 / 12.0),
        48000,
    )
}

fn oscillator(phase: f64, wave: u8, frequency: f64, rate: u32) -> f64 {
    // Additive harmonics stay below Nyquist, avoiding aliased saw/square edges.
    match wave {
        0 => (TAU * phase).sin(),
        3 => 1.0 - 4.0 * (phase - 0.5).abs(),
        kind => {
            let mut value = 0.0;
            for h in 1..=64 {
                if frequency * h as f64 >= rate as f64 * 0.45 {
                    break;
                }
                if kind == 2 && h % 2 == 0 {
                    continue;
                }
                value += (TAU * phase * h as f64).sin() / h as f64;
            }
            value
                * if kind == 1 {
                    2.0 / std::f64::consts::PI
                } else {
                    4.0 / std::f64::consts::PI
                }
        }
    }
}

/// Add native oscillator voices to the track bus before its effects and mixer.
pub fn render(bus: &mut [[f32; 2]], notes: &[MidiNote], synth: &Device, bpm: f64, rate: u32) {
    let p = &synth.parameters;
    let attack = p["attack_ms"] / 1000.0;
    let decay = p["decay_ms"] / 1000.0;
    let release = p["release_ms"] / 1000.0;
    let sustain = p["sustain"];
    let level = 10.0_f64.powf(p["gain_db"] / 20.0);
    let cutoff = p["cutoff_freq_hz"].min(rate as f64 * 0.45);
    let alpha = 1.0 - (-TAU * cutoff / rate as f64).exp();
    for n in notes.iter().filter(|n| !n.muted) {
        let start = (n.start_beats * 60.0 / bpm * rate as f64).round() as usize;
        let held = n.length_beats * 60.0 / bpm;
        let envelope = |t: f64| {
            if t < attack {
                t / attack
            } else {
                sustain + (1.0 - sustain) * (1.0 - (t - attack) / decay).max(0.0)
            }
        };
        let frequency = 440.0 * 2.0_f64.powf((n.key as f64 - 69.0) / 12.0);
        let count = ((held + release) * rate as f64).ceil() as usize;
        let mut filtered = 0.0;
        for (i, frame) in bus.iter_mut().skip(start).take(count).enumerate() {
            let t = i as f64 / rate as f64;
            let phase = (t * frequency).fract();
            let wave = oscillator(phase, p["wave_type"] as u8, frequency, rate);
            filtered += alpha * (wave - filtered);
            let env = if t < held {
                envelope(t)
            } else {
                envelope(held) * (1.0 - (t - held) / release).max(0.0)
            };
            let sample = (filtered * env * level * n.velocity as f64 / 127.0) as f32;
            frame[0] += sample;
            frame[1] += sample;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn muted_notes_render_silence() {
        let synth = Device::new("builtin.dot").unwrap();
        let mut bus = vec![[0.0; 2]; 8000];
        render(
            &mut bus,
            &[MidiNote {
                muted: true,
                ..MidiNote::default()
            }],
            &synth,
            120.0,
            8000,
        );
        assert!(bus.iter().all(|frame| *frame == [0.0; 2]));
    }
    #[test]
    fn raw_c4_oscillator_is_periodic_and_has_distinct_shapes() {
        let sine = oscillator_c4(0.25, 0);
        assert!((sine - 1.0).abs() < 1e-6);
        assert!((oscillator_c4(0.25, 3)).abs() < 1e-6);
        for wave in 0..4 {
            for phase in [0.0, 0.13, 0.37, 0.75] {
                assert!(
                    (oscillator_c4(phase, wave) - oscillator_c4(phase + 1.0, wave)).abs() < 1e-6
                );
            }
        }
        assert!((oscillator_c4(0.25, 1) - oscillator_c4(0.25, 2)).abs() > 0.3);
    }

    #[test]
    fn voices_follow_pitch_velocity_and_release() {
        let mut synth = Device::new("builtin.dot").unwrap();
        synth.parameters.insert("release_ms".into(), 50.0);
        let note = MidiNote {
            key: 69,
            velocity: 100,
            start_beats: 1.0,
            length_beats: 1.0,
            ..MidiNote::default()
        };
        for wave in 0..4 {
            synth.parameters.insert("wave_type".into(), wave as f64);
            let mut bus = vec![[0.0; 2]; 16000];
            render(&mut bus, std::slice::from_ref(&note), &synth, 120.0, 8000);
            assert!(bus[..4000].iter().all(|f| f[0] == 0.0));
            assert!(bus[4100..7900].iter().any(|f| f[0].abs() > 0.01));
            assert!(bus[8400..].iter().all(|f| f[0] == 0.0));
            assert!(bus.iter().all(|f| f[0].is_finite() && f[0] == f[1]));
        }
    }
}
