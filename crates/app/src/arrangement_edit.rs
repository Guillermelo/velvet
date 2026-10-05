use super::*;
use velvet_core::{Clip, MidiNote, MidiRegion, Track, TrackKind};

#[derive(Clone)]
pub(super) enum ClipCopy {
    Audio(Clip),
    Midi(Track),
}
impl ClipCopy {
    fn start(&self) -> f64 {
        match self {
            Self::Audio(c) => c.position.start_beats,
            Self::Midi(t) => t.midi_region().unwrap().start_beats,
        }
    }
    fn length(&self, bpm: f64) -> f64 {
        match self {
            Self::Audio(c) => c.duration_seconds(bpm) * bpm / 60.0,
            Self::Midi(t) => t.midi_region().unwrap().length_beats,
        }
    }
}

// Preserve muted notes too; playback's arranged_midi_notes intentionally omits them.
fn visible_notes(track: &Track) -> Vec<MidiNote> {
    let r = track.midi_region().unwrap();
    track
        .notes
        .iter()
        .filter_map(|n| {
            let start = n.start_beats.max(r.offset_beats);
            let end = (n.start_beats + n.length_beats).min(r.offset_beats + r.length_beats);
            (end > start).then(|| MidiNote {
                start_beats: start - r.offset_beats,
                length_beats: end - start,
                ..n.clone()
            })
        })
        .collect()
}
fn fresh_midi(mut track: Track, start: f64) -> Track {
    let length = track.midi_region().unwrap().length_beats;
    track.notes = visible_notes(&track);
    track.id = velvet_core::id("track");
    if let Some(synth) = &mut track.synth {
        synth.id = velvet_core::id("device");
    }
    for d in &mut track.devices {
        d.id = velvet_core::id("device");
    }
    track.midi_region = Some(MidiRegion {
        start_beats: start,
        offset_beats: 0.0,
        length_beats: length,
    });
    track
}
impl Velvet {
    fn selected_copy(&self) -> Option<ClipCopy> {
        let t = self
            .session
            .project
            .track(self.selected_track.as_ref()?)
            .ok()?;
        let cid = self.selected_clip.as_ref()?;
        if cid == &t.id && t.midi_region().is_some() {
            Some(ClipCopy::Midi(t.clone()))
        } else {
            t.clips
                .iter()
                .find(|c| &c.id == cid)
                .cloned()
                .map(ClipCopy::Audio)
        }
    }
    pub(super) fn arrangement_shortcuts(&mut self, ctx: &egui::Context) {
        let action = ctx.input(|i| {
            if !i.modifiers.command {
                return None;
            }
            [
                (egui::Key::C, "copy"),
                (egui::Key::X, "cut"),
                (egui::Key::V, "paste"),
                (egui::Key::D, "duplicate"),
                (egui::Key::E, "split"),
            ]
            .into_iter()
            .find(|(key, _)| i.key_pressed(*key))
            .map(|(_, a)| a)
        });
        if let Some(action) = action {
            self.edit_arrangement(action);
        }
    }
    pub(super) fn arrangement_menu(&mut self, ui: &mut egui::Ui) {
        let selected = self.selected_copy();
        let midi = matches!(selected, Some(ClipCopy::Midi(_)));
        ui.add_enabled_ui(self.job.is_none(), |ui| {
            for (label, action) in [
                ("Copy · Ctrl+C", "copy"),
                ("Cut · Ctrl+X", "cut"),
                ("Paste at marker · Ctrl+V", "paste"),
                ("Duplicate after clip · Ctrl+D", "duplicate"),
                ("Split at marker · Ctrl+E", "split"),
                ("Delete · Del", "delete"),
                ("Move to marker", "move"),
                ("Trim start to marker", "trim_start"),
                ("Trim end to marker", "trim_end"),
                ("Stretch / resample ×2", "double"),
                ("Stretch / resample ×½", "half"),
                ("Loop · Ctrl+L", "loop"),
            ] {
                let enabled = if action == "paste" {
                    self.clip_clipboard.is_some() && self.selected_track.is_some()
                } else if matches!(action, "split" | "trim_start" | "trim_end") {
                    selected.as_ref().is_some_and(|c| {
                        self.grid_start_beats > c.start()
                            && self.grid_start_beats
                                < c.start() + c.length(self.session.project.tempo.bpm)
                    })
                } else {
                    selected.is_some()
                };
                if ui.add_enabled(enabled, egui::Button::new(label)).clicked() {
                    self.edit_arrangement(action);
                    ui.close_menu();
                }
            }
            if matches!(selected, Some(ClipCopy::Audio(_))) {
                ui.separator();
                for (label, action) in [
                    ("Reverse audio", "audio_reverse"),
                    ("Crop audio to clip", "audio_crop"),
                    ("Normalize audio", "audio_normalize"),
                    ("Fade in · 10 ms", "audio_fade_in"),
                    ("Fade out · 10 ms", "audio_fade_out"),
                ] {
                    if ui.button(label).clicked() {
                        self.edit_arrangement(action);
                        ui.close_menu();
                    }
                }
                ui.label("Audio stretch uses resampling and changes pitch.");
            }
            if midi {
                ui.separator();
                for (label, action) in [
                    ("Reverse notes", "reverse"),
                    ("Quantize notes · 1/16", "quantize"),
                    ("Transpose +1 semitone", "up"),
                    ("Transpose −1 semitone", "down"),
                    ("Transpose +1 octave", "octave_up"),
                    ("Transpose −1 octave", "octave_down"),
                    ("Legato", "legato"),
                    ("Deactivate / activate notes", "mute"),
                    ("Crop MIDI to clip", "crop"),
                ] {
                    if ui.button(label).clicked() {
                        self.edit_arrangement(action);
                        ui.close_menu();
                    }
                }
            }
            ui.separator();
            if ui.button("Undo · Ctrl+Z").clicked() {
                self.history(false);
                ui.close_menu();
            }
            if ui.button("Redo · Ctrl+Shift+Z").clicked() {
                self.history(true);
                ui.close_menu();
            }
        });
    }
    fn bake_audio_edit(&mut self, mut clip: Clip, action: &str) -> anyhow::Result<()> {
        let tid = self
            .selected_track
            .clone()
            .ok_or_else(|| anyhow::anyhow!("Select an audio track"))?;
        let path = self
            .session
            .project
            .source_path(&self.session.root, &clip.source);
        let data = self.cache.get(&path)?;
        let first = (clip.position.offset_seconds * data.sample_rate as f64).round() as usize;
        let count = (clip.position.length_seconds * data.sample_rate as f64).round() as usize;
        anyhow::ensure!(
            count > 0 && count <= 64_000_000,
            "Invalid audio edit length"
        );
        let mut frames = Vec::with_capacity(count);
        frames.extend((0..count).map(|i| {
            data.frames
                .get(first.saturating_add(i))
                .copied()
                .unwrap_or([0.0; 2])
        }));
        match action {
            "audio_reverse" => frames.reverse(),
            "audio_normalize" => {
                let peak = frames.iter().flatten().fold(0.0_f32, |v, s| v.max(s.abs()));
                if peak > 0.0 {
                    for s in frames.iter_mut().flatten() {
                        *s /= peak;
                    }
                }
            }
            "audio_fade_in" | "audio_fade_out" => {
                let fade = (data.sample_rate as usize / 100).max(2).min(count);
                for i in 0..fade {
                    let index = if action == "audio_fade_in" {
                        i
                    } else {
                        count - 1 - i
                    };
                    let gain = i as f32 / (fade - 1).max(1) as f32;
                    for sample in &mut frames[index] {
                        *sample *= gain;
                    }
                }
            }
            _ => {}
        }
        let name = PathBuf::from("media").join(format!("{}.wav", velvet_core::id("edit")));
        let dest = self.session.root.join(&name);
        std::fs::create_dir_all(dest.parent().unwrap())?;
        let mix = Mix {
            sample_rate: data.sample_rate,
            frames: frames.clone(),
            missing: vec![],
            sources: vec![],
            peak: 1.0,
            device_signals: Default::default(),
        };
        velvet_audio::export(&mix, &dest)?;
        self.cache.files.insert(
            dest,
            Arc::new(AudioData {
                sample_rate: data.sample_rate,
                frames,
            }),
        );
        clip.source = Source {
            path: name,
            kind: SourceKind::Project,
        };
        clip.position.offset_seconds = 0.0;
        clip.position.length_seconds = count as f64 / data.sample_rate as f64;
        let mut tracks = self.session.project.tracks.clone();
        let t = tracks
            .iter_mut()
            .find(|t| t.id == tid)
            .ok_or_else(|| anyhow::anyhow!("Track missing"))?;
        let target = t
            .clips
            .iter_mut()
            .find(|c| c.id == clip.id)
            .ok_or_else(|| anyhow::anyhow!("Clip missing"))?;
        *target = clip;
        self.execute(Command::SetArrangementTracks { tracks });
        Ok(())
    }
    fn edit_arrangement(&mut self, action: &str) {
        if self.job.is_some() {
            return;
        }
        if action == "loop" {
            self.toggle_clip_loop();
            return;
        }
        if action == "delete" {
            self.remove_selection();
            return;
        }
        let selected = self.selected_copy();
        if action == "copy" || action == "cut" {
            if let Some(copy) = selected {
                self.clip_clipboard = Some(copy);
                if action == "cut" {
                    self.remove_selection();
                }
            }
            return;
        }
        if action.starts_with("audio_") {
            if let Some(ClipCopy::Audio(c)) = selected {
                if let Err(e) = self.bake_audio_edit(c, action) {
                    self.status = e.to_string();
                    self.error = true;
                }
            }
            return;
        }
        let Some(tid) = self.selected_track.clone() else {
            return;
        };
        let mut tracks = self.session.project.tracks.clone();
        let Some(index) = tracks.iter().position(|t| t.id == tid) else {
            return;
        };
        let bpm = self.session.project.tempo.bpm;
        if action == "paste" || action == "duplicate" {
            let copy = if action == "paste" {
                self.clip_clipboard.clone()
            } else {
                selected
            };
            let Some(copy) = copy else {
                return;
            };
            let start = if action == "paste" {
                self.grid_start_beats
            } else {
                copy.start() + copy.length(bpm)
            };
            match copy {
                ClipCopy::Audio(mut c) => {
                    if !matches!(tracks[index].kind, TrackKind::Audio) {
                        self.status = "Select an audio track to paste audio".into();
                        return;
                    }
                    c.id = velvet_core::id("clip");
                    c.position.start_beats = start;
                    self.selected_clip = Some(c.id.clone());
                    tracks[index].clips.push(c);
                }
                ClipCopy::Midi(t) => {
                    if !matches!(tracks[index].kind, TrackKind::Midi) {
                        self.status = "Select a MIDI track to paste MIDI".into();
                        return;
                    }
                    let mut copy = fresh_midi(t, start);
                    if tracks[index].midi_region().is_none() {
                        tracks[index].notes = copy.notes;
                        tracks[index].midi_region = copy.midi_region;
                        self.selected_clip = Some(tid);
                    } else {
                        // One MIDI region per track: preserve independent clips on adjacent lanes.
                        copy.name = format!("{} · copy", copy.name);
                        self.selected_track = Some(copy.id.clone());
                        self.selected_clip = Some(copy.id.clone());
                        tracks.insert(index + 1, copy);
                    }
                }
            }
        } else {
            let Some(copy) = selected else {
                return;
            };
            let start = copy.start();
            let length = copy.length(bpm);
            let delta = self.grid_start_beats - start;
            if matches!(action, "split" | "trim_start" | "trim_end")
                && !(delta > 0.0 && delta < length)
            {
                return;
            }
            match copy {
                ClipCopy::Audio(mut c) => {
                    let ci = tracks[index]
                        .clips
                        .iter()
                        .position(|v| v.id == c.id)
                        .unwrap();
                    let rate = c.playback_rate(bpm);
                    match action {
                        "move" => c.position.start_beats = self.grid_start_beats,
                        "trim_start" => {
                            let seconds = delta * 60.0 / bpm * rate;
                            c.position.start_beats += delta;
                            c.position.offset_seconds += seconds;
                            c.position.length_seconds -= seconds;
                        }
                        "trim_end" => c.position.length_seconds = delta * 60.0 / bpm * rate,
                        "split" => {
                            let seconds = delta * 60.0 / bpm * rate;
                            let mut right = c.clone();
                            right.id = velvet_core::id("clip");
                            right.position.start_beats += delta;
                            right.position.offset_seconds += seconds;
                            right.position.length_seconds -= seconds;
                            c.position.length_seconds = seconds;
                            tracks[index].clips.push(right);
                        }
                        "double" | "half" => {
                            let factor = if action == "double" { 2.0 } else { 0.5 };
                            c.source_bpm = Some(c.source_bpm.unwrap_or(bpm) * factor);
                        }
                        _ => return,
                    }
                    tracks[index].clips[ci] = c;
                }
                ClipCopy::Midi(t) => {
                    let mut region = t.midi_region().unwrap();
                    let mut notes = t.notes.clone();
                    match action {
                        "move" => region.start_beats = self.grid_start_beats,
                        "trim_start" => {
                            region.start_beats += delta;
                            region.offset_beats += delta;
                            region.length_beats -= delta;
                        }
                        "trim_end" => region.length_beats = delta,
                        "split" => {
                            let mut right = fresh_midi(t.clone(), start + delta);
                            right.name = format!("{} · split", t.name);
                            right.midi_region.as_mut().unwrap().offset_beats = delta;
                            right.midi_region.as_mut().unwrap().length_beats -= delta;
                            region.length_beats = delta;
                            tracks.insert(index + 1, right);
                        }
                        "double" | "half" | "reverse" | "quantize" | "up" | "down"
                        | "octave_up" | "octave_down" | "legato" | "mute" | "crop" => {
                            notes = visible_notes(&t);
                            region.offset_beats = 0.0;
                            let factor = if action == "double" {
                                2.0
                            } else if action == "half" {
                                0.5
                            } else {
                                1.0
                            };
                            let activate = notes.iter().all(|n| n.muted);
                            let starts: Vec<_> = notes.iter().map(|n| n.start_beats).collect();
                            for n in &mut notes {
                                match action {
                                    "double" | "half" => {
                                        n.start_beats *= factor;
                                        n.length_beats *= factor;
                                    }
                                    "reverse" => {
                                        n.start_beats =
                                            (length - n.start_beats - n.length_beats).max(0.0)
                                    }
                                    "quantize" => {
                                        n.start_beats = ((n.start_beats * 4.0).round() / 4.0)
                                            .min((length - n.length_beats).max(0.0))
                                    }
                                    "up" => n.key = n.key.saturating_add(1).min(127),
                                    "down" => n.key = n.key.saturating_sub(1),
                                    "octave_up" => n.key = n.key.saturating_add(12).min(127),
                                    "octave_down" => n.key = n.key.saturating_sub(12),
                                    "mute" => n.muted = !activate,
                                    "legato" => {
                                        n.length_beats = starts
                                            .iter()
                                            .copied()
                                            .filter(|s| *s > n.start_beats)
                                            .fold(length, f64::min)
                                            - n.start_beats
                                    }
                                    _ => {}
                                }
                            }
                            region.length_beats *= factor;
                        }
                        _ => return,
                    }
                    tracks[index].notes = notes;
                    tracks[index].midi_region = Some(region);
                }
            }
        }
        self.execute(Command::SetArrangementTracks { tracks });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn audio_split_is_atomic_and_preserves_source_time_with_tempo_sync() {
        let mut app = Velvet::new(&egui::Context::default());
        app.session = Session::new(Project::new("editing"), PathBuf::new());
        app.job = None;
        app.session
            .execute(Command::AddTrack {
                name: "Audio".into(),
            })
            .unwrap();
        let tid = app.session.project.tracks[0].id.clone();
        app.session
            .execute(Command::ImportAudioClip {
                track_id: tid.clone(),
                source: Source {
                    path: std::env::temp_dir().join("sample.wav"),
                    kind: SourceKind::External,
                },
                position: Position {
                    start_beats: 4.0,
                    offset_seconds: 1.0,
                    length_seconds: 4.0,
                },
            })
            .unwrap();
        let cid = app.session.project.tracks[0].clips[0].id.clone();
        app.session
            .execute(Command::SetClipTempo {
                track_id: tid.clone(),
                clip_id: cid.clone(),
                source_bpm: Some(60.0),
            })
            .unwrap();
        app.selected_track = Some(tid);
        app.selected_clip = Some(cid);
        app.grid_start_beats = 6.0;
        let before = app.session.project.clone();
        let history = app.session.history.len();
        app.edit_arrangement("split");
        let clips = &app.session.project.tracks[0].clips;
        assert_eq!(clips.len(), 2);
        assert_eq!(clips[0].position.length_seconds, 2.0);
        assert_eq!(clips[1].position.offset_seconds, 3.0);
        assert_eq!(clips[1].position.start_beats, 6.0);
        assert_eq!(app.session.history.len(), history + 1);
        assert!(app.session.undo());
        assert_eq!(app.session.project, before);
        assert!(app.session.redo());
        assert_eq!(app.session.project.tracks[0].clips.len(), 2);
    }
    #[test]
    fn midi_duplicate_preserves_instrument_and_split_playback() {
        let mut app = Velvet::new(&egui::Context::default());
        app.session = Session::new(Project::new("MIDI editing"), PathBuf::new());
        app.job = None;
        app.session
            .execute(Command::AddMidiTrack {
                name: "Keys".into(),
            })
            .unwrap();
        let tid = app.session.project.tracks[0].id.clone();
        app.session
            .execute(Command::SetTrackInstrument {
                track_id: tid.clone(),
                kind: Some("builtin.dot".into()),
            })
            .unwrap();
        app.session
            .execute(Command::SetMidiScore {
                track_id: tid.clone(),
                notes: vec![MidiNote {
                    start_beats: 0.0,
                    length_beats: 4.0,
                    ..Default::default()
                }],
                region: Some(MidiRegion {
                    start_beats: 0.0,
                    offset_beats: 0.0,
                    length_beats: 4.0,
                }),
            })
            .unwrap();
        app.selected_track = Some(tid.clone());
        app.selected_clip = Some(tid.clone());
        app.edit_arrangement("duplicate");
        assert_eq!(app.session.project.tracks.len(), 2);
        app.session.project.validate().unwrap();
        assert_eq!(
            app.session.project.tracks[1].arranged_midi_notes()[0].start_beats,
            4.0
        );
        assert!(app.session.undo());
        app.selected_track = Some(tid.clone());
        app.selected_clip = Some(tid);
        app.grid_start_beats = 2.0;
        app.edit_arrangement("split");
        app.session.project.validate().unwrap();
        let left = app.session.project.tracks[0].arranged_midi_notes();
        let right = app.session.project.tracks[1].arranged_midi_notes();
        assert_eq!(left[0].length_beats, 2.0);
        assert_eq!(right[0].start_beats, 2.0);
        assert_eq!(right[0].length_beats, 2.0);
    }
    #[test]
    fn audio_reverse_writes_separate_media_and_undo_restores_original() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("original.wav");
        let mix = Mix {
            sample_rate: 48000,
            frames: vec![[0.1; 2], [0.2; 2], [0.3; 2], [0.4; 2]],
            missing: vec![],
            sources: vec![],
            peak: 0.4,
            device_signals: Default::default(),
        };
        velvet_audio::export(&mix, &source).unwrap();
        let bytes = std::fs::read(&source).unwrap();
        let mut app = Velvet::new(&egui::Context::default());
        app.session = Session::new(Project::new("audio"), dir.path().to_path_buf());
        app.job = None;
        app.session
            .execute(Command::AddTrack {
                name: "Audio".into(),
            })
            .unwrap();
        let tid = app.session.project.tracks[0].id.clone();
        app.session
            .execute(Command::ImportAudioClip {
                track_id: tid.clone(),
                source: Source {
                    path: source.clone(),
                    kind: SourceKind::External,
                },
                position: Position {
                    start_beats: 0.0,
                    offset_seconds: 1.0 / 48000.0,
                    length_seconds: 2.0 / 48000.0,
                },
            })
            .unwrap();
        app.selected_track = Some(tid);
        let clip = app.session.project.tracks[0].clips[0].clone();
        app.bake_audio_edit(clip.clone(), "audio_reverse").unwrap();
        let edited = &app.session.project.tracks[0].clips[0];
        let path = app.session.project.source_path(dir.path(), &edited.source);
        let data = velvet_audio::decode(&path).unwrap();
        assert_eq!(data.frames.len(), 2);
        assert!((data.frames[0][0] - 0.3).abs() < 0.001);
        assert!((data.frames[1][0] - 0.2).abs() < 0.001);
        assert_eq!(std::fs::read(&source).unwrap(), bytes);
        assert!(app.session.undo());
        assert_eq!(app.session.project.tracks[0].clips[0], clip);
    }
    #[test]
    fn copy_preserves_trimmed_muted_notes_and_independent_ids() {
        let mut project = Project::new("edit");
        let mut session = Session::new(project.clone(), std::env::temp_dir());
        session
            .execute(Command::AddMidiTrack {
                name: "MIDI".into(),
            })
            .unwrap();
        project = session.project.clone();
        let t = &mut project.tracks[0];
        t.notes = vec![MidiNote {
            start_beats: 1.0,
            length_beats: 3.0,
            muted: true,
            ..Default::default()
        }];
        t.midi_region = Some(MidiRegion {
            start_beats: 8.0,
            offset_beats: 2.0,
            length_beats: 1.0,
        });
        let copy = fresh_midi(t.clone(), 12.0);
        assert_ne!(copy.id, t.id);
        assert_eq!(copy.notes[0].start_beats, 0.0);
        assert_eq!(copy.notes[0].length_beats, 1.0);
        assert!(copy.notes[0].muted);
        assert_eq!(copy.midi_region.unwrap().start_beats, 12.0);
    }
}
