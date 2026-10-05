use super::*;
#[cfg(test)]
use velvet_core::MidiNote;

pub(super) struct InstrumentDrag {
    pub kind: String,
}

pub(super) fn midi_region(track: &velvet_core::Track) -> Option<(f64, f64)> {
    track
        .midi_region()
        .map(|r| (r.start_beats, r.start_beats + r.length_beats))
}

pub(super) fn dragged_region(
    original: &velvet_core::MidiRegion,
    mode: u8,
    delta: f64,
    snap: bool,
) -> velvet_core::MidiRegion {
    let mut r = original.clone();
    let target = original.start_beats
        + if mode == 2 {
            original.length_beats
        } else {
            0.0
        }
        + delta;
    let target = if snap {
        (target * 4.0).round() / 4.0
    } else {
        target
    };
    match mode {
        0 => r.start_beats = target.max(0.0),
        1 => {
            let delta = (target - original.start_beats).clamp(
                -original.start_beats.min(original.offset_beats),
                original.length_beats - 0.01,
            );
            r.start_beats += delta;
            r.offset_beats += delta;
            r.length_beats -= delta;
        }
        _ => r.length_beats = (target - original.start_beats).max(0.01),
    }
    r
}

impl Velvet {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn midi_clip_ui(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &egui::Context,
        track: &velvet_core::Track,
        timeline: Rect,
        row: Rect,
        timeline_x: f32,
        color: Color32,
        navigating: bool,
    ) {
        let Some(mut region) = track.midi_region() else {
            return;
        };
        let zoom = self.zoom;
        let rect_for = |r: &velvet_core::MidiRegion| {
            Rect::from_min_max(
                Pos2::new(
                    timeline_x + r.start_beats as f32 * zoom,
                    row.top() + 5.0,
                ),
                Pos2::new(
                    timeline_x + (r.start_beats + r.length_beats) as f32 * zoom,
                    row.bottom() - 9.0,
                ),
            )
        };
        let rect = rect_for(&region);
        let id = egui::Id::new(("midi_region", &track.id));
        let response = ui.interact(
            rect.intersect(timeline),
            id,
            if navigating || self.job.is_some() {
                Sense::hover()
            } else {
                Sense::click_and_drag()
            },
        );
        let select = response.clicked() || response.drag_started() || response.double_clicked();
        if response.clicked() {
            if let Some(pos) = response.interact_pointer_pos() { self.mark_grid(((pos.x-timeline_x)/self.zoom) as f64); }
        }
        if select {
            self.selected_track = Some(track.id.clone());
            self.selected_clip = Some(track.id.clone());
            self.selected_device = None;
        }
        if response.hovered() && !navigating {
            let edge = response
                .hover_pos()
                .is_some_and(|p| p.x < rect.left() + 8.0 || p.x > rect.right() - 8.0);
            ctx.set_cursor_icon(if edge {
                egui::CursorIcon::ResizeHorizontal
            } else {
                egui::CursorIcon::Grab
            });
        }
        if response.double_clicked() {
            ctx.data_mut(|d| d.insert_temp(egui::Id::new("midi_edit_target"), track.id.clone()));
        }
        if response.drag_started() {
            let origin = ctx.input(|i| i.pointer.press_origin()).unwrap_or(rect.min);
            let mode: u8 = if origin.x < rect.left() + 8.0 {
                1
            } else if origin.x > rect.right() - 8.0 {
                if ctx.input(|i| i.modifiers.shift) { 3 } else { 2 }
            } else {
                0
            };
            ctx.data_mut(|d| {
                d.insert_temp(id, (region.clone(), mode, origin));
                d.insert_temp(egui::Id::new(("midi_stretch", &track.id)), mode == 3);
            });
        }
        if response.dragged() || response.drag_stopped() {
            if let Some((original, mode, origin)) =
                ctx.data(|d| d.get_temp::<(velvet_core::MidiRegion, u8, Pos2)>(id))
            {
                let delta = (response.interact_pointer_pos().unwrap_or(origin).x - origin.x) as f64
                    / self.zoom as f64;
                region = dragged_region(&original, mode, delta, self.snap);
            }
        }
        let visible = rect_for(&region);
        if response.drag_stopped() {
            ctx.data_mut(|d| d.remove::<(velvet_core::MidiRegion, u8, Pos2)>(id));
            if track.midi_region().as_ref() != Some(&region) {
                let original = track.midi_region().unwrap();
                let stretch = ctx.data(|d| d.get_temp::<bool>(egui::Id::new(("midi_stretch", &track.id)))).unwrap_or(false);
                if stretch {
                    let factor = region.length_beats / original.length_beats;
                    let notes = track.notes.iter().cloned().map(|mut n| {
                        n.start_beats *= factor;
                        n.length_beats *= factor;
                        n
                    }).collect();
                    region.offset_beats *= factor;
                    self.execute(Command::SetMidiScore { track_id: track.id.clone(), notes, region: Some(region.clone()) });
                } else {
                    self.execute(Command::SetMidiRegion { track_id: track.id.clone(), region: region.clone() });
                }
                ctx.data_mut(|d| d.remove::<bool>(egui::Id::new(("midi_stretch", &track.id))));
            }
        }
        response.context_menu(|ui| {
            self.selected_track = Some(track.id.clone());
            self.selected_clip = Some(track.id.clone());
            self.selected_device = None;
            self.arrangement_menu(ui);
            ui.separator();
            if ui.button("Edit MIDI notes").clicked() {
                self.selected_track = Some(track.id.clone());
                self.selected_clip = Some(track.id.clone());
                ctx.data_mut(|d| {
                    d.insert_temp(egui::Id::new("midi_edit_target"), track.id.clone())
                });
                ui.close_menu();
            }
            if ui
                .add_enabled(self.job.is_none(), egui::Button::new("Delete MIDI clip"))
                .clicked()
            {
                self.selected_track = Some(track.id.clone());
                self.selected_clip = Some(track.id.clone());
                self.selected_device = None;
                self.remove_selection();
                ui.close_menu();
            }
        });
        let selected = self.selected_clip.as_deref() == Some(&track.id);
        let p = ui.painter().with_clip_rect(timeline);
        p.rect_filled(
            visible,
            3.0,
            color.gamma_multiply(if selected { 0.24 } else { 0.14 }),
        );
        p.rect_stroke(
            visible,
            3.0,
            Stroke::new(
                if selected { 1.0_f32 } else { 0.5_f32 },
                color.gamma_multiply(if selected { 0.9 } else { 0.35 }),
            ),
            egui::StrokeKind::Inside,
        );
        let p = p.with_clip_rect(visible.shrink(3.0).intersect(timeline));
        p.text(
            visible.min + Vec2::new(7.0, 12.0),
            egui::Align2::LEFT_CENTER,
            format!("{} · MIDI", track.name),
            FontId::monospace(10.0),
            color,
        );
        let low = track.notes.iter().map(|n| n.key).min().unwrap_or(48) as f32;
        let high = track.notes.iter().map(|n| n.key).max().unwrap_or(60) as f32;
        for n in &track.notes {
            let x = timeline_x
                + (n.start_beats - region.offset_beats + region.start_beats) as f32 * self.zoom;
            let y = visible.bottom()
                - 7.0
                - (n.key as f32 - low + 1.0) / (high - low + 2.0)
                    * (visible.height() - 30.0).max(1.0);
            p.rect_filled(
                Rect::from_min_size(
                    Pos2::new(x, y),
                    Vec2::new((n.length_beats as f32 * self.zoom).max(3.0), 3.0),
                ),
                1.0,
                color,
            );
        }
        if selected {
            for x in [visible.left() + 3.0, visible.right() - 3.0] {
                p.line_segment(
                    [
                        Pos2::new(x, visible.center().y - 6.0),
                        Pos2::new(x, visible.center().y + 6.0),
                    ],
                    Stroke::new(1.0_f32, color),
                );
            }
        }
        response.on_hover_text(
            "Double-click to edit notes · Drag to move · Drag edges to crop · Shift+right edge to stretch · Right-click for editing",
        );
    }

