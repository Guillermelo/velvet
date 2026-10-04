use super::*;
use velvet_core::{Device, BUILTIN_DEVICES};

impl Velvet {
    pub(super) fn rack(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::bottom("rack")
            .default_height(248.0)
            .height_range(230.0..=400.0)
            .resizable(true)
            .frame(egui::Frame::new().fill(PANEL).inner_margin(6.0))
            .show(ctx, |ui| {
                ui.add_enabled_ui(self.job.is_none(), |ui| {
                    let Some(target) = self.selected_track.clone() else {
                        eyebrow(ui, "DEVICE CHAIN");
                        ui.label(
                            egui::RichText::new("Select a track to shape its sound.").color(MUTED),
                        );
                        return;
                    };
                    let Ok(devices) = self.session.project.devices(&target).map(|d| d.to_vec())
                    else {
                        return;
                    };
                    let name = if target == "master" {
                        "Master"
                    } else {
                        &self.session.project.track(&target).unwrap().name
                    };
                    ui.horizontal(|ui| {
                        eyebrow(ui, "DEVICE CHAIN");
                        ui.label(egui::RichText::new(format!("/ {name}")).color(MUTED));
                    });
                    let height = (ui.available_height() - 14.0).max(180.0);
                    egui::ScrollArea::horizontal()
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            ui.spacing_mut().item_spacing.x = 3.0;
                            ui.horizontal_top(|ui| {
                                for device in &devices {
                                    ui.push_id(&device.id, |ui| {
                                        let width = match device.kind.as_str() {
                                            "builtin.eq8" => 580.0,
                                            "builtin.gain" => 130.0,
                                            _ => 252.0,
                                        };
                                        let (rect, _) = ui.allocate_exact_size(
                                            Vec2::new(width, height),
                                            Sense::hover(),
                                        );
                                        ui.painter().rect_filled(rect, 2.0, BG);
                                        ui.painter().rect_stroke(
                                            rect,
                                            2.0,
                                            Stroke::new(1.0_f32, LINE),
                                            egui::StrokeKind::Inside,
                                        );
                                        let title =
                                            Rect::from_min_size(rect.min, Vec2::new(width, 24.0));
                                        ui.painter().rect_filled(
                                            title,
                                            2.0,
                                            Color32::from_rgb(48, 52, 57),
                                        );
                                        ui.scope_builder(
                                            egui::UiBuilder::new()
                                                .max_rect(title.shrink2(Vec2::new(5.0, 1.0)))
                                                .layout(egui::Layout::left_to_right(
                                                    egui::Align::Center,
                                                )),
                                            |ui| {
                                                let (dot, _) = ui.allocate_exact_size(
                                                    Vec2::splat(9.0),
                                                    Sense::hover(),
                                                );
                                                ui.painter().circle_filled(
                                                    dot.center(),
                                                    4.0,
                                                    ACCENT,
                                                );
                                                ui.label(
                                                    BUILTIN_DEVICES
                                                        .iter()
                                                        .find(|(_, kind, _)| *kind == device.kind)
                                                        .unwrap()
                                                        .0,
                                                );
                                                ui.with_layout(
                                                    egui::Layout::right_to_left(
                                                        egui::Align::Center,
                                                    ),
                                                    |ui| {
                                                        if ui
                                                            .small_button("×")
                                                            .on_hover_text("Remove effect")
                                                            .clicked()
                                                        {
                                                            self.execute(Command::RemoveDevice {
                                                                track_id: target.clone(),
                                                                device_id: device.id.clone(),
                                                            });
                                                        }
                                                    },
                                                );
                                            },
                                        );
                                        let body = Rect::from_min_max(
                                            Pos2::new(rect.left() + 8.0, title.bottom() + 6.0),
                                            rect.max - Vec2::new(8.0, 6.0),
                                        );
                                        if device.kind == "builtin.eq8" {
                                            self.eq_editor(ui, &target, device, body);
                                        } else {
                                            self.device_controls(ui, &target, device, body);
                                        }
                                    });
                                }
                                let width = ui.available_width().max(190.0);
                                let (rect, _) = ui
                                    .allocate_exact_size(Vec2::new(width, height), Sense::hover());
                                ui.painter()
                                    .rect_filled(rect, 2.0, Color32::from_rgb(34, 37, 41));
                                ui.scope_builder(
                                    egui::UiBuilder::new()
                                        .max_rect(rect.shrink(12.0))
                                        .layout(egui::Layout::top_down(egui::Align::Center)),
                                    |ui| {
                                        ui.add_space((height / 2.0 - 36.0).max(0.0));
                                        ui.label(
                                            egui::RichText::new("Add audio effects here")
                                                .color(MUTED),
                                        );
                                        ui.menu_button("+ Audio effect", |ui| {
                                            for &(label, kind, _) in BUILTIN_DEVICES {
                                                if ui.button(label).clicked() {
                                                    self.execute(Command::AddDevice {
                                                        track_id: target.clone(),
                                                        kind: kind.into(),
                                                    });
                                                    ui.close_menu();
                                                }
                                            }
                                        });
                                    },
                                );
                            });
                        });
                });
            });
    }
    fn set_device_parameter(&mut self, target: &str, device: &Device, parameter: &str, value: f64) {
        self.execute(Command::SetDeviceParameter {
            track_id: target.into(),
            device_id: device.id.clone(),
            parameter: parameter.into(),
            value,
        });
    }
    fn device_controls(&mut self, ui: &mut egui::Ui, target: &str, device: &Device, body: Rect) {
        let compact = device.kind == "builtin.compressor";
        let parameters: &[(&str, &str, &str)] = match device.kind.as_str() {
            "builtin.compressor" => &[
                ("threshold_db", "Threshold", " dB"),
                ("ratio", "Ratio", ":1"),
                ("makeup_db", "Makeup", " dB"),
                ("attack_ms", "Attack", " ms"),
                ("release_ms", "Release", " ms"),
                ("knee_db", "Knee", " dB"),
            ],
            "builtin.limiter" => &[
                ("input_gain_db", "Gain", " dB"),
                ("ceiling_db", "Ceiling", " dB"),
                ("release_ms", "Release", " ms"),
            ],
            "builtin.eq" => &[
                ("low_gain_db", "Low", " dB"),
                ("mid_gain_db", "Mid", " dB"),
                ("high_gain_db", "High", " dB"),
            ],
            _ => &[("gain_db", "Gain", " dB")],
        };
        ui.scope_builder(
            egui::UiBuilder::new()
                .max_rect(body)
                .layout(egui::Layout::top_down(egui::Align::Center)),
            |ui| {
                ui.add_space(
                    ((body.height() - if compact { 148.0 } else { 112.0 }) / 2.0).max(0.0),
                );
                for row in parameters.chunks(3) {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 4.0;
                        ui.add_space(((body.width() - row.len() as f32 * 76.0) / 2.0).max(0.0));
                        for &(parameter, label, suffix) in row {
                            if let Some(value) =
                                parameter_knob(ui, device, parameter, label, suffix, compact)
                            {
                                self.set_device_parameter(target, device, parameter, value);
                            }
                        }
                    });
                }
                let footer = match device.kind.as_str() {
                    "builtin.eq" => "120 Hz      1 kHz      6 kHz",
                    "builtin.limiter" => "Stereo linked · Lookahead 5 ms",
                    "builtin.compressor" => "Stereo linked · Peak",
                    _ => "",
                };
                ui.label(egui::RichText::new(footer).small().color(MUTED));
            },
        );
    }
}

