//! VST3 processing runs on the mix worker, never in the audio callback.
use anyhow::{ensure, Context, Result};
use std::path::PathBuf;
use velvet_core::{Device, MidiNote};
use vst3_host::{
    audio::{BusDirection, MediaType},
    midi::{MidiChannel, MidiEvent},
    Vst3Host,
};
pub use vst3_host::{Plugin, PluginWindow};

const BLOCK: usize = 512;

/// Keep OLE alive on the UI thread until all native plugin editors have been dropped.
/// The guard cannot move to another thread: OLE initialization is thread-local.
pub struct PluginUiThread(std::marker::PhantomData<*mut ()>);

#[cfg(windows)]
#[link(name = "ole32")]
extern "system" {
    fn OleInitialize(reserved: *mut std::ffi::c_void) -> i32;
    fn OleUninitialize();
}

pub fn initialize_ui_thread() -> Result<PluginUiThread> {
    #[cfg(windows)]
    {
        // SAFETY: null is required; the returned guard balances each successful call
        // on this same thread, including S_FALSE (already initialized).
        let result = unsafe { OleInitialize(std::ptr::null_mut()) };
        ensure!(
            result >= 0,
            "Cannot initialize OLE for VST3 editors: {result:#x}"
        );
    }
    Ok(PluginUiThread(std::marker::PhantomData))
}

impl Drop for PluginUiThread {
    fn drop(&mut self) {
        #[cfg(windows)]
        unsafe {
            OleUninitialize()
        }
    }
}

/// Called only by the desktop's disposable discovery subprocess.
pub fn is_instrument(path: &std::path::Path) -> Result<bool> {
    let mut host = Vst3Host::default();
    let plugin = host.load_plugin(path)?;
    Ok(plugin.info().category.contains("Instrument"))
}

/// Enumerate bundles without loading third-party code.
pub fn scan() -> Vec<PathBuf> {
    let mut paths = Vst3Host::default().scan_plugin_paths();
    paths.sort_by_key(|p| p.file_name().unwrap_or_default().to_ascii_lowercase());
    paths.dedup();
    paths
}

pub fn scan_folder(path: &std::path::Path) -> Vec<PathBuf> {
    Vst3Host::builder()
        .add_scan_path(path)
        .build()
        .map(|host| host.scan_plugin_paths())
        .unwrap_or_default()
}

pub fn load(device: &Device, rate: u32, bpm: f64) -> Result<Plugin> {
    let path = device.plugin_path().context("Not a VST3 device")?;
    let mut host = Vst3Host::builder()
        .sample_rate(rate as f64)
        .block_size(BLOCK)
        .tempo(bpm)
        .build()?;
    let mut plugin = host
        .load_plugin(path)
        .with_context(|| format!("Cannot load VST3 {}", path.display()))?;
    // Configure all buses in one inactive period. Repeated deactivate/reactivate
    // cycles used to make M1 reload its resources once for every output bus.
    plugin.stop_processing()?;
    let instrument = device.kind.starts_with("vst3.instrument:");
    ensure!(
        instrument == plugin.info().category.contains("Instrument"),
        "Choose this plugin from {}",
        if instrument { "Effects" } else { "Instruments" }
    );
    configure_buses(&mut plugin, instrument)?;
    // Match the persistent renderer: negotiate the default layout, then restore.
    if !device.plugin_state.is_empty() {
        plugin
            .load_state(&device.plugin_state)
            .context("Cannot restore plugin state")?;
        let flags = plugin.service_host_requests()?;
        if flags.io_changed() {
            configure_buses(&mut plugin, instrument)?;
        }
    }
    plugin.set_tempo(bpm)?;
    plugin.set_playing(true)?;
    Ok(plugin)
}

fn configure_buses(plugin: &mut Plugin, instrument: bool) -> Result<()> {
    let layout = plugin.audio_bus_layout()?;
    for (direction, buses) in [
        (BusDirection::Input, &layout.inputs),
        (BusDirection::Output, &layout.outputs),
    ] {
        for index in 0..buses.len() {
            // Some multi-output instruments write to their auxiliary outputs even
            // when deactivated. Allocate them; the rack uses only the main output.
            let active = index == 0 || (instrument && direction == BusDirection::Output);
            plugin.set_bus_active(MediaType::Audio, direction, index as i32, active)?;
        }
    }
    if plugin.info().has_midi_input {
        plugin.set_bus_active(MediaType::Event, BusDirection::Input, 0, true)?;
    }
    if instrument {
        let arrangements = plugin.bus_arrangements()?;
        if let Some(main) = arrangements.outputs.first() {
            plugin.set_bus_arrangements(&arrangements.inputs, std::slice::from_ref(main))?;
        }
    }
    Ok(())
}

/// Flush editor-to-processor parameter changes before serializing the processor state.
pub fn snapshot(plugin: &mut Plugin) -> Result<Vec<u8>> {
    flush_processor_changes(plugin)?;
    Ok(plugin.save_state()?)
}