    pub(super) fn instrument_drop(&mut self, ui: &mut egui::Ui, rect: Rect, target: &str) {
        if self.job.is_some()
            || !self
                .session
                .project
                .track(target)
                .is_ok_and(|t| matches!(t.kind, velvet_core::TrackKind::Midi))
        {
            return;
        }
        let response = ui.interact(
            rect.intersect(ui.clip_rect()),
            ui.id().with(("instrument_drop", target)),
            Sense::hover(),
        );
        if response.dnd_hover_payload::<InstrumentDrag>().is_some() {
            ui.painter().rect_stroke(
                rect.shrink(1.0),
                3.0,
                Stroke::new(2.0_f32, CYAN),
                egui::StrokeKind::Inside,
            );
        }
        if let Some(payload) = response.dnd_release_payload::<InstrumentDrag>() {
            self.execute(Command::SetTrackInstrument {
                track_id: target.into(),
                kind: Some(payload.kind.clone()),
            });
            self.selected_track = Some(target.into());
        }
    }

    pub(super) fn synth_controls(
        &mut self,
        ui: &mut egui::Ui,
        target: &str,
        synth: &velvet_core::Device,
        compact: bool,
    ) {
        if synth.plugin_path().is_some() {
            self.plugin_controls(ui, target, synth);
            return;
        }
        ui.push_id((&synth.id, compact), |ui| {
            ui.with_layout(egui::Layout::top_down(egui::Align::Center), |ui| {
                ui.add_enabled_ui(self.job.is_none(), |ui| {
                    if !compact {
                        eyebrow(ui, "VELVET INSTRUMENTS / 01");
                        ui.heading("dot");
                    }
                    ui.horizontal(|ui| {
                        eyebrow(ui, "OSCILLATOR MATRIX");
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(
                                egui::RichText::new("C4 / 261.63 Hz · RAW")
                                    .monospace()
                                    .size(9.0)
                                    .color(CYAN),
                            );
                        });
                    });
                    let (rect, _) = ui.allocate_exact_size(
                        Vec2::new(
                            ui.available_width().min(690.0),
                            if compact { 76.0 } else { 110.0 },
                        ),
                        Sense::hover(),
                    );
                    crate::matrix::oscillator_matrix(ui, rect, synth.parameters["wave_type"] as u8);
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new("Oscillator").small().color(MUTED));
                        let waves = ["Sine", "Saw", "Square", "Triangle"];
                        egui::ComboBox::from_id_salt("oscillator")
                            .width(100.0)
                            .selected_text(waves[synth.parameters["wave_type"] as usize])
                            .show_ui(ui, |ui| {
                                for (i, name) in waves.iter().enumerate() {
                                    if ui
                                        .selectable_label(
                                            synth.parameters["wave_type"] == i as f64,
                                            *name,
                                        )
                                        .clicked()
                                    {
                                        self.execute(Command::SetSynthParameter {
                                            track_id: target.into(),
                                            parameter: "wave_type".into(),
                                            value: i as f64,
                                        });
                                    }
                                }
                            });
                    });
                    let parameters = [
                        ("gain_db", "Output", " dB"),
                        ("cutoff_freq_hz", "Cutoff", " Hz"),
                        ("attack_ms", "Attack", " ms"),
                        ("decay_ms", "Decay", " ms"),
                        ("sustain", "Sustain", ""),
                        ("release_ms", "Release", " ms"),
                    ];
                    for row in parameters.chunks(6) {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 4.0;
                            ui.add_space(
                                ((ui.available_width() - row.len() as f32 * 76.0) / 2.0).max(0.0),
                            );
                            for &(key, label, suffix) in row {
                                if let Some(value) = crate::rack::parameter_knob(
                                    ui, synth, key, label, suffix, compact,
                                ) {
                                    self.execute(Command::SetSynthParameter {
                                        track_id: target.into(),
                                        parameter: key.into(),
                                        value,
                                    });
                                }
                            }
                        });
                    }
                    ui.label(
                        egui::RichText::new("Polyphonic · MIDI")
                            .small()
                            .color(MUTED),
                    );
                });
            });
        });
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(33));
    }

    pub(super) fn synth_editor(&mut self, ctx: &egui::Context) {
        let Some((target, instrument_id)) = self.synth_popout.clone() else {
            return;
        };
        let Some(synth) = self
            .session
            .project
            .track(&target)
            .ok()
            .and_then(|t| t.synth.clone())
            .filter(|s| s.id == instrument_id)
        else {
            self.synth_popout = None;
            return;
        };
        let name = self.session.project.track(&target).unwrap().name.clone();
        let mut open = true;
        egui::Window::new(format!("DOT / {name}"))
            .id(egui::Id::new(("dot_synth", &instrument_id)))
            .open(&mut open)
            .default_width(720.0)
            .resizable(false)
            .show(ctx, |ui| self.synth_controls(ui, &target, &synth, false));
        if !open {
            self.synth_popout = None;
        }
    }
}

