# Velvet

> An open-source, AI-native digital audio workstation written in Rust.

Velvet is a cross-platform digital audio workstation designed around three first-class interfaces:

- a desktop GUI,
- a CLI,
- an AI agent.

The DAW must remain fully usable without AI.

AI is an optional control layer over the same project and command system used by the GUI and CLI.

The initial target platforms are:

```text
Windows
Linux
```

Linux is the primary long-term development target, but the application must remain usable on Windows from the beginning.

---

# Core idea

Velvet is built around one simple principle:

```text
GUI
CLI
AI
 │
 ▼
Command System
 │
 ▼
Project State
 │
 ├── Audio Engine
 └── Project Persistence
```

The GUI, CLI and AI must not manipulate unrelated internal state directly.

They all issue commands.

Examples:

```text
AddTrack
RemoveTrack
RenameTrack

ImportAudioClip
MoveClip
TrimClip

SetTrackVolume
SetTrackPan
SetMute
SetSolo

AddDevice
RemoveDevice
SetDeviceParameter

Play
Pause
Stop
Seek

SaveProject
RenderProject
```

This makes the project:

- predictable,
- scriptable,
- reversible,
- AI-friendly,
- testable.

---

# The DAW must work without AI

Velvet is not an AI application pretending to be a DAW.

It is a real DAW with optional AI control.

Without configuring an OpenAI API key, the user must still be able to:

```text
create projects
open projects
save projects
import audio
move clips
trim clips
play audio
seek
mute tracks
solo tracks
change volume
change pan
add built-in devices
edit device parameters
undo
redo
render audio
```

AI only adds another way to perform those operations.

---

# Project format

Projects use a declarative, human-readable YAML file.

Example:

```text
my-song/
├── project.yaml
├── recordings/
├── generated/
├── cache/
└── renders/
```

`project.yaml` is the persistent representation of the project.

Example:

```yaml
format: 1

project:
  id: project_01
  name: My Song

audio:
  sample_rate: 48000

tempo:
  bpm: 120

tracks:

  - id: track_drums
    name: Drums
    type: audio

    mixer:
      volume_db: -1.0
      pan: 0.0
      mute: false
      solo: false

    clips:

      - id: clip_kick_loop

        source:
          path: "C:/Users/user/Music/Samples/kick-loop.wav"
          kind: external

        position:
          start_beats: 0
          offset_seconds: 0
          length_seconds: 8.0

    devices:

      - id: gain_01
        type: builtin.gain

        parameters:
          gain_db: 0.0
```

The YAML must be easy to:

- read manually,
- edit manually,
- inspect with AI,
- diff with Git,
- serialize from Rust,
- migrate between format versions.

---

# Runtime state

The YAML file is not the realtime audio engine.

When the project opens:

```text
project.yaml
     │
     ▼
Rust Project Model
     │
     ├── GUI
     ├── Commands
     ├── Audio Engine
     └── AI tools
```

Changes modify the in-memory Rust project state.

The project is then saved back to YAML.

The application should not rewrite the YAML continuously for every mouse movement or audio callback.

Persistence should use safe atomic saves.

Conceptually:

```text
project.yaml
     │
     ▼
project.yaml.tmp
     │
     ▼
validate
     │
     ▼
atomic replace
     │
     ▼
project.yaml
```

Autosave can later use a separate file:

```text
project.autosave.yaml
```

---

# Audio file handling

Imported audio must not be duplicated automatically.

If the user drags:

```text
D:/Samples/Kicks/kick-07.wav
```

into Velvet, the project should reference that file.

Example:

```yaml
source:
  path: "D:/Samples/Kicks/kick-07.wav"
  kind: external
```

The original file stays where it is.

Velvet must not silently copy it into the project.

---

# Missing files

If an external file is moved or deleted, the project remains valid but the clip is marked as missing.

Example UI:

```text
Missing audio file

kick-07.wav

Original location:
D:/Samples/Kicks/kick-07.wav

[ Locate ] [ Search ]
```

The user should be able to relink the file.

The clip metadata must remain in the project even while its source file is unavailable.

---

# Project-owned audio

Audio created by Velvet belongs inside the project.

Example:

```text
my-song/
├── project.yaml
├── recordings/
│   └── vocal-take-001.wav
├── generated/
│   └── bounced-bass-001.wav
├── cache/
└── renders/
```

Rules:

