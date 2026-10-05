//! Audible state regression: a master-level edit must survive editor -> state -> render.
use anyhow::{ensure, Context, Result};
use std::sync::{Arc, Mutex};
use velvet_audio::plugins::{self, Plugin, PluginWindow, RenderCache};
use velvet_core::{Device, MidiNote};
mod support;

fn level(plugin: &mut Plugin) -> Result<f32> {
    plugin.midi_panic()?;
    plugin.reconfigure(48000.0, 512)?;
    plugin.start_processing()?;
    let mut buffers = plugin.create_bus_audio_buffers(512)?;
    plugin.send_midi_event(vst3_host::midi::MidiEvent::NoteOn {
        channel: vst3_host::midi::MidiChannel::Ch1,
        note: 60,
        velocity: 100,
    })?;
    let mut energy = 0.0_f64;
    let mut count = 0;
    for _ in 0..96 {
        buffers.clear();
        plugin.process_bus_audio(&mut buffers)?;
        for sample in &buffers.outputs[0].channels[0] {
            energy += (*sample as f64).powi(2);
            count += 1;
        }
    }
    plugin.stop_processing()?;
    Ok((energy / count as f64).sqrt() as f32)
}

fn main() -> Result<()> {
    let _ui = plugins::initialize_ui_thread()?;
    let path = std::env::args()
        .nth(1)
        .context("plugin_state_roundtrip <plugin.vst3> [--program]")?;
    let mut device = Device::new(&format!("vst3.instrument:{path}"))?;
    let plugin = Arc::new(Mutex::new(plugins::load(&device, 48000, 120.0)?));
    let mut window = PluginWindow::new(plugin.clone());
    window.open()?;
    support::pump_messages();
    let parameters = plugin.lock().unwrap().get_parameters()?;
    let parameter = parameters
        .iter()
        .find(|p| p.name.eq_ignore_ascii_case("master volume"))
        .or_else(|| {
            parameters
                .iter()
                .find(|p| p.name.eq_ignore_ascii_case("volume"))
        })
        .or_else(|| {
            parameters
                .iter()
                .find(|p| p.name.eq_ignore_ascii_case("1 level"))
        })
        .context("No master volume parameter")?;
    let preset_parameter = if plugin.lock().unwrap().info().name == "M1" {
        parameters
            .iter()
            .find(|p| {
                p.id != parameter.id
                    && p.can_automate
                    && !p.is_read_only
                    && p.step_count == 0
                    && p.name.to_lowercase().contains("cutoff")
            })
            .or_else(|| {
                parameters.iter().find(|p| {
                    p.id != parameter.id && p.can_automate && !p.is_read_only && p.step_count == 0
                })
            })
            .context("No patch parameter for preset-file regression")?
    } else {
        parameter
    };
    let units = plugin.lock().unwrap().get_units()?;
    let program = units.iter().find(|u| u.programs.len() > 1);
    println!(
        "Testing {} parameter {}",
        plugin.lock().unwrap().info().name,
        parameter.name
    );
    let mut renderer = RenderCache::default();
    let presets = tempfile::tempdir()?;
    let mut levels = Vec::new();
    for (index, value) in [0.2, 0.8, 0.2, 0.8].into_iter().enumerate() {
        support::pump_messages();
        window.service_platform_events()?;
        let mut current = plugin.lock().unwrap();
        if std::env::args().any(|a| a == "--program") {
            if let Some(unit) = program {
                current.select_program(unit.id, index as i32)?;
            }
        }
        current.set_parameter(parameter.id, value)?;
        println!("Capturing edit {index}={value}");
        device.plugin_state = plugins::snapshot(&mut current)?;
        let actual = current.get_parameter(parameter.id)?;
        let preset = presets.path().join(format!("state-{index}.vstpreset"));
        let preset_value = current.get_parameter(preset_parameter.id)?;
        let preset_rms = level(&mut current)?;
        current.save_vstpreset(&preset)?;
        current.set_parameter(
            preset_parameter.id,
            if preset_value >= 0.5 { 0.0 } else { 1.0 },
        )?;
        plugins::snapshot(&mut current)?;
        let altered_rms = level(&mut current)?;
        current.load_vstpreset(&preset)?;
        plugins::snapshot(&mut current)?;
        let file_rms = level(&mut current)?;
        println!("Preset {} before={preset_rms:.6} altered={altered_rms:.6} restored={file_rms:.6}, controller={}", preset_parameter.name, current.get_parameter(preset_parameter.id)?);
        ensure!(
            (0.85..=1.15).contains(&(file_rms / preset_rms)),
            ".vstpreset did not restore audible patch"
        );
        // Check the audible processor state: M1's parameter query can remain
        // stale after restoring a preset even when the sound is restored.
        current.load_state(&device.plugin_state)?;
        ensure!(
            (current.get_parameter(parameter.id)? - value).abs() < 0.001,
            "Project state did not restore the edited parameter"
        );
        println!("Captured controller={actual}");
        let direct = level(&mut current)?;
        drop(current);
        println!("Live RMS={direct:.6}; restoring state");
        let mut frames = vec![[0.0; 2]; 48000];
        renderer.process(
            &mut frames,
            &device,
            &[MidiNote {
                key: 60,
                velocity: 100,
                length_beats: 2.0,
                ..Default::default()
            }],
            120.0,
            48000,
        )?;
        let rms = (frames.iter().map(|f| (f[0] as f64).powi(2)).sum::<f64>() / frames.len() as f64)
            .sqrt() as f32;
        println!("Restored RMS={rms:.6}");
        ensure!(
            (actual - value).abs() < 0.001,
            "Captured parameter differs from edit"
        );
        ensure!(
            direct > 0.00001 && rms > 0.00001,
            "Edited/restored plugin is silent"
        );
        // Synths may randomize phase and preserve release tails; compare levels rather
        // than samples. A discarded M1 state was over five times louder at value 0.2.
        ensure!(
            (0.5..=2.0).contains(&(rms / direct)),
            "Restored audio differs from edited audio"
        );
        if index == 0 || index == 3 {
            let mut fresh = vec![[0.0; 2]; 48000];
            plugins::process(
                &mut fresh,
                &device,
                &[MidiNote {
                    key: 60,
                    velocity: 100,
                    length_beats: 2.0,
                    ..Default::default()
                }],
                120.0,
                48000,
            )?;
            let fresh_rms = (fresh.iter().map(|f| (f[0] as f64).powi(2)).sum::<f64>()
                / fresh.len() as f64)
                .sqrt() as f32;
            ensure!(
                (0.5..=2.0).contains(&(fresh_rms / direct)),
                "Fresh project/export instance differs from edited audio"
            );
            println!("PASS: fresh project/export instance RMS={fresh_rms:.6}");
        }
        levels.push(rms);
    }
    ensure!(
        levels[1] > levels[0] * 1.5 && levels[3] > levels[2] * 1.5,
        "Master volume edits did not change the restored audio: {levels:?}"
    );
    println!("PASS: four audible edits survive state restoration");
    let mut live = velvet_audio::LiveMixer::default();
    let preview = live.preview(&device, 60, 100, 1, 120.0, 48000)?;
    ensure!(
        preview.peak > 0.00001,
        "Piano-roll VST preview produced silence"
    );
    println!("PASS: .vstpreset files and cached piano-roll preview");
    window.close();
    Ok(())
}
