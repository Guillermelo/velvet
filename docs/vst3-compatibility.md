# VST3 compatibility on Windows

The compatibility smoke test covers loading, parameter/state restoration,
MIDI synthesis, finite non-silent stereo audio, native editor open/close/reopen,
editor event pumping with DSP/state capture, project save/reopen, output scopes
and stereo WAV export.

Run in a normal Windows desktop process, with access to the installed plugins'
resources and the user's settings. In Codex, execute these plugin tests outside
its restricted command sandbox. This does not require running Velvet as an
administrator.

```powershell
cargo run -p velvet-audio --example plugin_smoke -- instrument "C:\Program Files\Common Files\VST3\KORG\M1.vst3" --editor
cargo run -p velvet-audio --example plugin_smoke -- instrument "C:\Program Files\Common Files\VST3\Spectrasonics\Omnisphere.vst3" --editor
```

Observed with M1 2.3.2 and Omnisphere 2.8.7c:

| Check | M1 | Omnisphere |
| --- | --- | --- |
| Output buses in normal desktop process | 8 stereo | 9 stereo |
| Restored MIDI audio peak in controlled smoke edit | 0.078281 | about 0.2155 |
| Native editor dimensions | 1108 × 632 | 1392 × 804 |
| Editor close and reopen, with state capture | pass | pass |
| Project restore, output scopes, WAV export | pass | pass |

The same complete smoke test passed Spire (editor 1024 × 535, peak 0.123381)
and Vital (editor 1364 × 799, peak 0.358795). These are controlled test patches,
not assertions that every factory patch should produce the same amplitude.

## Causes and fixes

The restricted command sandbox changed M1's initialization: it advertised 65
output buses and faulted in M1.dll while clearing an internal 130-channel table
with only 16 valid pointers. In the normal desktop process it advertises eight
stereo buses and processes without that access violation. Omnisphere's editor
also faults or stalls inside `IPlugView::attached` in the restricted process.
Changing bus activation does not repair those environment failures.

There was a second, independently reproduced M1 failure after state restore in
a normal process. Its bridge reads state in 1 MiB chunks. The host's memory
stream did not terminate at EOF. An initial patch which failed short reads
made M1 audible but discarded the new preset, leaving its previous sound.
The corrected implementation returns success when bytes are delivered and
failure only on an empty nonzero read at EOF. An audible regression alternates
master volume 0.2/0.8 and compares edited versus restored audio, rather than
checking only non-silence or controller values. See
`vendor/vst3-host/PATCHES.md` for the patch and test commands.

Omnisphere's Windows editor needs OLE initialized on its owning UI thread. Both
the app entry point and smoke example retain a thread-bound OLE guard until their
plugin windows and instances are destroyed. No temporary editor-query removal
is needed. Spire needs its editor size queried again after attachment: its
pre-attachment width can be zero. The fallback container permits attachment.

A separate normal-process Omnisphere hang was reproduced on fast editor close:
component termination returned, but module `ExitDll` waited indefinitely.
The previous loader treated every component as an independent DLL lifetime.
Windows now shares one initialized module per canonical path and retains it
until process exit, together with a stable factory host context. Editor/render
components still close and terminate normally. Updating a loaded plugin binary
requires restarting Velvet. The rapid-close profile now completes and renders
successive states instead of hanging during module teardown.

Both plugin-name compatibility blocks have been removed. Audio still allocates
all instrument output buses and uses the main mono/stereo output in the rack.

## Presets and responsiveness

The playback worker retains each device's plugin instance on one owning thread.
Unchanged track audio is reused; mixer gain and balance changes reuse the
prefader bus too. Removed devices and changed plugin paths/sample rates release
the old instance. Newer edits cancel old VST processing between 512-frame blocks.
Native plugin loading/state calls themselves cannot be interrupted safely.

Editor changes trigger debounced state captures through host notifications.
Private state changes without notifications retain a two-second fallback poll;
save, close and VST note preview capture explicitly. Old GUI parameter values
are discarded before restore so they cannot overwrite the new preset.

Vital does not clear voices in `releaseResources`. MIDI panic must be queued
**after** state restoration, which clears MIDI-mapped parameter queues.
This prevents old voices from leaking into subsequent renders.
The reset is processed in its own block before new notes, because delivering
reset and note-on together still allowed Vital's deferred voice cleanup to leak
the previous louder patch into the quieter render. Transport is reset afterward.

Run the audible and preset-file regression in a normal desktop process:

```powershell
cargo run -p velvet-audio --example plugin_state_roundtrip -- "C:\Program Files\Common Files\VST3\KORG\M1.vst3" --program
cargo run -p velvet-audio --example plugin_state_roundtrip -- "C:\Program Files\Common Files\VST3\Vital.vst3"
cargo run -p velvet-audio --example plugin_state_roundtrip -- "C:\Program Files\Common Files\VST3\Spire-1.5_x64\Spire.vst3"
cargo run -p velvet-audio --example plugin_state_roundtrip -- "C:\Program Files\Common Files\VST3\Spectrasonics\Omnisphere.vst3"
```

`plugin_profile <path>` measures cold loading, state capture and successive
cached eight-second renders. Cold loads include plugin resource initialization;
warm render timings are not a promise of immediate response for every preset.
See [the architecture assessment](vst3-host-architecture.md).

## Verification recorded on 2026-10-05

- M1, Omnisphere, Vital and Spire: native editor open/close/reopen, MIDI,
  finite audible output, project persistence, device scopes and WAV export pass.
- All four: four alternating audible edits, preset-file restore, cached playback
  and piano-roll preview pass. The first/last state also passes a fresh
  project/export instance. M1 additionally switches actual program indices 0–3.
- Omnisphere also completes twelve successive program selections and rapid
  editor-instance destruction with `plugin_profile <path> --program`. That
  diagnostic adds a deliberate 500 ms settling interval per program; its
  reported capture time includes this interval and is not host capture latency.
- Warm eight-second renders in the development build: M1 about 124–125 ms,
  Vital 72–73 ms, Spire 73–76 ms, Omnisphere 169–183 ms. Median state capture:
  M1 10.5 ms, Vital 4.4 ms, Spire 29.2 ms, Omnisphere 8.0 ms. These measurements
  exclude first instance/resource loading and are not preset-to-speaker latency.
- `velvet-audio --lib`: 24 pass, one hardware test ignored.
- Patched dependency: 208 pass; unavailable external SDK fixture skipped.
- `velvet-app` binary builds. Full app suite: 48 pass, two failures in existing
  matrix/grid assertions, four hardware tests ignored. Failures are
  `hover_and_tempo_motion_preserve_the_grid_and_follow_transport` and
  `synced_clip_loop_and_waveform_share_the_musical_grid`; no piano-roll or
  arrangement edits were reverted to suppress these failures.

An earlier smoke edit added 0.001 to M1's initial queried volume. That could
produce audio below the non-silence threshold. The controlled smoke edit now
uses a clearly audible normalized level; the independent state regression
alternates 0.2/0.8 and checks restoration against actual edited audio.

M1's queried cutoff parameter can remain stale after preset-file restore even
when the audible patch is correctly restored. The file regression therefore
checks audible output as well as project master-volume restoration.