```text
Imported sample
→ reference original location

Recorded audio
→ recordings/

Generated or bounced audio
→ generated/

Waveform cache and temporary analysis
→ cache/

Exports
→ renders/
```

---

# Collect project media

Velvet should eventually support an operation similar to "Collect All and Save".

For example:

```bash
velvet collect
```

This copies all external files required by the project into a project-owned media directory.

Example:

```text
my-song/
├── media/
│   ├── kick.wav
│   ├── snare.wav
│   └── vocal.wav
```

Then:

```yaml
source:
  path: media/kick.wav
  kind: project
```

This is optional.

Normal saving must not duplicate external media.

---

# Non-destructive editing

The initial editor is entirely non-destructive.

Operations such as:

```text
trim
move
gain
pan
mute
solo
device processing
```

modify project metadata or processing state.

They do not modify the original audio file.

For example, trimming a clip changes:

```yaml
position:
  offset_seconds: 2.4
  length_seconds: 5.8
```

The source file remains unchanged.

---

# Cross-platform file paths

Velvet must work on Windows and Linux.

Rust code should use:

```rust
std::path::Path
std::path::PathBuf
```

and must not build paths manually using `/` or `\`.

Project-owned paths should preferably be stored relative to the project root.

Example:

```yaml
source:
  path: recordings/vocal-001.wav
  kind: project
```

External media may require absolute paths.

Windows:

```yaml
source:
  path: "D:/Samples/kick.wav"
  kind: external
```

Linux:

```yaml
source:
  path: "/home/user/Samples/kick.wav"
  kind: external
```

Later versions may store additional metadata to improve project relocation and missing-file recovery.

---

# Stable IDs

Internal relationships should use stable IDs rather than display names.

Good:

```yaml
tracks:

  - id: track_a7c31
    name: Lead Vocals
```

And:

```yaml
track_id: track_a7c31
```

Not:

```yaml
track: Lead Vocals
```

Users can rename tracks without breaking project references.

---

# Arrangement View

Velvet's primary interface is an Arrangement View.

There is no Session View in the initial product.

The layout takes inspiration from the efficiency of Ableton Live's Arrangement View:

```text
┌──────────────────────────────────────────────────────────────────┐
│ ▶  ■  ●       120 BPM        4/4                CPU       ✦ AI │
├──────────────┬─────────────────────────────────────┬─────────────┤
│              │                                     │             │
│   Browser    │            Arrangement              │   Velvet AI │
│              │                                     │             │
│  Files       │ Drums   ███████  ███████           │             │
│  Samples     │ Bass      ███████████████           │             │
│  Devices     │ Vocals       ███████████████        │             │
│              │                                     │             │
│              │                                     │             │
├──────────────┴─────────────────────────────────────┴─────────────┤
│ Device Rack                                                      │
│                                                                  │
│ [ Gain ] → [ EQ ] → [ ... ]                                     │
└──────────────────────────────────────────────────────────────────┘
```

---

# Visual direction

Velvet should feel elegant before it feels impressive.

The interface should be:

```text
minimal
precise
quiet
dense
premium
fast
coherent
desktop-native
```

The product should avoid looking like:

```text
a developer tool
a dashboard
a generic AI application
a gaming interface
a neon cyberpunk UI
a mobile app stretched onto desktop
```

Velvet should not rely on excessive decoration to look modern.

The visual language should come from:

- careful spacing,
- typography,
- alignment,
- hierarchy,
- restrained contrast,
- subtle depth,
- deliberate motion,
- consistent interaction patterns.

---

# Elegant, not flashy

The UI should avoid visual noise.

Prefer:

```text
thin separators
subtle borders
soft panel contrast
small radius values
controlled shadows
compact controls
high information density
restrained accent colors
```

Avoid:

```text
huge rounded cards
heavy gradients
glassmorphism everywhere
large empty areas
oversized typography
excessive drop shadows
bright neon accents
unnecessary animations
```

The goal is to make Velvet feel like professional creative software.

---

# Color system

The default interface should use a restrained neutral palette.

Conceptually:

```text
Background        near-black / charcoal
Panels            slightly lighter neutral
Borders           subtle neutral contrast
Primary text      soft off-white
Secondary text    muted gray
Accent            one restrained brand color
Warnings          contextual only
Clip colors       user-controlled
```

The accent color should not dominate the application.

Track and clip colors should carry most of the expressive color.

The application chrome should remain neutral.

---

# Typography

Typography is part of the product identity.

Velvet should prioritize:

```text
excellent legibility
compact metrics
clear numerical values
consistent hierarchy
good rendering at small sizes
```

The UI should not use oversized headings.

Most information should fit comfortably inside a dense professional workspace.

Numbers such as:

```text
120.00 BPM
-4.2 dB
48 kHz
256 samples
01:14.238
```

should be visually clear and aligned.

Monospaced or tabular-number variants may be useful where appropriate.

---

# Motion

Animation should communicate state, not decorate the UI.

Good uses:

```text
panel opening
device insertion
selection changes
AI panel transitions
playhead movement
hover feedback
```

Animations should be:

```text
short
subtle
responsive
interruptible
```

The UI must never feel sluggish because of animation.

---

# Ableton-inspired workflow

Velvet may strongly borrow successful interaction patterns from Ableton Live:

- track headers on the left,
- horizontal arrangement timeline,
- clips arranged by time,
- browser on the left,
- device rack on the bottom,
- compact transport controls,
- dense information layout,
- drag-and-drop,
- keyboard shortcuts,
- minimal visual chrome.

The goal is to reproduce the efficiency of that workflow, not to create a pixel-perfect clone.

Velvet should develop its own:

- visual identity,
- typography,
- icons,
- component styling,
- spacing,
- branding.

There is no Session View.

The Arrangement View is the primary workspace.

---

# Drag and drop

Drag-and-drop audio import is part of the MVP.

The user should be able to drag:

```text
.wav
.flac
```

from the filesystem into the arrangement.

Dropping a file creates a clip referencing the original file.

Conceptually:

```text
Filesystem
    │
    │ drag
    ▼
