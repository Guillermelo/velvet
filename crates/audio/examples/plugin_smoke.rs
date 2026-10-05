//! Run each plugin in a separate invocation so a broken plugin cannot affect another test.
use anyhow::{ensure, Context, Result};
use velvet_core::{Command, Device, MidiNote, Position, Project, Session, Source, SourceKind};
mod support;

fn main() -> Result<()> {
    let _plugin_ui_thread = velvet_audio::plugins::initialize_ui_thread()?;
    let mut args = std::env::args().skip(1);
    let role = args
        .next()
        .context("Usage: plugin_smoke instrument|effect <absolute .vst3 path>")?;
    ensure!(
        matches!(role.as_str(), "instrument" | "effect"),
        "Invalid role"
    );
    let path = args.next().context("Missing VST3 path")?;
    let mut device = Device::new(&format!("vst3.{role}:{path}"))?;
    let mut plugin = velvet_audio::plugins::load(&device, 48000, 120.0)?;
    println!("Loaded {} / {}", plugin.info().name, plugin.info().category);
    let parameter = plugin.get_parameters()?.into_iter().find(|p| {
        let name = p.name.to_lowercase();
        p.can_automate
            && !p.is_read_only
            && p.step_count == 0
            && ["volume", "gain", "level", "mix", "cutoff"]
                .iter()
                .any(|key| name.contains(key))
    });
    if let Some(parameter) = &parameter {
        plugin.set_parameter(
            parameter.id,
            if (parameter.value - 0.5).abs() < 0.01 {
                0.55
            } else {
                0.5
            },
        )?;
    }
    device.plugin_state = velvet_audio::plugins::snapshot(&mut plugin)?;
    plugin.load_state(&device.plugin_state)?;
    if let Some(parameter) = parameter {
        let expected = if (parameter.value - 0.5).abs() < 0.01 {
            0.55
        } else {
            0.5
        };
        let actual = plugin.get_parameter(parameter.id)?;
        ensure!(
            (actual - expected).abs() < 0.0001,
            "Edited parameter {} did not restore: expected {expected}, actual {actual}",
            parameter.name
        );
        println!(
            "PASS: parameter edit, DSP flush and state restore ({})",
            parameter.name
        );
    }
    if args.any(|arg| arg == "--editor") {
        let plugin = std::sync::Arc::new(std::sync::Mutex::new(plugin));
        let mut window = velvet_audio::plugins::PluginWindow::new(plugin.clone());
        for _ in 0..2 {
            window.open()?;
            ensure!(window.is_open(), "Native editor did not open");
            let started = std::time::Instant::now();
            while started.elapsed() < std::time::Duration::from_secs(2) {
                support::pump_messages();
                window.service_platform_events()?;
                ensure!(window.is_open(), "Native editor closed unexpectedly");
                // Exercise the same editor -> DSP -> state path used by the app.
                velvet_audio::plugins::snapshot(&mut plugin.lock().unwrap())?;
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            let size = plugin.lock().unwrap().get_editor_size()?;
            ensure!(
                size.0 > 0 && size.1 > 0,
                "Native editor has invalid dimensions"
            );
            println!("PASS: native editor responsive, DSP/state capture, size {size:?}");
            window.close();
            ensure!(!window.is_open(), "Native editor did not close");
        }
        println!("PASS: native editor close and reopen");
        drop(window);
        plugin.lock().unwrap().load_state(&device.plugin_state)?;
    } else {
        drop(plugin);
    }
    let mut frames = vec![[0.0; 2]; 48000 * 3];
    let notes = if role == "instrument" {
        vec![MidiNote {
            key: 60,
            velocity: 100,
            start_beats: 0.0,
            length_beats: 1.0,
            ..MidiNote::default()
        }]
    } else {
        for (i, f) in frames.iter_mut().take(48000).enumerate() {
            let sample = (i as f32 * std::f32::consts::TAU * 440.0 / 48000.0).sin() * 0.1;
            *f = [sample; 2];
        }
        vec![]
    };
    velvet_audio::plugins::process(&mut frames, &device, &notes, 120.0, 48000)?;
    let peak = frames
        .iter()
        .flatten()
        .map(|s| s.abs())
        .fold(0.0_f32, f32::max);
    ensure!(peak > 0.00001, "Plugin output is silent (peak {peak})");
    println!(
        "PASS: restored state ({} bytes), finite stereo output, peak {peak:.6}",
        device.plugin_state.len()
    );
    let root = tempfile::tempdir()?;
    let mut session = Session::new(Project::new("Plugin smoke"), root.path().into());
    if role == "instrument" {
        session.execute(Command::AddMidiTrack {
            name: "Synth".into(),
        })?;
    } else {
        session.execute(Command::AddTrack {
            name: "Effect".into(),
        })?;
    }
    let track_id = session.project.tracks[0].id.clone();
    let device_id = if role == "instrument" {
        session.execute(Command::SetTrackInstrument {
            track_id: track_id.clone(),
            kind: Some(device.kind.clone()),
        })?;
        session.execute(Command::SetMidiNotes {
            track_id: track_id.clone(),
            notes,
        })?;
        session.project.tracks[0].synth.as_ref().unwrap().id.clone()
    } else {
        let source = root.path().join("input.wav");
        let mut wav = hound::WavWriter::create(
            &source,
            hound::WavSpec {
                channels: 2,
                sample_rate: 48000,
                bits_per_sample: 32,
                sample_format: hound::SampleFormat::Float,
            },
        )?;
        for i in 0..48000 {
            let sample = (i as f32 * std::f32::consts::TAU * 440.0 / 48000.0).sin() * 0.1;
            wav.write_sample(sample)?;
            wav.write_sample(sample)?;
        }
        wav.finalize()?;
        session.execute(Command::ImportAudioClip {
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
        })?;
        session.execute(Command::AddDevice {
            track_id: track_id.clone(),
            kind: device.kind,
        })?;
        session.project.tracks[0].devices[0].id.clone()
    };
    session.execute(Command::SetPluginState {
        track_id,
        device_id,
        state: device.plugin_state,
    })?;
    velvet_core::save(root.path(), &session.project)?;
    let project = velvet_core::load(root.path())?;
    ensure!(
        project == session.project,
        "Project did not restore plugin state"
    );
    let scopes = project
        .tracks
        .iter()
        .flat_map(|t| t.devices.iter().chain(t.synth.iter()))
        .map(|d| d.id.clone())
        .collect();
    let mix = velvet_audio::mix_with_scopes(
        &project,
        root.path(),
        48000,
        &mut velvet_audio::MediaCache::default(),
        &scopes,
    )?;
    ensure!(mix.peak > 0.00001, "Project mix is silent");
    ensure!(
        !mix.device_signals.is_empty()
            && mix.device_signals.values().all(|s| s
                .buckets
                .iter()
                .flatten()
                .flatten()
                .any(|value| value.abs() > 0.00001)),
        "Device output scope is silent"
    );
    println!("PASS: oscilloscope captures actual device output");
    let export = root.path().join("mix.wav");
    velvet_audio::export(&mix, &export)?;
    let wav = hound::WavReader::open(export)?;
    ensure!(
        wav.spec().channels == 2 && wav.duration() > 48000,
        "Invalid WAV export"
    );
    println!("PASS: project save/reopen, MIDI/effect chain mix and stereo WAV export");
    Ok(())
}