fn parameter_knob(
    ui: &mut egui::Ui,
    device: &Device,
    parameter: &str,
    label: &str,
    suffix: &str,
    compact: bool,
) -> Option<f64> {
    ui.push_id(parameter, |ui| {
        ui.vertical(|ui| {
            ui.set_width(72.0);
            ui.spacing_mut().item_spacing.y = if compact { 1.0 } else { 5.0 };
            if compact {
                ui.spacing_mut().button_padding.y = 1.0;
                ui.spacing_mut().interact_size.y = 16.0;
            }
            let mut value = device.parameters[parameter];
            let (min, max) = device.parameter_range(parameter).unwrap();
            let logarithmic = parameter.ends_with("_freq_hz")
                || parameter.ends_with("_q")
                || parameter.ends_with("_ms");
            ui.with_layout(egui::Layout::top_down(egui::Align::Center), |ui| {
                ui.label(egui::RichText::new(label).small().color(MUTED));
                let size = if compact { 24.0 } else { 46.0 };
                let (rect, response) =
                    ui.allocate_exact_size(Vec2::splat(size), Sense::click_and_drag());
                let response = response
                    .on_hover_text("Drag to adjust · Shift for precision · Double-click to reset");
                if response.dragged() {
                    let (delta, shift) = ui.input(|i| (i.pointer.delta(), i.modifiers.shift));
                    let delta = f64::from(delta.x - delta.y) * if shift { 0.0005 } else { 0.003 };
                    value = if logarithmic {
                        value * (delta * (max / min).ln()).exp()
                    } else {
                        value + delta * (max - min)
                    };
                    value = value.clamp(min, max);
                }
                if response.double_clicked() {
                    value = Device::new(&device.kind).unwrap().parameters[parameter];
                }
                let center = rect.center();
                let start = std::f32::consts::PI * 0.75;
                let sweep = std::f32::consts::PI * 1.5;
                let amount = if logarithmic {
                    (value / min).ln() / (max / min).ln()
                } else {
                    (value - min) / (max - min)
                } as f32;
                let point =
                    |angle: f32, radius: f32| center + Vec2::new(angle.cos(), angle.sin()) * radius;
                let radius = size / 2.0 - 3.0;
                ui.painter()
                    .circle_filled(center, radius - 4.0, Color32::from_rgb(38, 42, 46));
                for (fraction, color) in [(1.0, LINE), (amount, ACCENT)] {
                    let points = (0..=32)
                        .map(|i| point(start + sweep * fraction * i as f32 / 32.0, radius))
                        .collect();
                    ui.painter()
                        .add(egui::Shape::line(points, Stroke::new(2.0_f32, color)));
                }
                ui.painter().line_segment(
                    [
                        point(start + sweep * amount, 3.0),
                        point(start + sweep * amount, radius - 5.0),
                    ],
                    Stroke::new(2.0_f32, TEXT),
                );
                let scale = if parameter.ends_with("_freq_hz") {
                    1000.0
                } else {
                    1.0
                };
                let mut display = value / scale;
                let response = ui.add(
                    egui::DragValue::new(&mut display)
                        .range(min / scale..=max / scale)
                        .speed(if scale == 1.0 { 0.1 } else { 0.01 })
                        .fixed_decimals(if parameter.ends_with("_q") || scale > 1.0 {
                            2
                        } else {
                            1
                        })
                        .suffix(if scale > 1.0 { " kHz" } else { suffix }),
                );
                if response.changed() {
                    value = display * scale;
                }
            });
            (value != device.parameters[parameter]).then_some(value)
        })
        .inner
    })
    .inner
}