fn flush_processor_changes(plugin: &mut Plugin) -> Result<()> {
    plugin.service_run_loop();
    let flags = plugin.service_host_requests()?;
    if flags.io_changed() {
        plugin.stop_processing()?;
        configure_buses(plugin, plugin.info().category.contains("Instrument"))?;
    }
    plugin.start_processing()?;
    let result = (|| {
        let mut buffers = plugin.create_bus_audio_buffers(BLOCK)?;
        plugin.process_bus_audio(&mut buffers)
    })();
    let stopped = plugin.stop_processing();
    result?;
    stopped?;
    Ok(())
}

fn note_events(notes: &[MidiNote], bpm: f64, rate: u32) -> Vec<(usize, MidiEvent)> {
    let mut events = Vec::with_capacity(notes.len() * 2);
    for note in notes.iter().filter(|n| !n.muted) {
        let channel =
            MidiChannel::from_index(note.channel.saturating_sub(1)).unwrap_or(MidiChannel::Ch1);
        let sample = |beat: f64| (beat * 60.0 / bpm * rate as f64).round() as usize;
        let start = sample(note.start_beats);
        let end = sample(note.start_beats + note.length_beats).max(start + 1);
        events.push((
            start,
            MidiEvent::NoteOn {
                channel,
                note: note.key,
                velocity: note.velocity,
            },
        ));
        events.push((
            end,
            MidiEvent::NoteOff {
                channel,
                note: note.key,
                velocity: 0,
            },
        ));
    }
    // End an old note before starting a new one at the same sample.
    events.sort_by_key(|(sample, event)| {
        (
            *sample,
            usize::from(matches!(event, MidiEvent::NoteOn { .. })),
        )
    });
    events
}

pub fn process(
    frames: &mut [[f32; 2]],
    device: &Device,
    notes: &[MidiNote],
    bpm: f64,
    rate: u32,
) -> Result<()> {
    let mut plugin = load(device, rate, bpm)?;
    prepare_render(&mut plugin, device, rate)?;
    process_loaded(&mut plugin, frames, device, notes, bpm, rate, None)
}

struct Instance {
    plugin: Plugin,
    kind: String,
    rate: u32,
    default_state: Vec<u8>,
}

/// Owned by one persistent render thread. Native plugin objects never cross threads.
#[derive(Default)]
pub struct RenderCache {
    instances: std::collections::HashMap<String, Instance>,
    cancellation: Option<(std::sync::Arc<std::sync::atomic::AtomicU64>, u64)>,
    owner: std::marker::PhantomData<*mut ()>,
}

impl RenderCache {
    pub fn set_cancellation(
        &mut self,
        generation: std::sync::Arc<std::sync::atomic::AtomicU64>,
        expected: u64,
    ) {
        self.cancellation = Some((generation, expected));
    }

    pub fn retain(&mut self, ids: &std::collections::HashSet<String>) {
        self.instances.retain(|id, _| ids.contains(id));
    }

    pub fn process(
        &mut self,
        frames: &mut [[f32; 2]],
        device: &Device,
        notes: &[MidiNote],
        bpm: f64,
        rate: u32,
    ) -> Result<()> {
        check_cancelled(self.cancellation.as_ref())?;
        if self
            .instances
            .get(&device.id)
            .is_some_and(|i| i.kind != device.kind || i.rate != rate)
        {
            self.instances.remove(&device.id);
        }
        if !self.instances.contains_key(&device.id) {
            let mut initial = device.clone();
            initial.plugin_state.clear();
            let plugin = load(&initial, rate, bpm)?;
            let default_state = plugin.save_state()?;
            self.instances.insert(
                device.id.clone(),
                Instance {
                    plugin,
                    kind: device.kind.clone(),
                    rate,
                    default_state,
                },
            );
        }
        let instance = self.instances.get_mut(&device.id).unwrap();
        // Reset voices and transport for an independent render, including undo to defaults.
        let state = if device.plugin_state.is_empty() {
            &instance.default_state
        } else {
            &device.plugin_state
        };
        let result = (|| {
            instance.plugin.load_state(state)?;
            let flags = instance.plugin.service_host_requests()?;
            if flags.io_changed() {
                configure_buses(
                    &mut instance.plugin,
                    device.kind.starts_with("vst3.instrument:"),
                )?;
            }
            prepare_render(&mut instance.plugin, device, rate)?;
            process_loaded(
                &mut instance.plugin,
                frames,
                device,
                notes,
                bpm,
                rate,
                self.cancellation.as_ref(),
            )
        })();
        if result.is_err() && check_cancelled(self.cancellation.as_ref()).is_ok() {
            self.instances.remove(&device.id);
        }
        result
    }
}