#[cfg(test)]
mod drag_tests {
    use super::*;
    #[test]
    fn docked_controls_stay_inside_the_instrument_card() {
        let ctx = egui::Context::default();
        let mut app = Velvet::new(&ctx);
        app.session
            .execute(Command::AddMidiTrack {
                name: "MIDI".into(),
            })
            .unwrap();
        let target = app.session.project.tracks.last().unwrap().id.clone();
        app.session
            .execute(Command::SetTrackInstrument {
                track_id: target.clone(),
                kind: Some("builtin.dot".into()),
            })
            .unwrap();
        app.selected_track = Some(target);
        let mut output = None;
        for _ in 0..2 {
            output = Some(ctx.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1440.0, 900.0))),
                    ..Default::default()
                },
                |ctx| app.rack(ctx),
            ));
        }
        let output = output.unwrap();
        let card = output
            .shapes
            .iter()
            .find_map(|s| match &s.shape {
                egui::Shape::Rect(r) if (r.rect.width() - 520.0).abs() < 0.1 => Some(r.rect),
                _ => None,
            })
            .expect("Instrument card");
        for label in [
            "Output",
            "Cutoff",
            "Attack",
            "Decay",
            "Sustain",
            "Release",
            "Polyphonic · MIDI",
        ] {
            let control = output
                .shapes
                .iter()
                .find_map(|s| match &s.shape {
                    egui::Shape::Text(t) if t.galley.job.text == label => {
                        Some((Rect::from_min_size(t.pos, t.galley.size()), s.clip_rect))
                    }
                    _ => None,
                })
                .unwrap_or_else(|| panic!("Missing control: {label}"));
            assert!(
                card.contains_rect(control.0),
                "{label} escaped the instrument card: {:?}",
                control.0
            );
            assert!(control.1.contains_rect(control.0), "{label} is clipped");
        }
    }

    #[test]
    fn popout_keeps_instrument_in_chain_and_closes_when_removed() {
        let ctx = egui::Context::default();
        let mut app = Velvet::new(&ctx);
        app.session
            .execute(Command::AddMidiTrack {
                name: "MIDI".into(),
            })
            .unwrap();
        let target = app.session.project.tracks.last().unwrap().id.clone();
        app.session
            .execute(Command::SetTrackInstrument {
                track_id: target.clone(),
                kind: Some("builtin.dot".into()),
            })
            .unwrap();
        app.selected_track = Some(target.clone());
        let instrument = app
            .session
            .project
            .track(&target)
            .unwrap()
            .synth
            .clone()
            .unwrap();
        assert!(app.synth_popout.is_none());
        app.synth_popout = Some((target.clone(), instrument.id.clone()));
        let _ = ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1440.0, 900.0))),
                ..Default::default()
            },
            |ctx| {
                app.rack(ctx);
                app.synth_editor(ctx);
            },
        );
        assert_eq!(
            app.session.project.track(&target).unwrap().synth.as_ref(),
            Some(&instrument)
        );
        assert!(app.synth_popout.is_some());
        app.selected_track = None;
        let _ = ctx.run(Default::default(), |ctx| app.synth_editor(ctx));
        assert!(
            app.synth_popout.is_some(),
            "Floating editor must keep its original track"
        );
        app.session
            .execute(Command::SetTrackInstrument {
                track_id: target,
                kind: None,
            })
            .unwrap();
        let _ = ctx.run(Default::default(), |ctx| app.synth_editor(ctx));
        assert!(app.synth_popout.is_none());
    }

    #[test]
    fn instrument_drop_assigns_to_midi_and_ignores_audio() {
        let ctx = egui::Context::default();
        let mut app = Velvet::new(&ctx);
        app.session
            .execute(Command::AddTrack {
                name: "Audio".into(),
            })
            .unwrap();
        app.session
            .execute(Command::AddMidiTrack {
                name: "MIDI".into(),
            })
            .unwrap();
        let audio = app.session.project.tracks[0].id.clone();
        let midi = app.session.project.tracks[1].id.clone();
        let point = Pos2::new(60.0, 60.0);
        let rect = Rect::from_min_size(Pos2::new(20.0, 20.0), Vec2::splat(100.0));
        for target in [&audio, &midi] {
            for pressed in [true, false] {
                let input = egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 600.0))),
                    events: vec![
                        egui::Event::PointerMoved(point),
                        egui::Event::PointerButton {
                            pos: point,
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: Default::default(),
                        },
                    ],
                    ..Default::default()
                };
                let _ = ctx.run(input, |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        if !pressed {
                            egui::DragAndDrop::set_payload(
                                ctx,
                                InstrumentDrag {
                                    kind: "builtin.dot".into(),
                                },
                            );
                        }
                        app.instrument_drop(ui, rect, target);
                    });
                });
            }
        }
        assert!(app.session.project.track(&audio).unwrap().synth.is_none());
        assert_eq!(
            app.session
                .project
                .track(&midi)
                .unwrap()
                .synth
                .as_ref()
                .unwrap()
                .kind,
            "builtin.dot"
        );
        assert_eq!(app.selected_track.as_deref(), Some(midi.as_str()));
    }
}

