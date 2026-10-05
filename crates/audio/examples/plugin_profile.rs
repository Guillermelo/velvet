//! Measure the actual editor capture and preset-to-render path, outside the command sandbox.
use anyhow::{Context, Result};
use std::{
    sync::{Arc, Mutex},
    time::Instant,
};
use velvet_core::{Device, MidiNote};
mod support;

fn main() -> Result<()> {
    let _ui = velvet_audio::plugins::initialize_ui_thread()?;
    let path = std::env::args()
        .nth(1)
        .context("plugin_profile <plugin.vst3>")?;
    let mut device = Device::new(&format!("vst3.instrument:{path}"))?;
    let started = Instant::now();
    let plugin = velvet_audio::plugins::load(&device, 48000, 120.0)?;
    println!("load_ms={:.2}", started.elapsed().as_secs_f64() * 1000.0);
    let plugin = Arc::new(Mutex::new(plugin));
    let mut window = velvet_audio::plugins::PluginWindow::new(plugin.clone());
    window.open()?;
    let units = plugin.lock().unwrap().get_units()?;
    let program = std::env::args()
        .any(|arg| arg == "--program")
        .then(|| units.iter().find(|u| u.programs.len() > 1))
        .flatten();
    let parameters = plugin.lock().unwrap().get_parameters()?;
    println!(
        "level_parameters={:?}",
        parameters
            .iter()
            .filter(|p| ["volume", "gain", "level"]
                .iter()
                .any(|word| p.name.to_lowercase().contains(word)))
            .map(|p| (&p.name, p.id))
            .take(20)
            .collect::<Vec<_>>()
    );
    let parameter = parameters
        .iter()
        .find(|p| {
            ["master volume", "volume", "1 level"]
                .iter()
                .any(|name| p.name.eq_ignore_ascii_case(name))
        })
        .or_else(|| {
            parameters.iter().find(|p| {
                p.can_automate
                    && !p.is_read_only
                    && p.step_count == 0
                    && ["volume", "gain", "level"]
                        .iter()
                        .any(|word| p.name.to_lowercase().contains(word))
            })
        });
    let mut captures = Vec::new();
    let mut states = Vec::new();
    for i in 0..12 {
        support::pump_messages();
        window.service_platform_events()?;
        let started = Instant::now();
        if let Some(unit) = program {
            plugin
                .lock()
                .unwrap()
                .select_program(unit.id, (i % unit.programs.len()) as i32)?;
            // Allow a sample-based instrument's asynchronous program load to
            // settle before capturing and selecting the next program.
            let settle = Instant::now();
            while settle.elapsed() < std::time::Duration::from_millis(500) {
                support::pump_messages();
                window.service_platform_events()?;
                velvet_audio::plugins::snapshot(&mut plugin.lock().unwrap())?;
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        }
        if let Some(parameter) = &parameter {
            plugin
                .lock()
                .unwrap()
                .set_parameter(parameter.id, if i % 2 == 0 { 0.45 } else { 0.55 })?;
        }
        device.plugin_state = velvet_audio::plugins::snapshot(&mut plugin.lock().unwrap())?;
        states.push(device.plugin_state.clone());
        captures.push(started.elapsed().as_secs_f64() * 1000.0);
    }
    captures.sort_by(f64::total_cmp);
    println!(
        "capture_median_ms={:.2} capture_max_ms={:.2} state_bytes={} programs={}",
        captures[6],
        captures[11],
        device.plugin_state.len(),
        program.map_or(0, |u| u.programs.len())
    );
    println!(
        "state_changed={}",
        states.windows(2).any(|pair| pair[0] != pair[1])
    );
    window.close();
    println!("editor_closed");
    drop(window);
    drop(plugin);
    println!("editor_instance_released");
    let mut renderer = velvet_audio::plugins::RenderCache::default();
    let generation = Arc::new(std::sync::atomic::AtomicU64::new(0));
    renderer.set_cancellation(generation.clone(), 0);
    let mut previous = None;
    for i in 0..4 {
        device.plugin_state = states[i].clone();
        let mut frames = vec![[0.0; 2]; 48000 * 8];
        let notes = [MidiNote {
            key: 60,
            velocity: 100,
            length_beats: 8.0,
            ..Default::default()
        }];
        let started = Instant::now();
        renderer.process(&mut frames, &device, &notes, 120.0, 48000)?;
        let peak = frames
            .iter()
            .flatten()
            .copied()
            .map(f32::abs)
            .fold(0.0, f32::max);
        anyhow::ensure!(peak > 0.00001, "Restored plugin produced silence");
        let changed = previous
            .as_ref()
            .is_some_and(|previous| *previous != frames);
        println!(
            "cached_render_{i}_ms={:.2} peak={peak:.6} audio_changed={changed}",
            started.elapsed().as_secs_f64() * 1000.0
        );
        previous = Some(frames);
    }
    generation.store(1, std::sync::atomic::Ordering::Release);
    let mut frames = vec![[0.0; 2]; 48000];
    let cancelled = renderer.process(&mut frames, &device, &[], 120.0, 48000);
    anyhow::ensure!(cancelled.is_err(), "Obsolete render was not cancelled");
    renderer.set_cancellation(generation, 1);
    renderer.process(
        &mut frames,
        &device,
        &[MidiNote {
            key: 60,
            velocity: 100,
            ..Default::default()
        }],
        120.0,
        48000,
    )?;
    anyhow::ensure!(
        frames.iter().flatten().any(|sample| sample.abs() > 0.00001),
        "Plugin stayed silent after cancellation"
    );
    println!("PASS: successive states sound; stale render cancels; next render sounds");
    Ok(())
}