Arrangement
    │
    ▼
ImportAudioClip command
    │
    ▼
Project State
```

No automatic file duplication.

---

# Audio engine

The realtime audio engine is written in Rust.

The initial abstraction should support Windows and Linux.

Preferred initial audio I/O library:

```text
cpal
```

Expected backends include:

```text
Windows
→ WASAPI

Linux
→ ALSA / JACK depending on environment
```

PipeWire support can be accessed through compatible Linux audio backends where appropriate.

The rest of the application must not depend directly on platform-specific APIs.

Conceptually:

```text
Project State
     │
     ▼
Audio Graph
     │
     ▼
Audio Backend Abstraction
     │
     ├── Windows
     └── Linux
```

---

# Realtime safety

The realtime audio callback must remain isolated from expensive application work.

Do not perform inside the audio callback:

```text
filesystem access
network requests
OpenAI requests
YAML serialization
GUI work
blocking locks
unbounded allocation
plugin compilation
AI reasoning
```

Architecture:

```text
NON-REALTIME

GUI
CLI
AI
 │
 ▼
Commands
 │
 ▼
Project State
 │
 ▼
Realtime Message Queue

══════════════════════════════════

REALTIME

Audio Engine
 │
 ▼
DSP Graph
 │
 ▼
Audio Output
```

---

# Built-in devices

The MVP should include a very small device system.

Initial devices:

```text
Gain
Basic EQ
```

Possible later additions:

```text
Compressor
Filter
Delay
Reverb
Limiter
Utility
```

Device parameters are part of project state.

Example:

```yaml
devices:

  - id: eq_vocal
    type: builtin.eq

    parameters:
      low_gain_db: 0.0
      mid_gain_db: -2.0
      high_gain_db: 1.5
```

Built-in device processing must use the same conceptual interfaces that the GUI, CLI and AI can inspect.

---

# AI

AI is part of the MVP.

The first provider is:

```text
OpenAI
```

The user provides their own API key.

Example:

```bash
OPENAI_API_KEY=...
```

The key must not be stored inside:

```text
project.yaml
```

and must never be committed to Git.

Use environment variables or secure application configuration.

---

# AI architecture

The AI agent does not directly edit project memory and should not directly rewrite YAML.

It uses structured Velvet tools.

Example:

```text
User
 │
 │ "lower the vocals by 3 dB"
 ▼
OpenAI
 │
 ▼
Tool Call
 │
 ▼
track.set_volume
 │
 ▼
Command System
 │
 ▼
Project State
```

AI operations therefore behave like normal DAW operations.

They must support:

```text
validation
undo
redo
history
logging
```

---

# Initial AI tools

The first tool set should remain small.

Project:

```text
project.inspect
project.save
```

Tracks:

```text
track.list
track.create
track.rename
track.delete

