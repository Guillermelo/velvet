//! Native note editing with live project updates and grouped pointer history.
use super::*;
use std::collections::BTreeSet;
use velvet_core::{MidiNote, MidiRegion, Track, TrackKind};
mod midi;

const MIN_LENGTH: f64 = 1.0 / 960.0;
const MAX_NOTES: usize = 10_000;
const NAMES: [&str; 12] = [
    "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
];
const SCALES: &[(&str, &[u8])] = &[
    ("Chromatic", &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11]),
    ("Major", &[0, 2, 4, 5, 7, 9, 11]),
    ("Natural minor", &[0, 2, 3, 5, 7, 8, 10]),
    ("Harmonic minor", &[0, 2, 3, 5, 7, 8, 11]),
    ("Melodic minor", &[0, 2, 3, 5, 7, 9, 11]),
    ("Dorian", &[0, 2, 3, 5, 7, 9, 10]),
    ("Phrygian", &[0, 1, 3, 5, 7, 8, 10]),
    ("Lydian", &[0, 2, 4, 6, 7, 9, 11]),
    ("Mixolydian", &[0, 2, 4, 5, 7, 9, 10]),
    ("Locrian", &[0, 1, 3, 5, 6, 8, 10]),
    ("Major pentatonic", &[0, 2, 4, 7, 9]),
    ("Minor pentatonic", &[0, 3, 5, 7, 10]),
    ("Blues", &[0, 3, 5, 6, 7, 10]),
    ("Whole tone", &[0, 2, 4, 6, 8, 10]),
];
const CHORDS: &[(&str, &[i16])] = &[
    ("Single", &[0]),
    ("Major", &[0, 4, 7]),
    ("Minor", &[0, 3, 7]),
    ("Diminished", &[0, 3, 6]),
    ("Augmented", &[0, 4, 8]),
    ("Sus2", &[0, 2, 7]),
    ("Sus4", &[0, 5, 7]),
    ("Major 7", &[0, 4, 7, 11]),
    ("Minor 7", &[0, 3, 7, 10]),
    ("Dominant 7", &[0, 4, 7, 10]),
    ("Minor 9", &[0, 3, 7, 10, 14]),
    ("Major 9", &[0, 4, 7, 11, 14]),
    ("Power", &[0, 7, 12]),
];
const SNAPS: &[(&str, f64)] = &[
    ("None", 0.0),
    ("1/64", 0.0625),
    ("1/32", 0.125),
    ("1/16", 0.25),
    ("1/8", 0.5),
    ("1/4", 1.0),
    ("1/2", 2.0),
    ("Bar", 4.0),
    ("1/16 triplet", 1.0 / 6.0),
    ("1/8 triplet", 1.0 / 3.0),
    ("1/4 triplet", 2.0 / 3.0),
];

#[derive(Clone, Copy, PartialEq)]
enum Tool {
    Draw,
    Paint,
    Drum,
    Erase,
    Mute,
    Slice,
    Select,
    Stamp,
    Zoom,
}
impl Tool {
    fn label(self) -> &'static str {
        match self {
            Self::Draw => "Draw",
            Self::Paint => "Paint",
            Self::Drum => "Drum",
            Self::Erase => "Erase",
            Self::Mute => "Mute",
            Self::Slice => "Slice",
            Self::Select => "Select",
            Self::Stamp => "Chord",
            Self::Zoom => "Zoom",
        }
    }
}
#[derive(Clone, Copy)]
enum Action {
    Delete,
    Duplicate,
    Quantize(bool),
    Legato,
    Chop,
    Glue,
    Transpose(i16),
    Nudge(f64),
    FlipTime,
    FlipPitch,
    Strum,
    Flam,
    Humanize,
    Arpeggiate,
    Staccato,
    ScaleVelocity,
    Color,
    Mute(bool),
}
enum GestureKind {
    Move,
    Right,
    Left,
    Stretch,
    Draw,
    Paint,
    Erase,
    Mute,
    Select,
    Slice,
    Velocity,
    Range,
    Zoom,
    Pan,
}
struct Gesture {
    kind: GestureKind,
    before: Vec<MidiNote>,
    working: Vec<MidiNote>,
    indices: BTreeSet<usize>,
    visited: BTreeSet<usize>,
    origin: Pos2,
    beat: f64,
    key: u8,
    last_beat: f64,
    last_key: u8,
    scroll: (f64, f32),
    additive: bool,
    mute_value: bool,
}
pub(super) struct PianoRoll {
    target: String,
    pub(super) open: bool,
    pub(super) keyboard_focus: bool,
    selected: BTreeSet<usize>,
    last_notes: Vec<MidiNote>,
    clipboard: Vec<MidiNote>,
    gesture: Option<Gesture>,
    edit_history: Option<(Project, u64)>,
    tool: Tool,
    snap: usize,
    length: f64,
    velocity: u8,
    channel: u8,
    root: u8,
    scale: usize,
    snap_scale: bool,
    chord: usize,
    chord_once: bool,
    ghosts: bool,
    note_labels: bool,
    follow: bool,
    px_beat: f32,
    row_height: f32,
    scroll_beat: f64,
    top_key: f32,
    lane_height: f32,
    time_range: Option<(f64, f64)>,
    pub(super) loop_enabled: bool,
    quantize_strength: f64,
    swing: f64,
    humanize_time: f64,
    humanize_velocity: i16,
    strum_time: f64,
    velocity_scale: f64,
    velocity_offset: i16,
    seed: u64,
    // Last geometry is also useful for deterministic pointer integration tests.
    grid_rect: Rect,
    notice: Option<(bool, String)>,
    audition: bool,
}
impl Default for PianoRoll {
    fn default() -> Self {
        Self {
            target: String::new(),
            open: false,
            keyboard_focus: false,
            selected: BTreeSet::new(),
            last_notes: vec![],
            clipboard: vec![],
            gesture: None,
            edit_history: None,
            tool: Tool::Draw,
            snap: 3,
            length: 1.0,
            velocity: 100,
            channel: 1,
            root: 0,
            scale: 0,
            snap_scale: false,
            chord: 1,
            chord_once: false,
            ghosts: true,
            note_labels: true,
            follow: false,
            px_beat: 72.0,
            row_height: 18.0,
            scroll_beat: 0.0,
            top_key: 76.0,
            lane_height: 100.0,
            time_range: None,
            loop_enabled: false,
            quantize_strength: 1.0,
            swing: 0.0,
            humanize_time: 0.03,
            humanize_velocity: 12,
            strum_time: 0.04,
            velocity_scale: 1.0,
            velocity_offset: 0,
            seed: 0x56454c564554,
            grid_rect: Rect::NOTHING,
            notice: None,
            audition: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn note(key: u8, start: f64, length: f64) -> MidiNote {
        MidiNote {
            key,
            start_beats: start,
            length_beats: length,
            ..MidiNote::default()
        }
    }
    #[test]
    fn score_validation_and_scale_stamps_preserve_midi_limits() {
        assert!(
            read_vscore(br#"[{"key":255,"velocity":100,"start_beats":0,"length_beats":1}]"#)
                .is_err()
        );
        assert!(
            read_vscore(br#"[{"key":60,"velocity":100,"start_beats":0,"length_beats":-1}]"#)
                .is_err()
        );
        let mut roll = PianoRoll {
            snap_scale: true,
            scale: 1,
            chord: 7,
            ..PianoRoll::default()
        };
        let mut notes = vec![];
        roll.add_note(&mut notes, 0.0, 126, true);
        assert!(notes
            .iter()
            .all(|n| n.validate().is_ok() && roll.in_scale(n.key)));
        let original = vec![note(60, 1.0, 1.0), note(64, 1.0, 1.0), note(67, 1.0, 1.0)];
        let mut notes = original.clone();
        roll.snap_scale = false;
        roll.action(&mut notes, Action::Arpeggiate);
        assert_eq!(
            notes.iter().map(|n| n.start_beats).collect::<Vec<_>>(),
            vec![1.0, 1.25, 1.5]
        );
        roll.action(&mut notes, Action::FlipTime);
        assert_eq!(notes[0].start_beats, 1.5);
        assert_eq!(notes[2].start_beats, 1.0);
    }
    fn frame(
        app: &mut Velvet,
        ctx: &egui::Context,
        events: Vec<egui::Event>,
        modifiers: egui::Modifiers,
    ) {
        let _ = ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1440.0, 900.0))),
                events,
                modifiers,
                ..Default::default()
            },
            |ctx| {
                app.shortcuts(ctx);
                app.midi_editor(ctx);
            },
        );
    }
    fn setup(notes: Vec<MidiNote>) -> (Velvet, egui::Context) {
        let ctx = egui::Context::default();
        let mut app = Velvet::new(&ctx);
        app.session = Session::new(Project::new("Roll tests"), PathBuf::new());
        app.job = None;
        app.execute(Command::AddMidiTrack {
            name: "Keys".into(),
        });
        let id = app.session.project.tracks[0].id.clone();
        app.execute(Command::SetMidiNotes {
            track_id: id,
            notes,
        });
        app.piano_roll.open = true;
        for _ in 0..3 {
            frame(&mut app, &ctx, vec![], egui::Modifiers::default());
        }
        app.piano_roll.audition = false;
        app.piano_roll.top_key = 76.0;
        app.piano_roll.row_height = 18.0;
        app.piano_roll.px_beat = 72.0;
        app.piano_roll.scroll_beat = 0.0;
        frame(&mut app, &ctx, vec![], egui::Modifiers::default());
        (app, ctx)
    }
    fn pos(app: &Velvet, beat: f64, key: u8) -> Pos2 {
        let r = &app.piano_roll;
        Pos2::new(
            r.grid_rect.left() + ((beat - r.scroll_beat) * r.px_beat as f64) as f32,
            r.grid_rect.top() + (r.top_key - key as f32 + 0.5) * r.row_height,
        )
    }
    fn pointer(pos: Pos2, pressed: bool, m: egui::Modifiers) -> egui::Event {
        egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: m,
        }
    }
    fn drag(app: &mut Velvet, ctx: &egui::Context, a: Pos2, b: Pos2, m: egui::Modifiers) {
        frame(
            app,
            ctx,
            vec![egui::Event::PointerMoved(a), pointer(a, true, m)],
            m,
        );
        frame(app, ctx, vec![egui::Event::PointerMoved(b)], m);
        frame(app, ctx, vec![pointer(b, false, m)], m);
    }
    #[test]
    fn selection_stays_closed_and_shift_tab_toggles() {
        let (mut app, ctx) = setup(vec![]);
        app.piano_roll.open = false;
        app.piano_roll.target.clear();
        frame(&mut app, &ctx, vec![], Default::default());
        assert!(!app.piano_roll.open, "Selecting MIDI must not open the editor");
        let shift = egui::Modifiers { shift: true, ..Default::default() };
        for expected in [true, false, true] {
            frame(&mut app, &ctx, vec![egui::Event::Key {
                key: egui::Key::Tab, physical_key: None, pressed: true,
                repeat: false, modifiers: shift,
            }], shift);
            assert_eq!(app.piano_roll.open, expected);
            frame(&mut app, &ctx, vec![egui::Event::Key {
                key: egui::Key::Tab, physical_key: None, pressed: false,
                repeat: false, modifiers: shift,
            }], shift);
        }
    }
    #[test]
    fn window_fits_after_viewport_shrinks() {
        let (mut app, ctx) = setup(vec![]);
        app.piano_roll.open = true;
        for size in [Vec2::new(1440.0, 900.0), Vec2::new(1000.0, 650.0), Vec2::new(800.0, 500.0)] {
            let screen = Rect::from_min_size(Pos2::ZERO, size);
            for _ in 0..5 {
                let _ = ctx.run(egui::RawInput { screen_rect: Some(screen), ..Default::default() }, |ctx| app.midi_editor(ctx));
            }
            let window = ctx.memory(|m| m.area_rect(egui::Id::new("midi_editor"))).unwrap();
            assert!(screen.expand(1.0).contains_rect(window), "Window {window:?} exceeds {screen:?}");
        }
    }
    #[test]
    fn drawing_moving_resizing_and_undo_update_while_open() {
        let (mut app, ctx) = setup(vec![]);
        app.execute(Command::SetTrackInstrument {
            track_id: app.session.project.tracks[0].id.clone(),
            kind: Some("builtin.dot".into()),
        });
        app.scope_enabled.insert("master".into());
        let a = pos(&app, 1.0, 72);
        let b = pos(&app, 2.0, 74);
        let revision = app.session.revision;
        frame(
            &mut app,
            &ctx,
            vec![
                egui::Event::PointerMoved(a),
                pointer(a, true, egui::Modifiers::default()),
            ],
            egui::Modifiers::default(),
        );
        assert_eq!(
            app.session.project.tracks[0].notes,
            vec![note(72, 1.0, 1.0)]
        );
        assert!(app.piano_roll.open);
        app.edit_time = Instant::now() - Duration::from_millis(50);
        frame(
            &mut app,
            &ctx,
            vec![egui::Event::PointerMoved(b)],
            egui::Modifiers::default(),
        );
        assert_eq!(
            app.session.project.tracks[0].notes,
            vec![note(74, 2.0, 1.0)]
        );
        assert!(app.live_dirty, "Editing must invalidate the playback mix");
        app.update_live_mix();
        let (mixed_revision, mix) = app
            .live_job
            .take()
            .unwrap()
            .recv_timeout(Duration::from_secs(2))
            .unwrap();
        assert_eq!(mixed_revision, app.session.revision);
        assert!(
            mix.unwrap().peak > 0.0,
            "Live playback must include the new note before release"
        );
        frame(
            &mut app,
            &ctx,
            vec![pointer(b, false, egui::Modifiers::default())],
            egui::Modifiers::default(),
        );
        assert!(app.session.revision > revision);
        assert_eq!(
            app.session.project.tracks[0].notes,
            vec![note(74, 2.0, 1.0)]
        );
        let a = pos(&app, 2.5, 74);
        let b = pos(&app, 3.5, 73);
        drag(&mut app, &ctx, a, b, egui::Modifiers::default());
        assert_eq!(
            app.session.project.tracks[0].notes,
            vec![note(73, 3.0, 1.0)]
        );
        let a = pos(&app, 3.97, 73);
        let b = pos(&app, 4.97, 73);
        drag(&mut app, &ctx, a, b, egui::Modifiers::default());
        assert_eq!(app.session.project.tracks[0].notes[0].length_beats, 2.0);
        assert!(app.session.undo());
        assert_eq!(app.session.project.tracks[0].notes[0].length_beats, 1.0);
        assert!(app.session.undo());
        assert_eq!(
            app.session.project.tracks[0].notes,
            vec![note(74, 2.0, 1.0)]
        );
    }
    #[test]
    fn shift_clone_box_selection_delete_and_escape_preserve_originals() {
        let (mut app, ctx) = setup(vec![note(72, 1.0, 1.0)]);
        let shift = egui::Modifiers {
            shift: true,
            ..Default::default()
        };
        let a = pos(&app, 1.5, 72);
        let b = pos(&app, 3.5, 72);
        drag(&mut app, &ctx, a, b, shift);
        assert_eq!(
            app.session.project.tracks[0].notes,
            vec![note(72, 1.0, 1.0), note(72, 3.0, 1.0)]
        );
        let a = pos(&app, 3.5, 72);
        let b = pos(&app, 5.5, 74);
        let before_cancel = app.session.project.clone();
        frame(
            &mut app,
            &ctx,
            vec![egui::Event::PointerMoved(a), pointer(a, true, shift)],
            shift,
        );
        frame(&mut app, &ctx, vec![egui::Event::PointerMoved(b)], shift);
        frame(
            &mut app,
            &ctx,
            vec![egui::Event::Key {
                key: egui::Key::Escape,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: shift,
            }],
            shift,
        );
        frame(&mut app, &ctx, vec![pointer(b, false, shift)], shift);
        assert_eq!(app.session.project.tracks[0].notes.len(), 2);
        assert_eq!(app.session.project, before_cancel);
        assert!(app.session.undo());
        assert_eq!(
            app.session.project.tracks[0].notes,
            vec![note(72, 1.0, 1.0)]
        );
        assert!(app.session.redo());
        assert_eq!(app.session.project, before_cancel);
        assert!(app.piano_roll.open);
        let ctrl = egui::Modifiers {
            ctrl: true,
            ..Default::default()
        };
        let a = pos(&app, 0.5, 73);
        let b = pos(&app, 2.25, 71);
        drag(&mut app, &ctx, a, b, ctrl);
        assert_eq!(app.piano_roll.selected, BTreeSet::from([0]));
        frame(
            &mut app,
            &ctx,
            vec![egui::Event::Key {
                key: egui::Key::Delete,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Default::default(),
            }],
            egui::Modifiers::default(),
        );
        assert_eq!(app.session.project.tracks.len(), 1);
        assert_eq!(
            app.session.project.tracks[0].notes,
            vec![note(72, 3.0, 1.0)]
        );
        assert!(app.session.undo());
        assert_eq!(app.session.project.tracks[0].notes.len(), 2);
        app.piano_roll.open = false;
        app.piano_roll.keyboard_focus = false;
        frame(
            &mut app,
            &ctx,
            vec![egui::Event::Key {
                key: egui::Key::F7,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Default::default(),
            }],
            egui::Modifiers::default(),
        );
        assert!(app.piano_roll.open);
    }
    #[test]
    fn quantize_chop_glue_and_group_bounds_preserve_unselected_notes() {
        let mut r = PianoRoll::default();
        let mut notes = vec![note(60, 0.13, 1.0), note(64, 0.25, 1.0), note(80, 4.0, 1.0)];
        r.selected = BTreeSet::from([0, 1]);
        r.action(&mut notes, Action::Quantize(false));
        assert_eq!(notes[0].start_beats, 0.25);
        assert_eq!(notes[2], note(80, 4.0, 1.0));
        r.action(&mut notes, Action::Chop);
        assert_eq!(notes.len(), 9);
        r.action(&mut notes, Action::Glue);
        assert_eq!(notes.len(), 3);
        let mut notes = vec![note(0, 0.25, 1.0), note(7, 0.5, 1.0)];
        r.selected.clear();
        r.action(&mut notes, Action::Transpose(-12));
        assert_eq!(notes[1].key, 7);
        r.action(&mut notes, Action::Nudge(-10.0));
        assert_eq!(notes[0].start_beats, 0.0);
        assert_eq!(notes[1].start_beats, 0.25);
        r.snap = 0;
        let original = notes.clone();
        r.action(&mut notes, Action::Chop);
        assert_eq!(notes, original);
    }
    #[test]
    fn slice_mute_and_velocity_gestures_are_undoable() {
        let (mut app, ctx) = setup(vec![note(72, 1.0, 2.0), note(74, 1.0, 2.0)]);
        app.piano_roll.tool = Tool::Slice;
        let a = pos(&app, 2.0, 75);
        let b = pos(&app, 2.0, 71);
        drag(&mut app, &ctx, a, b, egui::Modifiers::default());
        assert_eq!(app.session.project.tracks[0].notes.len(), 4);
        assert!(app.session.project.tracks[0]
            .notes
            .iter()
            .all(|n| n.length_beats == 1.0));
        app.piano_roll.tool = Tool::Mute;
        let a = pos(&app, 1.5, 72);
        drag(&mut app, &ctx, a, a, egui::Modifiers::default());
        assert!(app.session.project.tracks[0].notes[0].muted);
        assert_eq!(app.session.project.tracks[0].arranged_midi_notes().len(), 3);
        assert!(app.session.undo());
        assert!(!app.session.project.tracks[0].notes[0].muted);
        frame(&mut app, &ctx, vec![], egui::Modifiers::default());
        app.piano_roll.selected = BTreeSet::from([0]);
        let ctrl = egui::Modifiers {
            ctrl: true,
            ..Default::default()
        };
        let mut keyboard = pos(&app, 1.0, 72);
        keyboard.x = app.piano_roll.grid_rect.left() - 30.0;
        drag(&mut app, &ctx, keyboard, keyboard, ctrl);
        assert_eq!(app.piano_roll.selected, BTreeSet::from([0, 2]));
        app.piano_roll.selected = BTreeSet::from([0]);
        let grid = app.piano_roll.grid_rect;
        let a = Pos2::new(
            pos(&app, 1.0, 72).x,
            grid.bottom() + 5.0 + app.piano_roll.lane_height * 0.7,
        );
        drag(&mut app, &ctx, a, a, egui::Modifiers::default());
        let notes = &app.session.project.tracks[0].notes;
        assert!(notes[0].velocity < 60);
        assert_eq!(notes[1].velocity, 100);
    }
    #[test]
    fn cropped_region_expansion_and_loop_translate_source_time() {
        let (mut app, _) = setup(vec![note(72, 0.0, 1.0), note(74, 3.0, 1.0)]);
        let mut track = app.session.project.tracks[0].clone();
        track.midi_region = Some(MidiRegion {
            start_beats: 8.0,
            offset_beats: 2.0,
            length_beats: 2.0,
        });
        let mut notes = track.notes.clone();
        notes[1].velocity = 50;
        assert_eq!(expanded_region(&track, &notes), track.midi_region);
        notes.push(note(76, 5.0, 2.0));
        let r = expanded_region(&track, &notes).unwrap();
        assert_eq!(r.length_beats, 5.0);
        assert_eq!(r.offset_beats, 2.0);
        app.session.project.tracks[0] = track;
        app.piano_roll.loop_enabled = true;
        app.piano_roll.time_range = Some((2.0, 4.0));
        assert_eq!(
            app.piano_roll.loop_beats(&app.session.project),
            Some((8.0, 10.0))
        );
        let tempo = 60.0 / app.session.project.tempo.bpm;
        assert_eq!(app.loop_range(), Some((8.0 * tempo, 10.0 * tempo)));
    }
}
#[derive(Default)]
struct Outcome {
    notes: Option<Vec<MidiNote>>,
    cancelled: bool,
    seek: Option<f64>,
    play: bool,
    undo: bool,
    redo: bool,
    fit_region: bool,
    imported: bool,
    preview: Option<(u8, u8, u8)>,
}