impl Velvet {
    fn eq_editor(&mut self, ui: &mut egui::Ui, target: &str, device: &Device, body: Rect) {
        let state_id = egui::Id::new((&device.id, "selected_band"));
        let selected = ui
            .ctx()
            .data_mut(|data| *data.get_temp_mut_or(state_id, 3_usize));
        let prefix = format!("band{selected}");
        let left = Rect::from_min_size(body.min, Vec2::new(72.0, body.height()));
        ui.scope_builder(
            egui::UiBuilder::new()
                .max_rect(left)
                .layout(egui::Layout::top_down(egui::Align::Center)),
            |ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                for (suffix, label, unit) in [
                    ("freq_hz", "Freq", " Hz"),
                    ("gain_db", "Gain", " dB"),
                    ("q", "Q", ""),
                ] {
                    let parameter = format!("{prefix}_{suffix}");
                    let gain_applies = suffix != "gain_db"
                        || matches!(
                            device.parameters[&format!("{prefix}_type")] as u8,
                            0 | 3 | 4
                        );
                    ui.add_enabled_ui(gain_applies, |ui| {
                        if let Some(value) =
                            parameter_knob(ui, device, &parameter, label, unit, true)
                        {
                            self.set_device_parameter(target, device, &parameter, value);
                        }
                    });
                }
            },
        );
        let right = Rect::from_min_max(Pos2::new(body.right() - 72.0, body.top()), body.max);
        ui.scope_builder(
            egui::UiBuilder::new()
                .max_rect(right)
                .layout(egui::Layout::top_down(egui::Align::Center)),
            |ui| {
                ui.add_space(8.0);
                ui.label(egui::RichText::new("Stereo").small().color(MUTED));
                ui.add_space(8.0);
                if let Some(value) =
                    parameter_knob(ui, device, "output_gain_db", "Output", " dB", false)
                {
                    self.set_device_parameter(target, device, "output_gain_db", value);
                }
                ui.label(
                    egui::RichText::new(format!("Band {selected}"))
                        .small()
                        .color(MUTED),
                );
            },
        );
        let graph = Rect::from_min_max(
            Pos2::new(body.left() + 80.0, body.top()),
            Pos2::new(body.right() - 80.0, body.bottom() - 42.0),
        );
        let plot = Rect::from_min_max(
            graph.min + Vec2::new(22.0, 5.0),
            graph.max - Vec2::new(7.0, 16.0),
        );
        let rate = self
            .player
            .as_ref()
            .map(|p| p.sample_rate)
            .unwrap_or(self.session.project.audio.sample_rate);
        let maximum = 20000.0_f64.min(rate as f64 * 0.45);
        let frequency_x = |frequency: f64| {
            plot.left()
                + ((frequency.clamp(20.0, maximum) / 20.0).ln() / (maximum / 20.0).ln()) as f32
                    * plot.width()
        };
        let gain_y =
            |gain: f64| plot.center().y - (gain.clamp(-24.0, 24.0) as f32 / 48.0) * plot.height();
        let painter = ui.painter().with_clip_rect(graph);
        painter.rect_filled(graph, 0.0, Color32::from_rgb(13, 16, 20));
        for frequency in [
            20.0, 30.0, 50.0, 70.0, 100.0, 200.0, 300.0, 500.0, 700.0, 1000.0, 2000.0, 3000.0,
            5000.0, 7000.0, 10000.0, 20000.0,
        ] {
            if frequency > maximum {
                continue;
            }
            let x = frequency_x(frequency);
            painter.line_segment(
                [Pos2::new(x, plot.top()), Pos2::new(x, plot.bottom())],
                Stroke::new(0.5_f32, LINE),
            );
            if [100.0, 1000.0, 10000.0].contains(&frequency) {
                painter.text(
                    Pos2::new(x, graph.bottom() - 7.0),
                    egui::Align2::CENTER_CENTER,
                    if frequency == 100.0 {
                        "100"
                    } else if frequency == 1000.0 {
                        "1k"
                    } else {
                        "10k"
                    },
                    FontId::proportional(9.0),
                    MUTED,
                );
            }
        }
        for gain in [-24.0, -12.0, 0.0, 12.0, 24.0] {
            let y = gain_y(gain);
            painter.line_segment(
                [Pos2::new(plot.left(), y), Pos2::new(plot.right(), y)],
                Stroke::new(0.5_f32, LINE),
            );
            painter.text(
                Pos2::new(graph.left() + 16.0, y),
                egui::Align2::RIGHT_CENTER,
                format!("{gain:.0}"),
                FontId::proportional(9.0),
                MUTED,
            );
        }
        let frequencies: Vec<_> = (0..=192)
            .map(|i| 20.0 * (maximum / 20.0).powf(i as f64 / 192.0))
            .collect();
        let curve = velvet_audio::eq_response(device, &frequencies, rate);
        let points = frequencies
            .iter()
            .zip(curve)
            .map(|(frequency, gain)| Pos2::new(frequency_x(*frequency), gain_y(gain)))
            .collect();
        painter.add(egui::Shape::line(
            points,
            Stroke::new(1.5_f32, Color32::from_rgb(52, 186, 196)),
        ));
        for band in 1..=8 {
            let prefix = format!("band{band}");
            if device.parameters[&format!("{prefix}_enabled")] == 0.0 {
                continue;
            }
            let frequency_parameter = format!("{prefix}_freq_hz");
            let gain_parameter = format!("{prefix}_gain_db");
            let kind = device.parameters[&format!("{prefix}_type")] as u8;
            let position = Pos2::new(
                frequency_x(device.parameters[&frequency_parameter]),
                gain_y(if matches!(kind, 0 | 3 | 4) {
                    device.parameters[&gain_parameter]
                } else {
                    0.0
                }),
            );
            let response = ui
                .interact(
                    Rect::from_center_size(position, Vec2::splat(22.0)).intersect(plot),
                    egui::Id::new((&device.id, "node", band)),
                    Sense::click_and_drag(),
                )
                .on_hover_text(format!(
                    "Band {band} · Drag frequency / gain · Double-click to reset gain"
                ));
            if response.clicked() || response.drag_started() {
                ui.ctx().data_mut(|data| data.insert_temp(state_id, band));
            }
            if response.dragged() {
                if let Some(pointer) = response.interact_pointer_pos() {
                    let x = ((pointer.x - plot.left()) / plot.width()).clamp(0.0, 1.0) as f64;
                    self.set_device_parameter(
                        target,
                        device,
                        &frequency_parameter,
                        20.0 * (maximum / 20.0).powf(x),
                    );
                    if matches!(kind, 0 | 3 | 4) {
                        let gain = ((plot.center().y - pointer.y) / plot.height() * 48.0)
                            .clamp(-24.0, 24.0) as f64;
                        self.set_device_parameter(target, device, &gain_parameter, gain);
                    }
                }
            }
            if response.double_clicked() {
                self.set_device_parameter(target, device, &gain_parameter, 0.0);
            }
            let color = Color32::from_rgb(239, 166, 67);
            painter.circle_filled(position, 6.0, if selected == band { color } else { BG });
            painter.circle_stroke(position, 5.5, Stroke::new(1.2_f32, color));
            painter.text(
                position,
                egui::Align2::CENTER_CENTER,
                format!("{band}"),
                FontId::proportional(9.0),
                if selected == band { BG } else { color },
            );
        }
        let selectors = Rect::from_min_max(
            Pos2::new(graph.left(), graph.bottom() + 2.0),
            Pos2::new(graph.right(), body.bottom()),
        );
        ui.scope_builder(
            egui::UiBuilder::new()
                .max_rect(selectors)
                .layout(egui::Layout::left_to_right(egui::Align::TOP)),
            |ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                for band in 1..=8 {
                    ui.push_id(band, |ui| {
                        ui.vertical(|ui| {
                            ui.set_width((selectors.width() / 8.0 - 2.0).max(34.0));
                            ui.spacing_mut().item_spacing = Vec2::new(1.0, 1.0);
                            ui.spacing_mut().button_padding = Vec2::new(2.0, 1.0);
                            let type_parameter = format!("band{band}_type");
                            let mut kind = device.parameters[&type_parameter] as usize;
                            egui::ComboBox::from_id_salt("type")
                                .width(26.0)
                                .selected_text(["B", "LC", "HC", "LS", "HS", "N"][kind])
                                .show_ui(ui, |ui| {
                                    for (i, name) in
                                        velvet_audio::EQ_FILTER_TYPES.iter().enumerate()
                                    {
                                        ui.selectable_value(&mut kind, i, *name);
                                    }
                                })
                                .response
                                .on_hover_text(velvet_audio::EQ_FILTER_TYPES[kind]);
                            if kind as f64 != device.parameters[&type_parameter] {
                                self.set_device_parameter(
                                    target,
                                    device,
                                    &type_parameter,
                                    kind as f64,
                                );
                            }
                            ui.horizontal(|ui| {
                                let enabled_parameter = format!("band{band}_enabled");
                                let mut enabled = device.parameters[&enabled_parameter] == 1.0;
                                if ui
                                    .checkbox(&mut enabled, "")
                                    .on_hover_text("Enable band")
                                    .changed()
                                {
                                    self.set_device_parameter(
                                        target,
                                        device,
                                        &enabled_parameter,
                                        if enabled { 1.0 } else { 0.0 },
                                    );
                                }
                                if ui
                                    .selectable_label(selected == band, band.to_string())
                                    .clicked()
                                {
                                    ui.ctx().data_mut(|data| data.insert_temp(state_id, band));
                                }
                            });
                        });
                    });
                }
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dragging_eq_node_edits_frequency_and_gain_without_pausing() {
        let ctx = egui::Context::default();
        let mut app = Velvet::new(&ctx);
        app.session = Session::new(Project::new("EQ UI"), PathBuf::new());
        app.job = None;
        app.execute(Command::AddDevice {
            track_id: "master".into(),
            kind: "builtin.eq8".into(),
        });
        app.selected_track = Some("master".into());
        app.session.transport.playing = true;
        app.session.transport.seconds = 4.0;
        let input = |events| egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1440.0, 900.0))),
            events,
            ..Default::default()
        };
        let _ = ctx.run(input(vec![]), |ctx| app.rack(ctx));
        let output = ctx.run(input(vec![]), |ctx| app.rack(ctx));
        let position = output
            .shapes
            .iter()
            .find_map(|shape| {
                if let egui::Shape::Text(text) = &shape.shape {
                    if text.galley.job.text == "3" {
                        return Some(text.pos + text.galley.size() / 2.0);
                    }
                }
                None
            })
            .expect("EQ band 3 handle must be visible");
        let button = |pressed| egui::Event::PointerButton {
            pos: position,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        let _ = ctx.run(
            input(vec![egui::Event::PointerMoved(position), button(true)]),
            |ctx| app.rack(ctx),
        );
        let _ = ctx.run(
            input(vec![egui::Event::PointerMoved(
                position + Vec2::new(25.0, -10.0),
            )]),
            |ctx| app.rack(ctx),
        );
        let device = &app.session.project.master_devices[0];
        assert!(device.parameters["band3_freq_hz"] > 1000.0);
        assert!(device.parameters["band3_gain_db"] > 0.0);
        assert!(app.session.transport.playing);
        assert_eq!(app.session.transport.seconds, 4.0);
        app.session.project.validate().unwrap();
    }
}