track.set_volume
track.set_pan

track.mute
track.solo
```

Clips:

```text
clip.list
clip.move
clip.trim
```

Devices:

```text
device.list
device.add
device.remove
device.get_parameters
device.set_parameter
```

Transport:

```text
transport.play
transport.pause
transport.stop
transport.seek
```

Later:

```text
audio.analyze
render.preview
render.export
plugin.search
plugin.create
```

---

# Example AI interactions

The first AI version should already support commands like:

```text
"lower the vocals by 3 dB"

"mute the bass"

"pan the guitar slightly left"

"move the second vocal clip later"

"add an EQ to the vocals"

"reduce the vocal mids"

"what tracks are in this project?"

"which tracks are muted?"

"make the drums slightly louder"
```

AI must inspect project state before making assumptions when required.

---

# AI panel

The AI assistant lives in a collapsible panel on the right side.

The bottom panel remains dedicated to devices.

Example:

```text
┌───────────────────────────────┐
│ Velvet AI                     │
├───────────────────────────────┤
│                               │
│ > Lower vocals by 2 dB        │
│                               │
│ Lead Vocals                   │
│ -4.0 dB → -6.0 dB             │
│                               │
│ [ Undo ]                      │
│                               │
├───────────────────────────────┤
│ Ask Velvet...                 │
└───────────────────────────────┘
```

The AI interface should visually match the DAW.

It should not look like a chatbot embedded inside another product.

AI should feel like a native control surface for Velvet.

The interface should clearly expose what changed.

Do not make invisible modifications.

---

# CLI

The CLI is part of the architecture from the start.

The executable is:

```text
velvet
```

Example:

```bash
velvet new song

velvet track list

velvet track add --name "Vocals"

velvet track volume track_01 -3

velvet clip import \
  --track track_01 \
  "D:/Vocals/vocal.wav"

velvet play

velvet save

velvet render output.wav
```

AI:

```bash
velvet ask "make the vocals slightly louder"
```

The CLI and GUI operate on the same command layer.

---

# Undo and redo

Commands should be designed for undo/redo from the beginning.

Example:

```text
SetTrackVolume
```

stores enough information to reverse:

```text
previous: -3 dB
new: -6 dB
```

AI commands enter the same history.

Example:

```text
AI:
"make vocals quieter"

History:

1. SetTrackVolume
   track: vocal_01
   -3 dB → -5 dB
```

The user can undo it normally.

---

# GUI technology

The initial GUI should prioritize iteration speed, custom rendering and cross-platform support.

Preferred MVP stack:

```text
egui / eframe
```

Reasons:

- Rust-native,
- Windows support,
- Linux support,
- rapid prototyping,
- custom widgets,
- custom rendering,
- simple deployment.

Velvet should not use stock egui styling as its final visual identity.

The application should define its own:

```text
theme
spacing system
typography
icons
component styles
hover states
selection states
timeline widgets
device widgets
menus
context menus
```

The GUI abstraction should remain reasonably isolated so it can be replaced later if it becomes a limitation.

---

# Suggested Rust stack

Initial dependencies may include:

```text
Audio
  cpal

Serialization
  serde
  serde_yaml

CLI
  clap

GUI
  egui
  eframe

HTTP
  reqwest

Async
  tokio

IDs
  uuid

Errors
  thiserror
  anyhow
```

Dependencies should remain minimal.

Do not add libraries for hypothetical future requirements.

---

# Workspace

Keep the repository modular without creating unnecessary micro-crates.

Suggested initial structure:

```text
velvet/
├── Cargo.toml
├── README.md
├── crates/
│
│   ├── core/
│   │   └── src/
│   │
│   ├── audio/
│   │   └── src/
│   │
│   ├── ai/
│   │   └── src/
│   │
│   ├── cli/
│   │   └── src/
│   │
│   └── app/
│       └── src/
│
└── examples/
```

Possible responsibilities:

```text
core
  project model
  commands
  undo/redo
  YAML
  tracks
  clips
  devices

audio
  playback
  audio graph
  DSP
  backend integration

ai
  OpenAI client
  tools
  tool execution adapter

cli
  CLI frontend

app
  Velvet desktop GUI
