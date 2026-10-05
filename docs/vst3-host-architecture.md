# VST3 host architecture assessment

The existing project is usable as a foundation. The main limitation was the
playback strategy: every preset edit created new plugin instances and rendered
the complete arrangement. Increasing bus counts or suppressing editor errors
could not fix state persistence or edit latency.

## Decisions retained and corrected

| Decision | Assessment and resulting implementation |
| --- | --- |
| Immutable mix read by CPAL | Retained. The callback does not allocate, serialize presets or load DLLs. Device errors and slow plugin calls cannot block it. |
| Offline render shared by export and playback preparation | Retained as the reference renderer. Playback adds `LiveMixer`, reusing device instances and unchanged prefader track buses. |
| A new render thread per edit | Replaced by one persistent owning thread. Creating, restoring, processing and destroying each cached plugin stays on that thread. `RenderCache` cannot be sent between threads. |
| Separate editor and render instances | Retained for the current prepared-audio design. The UI captures state; the worker restores it. This avoids a UI mutex inside the audio callback, but full preset serialization remains an edit cost. |
| Serialize every 500 ms | Changed to revision-driven captures after a short pause, capped during continuous edits. A two-second fallback supports private state changes without callbacks. Save/close/preview still capture explicitly. |
| Retain all queued GUI values | Coalesce the newest value per parameter and discard old pending GUI values before restoring state. Gesture history is independent. |
| Verify only successful restore and non-silent audio | Insufficient. Audible tests alternate large level changes, compare live and restored audio, exercise preset files and check warm previews. This exposed M1 keeping its previous preset. |
| Local patched dependency | Explicit Cargo override of published 0.9.0 under `vendor/vst3-host`, with MIT license and patch ledger. Disposable `target` experiments are not dependencies. |
| DLL lifetime equals component lifetime | Incorrect for simultaneous editor/render instances. Windows now initializes one module per canonical path and retains it until process exit. Components are still terminated independently; the factory host context stays valid. This also fixes the reproduced Omnisphere `ExitDll` hang after rapid editor teardown. Loaded DLLs require an app restart before binary replacement. |

Track cache validity includes MIDI, regions, clips, devices, tempo, sample rate,
root, render length, requested scopes and decoded source identity. Mixer gain,
balance, names and colors do not invalidate the prefader audio. Retained track
audio is bounded to about 128 MiB. Changes to plugin path/rate replace the
instance; removed devices are released on their owner thread.

The callback continues playing the last complete mix while the worker prepares
the newest revision. A superseded VST render stops between blocks and cleans
up processing. DLL initialization and opaque plugin state methods cannot be
forcibly interrupted within the process.

## What still limits future work

Prepared audio cannot provide a fixed, tiny preset-to-sound latency on long
arrangements. The changed track still needs rendering, and master effects still
process the resulting mix. A long instrument release or expensive patch also
costs real processing time. Opening a first native editor still loads resources
on its UI thread.

For immediate performance and live MIDI, the next coherent step is a continuous
audio graph with persistent processors, scheduled MIDI/parameter events and a
bounded audio handoff to CPAL. Controller/editor work must remain on the UI
thread. Reuse the project model, MIDI scheduling, VST setup and offline export;
replace the prepared-playback implementation behind the playback interface.
There is no need to rewrite the piano roll or project history to do this.

This remains a VST3 host. VST2 `.dll` plugins need a separate adapter and have
not been implemented or tested by these changes. Linux-native VST3 binaries
also need Linux audio/editor testing; Windows plugin binaries cannot be loaded
directly by the Linux loader. A Windows-to-Linux bridge is a separate integration.
[Yabridge](https://github.com/robbert-vdh/yabridge) supplies Linux wrappers for
Windows VST3 plugins through Wine. Velvet and these installed synthesizers have
not been validated with that bridge; the Windows verification does not imply
Linux editor, audio or licensing compatibility.

Third-party DLLs currently run in-process. Their access violations can still
terminate the app. The dependency's isolation feature is not automatically a
complete editor bridge, so adopting it needs explicit audio/state/editor tests;
merely enabling the Cargo feature is not fault containment.

## Evidence and primary references

- The audible M1 regression originally reported edited RMS 0.008732 and
  restored RMS 0.045768. Correct short reads produce restored RMS 0.008784.
  The test fails on the discarded-state implementation.
- [Steinberg persistence sequence](https://steinbergmedia.github.io/vst3_dev_portal/pages/FAQ/Persistence.html)
  defines processor state, controller synchronization and controller state.
- [SDK memory stream](https://github.com/steinbergmedia/vst3_public_sdk/blob/master/source/common/memorystream.cpp)
  preserves bytes and reports their count on short reads. Velvet additionally
  terminates empty nonzero reads, required by the reproduced M1 chunk reader.
- [Vital processor source](https://github.com/mtytel/vital/blob/main/src/plugin/synth_plugin.cpp)
  leaves `releaseResources` empty and displays a native dialog when state JSON
  cannot be parsed. Invalid state delivery can appear as a host freeze.
- [Vital MIDI handling](https://github.com/mtytel/vital/blob/main/src/common/midi_manager.cpp)
  implements All Sounds Off; the host must deliver it after restoring state.
- [Steinberg module loading](https://steinbergmedia.github.io/vst3_dev_portal/pages/Technical%2BDocumentation/VST%2BModule%2BArchitecture/Loading.html)
  separates module entry/exit from object creation and termination. Keeping a
  Windows module mapped until process exit is Velvet's deliberate lifetime
  policy; it fixes a reproduced `ExitDll` hang and prevents per-instance module
  teardown while another instance uses the same DLL.

These source observations explain the reproduced behavior; they do not certify
every third-party plugin or the installed binary's complete implementation.