#[cfg(test)]
mod region_tests {
    use super::*;
    #[test]
    fn midi_region_delete_and_undo_preserve_instrument() {
        let ctx = egui::Context::default();
        let mut app = Velvet::new(&ctx);
        app.session = Session::new(Project::new("Regions"), PathBuf::new());
        app.job = None;
        app.execute(Command::AddMidiTrack { name: "Dot".into() });
        let id = app.session.project.tracks[0].id.clone();
        app.execute(Command::SetTrackInstrument {
            track_id: id.clone(),
            kind: Some("builtin.dot".into()),
        });
        let notes = vec![MidiNote {
            key: 60,
            velocity: 100,
            start_beats: 4.25,
            length_beats: 4.0,
            ..MidiNote::default()
        }];
        app.execute(Command::SetMidiNotes {
            track_id: id.clone(),
            notes: notes.clone(),
        });
        assert_eq!(
            midi_region(&app.session.project.tracks[0]),
            Some((4.0, 12.0))
        );
        app.selected_track = Some(id.clone());
        app.selected_clip = Some(id);
        app.remove_selection();
        assert_eq!(app.session.project.tracks.len(), 1);
        assert!(app.session.project.tracks[0].synth.is_some());
        assert_eq!(midi_region(&app.session.project.tracks[0]), None);
        assert!(app.session.undo());
        assert_eq!(app.session.project.tracks[0].notes, notes);
    }
}