```

Do not split further unless there is a concrete reason.

---

# MVP

The first usable milestone should support:

## Projects

```text
Create project
Open project
Save project
YAML persistence
Schema version
```

## Audio

```text
Import WAV/FLAC
Reference external files
Detect missing files
Playback
Pause
Stop
Seek
Multiple tracks
```

## Arrangement

```text
Waveform display
Move clips
Trim clips
Select clips
Delete clips
Drag-and-drop import
```

## Mixer

```text
Volume
Pan
Mute
Solo
Master output
```

## Devices

```text
Gain
Basic EQ
Device chain
Parameter editing
```

## Editing

```text
Undo
Redo
Non-destructive editing
```

## CLI

```text
Project inspection
Track manipulation
Clip import
Mixer manipulation
Playback controls
```

## AI

```text
OpenAI integration
Velvet AI panel
Natural-language project inspection
Track manipulation
Mixer manipulation
Basic device manipulation
Undoable AI actions
```

## Rendering

```text
Export project to WAV
```

---

# Not MVP

Do not implement these yet:

```text
Session View
MIDI piano roll
MIDI recording
advanced automation
VST3 hosting
CLAP hosting
time warping
pitch correction
video
cloud collaboration
AI song generation
AI stem generation
plugin generation
multi-agent architecture
advanced mastering tools
```

They are future capabilities.

---

# Future plugin system

External plugin hosting should come after the core DAW architecture works.

Preferred first external format:

```text
CLAP
```

Possible later support:

```text
VST3
LV2
```

One long-term feature is AI-generated DSP.

Example:

```text
"create a soft clipper with drive and mix controls"
```

Possible architecture:

```text
Prompt
  │
  ▼
AI
  │
  ▼
DSP representation
  │
  ▼
Compile
  │
  ▼
Sandbox
  │
  ▼
Device
```

WebAssembly should be investigated as a sandboxed execution environment for generated DSP.

---

# Design principles

## DAW first

Velvet must be useful without AI.

## AI early

AI is a core product feature and should be integrated in the first usable milestone.

## Arrangement-first

The primary workflow is the horizontal track arrangement.

No Session View is required.

## Declarative projects

Project state must remain understandable outside the application.

## Reference, don't duplicate

External media remains external unless the user explicitly collects it.

## Non-destructive

Do not modify imported source media.

## Commands everywhere

GUI, CLI and AI use the same command system.

## Realtime isolation

AI, network and GUI workloads never execute in the realtime audio path.

## Cross-platform architecture

Windows and Linux are supported through portable abstractions.

## Elegant by default

Visual quality is a product requirement, not cleanup work for later.

## Small modules

Prefer explicit, understandable code over large speculative frameworks.

## Minimal changes

Features should be implemented with the smallest coherent amount of code necessary.

Avoid turning small requirements into large abstractions.

---

# Development philosophy

Do not attempt to build Ableton Live in the first release.

Build a small DAW whose architecture makes future growth possible.

The first major architectural target is:

```text
GUI + CLI + AI
       │
       ▼
   Commands
       │
       ▼
 Project State
       │
 ┌─────┴─────┐
 ▼           ▼
Audio       YAML
```

If this model is solid, features can be added incrementally without rebuilding the entire application.

Visual quality should evolve alongside functionality.

Do not create a technically functional interface first with the assumption that design can simply be "added later".

Core components should be designed intentionally from the start.

---

# Vision

A traditional DAW expects the user to manipulate every parameter manually.

Velvet supports both:

```text
manual control
```

and:

```text
intent-driven control
```

For example:

```text
"lower my vocals by 2 dB"

"add an EQ to this track"

"move the chorus guitar later"

"show me every track using EQ"

"why is this track muted?"

"make the drums slightly punchier"
```

The important part is not the chat interface.

The important part is that the entire DAW is built around a programmable, structured project model that humans, scripts and AI can all operate safely.

The name Velvet reflects the intended product experience:

```text
smooth
precise
quiet
refined
```

The application should feel polished without feeling ornamental.

---

# Status

```text
Experimental / pre-alpha
```

Initial priorities:

```text
1. Rust project model
2. YAML format
3. Command system
4. Cross-platform audio playback
5. Arrangement UI
6. Visual system
7. Drag-and-drop audio
8. Mixer
9. Built-in devices
10. OpenAI tool integration
11. Render/export
```

AI should be integrated as soon as the command system exposes enough useful operations.

---

# License

TBD.

A permissive open-source license such as:

```text
Apache-2.0
```

or:

```text
MIT
```

is likely appropriate for the initial project.