fn note_name(key: u8) -> String {
    format!("{}{}", NAMES[key as usize % 12], key as i16 / 12 - 1)
}
fn channel_color(channel: u8) -> Color32 {
    let palette = [
        CYAN,
        ROSE,
        ACCENT,
        Color32::from_rgb(183, 158, 245),
        Color32::from_rgb(237, 192, 116),
        Color32::from_rgb(113, 208, 194),
    ];
    palette[(channel.saturating_sub(1) as usize) % palette.len()]
}
fn bounds(notes: &[MidiNote], indices: &BTreeSet<usize>) -> Option<(f64, f64, u8, u8)> {
    let mut start = f64::INFINITY;
    let mut end: f64 = 0.0;
    let mut low = 127;
    let mut high = 0;
    for &i in indices {
        if let Some(n) = notes.get(i) {
            start = start.min(n.start_beats);
            end = end.max(n.start_beats + n.length_beats);
            low = low.min(n.key);
            high = high.max(n.key);
        }
    }
    start.is_finite().then_some((start, end, low, high))
}

impl PianoRoll {
    fn unit(&self) -> f64 {
        SNAPS[self.snap].1
    }
    fn snapped(&self, beat: f64, alt: bool) -> f64 {
        let unit = if alt { 0.0 } else { self.unit() };
        if unit == 0.0 {
            beat.max(0.0)
        } else {
            ((beat / unit).round() * unit).max(0.0)
        }
    }
    fn in_scale(&self, key: u8) -> bool {
        SCALES[self.scale]
            .1
            .contains(&((key + 12 - self.root) % 12))
    }
    fn pitch(&self, key: i16) -> u8 {
        let key = key.clamp(0, 127) as u8;
        if !self.snap_scale || self.in_scale(key) {
            return key;
        }
        (0..=127)
            .filter(|k| self.in_scale(*k))
            .min_by_key(|k| (*k as i16 - key as i16).abs())
            .unwrap_or(key)
    }
    fn scope(&self, notes: &[MidiNote]) -> BTreeSet<usize> {
        if self.selected.is_empty() {
            (0..notes.len()).collect()
        } else {
            self.selected.clone()
        }
    }
    fn fit(&mut self, notes: &[MidiNote], width: f32, height: f32, selection: bool) {
        let scope = if selection {
            self.selected.clone()
        } else {
            (0..notes.len()).collect()
        };
        if let Some((a, b, lo, hi)) = bounds(notes, &scope) {
            self.px_beat = (width / ((b - a + 1.0).max(4.0) as f32)).clamp(16.0, 240.0);
            self.scroll_beat = (a - 0.5).max(0.0);
            self.row_height = (height / (hi - lo + 5) as f32).clamp(10.0, 26.0);
            self.top_key = (hi as f32 + 2.0).min(127.0);
        }
    }
    fn copy(&mut self, notes: &[MidiNote]) {
        self.clipboard = self
            .scope(notes)
            .iter()
            .filter_map(|i| notes.get(*i).cloned())
            .collect();
    }
    fn paste(&mut self, notes: &mut Vec<MidiNote>, at: f64) {
        if self.clipboard.is_empty() || notes.len() + self.clipboard.len() > MAX_NOTES {
            return;
        }
        let first = self
            .clipboard
            .iter()
            .map(|n| n.start_beats)
            .fold(f64::INFINITY, f64::min);
        self.selected = (notes.len()..notes.len() + self.clipboard.len()).collect();
        notes.extend(self.clipboard.iter().cloned().map(|mut n| {
            n.start_beats = at + n.start_beats - first;
            n
        }));
    }
    fn random(&mut self) -> f64 {
        self.seed ^= self.seed << 13;
        self.seed ^= self.seed >> 7;
        self.seed ^= self.seed << 17;
        (self.seed >> 11) as f64 / ((1_u64 << 53) as f64)
    }
    fn action(&mut self, notes: &mut Vec<MidiNote>, action: Action) {
        if self.unit() == 0.0
            && matches!(
                action,
                Action::Quantize(_) | Action::Chop | Action::Arpeggiate
            )
        {
            self.notice = Some((false, "Choose a snap division for this tool".into()));
            return;
        }
        let scope = self.scope(notes);
        let Some((start, end, low, high)) = bounds(notes, &scope) else {
            return;
        };
        let step = self.unit().max(MIN_LENGTH);
        match action {
            Action::Delete => {
                if !self.selected.is_empty() {
                    let mut i = 0;
                    notes.retain(|_| {
                        let keep = !self.selected.contains(&i);
                        i += 1;
                        keep
                    });
                    self.selected.clear();
                }
            }
            Action::Duplicate => {
                if notes.len() + scope.len() > MAX_NOTES {
                    return;
                }
                let interval = self
                    .time_range
                    .map_or_else(|| ((end - start) / step).ceil() * step, |(a, b)| b - a)
                    .max(step);
                let copies: Vec<_> = scope
                    .iter()
                    .map(|i| {
                        let mut n = notes[*i].clone();
                        n.start_beats += interval;
                        n
                    })
                    .collect();
                self.selected = (notes.len()..notes.len() + copies.len()).collect();
                notes.extend(copies);
            }
            Action::Chop => {
                let count: f64 = scope
                    .iter()
                    .map(|i| (notes[*i].length_beats / step).ceil())
                    .sum();
                if count + (notes.len() - scope.len()) as f64 > MAX_NOTES as f64 {
                    return;
                }
                let mut result = vec![];
                let mut selection = BTreeSet::new();
                for (i, n) in notes.iter().enumerate() {
                    if !scope.contains(&i) {
                        result.push(n.clone());
                        continue;
                    }
                    let mut beat = n.start_beats;
                    let stop = beat + n.length_beats;
                    while beat < stop - MIN_LENGTH / 2.0 {
                        let mut part = n.clone();
                        part.start_beats = beat;
                        part.length_beats = (stop - beat).min(step);
                        selection.insert(result.len());
                        result.push(part);
                        beat += step;
                    }
                }
                *notes = result;
                self.selected = selection;
            }
            Action::Glue => {
                let mut chosen: Vec<_> = scope.iter().map(|i| notes[*i].clone()).collect();
                chosen.sort_by(|a, b| {
                    (a.key, a.channel)
                        .cmp(&(b.key, b.channel))
                        .then(a.start_beats.total_cmp(&b.start_beats))
                });
                let mut glued: Vec<MidiNote> = vec![];
                for n in chosen {
                    if let Some(prev) = glued.last_mut() {
                        if prev.key == n.key
                            && prev.channel == n.channel
                            && prev.muted == n.muted
                            && (prev.start_beats + prev.length_beats - n.start_beats).abs()
                                < MIN_LENGTH
                        {
                            prev.length_beats += n.length_beats;
                            continue;
                        }
                    }
                    glued.push(n);
                }
                let mut i = 0;
                notes.retain(|_| {
                    let keep = !scope.contains(&i);
                    i += 1;
                    keep
                });
                self.selected = (notes.len()..notes.len() + glued.len()).collect();
                notes.extend(glued);
            }
            Action::Arpeggiate => {
                // ponytail: one cycle per simultaneous chord; preset-driven multi-octave patterns are backlog.
                let mut order: Vec<_> = scope.iter().copied().collect();
                order.sort_by(|a, b| {
                    notes[*a]
                        .start_beats
                        .total_cmp(&notes[*b].start_beats)
                        .then(notes[*a].key.cmp(&notes[*b].key))
                });
                let mut group_start = -1.0;
                let mut rank = 0;
                for i in order {
                    let n = &mut notes[i];
                    if (n.start_beats - group_start).abs() > MIN_LENGTH {
                        group_start = n.start_beats;
                        rank = 0;
                    }
                    n.start_beats = group_start + rank as f64 * step;
                    n.length_beats = step;
                    rank += 1;
                }
            }
            Action::Flam => {
                if notes.len() + scope.len() > MAX_NOTES {
                    return;
                }
                let copies: Vec<_> = scope
                    .iter()
                    .map(|i| {
                        let mut n = notes[*i].clone();
                        n.start_beats = (n.start_beats - self.strum_time).max(0.0);
                        n.length_beats = self.strum_time.max(MIN_LENGTH);
                        n.velocity = (n.velocity as f32 * 0.65).round().max(1.0) as u8;
                        n
                    })
                    .collect();
                notes.extend(copies);
            }
            _ => {
                let mut order: Vec<_> = scope.iter().copied().collect();
                order.sort_by(|a, b| {
                    notes[*a]
                        .start_beats
                        .total_cmp(&notes[*b].start_beats)
                        .then(notes[*a].key.cmp(&notes[*b].key))
                });
                let original = notes.clone();
                let mut group_start = -1.0;
                let mut rank = 0;
                // Group movement is clamped once so boundary notes preserve spacing and intervals.
                let (time_delta, pitch_delta) = match action {
                    Action::Nudge(d) => (d.max(-start), 0),
                    Action::Transpose(d) => (0.0, d.clamp(-(low as i16), 127 - high as i16)),
                    _ => (0.0, 0),
                };
                for i in order {
                    let n = &mut notes[i];
                    match action {
                        Action::Quantize(lengths) => {
                            let index = (n.start_beats / step).round();
                            let target = index * step
                                + if index as i64 % 2 != 0 {
                                    self.swing * step
                                } else {
                                    0.0
                                };
                            n.start_beats += (target - n.start_beats) * self.quantize_strength;
                            if lengths {
                                let target = (n.length_beats / step).round().max(1.0) * step;
                                n.length_beats +=
                                    (target - n.length_beats) * self.quantize_strength;
                            }
                        }
                        Action::Legato => {
                            if let Some(next) = scope
                                .iter()
                                .map(|j| original[*j].start_beats)
                                .filter(|t| *t > n.start_beats + MIN_LENGTH / 2.0)
                                .min_by(f64::total_cmp)
                            {
                                n.length_beats = next - n.start_beats;
                            }
                        }
                        Action::Transpose(_) => n.key = self.pitch(n.key as i16 + pitch_delta),
                        Action::Nudge(_) => n.start_beats += time_delta,
                        Action::FlipTime => {
                            n.start_beats = start + end - n.start_beats - n.length_beats
                        }
                        Action::FlipPitch => {
                            n.key = self.pitch(low as i16 + high as i16 - n.key as i16)
                        }
                        Action::Staccato => n.length_beats = (n.length_beats * 0.5).max(MIN_LENGTH),
                        Action::Strum => {
                            if (original[i].start_beats - group_start).abs() > MIN_LENGTH {
                                group_start = original[i].start_beats;
                                rank = 0;
                            }
                            n.start_beats += rank as f64 * self.strum_time;
                            rank += 1;
                        }
                        Action::Humanize => {
                            n.start_beats = (n.start_beats
                                + (self.random() * 2.0 - 1.0) * self.humanize_time)
                                .max(0.0);
                            n.velocity = (n.velocity as i16
                                + ((self.random() * 2.0 - 1.0) * self.humanize_velocity as f64)
                                    .round() as i16)
                                .clamp(1, 127) as u8;
                        }
                        Action::ScaleVelocity => {
                            n.velocity = (n.velocity as f64 * self.velocity_scale
                                + self.velocity_offset as f64)
                                .round()
                                .clamp(1.0, 127.0) as u8
                        }
                        Action::Color => n.channel = self.channel,
                        Action::Mute(value) => n.muted = value,
                        _ => {}
                    }
                }
            }
        }
    }
    fn toolbar(
        &mut self,
        ui: &mut egui::Ui,
        notes: &mut Vec<MidiNote>,
        outcome: &mut Outcome,
        playing: bool,
        bpm: f64,
    ) {
        ui.horizontal_wrapped(|ui| {
            if ui.button(if playing { "Pause" } else { "Play" }).clicked() {
                outcome.play = true;
            }
            if ui.button("Undo").clicked() {
                outcome.undo = true;
            }
            if ui.button("Redo").clicked() {
                outcome.redo = true;
            }
            ui.separator();
            for (tool, shortcut) in [
                (Tool::Draw, "P"),
                (Tool::Paint, "B"),
                (Tool::Drum, "N"),
                (Tool::Erase, "D"),
                (Tool::Mute, "T"),
                (Tool::Slice, "C"),
                (Tool::Select, "E"),
                (Tool::Stamp, ""),
                (Tool::Zoom, "Z"),
            ] {
                ui.selectable_value(&mut self.tool, tool, tool.label())
                    .on_hover_text(format!("{} · {}", tool.label(), shortcut));
            }
            ui.menu_button("Edit", |ui| {
                if ui.button("Copy  ·  Ctrl+C").clicked() {
                    self.copy(notes);
                    ui.close_menu();
                }
                if ui.button("Cut  ·  Ctrl+X").clicked() {
                    self.copy(notes);
                    self.selected = self.scope(notes);
                    self.action(notes, Action::Delete);
                    ui.close_menu();
                }
                if ui
                    .add_enabled(
                        !self.clipboard.is_empty(),
                        egui::Button::new("Paste  ·  Ctrl+V"),
                    )
                    .clicked()
                {
                    self.paste(notes, self.time_range.map_or(self.scroll_beat, |(a, _)| a));
                    ui.close_menu();
                }
                for (label, action) in [
                    ("Duplicate · Ctrl+B", Action::Duplicate),
                    ("Delete selected", Action::Delete),
                    ("Mute selected / all", Action::Mute(true)),
                    ("Unmute selected / all", Action::Mute(false)),
                    ("Apply color / MIDI channel", Action::Color),
                ] {
                    if ui.button(label).clicked() {
                        self.action(notes, action);
                        ui.close_menu();
                    }
                }
                if ui.button("Fit arrangement clip to notes").clicked() {
                    outcome.fit_region = true;
                    ui.close_menu();
                }
                ui.separator();
                if ui.button("Clear all notes").clicked() {
                    notes.clear();
                    self.selected.clear();
                    ui.close_menu();
                }
            });
            ui.menu_button("Select", |ui| {
                if ui.button("All · Ctrl+A").clicked() {
                    self.selected = (0..notes.len()).collect();
                    ui.close_menu();
                }
                if ui.button("None · Ctrl+D").clicked() {
                    self.selected.clear();
                    ui.close_menu();
                }
                if ui.button("Invert · Shift+I").clicked() {
                    self.selected = (0..notes.len())
                        .filter(|i| !self.selected.contains(i))
                        .collect();
                    ui.close_menu();
                }
                if ui.button("Muted").clicked() {
                    self.selected = notes
                        .iter()
                        .enumerate()
                        .filter_map(|(i, n)| n.muted.then_some(i))
                        .collect();
                    ui.close_menu();
                }
                if ui.button("Current color / channel").clicked() {
                    self.selected = notes
                        .iter()
                        .enumerate()
                        .filter_map(|(i, n)| (n.channel == self.channel).then_some(i))
                        .collect();
                    ui.close_menu();
                }
                if ui.button("Overlapping notes of same pitch").clicked() {
                    self.selected = notes
                        .iter()
                        .enumerate()
                        .filter_map(|(i, n)| {
                            notes[..i]
                                .iter()
                                .any(|p| {
                                    p.key == n.key
                                        && p.channel == n.channel
                                        && p.start_beats < n.start_beats + n.length_beats
                                        && n.start_beats < p.start_beats + p.length_beats
                                })
                                .then_some(i)
                        })
                        .collect();
                    ui.close_menu();
                }
                if ui.button("Time range around selection").clicked() {
                    if let Some((a, b, _, _)) = bounds(notes, &self.selected) {
                        self.time_range = Some((a, b));
                    }
                    ui.close_menu();
                }
            });
            ui.menu_button("Tools", |ui| {
                for (label, action) in [
                    ("Quantize starts · Shift+Q", Action::Quantize(false)),
                    ("Quantize starts + lengths · Ctrl+Q", Action::Quantize(true)),
                    ("Legato · Ctrl+L", Action::Legato),
                    ("Staccato / half lengths", Action::Staccato),
                    ("Chop to snap · Ctrl+U", Action::Chop),
                    ("Glue touching notes · Ctrl+G", Action::Glue),
                    ("Arpeggiate ascending", Action::Arpeggiate),
                    ("Strum", Action::Strum),
                    ("Flam", Action::Flam),
                    ("Humanize timing + velocity", Action::Humanize),
                    ("Reverse time", Action::FlipTime),
                    ("Flip pitch", Action::FlipPitch),
                    ("Scale velocity", Action::ScaleVelocity),
                    ("Transpose octave up", Action::Transpose(12)),
                    ("Transpose octave down", Action::Transpose(-12)),
                ] {
                    if ui.button(label).clicked() {
                        self.action(notes, action);
                        ui.close_menu();
                    }
                }
                ui.separator();
                ui.label("Applies to selection; no selection = all notes");
                ui.add(
                    egui::Slider::new(&mut self.quantize_strength, 0.0..=1.0)
                        .text("Quantize strength"),
                );
                ui.add(egui::Slider::new(&mut self.swing, 0.0..=0.75).text("Swing"));
                ui.add(
                    egui::Slider::new(&mut self.strum_time, 0.005..=0.25)
                        .text("Strum / flam beats"),
                );
                ui.add(
                    egui::Slider::new(&mut self.humanize_time, 0.0..=0.25)
                        .text("Timing jitter / beats"),
                );
                ui.add(
                    egui::Slider::new(&mut self.humanize_velocity, 0..=50).text("Velocity jitter"),
                );
                ui.add(
                    egui::Slider::new(&mut self.velocity_scale, 0.0..=2.0)
                        .text("Velocity multiply"),
                );
                ui.add(
                    egui::Slider::new(&mut self.velocity_offset, -100..=100)
                        .text("Velocity offset"),
                );
            });
            ui.menu_button("View", |ui| {
                ui.checkbox(&mut self.ghosts, "Ghost notes from other MIDI tracks");
                ui.checkbox(&mut self.note_labels, "Note names");
                ui.checkbox(&mut self.follow, "Follow playhead");
                ui.add(egui::Slider::new(&mut self.row_height, 10.0..=32.0).text("Key height"));
                ui.add(
                    egui::Slider::new(&mut self.lane_height, 48.0..=180.0).text("Velocity height"),
                );
                if ui.button("Fit all notes").clicked() {
                    self.fit(
                        notes,
                        self.grid_rect.width(),
                        self.grid_rect.height(),
                        false,
                    );
                    ui.close_menu();
                }
                if ui
                    .add_enabled(
                        !self.selected.is_empty(),
                        egui::Button::new("Fit selection"),
                    )
                    .clicked()
                {
                    self.fit(notes, self.grid_rect.width(), self.grid_rect.height(), true);
                    ui.close_menu();
                }
            });
            ui.menu_button("Score", |ui| {
                if ui.button("Import MIDI…").clicked() {
                    if let Some(path)=rfd::FileDialog::new().add_filter("MIDI",&["mid","midi"]).pick_file() {
                        match read_score_file(&path).and_then(|bytes|midi::decode(&bytes)) {
                            Ok(imported)=>{*notes=imported;self.selected.clear();outcome.imported=true;self.notice=Some((false,"Imported notes · MIDI tempo maps, CC and pedal events are not imported".into()));},
                            Err(e)=>self.notice=Some((true,format!("MIDI import failed: {e}"))),
                        }
                    }
                    ui.close_menu();
                }
                if ui.button("Export MIDI…").clicked() {
                    if let Some(path)=rfd::FileDialog::new().add_filter("MIDI",&["mid"]).set_file_name("notes.mid").save_file() {
                        let scope:Vec<_>=self.scope(notes).iter().map(|i|notes[*i].clone()).collect();
                        self.notice=Some(match midi::encode(&scope,bpm).and_then(|bytes|velvet_core::atomic_write(&path,&bytes)) {
                            Ok(())=>(false,"MIDI exported · muted notes omitted".into()),Err(e)=>(true,format!("MIDI export failed: {e}")),
                        });
                    }
                    ui.close_menu();
                }
                ui.separator();
                if ui.button("Save Velvet score…").clicked() {
                    if let Some(path) = rfd::FileDialog::new()
                        .add_filter("Velvet score", &["vscore"])
                        .set_file_name("notes.vscore")
                        .save_file()
                    {
                        match serde_json::to_vec_pretty(notes)
                            .map_err(anyhow::Error::from)
                            .and_then(|bytes| velvet_core::atomic_write(&path, &bytes))
                        {
                            Ok(()) => self.notice=Some((false,"Velvet score saved".into())),
                            Err(e) => self.notice=Some((true,format!("Score save failed: {e}"))),
                        }
                    }
                    ui.close_menu();
                }
                if ui.button("Load Velvet score…").clicked() {
                    if let Some(path) = rfd::FileDialog::new()
                        .add_filter("Velvet score", &["vscore"])
                        .pick_file()
                    {
                        // Validation occurs in the shared command layer; failed imports preserve the project.
                        match read_score_file(&path).and_then(|bytes|read_vscore(&bytes)) {
                            Ok(imported)=>{*notes=imported;self.selected.clear();outcome.imported=true;self.notice=Some((false,"Velvet score loaded".into()));},
                            Err(e)=>self.notice=Some((true,format!("Score load failed: {e}"))),
                        }
                    }
                    ui.close_menu();
                }
            });
            ui.menu_button("?", |ui| {
                ui.label("P/B/N/D/T/C/E/Z: tools · Space: transport");
                ui.label("Shift+Tab: open/close piano roll");
                ui.label("Ctrl+drag: box select · Ctrl+Shift+click: add/remove selection");
                ui.label("Shift+drag note: clone · Alt: bypass snap");
                ui.label("Drag either edge: resize · Shift+right edge: stretch selection");
                ui.label("Arrows: nudge / transpose · Ctrl+Up/Down: octave");
                ui.label("Ctrl+wheel: horizontal zoom · Alt+wheel: key zoom");
                ui.label("Wheel: pitch scroll · Shift+wheel: time scroll · Middle drag: pan");
                ui.label("Alt+wheel over note: velocity · Drag velocity lane: paint levels");
                ui.label("Ruler click: seek · Ctrl/Shift+drag ruler: time selection");
                ui.label(
                    "Esc: cancel gesture / close editor · Double-click note: select for inspector",
                );
            });
        });
        ui.horizontal_wrapped(|ui| {
            ui.label(egui::RichText::new("SNAP").small().color(MUTED));
            egui::ComboBox::from_id_salt("roll_snap").width(90.0).selected_text(SNAPS[self.snap].0).show_ui(ui,|ui| { for (i,(label,_)) in SNAPS.iter().enumerate() { ui.selectable_value(&mut self.snap,i,*label); } });
            ui.label("Length"); ui.add(egui::DragValue::new(&mut self.length).speed(0.125).range(MIN_LENGTH..=256.0).suffix(" b"));
            ui.label("Vel"); ui.add(egui::DragValue::new(&mut self.velocity).range(1..=127));
            ui.colored_label(channel_color(self.channel),"Color / ch"); ui.add(egui::DragValue::new(&mut self.channel).range(1..=16));
            egui::ComboBox::from_id_salt("roll_root").width(45.0).selected_text(NAMES[self.root as usize]).show_ui(ui,|ui| { for (i,n) in NAMES.iter().enumerate() { ui.selectable_value(&mut self.root,i as u8,*n); } });
            egui::ComboBox::from_id_salt("roll_scale").width(125.0).selected_text(SCALES[self.scale].0).show_ui(ui,|ui| { for (i,(n,_)) in SCALES.iter().enumerate() { ui.selectable_value(&mut self.scale,i,*n); } });
            ui.checkbox(&mut self.snap_scale,"Snap scale");
            ui.checkbox(&mut self.audition,"Audition").on_hover_text("Dry preview with native Dot while stopped; VST3 live monitoring is pending");
            ui.checkbox(&mut self.loop_enabled,"Loop range").on_hover_text("Ctrl/Shift+drag ruler to set the loop range; plays the arrangement within that range");
            if self.tool==Tool::Stamp {
                egui::ComboBox::from_id_salt("roll_chord").selected_text(CHORDS[self.chord].0).show_ui(ui,|ui| { for (i,(label,_)) in CHORDS.iter().enumerate() { ui.selectable_value(&mut self.chord,i,*label); } });
                ui.checkbox(&mut self.chord_once,"Once");
            }
        });
    }
    fn shortcuts(&mut self, ctx: &egui::Context, notes: &mut Vec<MidiNote>) {
        if !self.keyboard_focus || ctx.wants_keyboard_input() || self.gesture.is_some() {
            return;
        }
        let (m, keys) = ctx.input(|i| (i.modifiers, i.keys_down.clone()));
        let pressed = |k| keys.contains(&k) && ctx.input(|i| i.key_pressed(k));
        if m.ctrl {
            if pressed(egui::Key::A) {
                self.selected = (0..notes.len()).collect();
            }
            if pressed(egui::Key::D) {
                self.selected.clear();
            }
            if pressed(egui::Key::C) {
                self.copy(notes);
            }
            if pressed(egui::Key::X) {
                self.copy(notes);
                self.selected = self.scope(notes);
                self.action(notes, Action::Delete);
            }
            if pressed(egui::Key::V) {
                self.paste(notes, self.time_range.map_or(self.scroll_beat, |(a, _)| a));
            }
            for (k, a) in [
                (egui::Key::B, Action::Duplicate),
                (egui::Key::Q, Action::Quantize(true)),
                (egui::Key::L, Action::Legato),
                (egui::Key::U, Action::Chop),
                (egui::Key::G, Action::Glue),
            ] {
                if pressed(k) {
                    self.action(notes, a);
                }
            }
        } else if !m.alt && !m.shift {
            for (k, t) in [
                (egui::Key::P, Tool::Draw),
                (egui::Key::B, Tool::Paint),
                (egui::Key::N, Tool::Drum),
                (egui::Key::D, Tool::Erase),
                (egui::Key::T, Tool::Mute),
                (egui::Key::C, Tool::Slice),
                (egui::Key::E, Tool::Select),
                (egui::Key::Z, Tool::Zoom),
            ] {
                if pressed(k) {
                    self.tool = t;
                }
            }
        }
        if m.shift && pressed(egui::Key::Q) {
            self.action(notes, Action::Quantize(false));
        }
        if m.shift && pressed(egui::Key::I) {
            self.selected = (0..notes.len())
                .filter(|i| !self.selected.contains(i))
                .collect();
        }
        if pressed(egui::Key::Delete) || pressed(egui::Key::Backspace) {
            self.action(notes, Action::Delete);
        }
        for (k, delta) in [(egui::Key::ArrowUp, 1), (egui::Key::ArrowDown, -1)] {
            if pressed(k) {
                self.action(
                    notes,
                    Action::Transpose(delta * if m.ctrl { 12 } else { 1 }),
                );
            }
        }
        for (k, delta) in [(egui::Key::ArrowLeft, -1.0), (egui::Key::ArrowRight, 1.0)] {
            if pressed(k) {
                self.action(
                    notes,
                    Action::Nudge(
                        delta
                            * if m.alt {
                                1.0 / self.px_beat as f64
                            } else {
                                self.unit().max(MIN_LENGTH)
                            },
                    ),
                );
            }
        }
    }
    fn beat_at(&self, rect: Rect, pos: Pos2) -> f64 {
        (self.scroll_beat + (pos.x - rect.left()) as f64 / self.px_beat as f64).max(0.0)
    }
    fn key_at(&self, rect: Rect, pos: Pos2) -> u8 {
        (self.top_key - (pos.y - rect.top()) / self.row_height)
            .ceil()
            .clamp(0.0, 127.0) as u8
    }
    fn note_rect(&self, grid: Rect, note: &MidiNote) -> Rect {
        Rect::from_min_size(
            Pos2::new(
                grid.left() + ((note.start_beats - self.scroll_beat) * self.px_beat as f64) as f32,
                grid.top() + (self.top_key - note.key as f32) * self.row_height,
            ),
            Vec2::new(
                (note.length_beats * self.px_beat as f64) as f32,
                self.row_height,
            ),
        )
    }
    fn hit(&self, grid: Rect, pos: Pos2, notes: &[MidiNote]) -> Option<usize> {
        notes
            .iter()
            .enumerate()
            .rev()
            .find(|(_, n)| self.note_rect(grid, n).contains(pos))
            .map(|(i, _)| i)
    }
    fn add_note(
        &self,
        notes: &mut Vec<MidiNote>,
        beat: f64,
        key: u8,
        stamp: bool,
    ) -> BTreeSet<usize> {
        let intervals = if stamp { CHORDS[self.chord].1 } else { &[0] };
        let mut added = BTreeSet::new();
        for interval in intervals {
            let pitch = key as i16 + interval;
            if !(0..=127).contains(&pitch) || notes.len() >= MAX_NOTES {
                continue;
            }
            let key = self.pitch(pitch);
            if notes.iter().any(|n| {
                n.key == key
                    && n.channel == self.channel
                    && (n.start_beats - beat).abs() < MIN_LENGTH / 2.0
            }) {
                continue;
            }
            added.insert(notes.len());
            notes.push(MidiNote {
                key,
                velocity: self.velocity,
                start_beats: beat,
                length_beats: if self.tool == Tool::Drum {
                    self.unit().max(MIN_LENGTH)
                } else {
                    self.length
                },
                channel: self.channel,
                muted: false,
            });
        }
        added
    }
    fn begin(
        &mut self,
        kind: GestureKind,
        notes: &[MidiNote],
        pos: Pos2,
        grid: Rect,
        additive: bool,
    ) {
        self.gesture = Some(Gesture {
            kind,
            before: notes.to_vec(),
            working: notes.to_vec(),
            indices: self.selected.clone(),
            visited: BTreeSet::new(),
            origin: pos,
            beat: self.beat_at(grid, pos),
            key: self.key_at(grid, pos),
            last_beat: -1.0,
            last_key: 0,
            scroll: (self.scroll_beat, self.top_key),
            additive,
            mute_value: true,
        });
    }
    fn pointer(
        &mut self,
        ui: &mut egui::Ui,
        grid: Rect,
        ruler: Rect,
        lane: Rect,
        notes: &mut Vec<MidiNote>,
        outcome: &mut Outcome,
    ) {
        let ctx = ui.ctx();
        let (pos, m, primary, secondary, middle, release, down, wheel) = ctx.input(|i| {
            (
                i.pointer.hover_pos(),
                i.modifiers,
                i.pointer.primary_pressed(),
                i.pointer.secondary_pressed(),
                i.pointer.button_pressed(egui::PointerButton::Middle),
                i.pointer.any_released(),
                i.pointer.any_down(),
                i.raw_scroll_delta,
            )
        });
        let Some(pos) = pos else {
            return;
        };
        let over_grid = grid.contains(pos) && ui.rect_contains_pointer(grid);
        if self.gesture.is_none() && (over_grid || lane.contains(pos)) && wheel != Vec2::ZERO {
            if m.alt && over_grid && self.hit(grid, pos, notes).is_some() && !m.ctrl {
                let i = self.hit(grid, pos, notes).unwrap();
                let scope = if self.selected.contains(&i) {
                    self.selected.clone()
                } else {
                    BTreeSet::from([i])
                };
                for i in scope {
                    notes[i].velocity = (notes[i].velocity as i16
                        + if wheel.y > 0.0 { 4 } else { -4 })
                    .clamp(1, 127) as u8;
                }
            } else if m.ctrl && !m.alt {
                let beat = self.beat_at(grid, pos);
                self.px_beat = (self.px_beat * (wheel.y * 0.005).exp()).clamp(12.0, 360.0);
                self.scroll_beat =
                    (beat - (pos.x - grid.left()) as f64 / self.px_beat as f64).max(0.0);
            } else if m.alt {
                let key = self.top_key - (pos.y - grid.top()) / self.row_height;
                self.row_height = (self.row_height * (wheel.y * 0.005).exp()).clamp(8.0, 36.0);
                self.top_key = (key + (pos.y - grid.top()) / self.row_height).clamp(0.0, 127.0);
            } else if m.shift {
                self.scroll_beat =
                    (self.scroll_beat - (wheel.y + wheel.x) as f64 / self.px_beat as f64).max(0.0);
            } else {
                self.top_key = (self.top_key + wheel.y / self.row_height).clamp(0.0, 127.0);
                self.scroll_beat =
                    (self.scroll_beat - wheel.x as f64 / self.px_beat as f64).max(0.0);
            }
            ctx.input_mut(|i| i.smooth_scroll_delta = Vec2::ZERO);
        }
        if self.gesture.is_none() {
            if over_grid && middle {
                self.begin(GestureKind::Pan, notes, pos, grid, false);
            } else if ruler.contains(pos) && ui.rect_contains_pointer(ruler) && primary {
                if m.ctrl || m.shift {
                    self.begin(GestureKind::Range, notes, pos, grid, false);
                } else {
                    outcome.seek = Some(self.snapped(self.beat_at(grid, pos), m.alt));
                }
            } else if lane.contains(pos) && ui.rect_contains_pointer(lane) && primary {
                self.begin(GestureKind::Velocity, notes, pos, grid, false);
            } else if over_grid && (primary || secondary) {
                let hit = self.hit(grid, pos, notes);
                if secondary || self.tool == Tool::Erase {
                    self.begin(GestureKind::Erase, notes, pos, grid, false);
                } else if m.ctrl || self.tool == Tool::Select {
                    if let Some(i) = hit {
                        if m.shift && self.selected.contains(&i) {
                            self.selected.remove(&i);
                        } else {
                            if !m.shift {
                                self.selected.clear();
                            }
                            self.selected.insert(i);
                        }
                    } else {
                        if !m.shift {
                            self.selected.clear();
                        }
                        self.begin(GestureKind::Select, notes, pos, grid, m.shift);
                    }
                } else if self.tool == Tool::Zoom {
                    self.begin(GestureKind::Zoom, notes, pos, grid, false);
                } else if self.tool == Tool::Slice {
                    self.begin(GestureKind::Slice, notes, pos, grid, false);
                } else if self.tool == Tool::Mute || (self.tool == Tool::Drum && hit.is_some()) {
                    self.begin(GestureKind::Mute, notes, pos, grid, false);
                    if let Some(i) = hit {
                        self.gesture.as_mut().unwrap().mute_value = !notes[i].muted;
                    }
                } else if let Some(i) = hit {
                    let rect = self.note_rect(grid, &notes[i]);
                    if !self.selected.contains(&i) {
                        self.selected = BTreeSet::from([i]);
                    }
                    self.length = notes[i].length_beats;
                    self.velocity = notes[i].velocity;
                    let edge = (rect.width() * 0.25).clamp(2.0, 7.0);
                    let kind = if pos.x >= rect.right() - edge {
                        if m.shift && self.selected.len() > 1 {
                            GestureKind::Stretch
                        } else {
                            GestureKind::Right
                        }
                    } else if pos.x <= rect.left() + edge {
                        GestureKind::Left
                    } else {
                        GestureKind::Move
                    };
                    let copy = m.shift && matches!(kind, GestureKind::Move);
                    self.begin(kind, notes, pos, grid, false);
                    self.gesture.as_mut().unwrap().additive = copy;
                    if copy && notes.len() + self.selected.len() <= MAX_NOTES {
                        let copies: Vec<_> =
                            self.selected.iter().map(|i| notes[*i].clone()).collect();
                        let start = notes.len();
                        notes.extend(copies);
                        self.selected = (start..notes.len()).collect();
                        let g = self.gesture.as_mut().unwrap();
                        g.working = notes.clone();
                        g.indices = self.selected.clone();
                    }
                } else {
                    let stamp = self.tool == Tool::Stamp;
                    let kind = if matches!(self.tool, Tool::Paint | Tool::Drum) {
                        GestureKind::Paint
                    } else {
                        GestureKind::Draw
                    };
                    self.begin(kind, notes, pos, grid, false);
                    let beat = self.snapped(self.beat_at(grid, pos), m.alt);
                    let key = self.key_at(grid, pos);
                    self.selected = self.add_note(notes, beat, key, stamp);
                    let g = self.gesture.as_mut().unwrap();
                    g.working = notes.clone();
                    g.indices = self.selected.clone();
                    g.last_beat = beat;
                    g.last_key = key;
                    if stamp && self.chord_once {
                        self.tool = Tool::Draw;
                    }
                }
            }
        }
        if let Some(mut g) = self.gesture.take() {
            let beat = self.beat_at(grid, pos);
            let key = self.key_at(grid, pos);
            match g.kind {
                GestureKind::Move
                | GestureKind::Right
                | GestureKind::Left
                | GestureKind::Stretch => {
                    // Always calculate from the press snapshot, never accumulate snapped deltas.
                    let base = if g.working.len() != g.before.len() {
                        g.working.clone()
                    } else {
                        g.before.clone()
                    };
                    let (a, b, lo, hi) = bounds(&base, &g.indices).unwrap_or((0.0, 1.0, 0, 127));
                    let anchor = g.indices.iter().next().copied().unwrap_or(0);
                    if let Some(anchor_note) = base.get(anchor) {
                        let reference = match g.kind {
                            GestureKind::Right => {
                                anchor_note.start_beats + anchor_note.length_beats
                            }
                            _ => anchor_note.start_beats,
                        };
                        let delta = if (beat - g.beat).abs() < MIN_LENGTH / 100.0 {
                            0.0
                        } else {
                            self.snapped(reference + beat - g.beat, m.alt) - reference
                        };
                        let mut pitch_delta =
                            (key as i16 - g.key as i16).clamp(-(lo as i16), 127 - hi as i16);
                        if m.shift && !g.additive && matches!(g.kind, GestureKind::Move) {
                            pitch_delta = 0;
                        }
                        let shortest = g
                            .indices
                            .iter()
                            .map(|i| base[*i].length_beats)
                            .fold(f64::INFINITY, f64::min);
                        let mut delta = match g.kind {
                            GestureKind::Move => delta.max(-a),
                            GestureKind::Right => delta.max(MIN_LENGTH - shortest),
                            GestureKind::Left => delta.clamp(-a, shortest - MIN_LENGTH),
                            _ => delta,
                        };
                        if m.ctrl && matches!(g.kind, GestureKind::Move) {
                            delta = 0.0;
                        }
                        let mut moved = base.clone();
                        for &i in &g.indices {
                            let n = &mut moved[i];
                            match g.kind {
                                GestureKind::Move => {
                                    n.start_beats += delta;
                                    n.key = self.pitch(n.key as i16 + pitch_delta);
                                }
                                GestureKind::Right => n.length_beats += delta,
                                GestureKind::Left => {
                                    n.start_beats += delta;
                                    n.length_beats -= delta;
                                }
                                GestureKind::Stretch => {
                                    let factor = ((b - a + beat - g.beat) / (b - a))
                                        .max(MIN_LENGTH / shortest);
                                    n.start_beats = a + (n.start_beats - a) * factor;
                                    n.length_beats *= factor;
                                }
                                _ => {}
                            }
                        }
                        *notes = moved;
                        // Clone source must remain immutable during a gesture.
                    }
                }
                GestureKind::Draw => {
                    *notes = g.working.clone();
                    if m.shift {
                        for &i in &g.indices {
                            notes[i].length_beats = (self.snapped(beat, m.alt)
                                - notes[i].start_beats)
                                .max(self.unit().max(MIN_LENGTH));
                        }
                    } else if let Some((a, _, lo, hi)) = bounds(notes, &g.indices) {
                        let delta = (self.snapped(a + beat - g.beat, m.alt) - a).max(-a);
                        let pitch_delta =
                            (key as i16 - g.key as i16).clamp(-(lo as i16), 127 - hi as i16);
                        for &i in &g.indices {
                            notes[i].start_beats += delta;
                            notes[i].key = self.pitch(notes[i].key as i16 + pitch_delta);
                        }
                    }
                }
                GestureKind::Paint => {
                    *notes = g.working.clone();
                    let beat = self.snapped(beat, m.alt);
                    let key = self.pitch(key as i16);
                    if (beat - g.last_beat).abs() > MIN_LENGTH / 2.0 || key != g.last_key {
                        let step = self.unit().max(MIN_LENGTH).max(1.0 / self.px_beat as f64);
                        let count = ((beat - g.last_beat).abs() / step).round() as usize;
                        for j in 1..=count.min(MAX_NOTES) {
                            let t = g.last_beat + (beat - g.last_beat) * j as f64 / count as f64;
                            self.add_note(notes, self.snapped(t, m.alt), key, false);
                        }
                        self.add_note(notes, beat, key, false);
                        g.working = notes.clone();
                        g.last_beat = beat;
                        g.last_key = key;
                    }
                }
                GestureKind::Erase | GestureKind::Mute => {
                    // Indices refer to the original vector until release; retain once at commit.
                    *notes = g.before.clone();
                    for (i, n) in notes.iter_mut().enumerate() {
                        if segment_hits_rect(g.origin, pos, self.note_rect(grid, n)) {
                            g.visited.insert(i);
                        }
                        if g.visited.contains(&i) {
                            n.muted = if matches!(g.kind, GestureKind::Erase) {
                                true
                            } else {
                                g.mute_value
                            };
                        }
                    }
                    g.origin = pos;
                }
                GestureKind::Select => {
                    let selection = Rect::from_two_pos(g.origin, pos);
                    self.selected = if g.additive {
                        g.indices.clone()
                    } else {
                        BTreeSet::new()
                    };
                    for (i, n) in notes.iter().enumerate() {
                        if self.note_rect(grid, n).intersects(selection) {
                            self.selected.insert(i);
                        }
                    }
                }
                GestureKind::Range => {
                    let a = self.snapped(g.beat, m.alt);
                    let b = self.snapped(beat, m.alt);
                    self.time_range = (a != b).then_some((a.min(b), a.max(b)));
                }
                GestureKind::Zoom => {}
                GestureKind::Slice => {}
                GestureKind::Velocity => {
                    *notes = g.working.clone();
                    let a = self.beat_at(grid, g.origin).min(beat);
                    let b = self.beat_at(grid, g.origin).max(beat);
                    let value = (127.0 * (1.0 - (pos.y - lane.top()) / lane.height()))
                        .round()
                        .clamp(1.0, 127.0) as u8;
                    for (i, n) in notes.iter_mut().enumerate() {
                        if (self.selected.is_empty() || self.selected.contains(&i))
                            && n.start_beats >= a - 6.0 / self.px_beat as f64
                            && n.start_beats <= b + 6.0 / self.px_beat as f64
                        {
                            n.velocity = value;
                        }
                    }
                    g.working = notes.clone();
                    g.origin = pos;
                }
                GestureKind::Pan => {
                    self.scroll_beat =
                        (g.scroll.0 - (pos.x - g.origin.x) as f64 / self.px_beat as f64).max(0.0);
                    self.top_key =
                        (g.scroll.1 + (pos.y - g.origin.y) / self.row_height).clamp(0.0, 127.0);
                }
            }
            if release || !down {
                if matches!(g.kind, GestureKind::Range) {
                    if let Some((a, b)) = self.time_range {
                        self.selected = notes
                            .iter()
                            .enumerate()
                            .filter_map(|(i, n)| {
                                (n.start_beats < b && n.start_beats + n.length_beats > a)
                                    .then_some(i)
                            })
                            .collect();
                    }
                }
                if matches!(g.kind, GestureKind::Erase) {
                    let mut i = 0;
                    notes.retain(|_| {
                        let keep = !g.visited.contains(&i);
                        i += 1;
                        keep
                    });
                    self.selected.clear();
                }
                if matches!(g.kind, GestureKind::Slice) {
                    self.slice(notes, grid, g.origin, pos, m.alt);
                }
                if matches!(g.kind, GestureKind::Zoom) {
                    let a = g.beat.min(beat);
                    let b = g.beat.max(beat);
                    if b - a > 0.1 {
                        self.scroll_beat = a;
                        self.px_beat = (grid.width() / (b - a) as f32).clamp(12.0, 360.0);
                    } else {
                        self.fit(notes, grid.width(), grid.height(), false);
                    }
                }
            } else {
                // Keep the press snapshot stable while the project updates live.
                if !matches!(
                    g.kind,
                    GestureKind::Move
                        | GestureKind::Right
                        | GestureKind::Left
                        | GestureKind::Stretch
                        | GestureKind::Draw
                ) {
                    g.working = notes.clone();
                }
                self.gesture = Some(g);
            }
        }
        if over_grid {
            ctx.set_cursor_icon(match self.tool {
                Tool::Erase => egui::CursorIcon::NotAllowed,
                Tool::Select | Tool::Slice | Tool::Zoom => egui::CursorIcon::Crosshair,
                _ => {
                    if let Some(i) = self.hit(grid, pos, notes) {
                        let r = self.note_rect(grid, &notes[i]);
                        if pos.x - r.left() < 7.0 || r.right() - pos.x < 7.0 {
                            egui::CursorIcon::ResizeHorizontal
                        } else {
                            egui::CursorIcon::Grab
                        }
                    } else {
                        egui::CursorIcon::Crosshair
                    }
                }
            });
        }
    }
    fn slice(&mut self, notes: &mut Vec<MidiNote>, grid: Rect, from: Pos2, to: Pos2, alt: bool) {
        let mut parts = vec![];
        let available = MAX_NOTES.saturating_sub(notes.len());
        for n in notes.iter_mut() {
            let rect = self.note_rect(grid, n);
            let y = rect.center().y;
            if (to.y - from.y).abs() <= 1.0 && (from.y < rect.top() || from.y > rect.bottom()) {
                continue;
            }
            if (to.y - from.y).abs() > 1.0 && (y < from.y.min(to.y) || y > from.y.max(to.y)) {
                continue;
            }
            let x = if (to.y - from.y).abs() < 1.0 {
                to.x
            } else {
                from.x + (to.x - from.x) * (y - from.y) / (to.y - from.y)
            };
            let beat = self.snapped(self.beat_at(grid, Pos2::new(x, y)), alt);
            if beat > n.start_beats + MIN_LENGTH
                && beat < n.start_beats + n.length_beats - MIN_LENGTH
                && parts.len() < available
            {
                let mut part = n.clone();
                part.start_beats = beat;
                part.length_beats = n.start_beats + n.length_beats - beat;
                n.length_beats = beat - n.start_beats;
                parts.push(part);
            }
        }
        if notes.len() + parts.len() <= MAX_NOTES {
            notes.extend(parts);
            self.selected.clear();
        }
    }
    fn paint(
        &self,
        ui: &egui::Ui,
        areas: [Rect; 4],
        notes: &[MidiNote],
        ghosts: &[MidiNote],
        play_beat: f64,
        region: Option<&MidiRegion>,
    ) {
        let [grid, ruler, keyboard, lane] = areas;
        let p = ui.painter();
        p.rect_filled(grid, 0.0, BG);
        p.rect_filled(lane, 0.0, BG);
        p.rect_filled(keyboard, 0.0, PANEL);
        p.rect_filled(ruler, 0.0, PANEL);
        let gp = p.with_clip_rect(grid);
        let kp = p.with_clip_rect(keyboard);
        for key in 0..=127_u8 {
            let y = grid.top() + (self.top_key - key as f32) * self.row_height;
            if y + self.row_height < grid.top() || y > grid.bottom() {
                continue;
            }
            let row = Rect::from_min_size(
                Pos2::new(grid.left(), y),
                Vec2::new(grid.width(), self.row_height),
            );
            let black = matches!(key % 12, 1 | 3 | 6 | 8 | 10);
            let root = key % 12 == self.root;
            if black {
                gp.rect_filled(row, 0.0, Color32::from_rgb(10, 12, 15));
            }
            if self.scale != 0 && self.in_scale(key) {
                gp.rect_filled(
                    row,
                    0.0,
                    CYAN.gamma_multiply(if root { 0.075 } else { 0.025 }),
                );
            }
            gp.line_segment(
                [row.left_bottom(), row.right_bottom()],
                Stroke::new(
                    0.5_f32,
                    if key % 12 == 0 {
                        MUTED.gamma_multiply(0.1)
                    } else {
                        LINE.gamma_multiply(0.12)
                    },
                ),
            );
            let kr = Rect::from_min_size(
                Pos2::new(keyboard.left(), y),
                Vec2::new(keyboard.width(), self.row_height),
            );
            kp.rect_filled(
                kr.shrink2(Vec2::new(1.0, 0.5)),
                1.0,
                if black {
                    BG
                } else {
                    Color32::from_rgb(31, 36, 44)
                },
            );
            if black {
                kp.rect_filled(
                    Rect::from_min_size(kr.min, Vec2::new(kr.width() * 0.64, kr.height() - 1.0)),
                    1.0,
                    Color32::from_rgb(7, 9, 12),
                );
            }
            kp.text(
                kr.right_center() - Vec2::new(6.0, 0.0),
                egui::Align2::RIGHT_CENTER,
                note_name(key),
                FontId::monospace(9.0),
                if root { CYAN } else { MUTED },
            );
        }
        let visible = grid.width() as f64 / self.px_beat as f64;
        let step = if self.unit() > 0.0 { self.unit() } else { 0.25 };
        let display_step = step * (8.0 / (step * self.px_beat as f64)).ceil().max(1.0);
        let first = (self.scroll_beat / display_step).floor() as i64;
        let last = ((self.scroll_beat + visible) / display_step).ceil() as i64;
        let rp = p.with_clip_rect(ruler);
        let lp = p.with_clip_rect(lane);
        for index in first..=last {
            let beat = index as f64 * display_step;
            let x = grid.left() + ((beat - self.scroll_beat) * self.px_beat as f64) as f32;
            let bar = (beat / 4.0 - (beat / 4.0).round()).abs() < 0.0001;
            let quarter = (beat - beat.round()).abs() < 0.0001;
            let color = if bar {
                MUTED.gamma_multiply(0.4)
            } else if quarter {
                LINE.gamma_multiply(0.25)
            } else {
                LINE.gamma_multiply(0.09)
            };
            gp.line_segment(
                [Pos2::new(x, grid.top()), Pos2::new(x, grid.bottom())],
                Stroke::new(if bar { 1.0_f32 } else { 0.5_f32 }, color),
            );
            lp.line_segment(
                [Pos2::new(x, lane.top()), Pos2::new(x, lane.bottom())],
                Stroke::new(0.5_f32, color),
            );
            if bar {
                rp.text(
                    Pos2::new(x + 5.0, ruler.center().y),
                    egui::Align2::LEFT_CENTER,
                    format!("{}", (beat / 4.0) as i64 + 1),
                    FontId::monospace(10.0),
                    TEXT,
                );
            }
        }
        if let Some((a, b)) = self.time_range {
            let x = |t| grid.left() + ((t - self.scroll_beat) * self.px_beat as f64) as f32;
            rp.rect_filled(
                Rect::from_min_max(
                    Pos2::new(x(a), ruler.top()),
                    Pos2::new(x(b), ruler.bottom()),
                ),
                1.0,
                CYAN.gamma_multiply(0.2),
            );
            gp.rect_filled(
                Rect::from_min_max(Pos2::new(x(a), grid.top()), Pos2::new(x(b), grid.bottom())),
                0.0,
                CYAN.gamma_multiply(0.025),
            );
        }
        if let Some(r) = region {
            for beat in [r.offset_beats, r.offset_beats + r.length_beats] {
                let x = grid.left() + ((beat - self.scroll_beat) * self.px_beat as f64) as f32;
                gp.line_segment(
                    [Pos2::new(x, grid.top()), Pos2::new(x, grid.bottom())],
                    Stroke::new(1.0_f32, ROSE.gamma_multiply(0.35)),
                );
            }
        }
        for n in ghosts {
            gp.rect_filled(
                self.note_rect(grid, n).shrink2(Vec2::new(0.8, 2.0)),
                2.0,
                MUTED.gamma_multiply(0.13),
            );
        }
        for (i, n) in notes.iter().enumerate() {
            let rect = self.note_rect(grid, n).shrink2(Vec2::new(0.5, 1.5));
            if !rect.intersects(grid) {
                continue;
            }
            let color = channel_color(n.channel);
            let selected = self.selected.contains(&i);
            gp.rect_filled(
                rect,
                2.0,
                color.gamma_multiply(if n.muted {
                    0.055
                } else if selected {
                    0.32
                } else {
                    0.17
                }),
            );
            gp.rect_stroke(
                rect,
                2.0,
                Stroke::new(
                    if selected { 1.3_f32 } else { 0.6_f32 },
                    if selected {
                        TEXT
                    } else {
                        color.gamma_multiply(if n.muted { 0.25 } else { 0.8 })
                    },
                ),
                egui::StrokeKind::Inside,
            );
            if !n.muted {
                gp.line_segment(
                    [
                        rect.left_top() + Vec2::new(2.0, 2.0),
                        Pos2::new(
                            (rect.left() + rect.width() * n.velocity as f32 / 127.0)
                                .max(rect.left() + 2.0),
                            rect.top() + 2.0,
                        ),
                    ],
                    Stroke::new(1.0_f32, color.gamma_multiply(0.5)),
                );
            }
            if self.note_labels && rect.width() > 27.0 {
                gp.text(
                    Pos2::new(rect.left().max(grid.left()) + 5.0, rect.center().y),
                    egui::Align2::LEFT_CENTER,
                    note_name(n.key),
                    FontId::monospace(9.0),
                    if n.muted { MUTED } else { color },
                );
            }
            if n.muted {
                gp.line_segment(
                    [rect.left_center(), rect.right_center()],
                    Stroke::new(0.6_f32, MUTED.gamma_multiply(0.45)),
                );
            }
        }
        for (i, n) in notes.iter().enumerate() {
            let x = grid.left() + ((n.start_beats - self.scroll_beat) * self.px_beat as f64) as f32;
            let y = lane.bottom() - n.velocity as f32 / 127.0 * (lane.height() - 7.0);
            let color = if self.selected.contains(&i) {
                TEXT
            } else {
                channel_color(n.channel).gamma_multiply(if n.muted { 0.18 } else { 0.75 })
            };
            lp.line_segment(
                [Pos2::new(x, lane.bottom() - 1.0), Pos2::new(x, y)],
                Stroke::new(1.4_f32, color),
            );
            lp.circle_filled(Pos2::new(x, y), 2.5, color);
            lp.line_segment(
                [
                    Pos2::new(x, y),
                    Pos2::new(x + (n.length_beats * self.px_beat as f64) as f32, y),
                ],
                Stroke::new(0.5_f32, color.gamma_multiply(0.18)),
            );
        }
        let x = grid.left() + ((play_beat - self.scroll_beat) * self.px_beat as f64) as f32;
        gp.line_segment(
            [Pos2::new(x, grid.top()), Pos2::new(x, grid.bottom())],
            Stroke::new(1.0_f32, ACCENT),
        );
        rp.line_segment(
            [Pos2::new(x, ruler.top()), Pos2::new(x, ruler.bottom())],
            Stroke::new(2.0_f32, ACCENT),
        );
        lp.line_segment(
            [Pos2::new(x, lane.top()), Pos2::new(x, lane.bottom())],
            Stroke::new(1.0_f32, ACCENT.gamma_multiply(0.45)),
        );
        if let Some(g) = &self.gesture {
            if let Some(pos) = ui.ctx().input(|i| i.pointer.hover_pos()) {
                match g.kind {
                    GestureKind::Select | GestureKind::Zoom => {
                        let rect = Rect::from_two_pos(g.origin, pos);
                        gp.rect_filled(rect, 1.0, CYAN.gamma_multiply(0.06));
                        gp.rect_stroke(
                            rect,
                            1.0,
                            Stroke::new(1.0_f32, CYAN),
                            egui::StrokeKind::Inside,
                        );
                    }
                    GestureKind::Slice => {
                        gp.line_segment([g.origin, pos], Stroke::new(1.0_f32, ROSE));
                    }
                    _ => {}
                }
            }
        }
        p.line_segment(
            [grid.left_bottom(), grid.right_bottom()],
            Stroke::new(1.0_f32, LINE),
        );
    }
    fn minimap(&mut self, ui: &mut egui::Ui, notes: &[MidiNote]) {
        let (rect, response) = ui.allocate_exact_size(
            Vec2::new(ui.available_width(), 24.0),
            Sense::click_and_drag(),
        );
        let end = notes
            .iter()
            .map(|n| n.start_beats + n.length_beats)
            .fold(16.0, f64::max)
            .max(self.scroll_beat + self.grid_rect.width() as f64 / self.px_beat as f64);
        let p = ui.painter();
        p.rect_filled(rect, 2.0, BG);
        for n in notes {
            let x = rect.left() + n.start_beats as f32 / end as f32 * rect.width();
            let y = rect.bottom() - 3.0 - n.key as f32 / 127.0 * (rect.height() - 6.0);
            p.line_segment(
                [
                    Pos2::new(x, y),
                    Pos2::new(x + n.length_beats as f32 / end as f32 * rect.width(), y),
                ],
                Stroke::new(1.0_f32, channel_color(n.channel).gamma_multiply(0.5)),
            );
        }
        let x = rect.left() + self.scroll_beat as f32 / end as f32 * rect.width();
        let width = (self.grid_rect.width() / self.px_beat) / end as f32 * rect.width();
        p.rect_stroke(
            Rect::from_min_size(
                Pos2::new(x, rect.top() + 1.0),
                Vec2::new(width, rect.height() - 2.0),
            ),
            2.0,
            Stroke::new(1.0_f32, CYAN.gamma_multiply(0.55)),
            egui::StrokeKind::Inside,
        );
        if response.clicked() || response.dragged() {
            if let Some(pos) = response.interact_pointer_pos() {
                self.scroll_beat = (((pos.x - rect.left()) / rect.width()) as f64 * end
                    - self.grid_rect.width() as f64 / self.px_beat as f64 / 2.0)
                    .max(0.0);
            }
        }
    }
    fn ui(
        &mut self,
        ui: &mut egui::Ui,
        track: &Track,
        ghosts: &[MidiNote],
        play_beat: f64,
        playing: bool,
        bpm: f64,
    ) -> Outcome {
        let mut outcome = Outcome::default();
        let mut notes = self
            .gesture
            .as_ref()
            .map_or_else(|| track.notes.clone(), |g| g.working.clone());
        if ui.ctx().input(|i| i.key_pressed(egui::Key::Escape)) && self.keyboard_focus {
            if let Some(gesture) = self.gesture.take() {
                notes = gesture.before;
                outcome.cancelled = true;
                self.selected.clear();
            } else {
                self.open = false;
                self.keyboard_focus = false;
            }
        }
        self.toolbar(ui, &mut notes, &mut outcome, playing, bpm);
        self.shortcuts(ui.ctx(), &mut notes);
        self.minimap(ui, &notes);
        let size = Vec2::new(
            ui.available_width().max(1.0),
            (ui.available_height() - 58.0).max(1.0),
        );
        let (all, _) = ui.allocate_exact_size(size, Sense::hover());
        let key_width = 62.0;
        let ruler_height = 26.0;
        let lane_height = self.lane_height.min((all.height() - ruler_height - 44.0).max(0.0));
        let grid = Rect::from_min_max(
            all.min + Vec2::new(key_width, ruler_height),
            all.max - Vec2::new(14.0, lane_height + 14.0),
        );
        self.grid_rect = grid;
        let ruler = Rect::from_min_max(
            Pos2::new(grid.left(), all.top()),
            Pos2::new(grid.right(), grid.top()),
        );
        let keyboard = Rect::from_min_max(
            Pos2::new(all.left(), grid.top()),
            Pos2::new(grid.left() - 1.0, grid.bottom()),
        );
        let lane = Rect::from_min_max(
            Pos2::new(grid.left(), grid.bottom() + 5.0),
            Pos2::new(grid.right(), all.bottom() - 14.0),
        );
        ui.interact(grid, ui.id().with("note_grid"), Sense::click_and_drag());
        ui.interact(ruler, ui.id().with("note_ruler"), Sense::click_and_drag());
        ui.interact(lane, ui.id().with("velocity_lane"), Sense::click_and_drag());
        if self.follow && playing && self.gesture.is_none() {
            let visible = grid.width() as f64 / self.px_beat as f64;
            if play_beat < self.scroll_beat || play_beat > self.scroll_beat + visible {
                self.scroll_beat = (play_beat - visible * 0.15).max(0.0);
            }
        }
        self.pointer(ui, grid, ruler, lane, &mut notes, &mut outcome);
        if (self.audition && !playing) || ui.ctx().input(|i| i.modifiers.ctrl) {
            if let Some(pos) = ui.ctx().input(|i| i.pointer.hover_pos()) {
                if ui.ctx().input(|i| i.pointer.primary_pressed())
                    && ui.rect_contains_pointer(keyboard)
                    && keyboard.contains(pos)
                {
                    let key = self.key_at(grid, pos);
                    if ui.ctx().input(|i| i.modifiers.ctrl) {
                        self.selected = notes
                            .iter()
                            .enumerate()
                            .filter_map(|(i, n)| (n.key == key).then_some(i))
                            .collect();
                    } else if self.audition && !playing {
                        outcome.preview = Some((key, self.velocity, self.channel));
                    }
                } else if ui.ctx().input(|i| i.pointer.primary_pressed())
                    && ui.rect_contains_pointer(grid)
                    && grid.contains(pos)
                    && matches!(self.tool, Tool::Draw | Tool::Paint | Tool::Stamp)
                    && self.audition
                    && !playing
                {
                    let key = self.pitch(self.key_at(grid, pos) as i16);
                    outcome.preview = Some((key, self.velocity, self.channel));
                }
            }
        }
        self.paint(
            ui,
            [grid, ruler, keyboard, lane],
            &notes,
            ghosts,
            play_beat,
            track.midi_region.as_ref(),
        );
        let p = ui.painter();
        p.text(
            Pos2::new(all.left() + 5.0, lane.top() + 12.0),
            egui::Align2::LEFT_CENTER,
            "VELOCITY",
            FontId::monospace(8.0),
            MUTED,
        );
        p.text(
            Pos2::new(all.left() + 5.0, lane.top() + 27.0),
            egui::Align2::LEFT_CENTER,
            "1 — 127",
            FontId::monospace(8.0),
            MUTED,
        );
        // Scrollbars retain access to the entire MIDI range and long scores.
        let end = notes
            .iter()
            .map(|n| n.start_beats + n.length_beats)
            .fold(64.0, f64::max)
            + 16.0;
        let horizontal = Rect::from_min_max(
            Pos2::new(grid.left(), all.bottom() - 12.0),
            Pos2::new(grid.right(), all.bottom()),
        );
        ui.scope_builder(egui::UiBuilder::new().max_rect(horizontal), |ui| {
            ui.spacing_mut().slider_width = horizontal.width() - 10.0;
            ui.add(egui::Slider::new(&mut self.scroll_beat, 0.0..=end).show_value(false));
        });
        let vertical = Rect::from_min_max(
            Pos2::new(grid.right() + 2.0, grid.top()),
            Pos2::new(all.right(), grid.bottom()),
        );
        ui.scope_builder(egui::UiBuilder::new().max_rect(vertical), |ui| {
            ui.spacing_mut().slider_width = vertical.height() - 10.0;
            ui.add(
                egui::Slider::new(&mut self.top_key, 0.0..=127.0)
                    .vertical()
                    .show_value(false),
            );
        });
        ui.advance_cursor_after_rect(all);
        ui.horizontal_wrapped(|ui| {
            ui.colored_label(
                CYAN,
                format!("{} selected / {} notes", self.selected.len(), notes.len()),
            );
            if self.selected.len() == 1 && self.gesture.is_none() {
                let i = *self.selected.iter().next().unwrap();
                if let Some(n) = notes.get_mut(i) {
                    ui.label("Key");
                    ui.add(egui::DragValue::new(&mut n.key).range(0..=127));
                    ui.label("Beat");
                    ui.add(
                        egui::DragValue::new(&mut n.start_beats)
                            .range(0.0..=14400.0)
                            .speed(0.01),
                    );
                    ui.label("Length");
                    ui.add(
                        egui::DragValue::new(&mut n.length_beats)
                            .range(MIN_LENGTH..=14400.0)
                            .speed(0.01),
                    );
                    ui.label("Vel");
                    ui.add(egui::DragValue::new(&mut n.velocity).range(1..=127));
                    ui.label("Ch");
                    ui.add(egui::DragValue::new(&mut n.channel).range(1..=16));
                    ui.checkbox(&mut n.muted, "Muted");
                }
            } else {
                ui.label(
                    egui::RichText::new(
                        "Ctrl+drag select · Alt free snap · Shift+drag clone · ? shortcuts",
                    )
                    .small()
                    .color(MUTED),
                );
            }
        });
        if let Some((error, message)) = &self.notice {
            ui.colored_label(
                if *error { ROSE } else { MUTED },
                egui::RichText::new(message).small(),
            );
        }
        if notes != track.notes {
            outcome.notes = Some(notes);
        }
        outcome
    }
    pub(super) fn loop_beats(&self, project: &Project) -> Option<(f64, f64)> {
        if !self.loop_enabled {
            return None;
        }
        let (a, b) = self.time_range?;
        let track = project.track(&self.target).ok()?;
        let offset = track
            .midi_region()
            .map_or(0.0, |r| r.start_beats - r.offset_beats);
        Some(((a + offset).max(0.0), (b + offset).max(0.0)))
    }
}
fn segment_hits_rect(a: Pos2, b: Pos2, rect: Rect) -> bool {
    let mut entry = 0.0_f32;
    let mut exit = 1.0_f32;
    for (start, delta, low, high) in [
        (a.x, b.x - a.x, rect.left(), rect.right()),
        (a.y, b.y - a.y, rect.top(), rect.bottom()),
    ] {
        if delta.abs() < f32::EPSILON {
            if start < low || start > high {
                return false;
            }
        } else {
            let x = (low - start) / delta;
            let y = (high - start) / delta;
            entry = entry.max(x.min(y));
            exit = exit.min(x.max(y));
        }
    }
    entry <= exit
}
fn read_score_file(path: &std::path::Path) -> anyhow::Result<Vec<u8>> {
    use std::io::Read;
    let file = std::fs::File::open(path)?;
    let mut bytes = vec![];
    file.take(16 * 1024 * 1024 + 1).read_to_end(&mut bytes)?;
    anyhow::ensure!(bytes.len() <= 16 * 1024 * 1024, "Score file exceeds 16 MB");
    Ok(bytes)
}
fn read_vscore(bytes: &[u8]) -> anyhow::Result<Vec<MidiNote>> {
    let notes: Vec<MidiNote> = serde_json::from_slice(bytes)?;
    anyhow::ensure!(notes.len() <= MAX_NOTES, "Score exceeds 10,000 notes");
    for n in &notes {
        n.validate()?;
    }
    Ok(notes)
}

