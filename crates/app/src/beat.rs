use super::*;
use velvet_core::{
    beat::{preset, value, BeatPoint, TIME_PRESETS, VOLUME_PRESETS},
    Device,
};

impl Velvet {
    pub(super) fn beat_editor(
        &mut self,
        ui: &mut egui::Ui,
        target: &str,
        device: &Device,
        body: Rect,
    ) {
        ui.scope_builder(
            egui::UiBuilder::new()
                .max_rect(body)
                .layout(egui::Layout::top_down(egui::Align::Min)),
            |ui| {
                ui.set_clip_rect(body.intersect(ui.clip_rect()));
                let lane_id = ui.id().with("beat_lane");
                let mut time = ui
                    .ctx()
                    .data_mut(|d| d.get_temp::<bool>(lane_id).unwrap_or(true));
                let p = &device.parameters;
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut time, true, egui::RichText::new("TIME").color(CYAN));
                    ui.selectable_value(
                        &mut time,
                        false,
                        egui::RichText::new("VOLUME").color(ROSE),
                    );
                    ui.separator();
                    ui.label(
                        egui::RichText::new(format!(
                            "{}  /  {}",
                            TIME_PRESETS[p["time_slot"] as usize],
                            VOLUME_PRESETS[p["volume_slot"] as usize]
                        ))
                        .small()
                        .color(MUTED),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let mut bypass = p["bypass_enabled"] == 1.0;
                        if ui.checkbox(&mut bypass, "Bypass").changed() {
                            self.beat_parameter(
                                target,
                                device,
                                "bypass_enabled",
                                f64::from(bypass),
                            );
                        }
                    });
                });
                ui.ctx().data_mut(|d| d.insert_temp(lane_id, time));
                let lane = if time { "time" } else { "volume" };
                let slot = p[&format!("{lane}_slot")] as usize;
                let names = if time { TIME_PRESETS } else { VOLUME_PRESETS };
                let color = if time { CYAN } else { ROSE };
                let key = format!("{lane}:{slot}");
                let points = device
                    .beat_envelopes
                    .get(&key)
                    .cloned()
                    .unwrap_or_else(|| preset(time, slot));
                ui.horizontal_top(|ui| {
                    ui.vertical(|ui| {
                        ui.set_width(360.0);
                        ui.spacing_mut().item_spacing = Vec2::new(3.0, 3.0);
                        egui::Grid::new("beat_slots")
                            .spacing(Vec2::splat(3.0))
                            .show(ui, |ui| {
                                for (i, name) in names.iter().enumerate() {
                                    let edited =
                                        device.beat_envelopes.contains_key(&format!("{lane}:{i}"));
                                    let label =
                                        format!("{}{}", name, if edited { " •" } else { "" });
                                    let button = egui::Button::new(
                                        egui::RichText::new(label)
                                            .monospace()
                                            .size(8.5)
                                            .color(if i == slot { color } else { MUTED }),
                                    )
                                    .truncate()
                                    .fill(if i == slot {
                                        color.gamma_multiply(0.12)
                                    } else {
                                        BG
                                    })
                                    .stroke(Stroke::new(
                                        0.5_f32,
                                        if i == slot { color } else { LINE },
                                    ));
                                    if ui
                                        .add_sized([87.0, 23.0], button)
                                        .on_hover_text(format!("Slot {} · {name}", i + 1))
                                        .clicked()
                                    {
                                        self.beat_parameter(
                                            target,
                                            device,
                                            &format!("{lane}_slot"),
                                            i as f64,
                                        );
                                        if p["link_enabled"] == 1.0 {
                                            self.beat_parameter(
                                                target,
                                                device,
                                                if time { "volume_slot" } else { "time_slot" },
                                                i as f64,
                                            );
                                        }
                                    }
                                    if i % 4 == 3 {
                                        ui.end_row();
                                    }
                                }
                            });
                    });
                    ui.vertical(|ui| {
                        ui.set_width((body.width() - 380.0).max(200.0));
                        ui.spacing_mut().item_spacing.x = 4.0;
                        ui.spacing_mut().button_padding.x = 3.0;
                        let snap_id = ui.id().with("snap");
                        ui.horizontal(|ui| {
                            ui.label(
                                egui::RichText::new(format!("{:02} / {}", slot + 1, names[slot]))
                                    .monospace()
                                    .color(color),
                            );
                            if ui
                                .small_button("Reset")
                                .on_hover_text("Restore this factory slot")
                                .clicked()
                            {
                                self.beat_envelope(target, device, lane, slot, None);
                            }
                            if ui
                                .small_button("Save")
                                .on_hover_text("Export this envelope preset")
                                .clicked()
                            {
                                if let Some(path) = rfd::FileDialog::new()
                                    .add_filter("Beat envelope", &["json"])
                                    .set_file_name(format!("{}.json", names[slot]))
                                    .save_file()
                                {
                                    let result = serde_json::to_vec_pretty(&points)
                                        .map_err(anyhow::Error::from)
                                        .and_then(|data| Ok(std::fs::write(path, data)?));
                                    self.report(result, "Beat preset saved");
                                }
                            }
                            if ui
                                .small_button("Load")
                                .on_hover_text("Import a Beat envelope into this slot")
                                .clicked()
                            {
                                if let Some(path) = rfd::FileDialog::new()
                                    .add_filter("Beat envelope", &["json"])
                                    .pick_file()
                                {
                                    let result = (|| -> anyhow::Result<Vec<BeatPoint>> {
                                        anyhow::ensure!(
                                            std::fs::metadata(&path)?.len() <= 65536,
                                            "Preset file exceeds 64 KiB"
                                        );
                                        Ok(serde_json::from_slice(&std::fs::read(path)?)?)
                                    })();
                                    match result {
                                        Ok(points) => self.beat_envelope(
                                            target,
                                            device,
                                            lane,
                                            slot,
                                            Some(points),
                                        ),
                                        Err(error) => self.report(Err(error), ""),
                                    }
                                }
                            }
                            let mut snap = ui
                                .ctx()
                                .data_mut(|d| d.get_temp::<bool>(snap_id).unwrap_or(true));
                            ui.checkbox(&mut snap, "Snap");
                            ui.ctx().data_mut(|d| d.insert_temp(snap_id, snap));
                        });
                        let snap = ui
                            .ctx()
                            .data_mut(|d| d.get_temp::<bool>(snap_id).unwrap_or(true));
                        let graph_height = (body.height() - 170.0).max(60.0);
                        self.beat_graph(
                            ui,
                            target,
                            device,
                            lane,
                            slot,
                            &points,
                            time,
                            color,
                            snap,
                            graph_height,
                        );
                        ui.horizontal(|ui| {
                            ui.label(egui::RichText::new("LOOP").small().color(MUTED));
                            egui::ComboBox::from_id_salt("beat_length")
                                .selected_text(format!("{} beats", p["loop_beats"]))
                                .width(70.0)
                                .show_ui(ui, |ui| {
                                    for beats in [0.25, 0.5, 1.0, 2.0, 4.0, 8.0] {
                                        if ui
                                            .selectable_label(
                                                p["loop_beats"] == beats,
                                                format!("{beats} beats"),
                                            )
                                            .clicked()
                                        {
                                            self.beat_parameter(
                                                target,
                                                device,
                                                "loop_beats",
                                                beats,
                                            );
                                        }
                                    }
                                });
                            for (parameter, label) in
                                [("link_enabled", "Link"), ("hold_enabled", "Hold")]
                            {
                                let mut enabled = p[parameter] == 1.0;
                                if ui
                                    .checkbox(&mut enabled, label)
                                    .on_hover_text(if parameter == "hold_enabled" {
                                        "Capture the first loop and repeat it"
                                    } else {
                                        "Select matching time and volume slots"
                                    })
                                    .changed()
                                {
                                    self.beat_parameter(
                                        target,
                                        device,
                                        parameter,
                                        f64::from(enabled),
                                    );
                                }
                            }
                        });
                        egui::Grid::new("beat_controls")
                            .num_columns(6)
                            .spacing(Vec2::new(6.0, 1.0))
                            .show(ui, |ui| {
                                for (index, (parameter, label)) in [
                                    ("time_mix", "Time"),
                                    ("volume_mix", "Volume"),
                                    ("mix", "Wet"),
                                    ("attack_ms", "Attack"),
                                    ("release_ms", "Release"),
                                    ("smooth_ms", "Smooth"),
                                ]
                                .into_iter()
                                .enumerate()
                                {
                                    ui.label(egui::RichText::new(label).small().color(MUTED));
                                    let mut v = p[parameter];
                                    let (min, max) = device.parameter_range(parameter).unwrap();
                                    if ui
                                        .add(
                                            egui::DragValue::new(&mut v)
                                                .range(min..=max)
                                                .speed(if max <= 1.0 { 0.01 } else { 0.5 })
                                                .suffix(if max <= 1.0 { "" } else { " ms" }),
                                        )
                                        .changed()
                                    {
                                        self.beat_parameter(target, device, parameter, v);
                                    }
                                    if index % 3 == 2 {
                                        ui.end_row();
                                    }
                                }
                            });
                        ui.horizontal(|ui| {
                            for (parameter, label) in
                                [("offset_beats", "Offset"), ("tension", "Tension")]
                            {
                                ui.label(egui::RichText::new(label).small().color(MUTED));
                                let mut v = p[parameter];
                                let (min, max) = device.parameter_range(parameter).unwrap();
                                if ui
                                    .add(egui::DragValue::new(&mut v).range(min..=max).speed(0.01))
                                    .changed()
                                {
                                    self.beat_parameter(target, device, parameter, v);
                                }
                            }
                        });
                    });
                });
            },
        );
    }

    fn beat_parameter(&mut self, target: &str, device: &Device, parameter: &str, value: f64) {
        self.execute(Command::SetDeviceParameter {
            track_id: target.into(),
            device_id: device.id.clone(),
            parameter: parameter.into(),
            value,
        });
    }
    fn beat_envelope(
        &mut self,
        target: &str,
        device: &Device,
        lane: &str,
        slot: usize,
        points: Option<Vec<BeatPoint>>,
    ) {
        self.execute(Command::SetBeatEnvelope {
            track_id: target.into(),
            device_id: device.id.clone(),
            lane: lane.into(),
            slot,
            points,
        });
    }
    #[allow(clippy::too_many_arguments)]
    fn beat_graph(
        &mut self,
        ui: &mut egui::Ui,
        target: &str,
        device: &Device,
        lane: &str,
        slot: usize,
        points: &[BeatPoint],
        time: bool,
        color: Color32,
        snap: bool,
        height: f32,
    ) {
        let (rect, response) = ui.allocate_exact_size(
            Vec2::new(ui.available_width(), height),
            Sense::click_and_drag(),
        );
        let graph = rect.shrink2(Vec2::new(22.0, 12.0));
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 3.0, BG);
        let max = if time { 2.0 } else { 1.0 };
        let screen = |x: f64, y: f64| {
            Pos2::new(
                graph.left() + x as f32 * graph.width(),
                if time {
                    graph.top() + (y / max) as f32 * graph.height()
                } else {
                    graph.bottom() - y as f32 * graph.height()
                },
            )
        };
        for i in 0..=16 {
            let x = graph.left() + graph.width() * i as f32 / 16.0;
            painter.line_segment(
                [Pos2::new(x, graph.top()), Pos2::new(x, graph.bottom())],
                Stroke::new(
                    0.5_f32,
                    if i % 4 == 0 {
                        MUTED.gamma_multiply(0.4)
                    } else {
                        LINE
                    },
                ),
            );
        }
        for i in 0..=8 {
            let y = graph.top() + graph.height() * i as f32 / 8.0;
            painter.line_segment(
                [Pos2::new(graph.left(), y), Pos2::new(graph.right(), y)],
                Stroke::new(0.5_f32, LINE),
            );
        }
        painter.text(
            rect.left_top() + Vec2::new(3.0, 2.0),
            egui::Align2::LEFT_TOP,
            if time { "0" } else { "1" },
            FontId::monospace(9.0),
            MUTED,
        );
        painter.text(
            rect.left_bottom() + Vec2::new(3.0, -2.0),
            egui::Align2::LEFT_BOTTOM,
            &if time {
                format!("−{}", device.parameters["loop_beats"] * 2.0)
            } else {
                "0".into()
            },
            FontId::monospace(9.0),
            MUTED,
        );
        if time {
            painter.line_segment(
                [screen(0.0, 0.0), screen(1.0, 1.0)],
                Stroke::new(0.7_f32, ROSE.gamma_multiply(0.35)),
            );
        }
        let path = (0..=512)
            .map(|i| {
                let x = i as f64 / 512.0;
                screen(x, value(points, x))
            })
            .collect();
        painter.add(egui::Shape::line(path, Stroke::new(1.5_f32, color)));
        for point in points {
            painter.circle_filled(screen(point.x, point.y), 3.0, color);
        }
        let seconds = self
            .player
            .as_ref()
            .map_or(self.session.transport.seconds, |p| p.seconds());
        let phase =
            (seconds * self.session.project.tempo.bpm / 60.0 / device.parameters["loop_beats"])
                .fract();
        let x = screen(phase, 0.0).x;
        painter.line_segment(
            [Pos2::new(x, graph.top()), Pos2::new(x, graph.bottom())],
            Stroke::new(1.0_f32, TEXT.gamma_multiply(0.6)),
        );
        if self.session.transport.playing {
            ui.ctx().request_repaint_after(Duration::from_millis(33));
        }
        response.clone().on_hover_text("Drag a node · Right-click empty space to add · Alt-click a node to delete · Right-click a node for curve · Alt disables snap");
        let nearest = |pos: Pos2| {
            points
                .iter()
                .enumerate()
                .min_by(|(_, a), (_, b)| {
                    screen(a.x, a.y)
                        .distance_sq(pos)
                        .total_cmp(&screen(b.x, b.y).distance_sq(pos))
                })
                .filter(|(_, p)| screen(p.x, p.y).distance(pos) < 9.0)
                .map(|(i, _)| i)
        };
        let drag_id = response.id.with((lane, slot, "node"));
        if response.drag_started() {
            if let Some(pos) = response.interact_pointer_pos() {
                ui.ctx().data_mut(|d| d.insert_temp(drag_id, nearest(pos)));
            }
        }
        let mut edited = points.to_vec();
        let mut changed = false;
        if let Some(pos) = response.interact_pointer_pos() {
            if response.dragged() {
                if let Some(index) = ui
                    .ctx()
                    .data_mut(|d| d.get_temp::<Option<usize>>(drag_id).flatten())
                {
                    let mut x = ((pos.x - graph.left()) / graph.width()).clamp(0.0, 1.0) as f64;
                    let mut y = ((pos.y - graph.top()) / graph.height()).clamp(0.0, 1.0) as f64;
                    if !time {
                        y = 1.0 - y;
                    }
                    y *= max;
                    if snap && !ui.input(|i| i.modifiers.alt) {
                        x = (x * 16.0).round() / 16.0;
                        y = (y * 16.0).round() / 16.0;
                    }
                    let end = edited.len() - 1;
                    x = if index == 0 {
                        0.0
                    } else if index == end {
                        1.0
                    } else {
                        {
                            let a = edited[index - 1].x;
                            let b = edited[index + 1].x;
                            let gap = ((b - a) * 0.01).min(1e-8);
                            x.clamp(a + gap, b - gap)
                        }
                    };
                    edited[index].x = x;
                    edited[index].y = y;
                    changed = true;
                }
            }
            if response.clicked() && ui.input(|i| i.modifiers.alt) {
                if let Some(index) = nearest(pos).filter(|i| *i > 0 && *i + 1 < edited.len()) {
                    edited.remove(index);
                    changed = true;
                }
            }
            if response.secondary_clicked()
                && nearest(pos).is_none()
                && graph.contains(pos)
                && edited.len() < 256
            {
                let mut x = ((pos.x - graph.left()) / graph.width()) as f64;
                let mut y = ((pos.y - graph.top()) / graph.height()) as f64;
                if !time {
                    y = 1.0 - y;
                }
                y *= max;
                if snap {
                    x = (x * 16.0).round() / 16.0;
                    y = (y * 16.0).round() / 16.0;
                }
                if edited.iter().all(|p| (p.x - x).abs() > 1e-8) {
                    edited.push(BeatPoint { x, y, curve: 0 });
                    edited.sort_by(|a, b| a.x.total_cmp(&b.x));
                    changed = true;
                }
            }
        }
        if response.secondary_clicked() {
            let index = response.interact_pointer_pos().and_then(nearest);
            ui.ctx()
                .data_mut(|d| d.insert_temp(drag_id.with("menu"), index));
        }
        response.context_menu(|ui| {
            if let Some(index) = ui
                .ctx()
                .data_mut(|d| d.get_temp::<Option<usize>>(drag_id.with("menu")).flatten())
                .filter(|i| *i < edited.len())
            {
                for (curve, label) in [(0, "Linear"), (1, "Hold / step"), (2, "Smooth")] {
                    if ui
                        .selectable_label(edited[index].curve == curve, label)
                        .clicked()
                    {
                        edited[index].curve = curve;
                        changed = true;
                        ui.close_menu();
                    }
                }
            } else {
                ui.label("Right-click a node to choose its curve");
            }
        });
        if changed && edited != points {
            self.beat_envelope(target, device, lane, slot, Some(edited));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn beat_card_contains_all_slots_and_controls_and_selects_preset() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut app = Velvet::new(&ctx);
        app.session = Session::new(Project::new("Beat UI"), PathBuf::new());
        app.job = None;
        app.execute(Command::AddDevice {
            track_id: "master".into(),
            kind: "builtin.beat".into(),
        });
        let body = Rect::from_min_size(Pos2::new(10.0, 10.0), Vec2::new(834.0, 290.0));
        let render = |app: &mut Velvet, events| {
            ctx.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1000.0, 600.0))),
                    events,
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        let device = app.session.project.master_devices[0].clone();
                        let result = ui
                            .scope_builder(egui::UiBuilder::new().max_rect(body), |ui| {
                                app.beat_editor(ui, "master", &device, body)
                            });
                        assert!(
                            result.response.rect.right() <= body.right() + 1.0
                                && result.response.rect.bottom() <= body.bottom() + 1.0,
                            "Beat overflow: {:?}",
                            result.response.rect
                        );
                    });
                },
            )
        };
        let _ = render(&mut app, vec![]);
        let output = render(&mut app, vec![]);
        let text_pos = |name: &str| {
            output
                .shapes
                .iter()
                .find_map(|s| match &s.shape {
                    egui::Shape::Text(t) if t.galley.text() == name => {
                        Some(t.pos + t.galley.size() * 0.5)
                    }
                    _ => None,
                })
                .unwrap_or_else(|| panic!("Missing control: {name}"))
        };
        for name in TIME_PRESETS {
            text_pos(name);
        }
        for name in [
            "TIME", "VOLUME", "Save", "Load", "Hold", "Link", "Attack", "Release", "Wet",
        ] {
            text_pos(name);
        }
        let pos = text_pos("Half speed");
        let click = |pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        render(&mut app, vec![egui::Event::PointerMoved(pos), click(true)]);
        render(&mut app, vec![click(false)]);
        assert_eq!(
            app.session.project.master_devices[0].parameters["time_slot"],
            1.0
        );
        for slot in 0..36 {
            let device = &mut app.session.project.master_devices[0];
            device.parameters.insert("time_slot".into(), slot as f64);
            device
                .beat_envelopes
                .insert(format!("time:{slot}"), preset(true, slot));
            render(&mut app, vec![]);
        }
        let pos = text_pos("VOLUME");
        let click = |pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        render(&mut app, vec![egui::Event::PointerMoved(pos), click(true)]);
        render(&mut app, vec![click(false)]);
        for slot in 0..36 {
            let device = &mut app.session.project.master_devices[0];
            device.parameters.insert("volume_slot".into(), slot as f64);
            device
                .beat_envelopes
                .insert(format!("volume:{slot}"), preset(false, slot));
            render(&mut app, vec![]);
        }
        assert!(!app.error, "{}", app.status);
    }
}