#[cfg(test)]
mod midi_edit_tests {
    use super::*;
    fn frame(app: &mut Velvet, ctx: &egui::Context, events: Vec<egui::Event>) {
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 400.0))),
            events,
            ..Default::default()
        };
        let _ = ctx.run(input, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let track = app.session.project.tracks[0].clone();
                let row = Rect::from_min_max(Pos2::new(100.0, 50.0), Pos2::new(700.0, 150.0));
                app.midi_clip_ui(ui, ctx, &track, row, row, 100.0, CYAN, false);
            });
        });
    }
    fn pointer(point: Pos2, pressed: bool) -> egui::Event {
        egui::Event::PointerButton {
            pos: point,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::default(),
        }
    }
    #[test]
    fn pointer_drag_moves_and_crops_the_midi_block() {
        let ctx = egui::Context::default();
        let mut app = Velvet::new(&ctx);
        app.session = Session::new(Project::new("Drag"), PathBuf::new());
        app.job = None;
        app.zoom = 40.0;
        app.snap = true;
        app.execute(Command::AddMidiTrack { name: "Dot".into() });
        let id = app.session.project.tracks[0].id.clone();
        app.execute(Command::SetMidiNotes {
            track_id: id.clone(),
            notes: vec![MidiNote {
                key: 60,
                velocity: 100,
                start_beats: 0.0,
                length_beats: 1.0,
                ..MidiNote::default()
            }],
        });
        frame(&mut app, &ctx, vec![]);
        let start = Pos2::new(180.0, 100.0);
        let end = Pos2::new(260.0, 100.0);
        frame(
            &mut app,
            &ctx,
            vec![egui::Event::PointerMoved(start), pointer(start, true)],
        );
        frame(&mut app, &ctx, vec![egui::Event::PointerMoved(end)]);
        frame(&mut app, &ctx, vec![pointer(end, false)]);
        assert_eq!(
            app.session.project.tracks[0]
                .midi_region()
                .unwrap()
                .start_beats,
            2.0
        );
        assert_eq!(app.selected_clip.as_ref(), Some(&id));
        frame(&mut app, &ctx, vec![]);
        let edge = Pos2::new(337.0, 100.0);
        let crop = Pos2::new(297.0, 100.0);
        frame(
            &mut app,
            &ctx,
            vec![egui::Event::PointerMoved(edge), pointer(edge, true)],
        );
        frame(&mut app, &ctx, vec![egui::Event::PointerMoved(crop)]);
        frame(&mut app, &ctx, vec![pointer(crop, false)]);
        assert_eq!(
            app.session.project.tracks[0]
                .midi_region()
                .unwrap()
                .length_beats,
            3.0
        );
        assert!(app.session.undo());
        assert_eq!(
            app.session.project.tracks[0]
                .midi_region()
                .unwrap()
                .length_beats,
            4.0
        );
    }
    #[test]
    fn move_crop_restore_and_save_follow_region_without_destroying_notes() {
        let mut session = Session::new(Project::new("MIDI edits"), PathBuf::new());
        session
            .execute(Command::AddMidiTrack { name: "Dot".into() })
            .unwrap();
        let id = session.project.tracks[0].id.clone();
        let notes = vec![
            MidiNote {
                key: 60,
                velocity: 100,
                start_beats: 0.0,
                length_beats: 1.0,
                ..MidiNote::default()
            },
            MidiNote {
                key: 64,
                velocity: 100,
                start_beats: 3.0,
                length_beats: 1.0,
                ..MidiNote::default()
            },
        ];
        session
            .execute(Command::SetMidiNotes {
                track_id: id.clone(),
                notes: notes.clone(),
            })
            .unwrap();
        let original = session.project.tracks[0].midi_region().unwrap();
        let moved = dragged_region(&original, 0, 8.13, true);
        assert_eq!(moved.start_beats, 8.25);
        assert_eq!(moved.offset_beats, 0.0);
        session
            .execute(Command::SetMidiRegion {
                track_id: id.clone(),
                region: moved.clone(),
            })
            .unwrap();
        assert_eq!(
            session.project.tracks[0].arranged_midi_notes()[0].start_beats,
            8.25
        );
        let cropped = dragged_region(&moved, 1, 2.0, true);
        session
            .execute(Command::SetMidiRegion {
                track_id: id.clone(),
                region: cropped.clone(),
            })
            .unwrap();
        let audible = session.project.tracks[0].arranged_midi_notes();
        assert_eq!(audible.len(), 1);
        assert_eq!(audible[0].start_beats, 11.25);
        assert_eq!(session.project.tracks[0].notes, notes);
        let json = serde_json::to_string(&session.project).unwrap();
        let restored: Project = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.tracks[0].midi_region(), Some(cropped.clone()));
        assert_eq!(restored.tracks[0].arranged_midi_notes(), audible);
        assert!(session.undo());
        assert_eq!(session.project.tracks[0].arranged_midi_notes().len(), 2);
        let right = dragged_region(&moved, 2, -3.5, true);
        session
            .execute(Command::SetMidiRegion {
                track_id: id.clone(),
                region: right,
            })
            .unwrap();
        let audible = session.project.tracks[0].arranged_midi_notes();
        assert_eq!(audible.len(), 1);
        assert_eq!(audible[0].length_beats, 0.5);
        assert!(session.undo());
        assert_eq!(session.project.tracks[0].notes, notes);
        let invalid = velvet_core::MidiRegion {
            start_beats: f64::NAN,
            ..moved.clone()
        };
        assert!(session
            .execute(Command::SetMidiRegion {
                track_id: id,
                region: invalid
            })
            .is_err());
        assert_eq!(session.project.tracks[0].midi_region(), Some(moved.clone()));
        assert_eq!(dragged_region(&moved, 0, -100.0, false).start_beats, 0.0);
        assert_eq!(dragged_region(&original, 1, -100.0, false), original);
        assert!(dragged_region(&moved, 2, -100.0, false).length_beats >= 0.01);
    }
}
