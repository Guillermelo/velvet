# Continuous audio migration

## First implementation — 2026-10-05

Playback still prepares a complete mix. This change does not meet the continuous
playback acceptance criteria yet.

`crates/audio/src/devices.rs` now owns persistent processors for builtin gain,
three-band EQ, EQ8 and compressor. Construction captures validated parameters
and allocates filter storage. Repeated processing calls preserve stereo filter
history and compressor gain reduction without allocation, project reads or locks.
Constructing a new processor resets its state; live parameter updates and explicit
transport reset policy remain to be implemented.

The existing offline rack constructs these processors and processes the entire
buffer once. This keeps the current app/export path connected to the DSP used by
the future graph, without changing the shared project or piano roll.

Continuity tests compare single-buffer and partitioned processing at 8, 44.1, 48
and 192 kHz, using block lengths 1, 63, 256, 511 and 1024. Transients straddle
boundaries and leave quiet tails. The tests also deliberately reconstruct the
stateful processors every 64 frames and confirm the fixtures detect state loss.
Existing frequency-response, compressor and offline limiter tests remain intact.

## Remaining implementation

1. Persistent synth voices and Beat delays/hold; bounded limiter lookahead with
   declared latency and offline compensation. The limiter still uses the existing
   full-buffer envelope; it must not be put on a continuous worker unchanged.
2. Persistent track/master graph and sample-offset MIDI scheduler. Keep plugin
   creation, processing and teardown on its owning worker; keep native editors
   and their controllers on the UI thread. Preserve the local VST3 patches.
3. Bounded worker-to-CPAL audio handoff, revision/transport commands, underrun
   counters and recent-output scope transfer. The worker must produce blocks on
   demand with bounded lead rather than render an arrangement in advance.
4. Central playback coordination connected to the app, continuous note preview,
   seek/loop/tempo/reset policies and latency compensation. Offline export should
   drive the same graph with an offline clock.
5. Hardware measurements and individual instrument/effect/editor regression
   checks from `realtime-audio-migration-brief.md`.

No continuous plugin, CPAL latency or compatibility claim follows from the DSP
tests. External hardware MIDI remains a separate extension.

## Verification

- Before edits: app tests 48 passed, 2 failed, 4 hardware tests ignored. Failures:
  `matrix::tests::hover_and_tempo_motion_preserve_the_grid_and_follow_transport`
  (sparse white contours), and
  `tests::synced_clip_loop_and_waveform_share_the_musical_grid`
  (missing sixteenth-note grid). Both precede this migration change.
- After the DSP change: audio tests 25 passed, 0 failed, 1 hardware test ignored.
  The new continuity test is included. Hardware/plugin smoke checks were not run.

Existing uncommitted work is the baseline. This step edits only `devices.rs` and
adds this progress record; no shared UI files or VST dependency patches are edited.