fn prepare_render(plugin: &mut Plugin, device: &Device, rate: u32) -> Result<()> {
    plugin.reconfigure(rate as f64, BLOCK)?;
    // Restore clears MIDI-mapped queues. Deliver panic afterward, in a block
    // preceding new notes, then reset transport for this independent render.
    plugin.midi_panic()?;
    if device.kind.starts_with("vst3.instrument:") {
        flush_processor_changes(plugin)?;
        plugin.reconfigure(rate as f64, BLOCK)?;
    }
    Ok(())
}

fn process_loaded(
    plugin: &mut Plugin,
    frames: &mut [[f32; 2]],
    device: &Device,
    notes: &[MidiNote],
    bpm: f64,
    rate: u32,
    cancellation: Option<&(std::sync::Arc<std::sync::atomic::AtomicU64>, u64)>,
) -> Result<()> {
    plugin.set_tempo(bpm)?;
    plugin.set_playing(true)?;
    plugin.start_processing()?;
    let result = (|| -> Result<()> {
        let mut buffers = plugin.create_bus_audio_buffers(BLOCK)?;
        let output_bus = buffers
            .outputs
            .iter()
            .position(|b| b.active && !b.channels.is_empty())
            .context("VST3 has no active audio output")?;
        let channels = buffers.outputs[output_bus].channels.len();
        ensure!(channels <= 2, "VST3 main output must be mono or stereo");
        let latency = plugin.latency_samples() as usize;
        ensure!(
            latency <= rate as usize * 10,
            "VST3 latency exceeds 10 seconds"
        );
        let events = note_events(notes, bpm, rate);
        let mut next = 0;
        let mut rendered = vec![[0.0; 2]; frames.len()];
        for start in (0..frames.len() + latency).step_by(BLOCK) {
            check_cancelled(cancellation)?;
            buffers.clear();
            if let Some(input) = buffers
                .inputs
                .iter_mut()
                .find(|b| b.active && !b.channels.is_empty())
            {
                ensure!(
                    input.channels.len() <= 2,
                    "VST3 main input must be mono or stereo"
                );
                for i in 0..BLOCK {
                    if let Some(frame) = frames.get(start + i) {
                        if input.channels.len() == 1 {
                            input.channels[0][i] = (frame[0] + frame[1]) * 0.5;
                        } else {
                            input.channels[0][i] = frame[0];
                            input.channels[1][i] = frame[1];
                        }
                    }
                }
            }
            while next < events.len() && events[next].0 < start + BLOCK {
                plugin.send_midi_event_at(events[next].1, (events[next].0 - start) as i32)?;
                next += 1;
            }
            plugin
                .process_bus_audio(&mut buffers)
                .with_context(|| format!("VST3 processing failed: {}", device.display_name()))?;
            let output = &buffers.outputs[output_bus].channels;
            for (i, (left, right)) in output[0].iter().zip(&output[channels - 1]).enumerate() {
                if let Some(index) = (start + i).checked_sub(latency) {
                    if let Some(frame) = rendered.get_mut(index) {
                        *frame = [*left, *right];
                        ensure!(
                            frame.iter().all(|s| s.is_finite()),
                            "VST3 produced invalid audio"
                        );
                    }
                }
            }
        }
        frames.copy_from_slice(&rendered);
        Ok(())
    })();
    let stopped = plugin.stop_processing();
    result?;
    stopped?;
    Ok(())
}

fn check_cancelled(
    cancellation: Option<&(std::sync::Arc<std::sync::atomic::AtomicU64>, u64)>,
) -> Result<()> {
    if let Some((generation, expected)) = cancellation {
        ensure!(
            generation.load(std::sync::atomic::Ordering::Acquire) == *expected,
            "Render superseded by a newer edit"
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn muted_notes_are_not_sent_and_color_selects_the_midi_channel() {
        let notes = [
            MidiNote {
                channel: 16,
                ..MidiNote::default()
            },
            MidiNote {
                muted: true,
                ..MidiNote::default()
            },
        ];
        let events = note_events(&notes, 120.0, 48000);
        assert_eq!(events.len(), 2);
        assert!(matches!(
            events[0].1,
            MidiEvent::NoteOn {
                channel: MidiChannel::Ch16,
                ..
            }
        ));
        assert!(matches!(
            events[1].1,
            MidiEvent::NoteOff {
                channel: MidiChannel::Ch16,
                ..
            }
        ));
    }
    #[test]
    fn midi_events_keep_sample_offsets_and_release_before_retrigger() {
        let note = |start| MidiNote {
            key: 60,
            velocity: 100,
            start_beats: start,
            length_beats: 0.5,
            ..MidiNote::default()
        };
        let events = note_events(&[note(0.0), note(0.5)], 120.0, 48000);
        assert_eq!(
            events.iter().map(|e| e.0).collect::<Vec<_>>(),
            [0, 12000, 12000, 24000]
        );
        assert!(matches!(events[1].1, MidiEvent::NoteOff { .. }));
        assert!(matches!(events[2].1, MidiEvent::NoteOn { .. }));
    }
}