// Preserve intentionally cropped notes; expand only for newly added or retimed notes.
fn expanded_region(track: &Track, notes: &[MidiNote]) -> Option<MidiRegion> {
    if notes.is_empty() {
        return None;
    }
    let mut region = track.midi_region.clone()?;
    let mut timings = std::collections::BTreeMap::new();
    for n in &track.notes {
        *timings
            .entry((n.start_beats.to_bits(), n.length_beats.to_bits()))
            .or_insert(0_usize) += 1;
    }
    for n in notes {
        if let Some(count) = timings.get_mut(&(n.start_beats.to_bits(), n.length_beats.to_bits())) {
            if *count > 0 {
                *count -= 1;
                continue;
            }
        }
        let end = region.offset_beats + region.length_beats;
        if n.start_beats < region.offset_beats {
            let delta = region.offset_beats - n.start_beats;
            // At arrangement zero, source time zero remains the lower boundary.
            let shift = delta.min(region.start_beats);
            region.start_beats -= shift;
            region.offset_beats -= shift;
            region.length_beats += shift;
        }
        region.length_beats = region
            .length_beats
            .max(n.start_beats + n.length_beats - region.offset_beats)
            .max(end - region.offset_beats);
    }
    Some(region)
}

impl Velvet {
    pub(super) fn midi_editor(&mut self, ctx: &egui::Context) {
        let edit = ctx.data_mut(|d| d.remove_temp::<String>(egui::Id::new("midi_edit_target")));
        if let Some(target) = edit.as_ref() {
            self.selected_track = Some(target.clone());
        }
        let Some(target) = self.selected_track.clone() else {
            self.piano_roll.keyboard_focus = false;
            return;
        };
        let Ok(track) = self.session.project.track(&target).cloned() else {
            self.piano_roll.keyboard_focus = false;
            return;
        };
        if !matches!(track.kind, TrackKind::Midi) {
            self.piano_roll.keyboard_focus = false;
            return;
        }
        let mut roll = std::mem::take(&mut self.piano_roll);
        if roll.target != target {
            if let Some((before, revision)) = roll.edit_history.take() {
                self.session
                    .group_changes(before, self.session.revision.saturating_sub(revision));
                self.gesture = None;
            }
            roll.target = target.clone();
            roll.keyboard_focus = roll.open;
            roll.selected.clear();
            roll.gesture = None;
            roll.time_range = None;
            roll.loop_enabled = false;
            roll.last_notes = track.notes.clone();
            roll.fit(&track.notes, 900.0, 380.0, false);
            roll.scroll_beat = track.midi_region().map_or(0.0, |r| r.offset_beats);
        }
        if edit.is_some() {
            roll.open = true;
            roll.keyboard_focus = true;
            ctx.move_to_top(egui::LayerId::new(
                egui::Order::Middle,
                egui::Id::new("midi_editor"),
            ));
        }
        if roll.last_notes != track.notes {
            roll.selected.clear();
            roll.gesture = None;
            roll.last_notes = track.notes.clone();
        }
        if !roll.open {
            roll.keyboard_focus = false;
            roll.gesture = None;
            if let Some((before, revision)) = roll.edit_history.take() {
                self.session.group_changes(before, self.session.revision.saturating_sub(revision));
                self.gesture = None;
            }
            self.piano_roll = roll;
            return;
        }
        let ghosts: Vec<_> = if roll.ghosts {
            let current = track
                .midi_region()
                .map_or(0.0, |r| r.offset_beats - r.start_beats);
            self.session
                .project
                .tracks
                .iter()
                .filter(|t| t.id != target)
                .flat_map(|t| t.arranged_midi_notes())
                .map(|mut n| {
                    n.start_beats += current;
                    n
                })
                .filter(|n| n.start_beats + n.length_beats >= 0.0)
                .collect()
        } else {
            vec![]
        };
        let source_offset = track
            .midi_region()
            .map_or(0.0, |r| r.offset_beats - r.start_beats);
        let play_beat =
            self.session.transport.seconds * self.session.project.tempo.bpm / 60.0 + source_offset;
        let mut open = true;
        let mut outcome = Outcome::default();
        let window_bounds = ctx.screen_rect().shrink(8.0);
        let max_size = (window_bounds.size() - Vec2::new(24.0, 48.0)).max(Vec2::new(1.0, 1.0));
        let window = egui::Window::new(format!("PIANO ROLL / {}", track.name))
            .id(egui::Id::new("midi_editor"))
            .default_size(Vec2::new(1100.0, 660.0).min(max_size))
            .min_size(Vec2::new(480.0, 320.0).min(max_size))
            .max_size(max_size)
            .constrain_to(window_bounds)
            .resizable(true)
            .collapsible(false)
            .open(&mut open)
            .show(ctx, |ui| {
                ui.add_enabled_ui(self.job.is_none(), |ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.colored_label(CYAN, "VELVET / NOTES");
                        egui::ComboBox::from_id_salt("roll_track")
                            .selected_text(&track.name)
                            .show_ui(ui, |ui| {
                                for t in self
                                    .session
                                    .project
                                    .tracks
                                    .iter()
                                    .filter(|t| matches!(t.kind, TrackKind::Midi))
                                {
                                    if ui.selectable_label(t.id == target, &t.name).clicked() {
                                        self.selected_track = Some(t.id.clone());
                                        self.selected_clip = t.midi_region().map(|_| t.id.clone());
                                        self.selected_device = None;
                                    }
                                }
                            });
                        ui.label(
                            egui::RichText::new(track.synth.as_ref().map_or_else(
                                || "No instrument · drag one here".into(),
                                |s| s.display_name(),
                            ))
                            .small()
                            .color(MUTED),
                        );
                    });
                    self.instrument_drop(ui, ui.min_rect(), &target);
                    outcome = roll.ui(
                        ui,
                        &track,
                        &ghosts,
                        play_beat,
                        self.session.transport.playing,
                        self.session.project.tempo.bpm,
                    );
                });
            });
        if let Some(window) = window {
            if ctx.input(|i| i.pointer.any_pressed()) {
                if let Some(pos) = ctx.input(|i| i.pointer.hover_pos()) {
                    roll.keyboard_focus = window.response.rect.contains(pos);
                }
            }
        }
        roll.open &= open;
        if !roll.open {
            roll.keyboard_focus = false;
            roll.gesture = None;
        }
        if let Some(notes) = &outcome.notes {
            roll.last_notes = notes.clone();
        }
        self.piano_roll = roll;
        if let Some(notes) = outcome.notes {
            if self.piano_roll.gesture.is_some() && self.piano_roll.edit_history.is_none() {
                self.piano_roll.edit_history =
                    Some((self.session.project.clone(), self.session.revision));
            }
            let region = if outcome.cancelled {
                self.piano_roll
                    .edit_history
                    .as_ref()
                    .and_then(|(project, _)| project.track(&target).ok())
                    .and_then(|t| t.midi_region.clone())
            } else if outcome.imported {
                bounds(&notes, &(0..notes.len()).collect()).map(|(_, end, _, _)| MidiRegion {
                    start_beats: track.midi_region().map_or(0.0, |r| r.start_beats),
                    offset_beats: 0.0,
                    length_beats: end.max(4.0).ceil(),
                })
            } else {
                expanded_region(&track, &notes)
            };
            self.execute(Command::SetMidiScore {
                track_id: target.clone(),
                notes,
                region,
            });
        }
        if self.piano_roll.gesture.is_none() {
            if let Some((before, revision)) = self.piano_roll.edit_history.take() {
                self.session
                    .group_changes(before, self.session.revision.saturating_sub(revision));
                self.gesture = None;
            }
        }
        if outcome.fit_region {
            let t = self.session.project.track(&target).unwrap();
            if let Some((a, b, _, _)) = bounds(&t.notes, &(0..t.notes.len()).collect()) {
                let old = t.midi_region();
                let offset = a.floor();
                self.execute(Command::SetMidiRegion {
                    track_id: target.clone(),
                    region: MidiRegion {
                        start_beats: old
                            .map_or(offset, |r| r.start_beats + (offset - r.offset_beats))
                            .max(0.0),
                        offset_beats: offset,
                        length_beats: (b - offset).max(MIN_LENGTH),
                    },
                });
            }
        }
        if let Some(beat) = outcome.seek {
            self.execute(Command::Seek {
                seconds: (beat - source_offset).max(0.0) * 60.0 / self.session.project.tempo.bpm,
            });
        }
        if outcome.undo {
            self.history(false);
        }
        if outcome.redo {
            self.history(true);
        }
        if outcome.play {
            self.toggle_playback();
        }
        if let Some((key, velocity, channel)) = outcome.preview {
            self.preview_note(&track, key, velocity, channel);
        }
        if let Some(player) = &self.player {
            player.set_loop(self.loop_range());
        }
    }
    fn preview_note(&mut self, track: &Track, key: u8, velocity: u8, channel: u8) {
        if self.note_preview_job.is_some() {
            return;
        }
        if track.synth.as_ref().is_some_and(|s| s.plugin_path().is_some())
            && !self.capture_plugin_state() {
            return;
        }
        let Some(synth) = self.session.project.track(&track.id).ok()
            .and_then(|track| track.synth.clone()) else {
            return;
        };
        let rate = match Player::output_rate() {
            Ok(rate) => rate,
            Err(e) => {
                self.report(Err(e), "");
                return;
            }
        };
        let bpm = self.session.project.tempo.bpm;
        let (tx, rx) = mpsc::channel();
        self.note_preview_job = Some(rx);
        self.note_preview = None;
        self.render(move |renderer| {
            let _ = tx.send(renderer.preview(&synth, key, velocity, channel, bpm, rate));
        });
    }
    pub(super) fn poll_note_preview(&mut self) {
        if let Some(rx) = &self.note_preview_job {
            match rx.try_recv() {
                Ok(mix) => {
                    self.note_preview_job = None;
                    match mix.and_then(|mix| Player::new(Arc::new(mix), 0.0)) {
                        Ok(p) => {
                            p.set_monitor_gain(self.monitor_volume, false);
                            p.play();
                            self.note_preview = Some(p);
                        }
                        Err(e) => self.report(Err(e), ""),
                    }
                }
                Err(mpsc::TryRecvError::Disconnected) => self.note_preview_job = None,
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
        if self.session.transport.playing
            || self.note_preview.as_ref().is_some_and(|p| !p.playing())
        {
            self.note_preview = None;
        }
    }
}
