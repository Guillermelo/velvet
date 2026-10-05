use velvet_core::{
    beat::{preset, value},
    Device,
};

/// Causal playback: offsets can only read audio already received, up to two bars.
pub(crate) fn process(frames: &mut [[f32; 2]], device: &Device, bpm: f64, rate: u32) {
    let p = &device.parameters;
    if p["bypass_enabled"] == 1.0 || p["mix"] == 0.0 {
        return;
    }
    let samples_per_beat = rate as f64 * 60.0 / bpm;
    let length = samples_per_beat * p["loop_beats"];
    let capacity = (samples_per_beat * 8.0).ceil() as usize + 2;
    let mut buffer = vec![[0.0_f32; 2]; capacity];
    let mut held = if p["hold_enabled"] == 1.0 {
        vec![[0.0_f32; 2]; length.ceil() as usize + 1]
    } else {
        Vec::new()
    };
    let envelope = |time: bool| {
        let lane = if time { "time" } else { "volume" };
        let slot = p[&format!("{lane}_slot")] as usize;
        device
            .beat_envelopes
            .get(&format!("{lane}:{slot}"))
            .cloned()
            .unwrap_or_else(|| preset(time, slot))
    };
    let time = envelope(true);
    let volume = envelope(false);
    let coefficient = |ms: f64| {
        if ms == 0.0 {
            0.0
        } else {
            (-1.0 / (rate as f64 * ms * 0.001)).exp()
        }
    };
    let attack = coefficient(p["attack_ms"]);
    let release = coefficient(p["release_ms"]);
    let smooth = (rate as f64 * p["smooth_ms"] * 0.001).round() as usize;
    let mut gain = value(&volume, 0.0);
    let mut previous_delay = 0.0;
    let mut old_delay = 0.0;
    let mut fade = smooth;
    for (i, frame) in frames.iter_mut().enumerate() {
        let dry = *frame;
        buffer[i % capacity] = dry;
        if !held.is_empty() && (i as f64) < length {
            held[i] = dry;
        }
        let phase = (i as f64 / length).fract();
        let delay = (value(&time, phase) * length * p["time_mix"]
            + p["offset_beats"] * samples_per_beat)
            .min(samples_per_beat * 8.0);
        if (delay - previous_delay).abs() > 4.0 && smooth > 0 {
            old_delay = previous_delay;
            fade = 0;
        }
        let read = |delay: f64| {
            let position = i as f64 - delay;
            if position < 0.0 {
                return [0.0; 2];
            }
            let a = position.floor() as usize;
            let b = (a + 1).min(i);
            let fraction = position.fract() as f32;
            std::array::from_fn::<_, 2, _>(|c| {
                buffer[a % capacity][c] * (1.0 - fraction) + buffer[b % capacity][c] * fraction
            })
        };
        let mut wet = read(delay);
        if p["hold_enabled"] == 1.0 {
            let position = phase * length;
            let a = position.floor() as usize;
            let b = (a + 1).min(held.len() - 1);
            let fraction = position.fract() as f32;
            wet = if (i as f64) < length {
                dry
            } else {
                std::array::from_fn(|c| held[a][c] * (1.0 - fraction) + held[b][c] * fraction)
            };
        }
        if fade < smooth && p["hold_enabled"] == 0.0 {
            let old = read(old_delay);
            let t = fade as f32 / smooth as f32;
            for c in 0..2 {
                wet[c] = old[c] * (1.0 - t) + wet[c] * t;
            }
            fade += 1;
        }
        previous_delay = delay;
        let target = value(&volume, phase).powf(2.0_f64.powf(p["tension"] * 2.0));
        let k = if target > gain { attack } else { release };
        gain = target + k * (gain - target);
        let multiplier = (1.0 - p["volume_mix"] + gain * p["volume_mix"]) as f32;
        let mix = p["mix"] as f32;
        for c in 0..2 {
            frame[c] = dry[c] * (1.0 - mix) + wet[c] * multiplier * mix;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn beat_is_causal_tempo_synced_stereo_and_dry_is_exact() {
        let mut d = Device::new("builtin.beat").unwrap();
        for (bpm, rate) in [(120.0, 8000), (90.0, 44100)] {
            let length = (rate as f64 * 60.0 / bpm * 4.0) as usize;
            let input: Vec<_> = (0..length * 2)
                .map(|i| [i as f32 / length as f32, -(i as f32) / length as f32])
                .collect();
            let mut output = input.clone();
            process(&mut output, &d, bpm, rate);
            assert_eq!(input, output);
            d.parameters.insert("time_slot".into(), 1.0);
            d.parameters.insert("smooth_ms".into(), 0.0);
            process(&mut output, &d, bpm, rate);
            assert!((output[length / 2][0] - 0.25).abs() < 0.001);
            assert_eq!(output[length / 2][0], -output[length / 2][1]);
            d.parameters.insert("time_slot".into(), 4.0);
            let mut reverse = input.clone();
            process(&mut reverse, &d, bpm, rate);
            assert_eq!(reverse[length / 2], [0.0; 2]);
            assert!((reverse[length + length / 4][0] - 0.75).abs() < 0.001);
            d.parameters.insert("mix".into(), 0.0);
            process(&mut reverse, &d, bpm, rate);
            let mut dry = input.clone();
            process(&mut dry, &d, bpm, rate);
            assert_eq!(dry, input);
            d = Device::new("builtin.beat").unwrap();
        }
    }
    #[test]
    fn all_slots_produce_finite_audio_and_volume_gate_works() {
        let mut d = Device::new("builtin.beat").unwrap();
        for slot in 0..36 {
            d.parameters.insert("time_slot".into(), slot as f64);
            d.parameters.insert("volume_slot".into(), slot as f64);
            let mut frames = vec![[0.5, -0.25]; 32000];
            process(&mut frames, &d, 120.0, 8000);
            assert!(frames
                .iter()
                .flatten()
                .all(|s| s.is_finite() && s.abs() <= 0.5));
        }
        d.parameters.insert("time_slot".into(), 0.0);
        d.parameters.insert("volume_slot".into(), 3.0);
        d.parameters.insert("attack_ms".into(), 0.0);
        d.parameters.insert("release_ms".into(), 0.0);
        let mut frames = vec![[1.0; 2]; 16000];
        process(&mut frames, &d, 120.0, 8000);
        assert_eq!(frames[500], [1.0; 2]);
        assert_eq!(frames[1500], [0.0; 2]);
    }
    #[test]
    fn repeat_replays_a_slice_and_hold_survives_more_than_two_bars() {
        let mut d = Device::new("builtin.beat").unwrap();
        d.parameters.insert("time_slot".into(), 7.0);
        d.parameters.insert("smooth_ms".into(), 0.0);
        let input: Vec<_> = (0..80000).map(|i| [i as f32 / 80000.0; 2]).collect();
        let mut output = input.clone();
        process(&mut output, &d, 120.0, 8000);
        // Four repeats of the first beat in a 16,000-sample bar.
        assert!((output[5000][0] - input[1000][0]).abs() < 1e-5);
        assert!((output[9000][0] - input[1000][0]).abs() < 1e-5);
        d.parameters.insert("hold_enabled".into(), 1.0);
        output.clone_from(&input);
        process(&mut output, &d, 120.0, 8000);
        assert!((output[65000][0] - input[1000][0]).abs() < 1e-5);
    }
    #[test]
    fn track_chain_and_wav_export_include_beat_before_gain() {
        use velvet_core::{Command, Position, Project, Session, Source, SourceKind};
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("input.wav");
        let mut writer = hound::WavWriter::create(
            &source,
            hound::WavSpec {
                channels: 2,
                sample_rate: 8000,
                bits_per_sample: 32,
                sample_format: hound::SampleFormat::Float,
            },
        )
        .unwrap();
        for i in 0..32000 {
            for sign in [1.0, -1.0] {
                writer.write_sample(i as f32 / 32000.0 * sign).unwrap();
            }
        }
        writer.finalize().unwrap();
        let mut s = Session::new(Project::new("Beat render"), root.path().into());
        s.execute(Command::AddTrack {
            name: "Track".into(),
        })
        .unwrap();
        let track = s.project.tracks[0].id.clone();
        s.execute(Command::ImportAudioClip {
            track_id: track.clone(),
            source: Source {
                path: source,
                kind: SourceKind::External,
            },
            position: Position {
                start_beats: 0.0,
                offset_seconds: 0.0,
                length_seconds: 4.0,
            },
        })
        .unwrap();
        for kind in ["builtin.beat", "builtin.gain"] {
            s.execute(Command::AddDevice {
                track_id: track.clone(),
                kind: kind.into(),
            })
            .unwrap();
        }
        let beat = s.project.tracks[0].devices[0].id.clone();
        let gain = s.project.tracks[0].devices[1].id.clone();
        for (id, parameter, value) in [
            (beat.clone(), "time_slot", 7.0),
            (beat.clone(), "smooth_ms", 0.0),
            (gain, "gain_db", -6.0),
        ] {
            s.execute(Command::SetDeviceParameter {
                track_id: track.clone(),
                device_id: id,
                parameter: parameter.into(),
                value,
            })
            .unwrap();
        }
        let enabled = std::collections::HashSet::from([beat.clone()]);
        let mut cache = crate::MediaCache::default();
        let mix =
            crate::mix_with_scopes(&s.project, root.path(), 8000, &mut cache, &enabled).unwrap();
        let expected = 1000.0 / 32000.0 * super::super::db(-6.0);
        assert!((mix.frames[5000][0] - expected).abs() < 1e-5);
        assert!((mix.device_signals[&beat].buckets[5000][0][0] - 1000.0 / 32000.0).abs() < 1e-5);
        let output = root.path().join("output.wav");
        crate::export(&mix, &output).unwrap();
        let mut reader = hound::WavReader::open(output).unwrap();
        assert_eq!(reader.spec().sample_rate, 8000);
        let sample = reader.samples::<i32>().nth(10000).unwrap().unwrap() as f32 / 8_388_607.0;
        assert!((sample - expected).abs() < 1e-5);
    }
}
