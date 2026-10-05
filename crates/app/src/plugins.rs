use super::*;
use std::sync::Mutex;
use velvet_audio::plugins::{Plugin, PluginWindow};
use velvet_core::Device;

pub(super) struct Editor {
    window: PluginWindow,
    plugin: Arc<Mutex<Plugin>>,
    target: String,
    device: String,
    state: Vec<u8>,
    last_capture: Vec<u8>,
    checked: Instant,
    revision: u64,
    changed: Instant,
    pending: bool,
    default_state: Option<Vec<u8>>,
}

impl Velvet {
    pub(super) fn scan_plugin_roles(&mut self) {
        #[cfg(not(test))]
        if self.plugin_scan.is_none() {
            let paths: Vec<_> = self
                .plugin_paths
                .iter()
                .filter(|p| !self.plugin_roles.contains_key(*p))
                .cloned()
                .collect();
            let Ok(exe) = std::env::current_exe() else {
                return;
            };
            let (tx, rx) = mpsc::channel();
            self.plugin_scan = Some(rx);
            std::thread::spawn(move || {
                for path in paths {
                    let mut command = std::process::Command::new(&exe);
                    command
                        .arg("--velvet-plugin-info")
                        .arg(&path)
                        .stdout(std::process::Stdio::piped())
                        .stderr(std::process::Stdio::null());
                    #[cfg(windows)]
                    {
                        use std::os::windows::process::CommandExt;
                        command.creation_flags(0x08000000);
                    }
                    let Ok(mut child) = command.spawn() else {
                        continue;
                    };
                    let started = Instant::now();
                    loop {
                        match child.try_wait() {
                            Ok(Some(_)) => break,
                            Ok(None) if started.elapsed() < Duration::from_secs(10) => {
                                std::thread::sleep(Duration::from_millis(20))
                            }
                            _ => {
                                let _ = child.kill();
                                break;
                            }
                        }
                    }
                    if let Ok(output) = child.wait_with_output() {
                        if output.status.success() {
                            let text = String::from_utf8_lossy(&output.stdout);
                            let role = text.lines().find_map(|line| {
                                line.strip_prefix("VELVET_PLUGIN_ROLE=")
                                    .and_then(|v| v.parse::<bool>().ok())
                            });
                            if let Some(role) = role {
                                if tx.send((path, role)).is_err() {
                                    return;
                                }
                            }
                        }
                    }
                }
            });
        }
    }
    pub(super) fn plugin_controls(&mut self, ui: &mut egui::Ui, target: &str, device: &Device) {
        let body = ui.available_rect_before_wrap();
        ui.scope_builder(
            egui::UiBuilder::new()
                .max_rect(body)
                .layout(egui::Layout::top_down(egui::Align::Min)),
            |ui| {
                ui.set_clip_rect(body.intersect(ui.clip_rect()));
                ui.label(egui::RichText::new("VST3").small().color(MUTED));
                if ui.button("Open plugin editor").clicked() {
                    self.open_plugin_editor(target, device);
                }
                let mut enabled = self.scope_enabled.contains(&device.id);
                if ui.checkbox(&mut enabled, "Oscilloscope").changed() {
                    if enabled {
                        self.scope_enabled.insert(device.id.clone());
                        self.live_dirty = true;
                        self.edit_time = Instant::now();
                    } else {
                        self.scope_enabled.remove(&device.id);
                        self.scope_signals.remove(&device.id);
                    }
                }
                self.draw_plugin_scope(ui, device, enabled);
            },
        );
    }

    fn draw_plugin_scope(&self, ui: &mut egui::Ui, device: &Device, enabled: bool) {
        let height = (ui.available_height() - 4.0).clamp(0.0, 180.0);
        let (rect, _) =
            ui.allocate_exact_size(Vec2::new(ui.available_width(), height), Sense::hover());
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 3.0, BG);
        painter.rect_stroke(
            rect,
            3.0,
            Stroke::new(0.5_f32, LINE),
            egui::StrokeKind::Inside,
        );
        if rect.height() < 28.0 {
            return;
        }
        let graph = rect.shrink2(Vec2::new(8.0, 14.0));
        for fraction in [0.25, 0.5, 0.75] {
            let y = graph.top() + graph.height() * fraction;
            painter.line_segment(
                [Pos2::new(graph.left(), y), Pos2::new(graph.right(), y)],
                Stroke::new(0.5_f32, LINE),
            );
        }
        let signal = enabled
            .then(|| self.scope_signals.get(&device.id))
            .flatten();
        let status = if !enabled {
            "OFF"
        } else if signal.is_none() {
            "Preparing signal…"
        } else {
            "L / R · OUTPUT"
        };
        painter.text(
            rect.left_top() + Vec2::new(8.0, 3.0),
            egui::Align2::LEFT_TOP,
            status,
            FontId::monospace(9.0),
            MUTED,
        );
        let Some(signal) = signal else {
            return;
        };
        if signal.buckets.is_empty() {
            return;
        }
        let seconds = self
            .player
            .as_ref()
            .map_or(self.session.transport.seconds, |p| p.seconds());
        let start = (seconds * signal.sample_rate as f64 / signal.stride as f64) as usize;
        let span = ((signal.sample_rate as f64 * 0.04 / signal.stride as f64) as usize).max(2);
        for (channel, color) in [(0, CYAN), (1, ROSE)] {
            let mut points = Vec::with_capacity(span.min(2048));
            let step = span.div_ceil(2048).max(1);
            for offset in (0..span).step_by(step) {
                let bounds = signal
                    .buckets
                    .get(start + offset)
                    .copied()
                    .unwrap_or([[0.0; 2]; 2]);
                let x = graph.left() + offset as f32 / (span - 1) as f32 * graph.width();
                let y =
                    |value: f32| graph.center().y - value.clamp(-1.0, 1.0) * graph.height() * 0.46;
                painter.line_segment(
                    [
                        Pos2::new(x, y(bounds[0][channel])),
                        Pos2::new(x, y(bounds[1][channel])),
                    ],
                    Stroke::new(0.8_f32, color.gamma_multiply(0.6)),
                );
                points.push(Pos2::new(
                    x,
                    y((bounds[0][channel] + bounds[1][channel]) * 0.5),
                ));
            }
            painter.add(egui::Shape::line(points, Stroke::new(1.0_f32, color)));
        }
        if self.session.transport.playing {
            ui.ctx().request_repaint_after(Duration::from_millis(33));
        }
    }

    pub(super) fn open_plugin_editor(&mut self, target: &str, device: &Device) {
        if self.plugin_editor.as_ref().is_some_and(|editor| {
            editor.target == target && editor.device == device.id && editor.window.is_open()
        }) {
            return;
        }
        // Commit the previous editor before replacing it.
        if !self.capture_plugin_state() {
            return;
        }
        self.plugin_editor = None;
        let result = (|| -> anyhow::Result<Editor> {
            let plugin = Arc::new(Mutex::new(velvet_audio::plugins::load(
                device,
                48000,
                self.session.project.tempo.bpm,
            )?));
            let default_state = device
                .plugin_state
                .is_empty()
                .then(|| plugin.lock().unwrap().save_state())
                .transpose()?;
            let mut window = PluginWindow::new(plugin.clone());
            window.open()?;
            let last_capture = velvet_audio::plugins::snapshot(&mut plugin.lock().unwrap())?;
            let revision = plugin.lock().unwrap().edit_revision();
            Ok(Editor {
                window,
                plugin,
                target: target.into(),
                device: device.id.clone(),
                state: device.plugin_state.clone(),
                last_capture,
                checked: Instant::now(),
                revision,
                changed: Instant::now(),
                pending: false,
                default_state,
            })
        })();
        match result {
            Ok(editor) => self.plugin_editor = Some(editor),
            Err(error) => {
                self.status = format!("{error:#}");
                self.error = true;
            }
        }
    }

    fn editor_device(&self, editor: &Editor) -> Option<&Device> {
        self.session
            .project
            .devices(&editor.target)
            .ok()?
            .iter()
            .find(|d| d.id == editor.device)
            .or_else(|| {
                self.session
                    .project
                    .track(&editor.target)
                    .ok()?
                    .synth
                    .as_ref()
                    .filter(|d| d.id == editor.device)
            })
    }

    pub(super) fn capture_plugin_state(&mut self) -> bool {
        let Some(mut editor) = self.plugin_editor.take() else {
            return true;
        };
        if self.editor_device(&editor).is_none() {
            return true;
        }
        let result = velvet_audio::plugins::snapshot(&mut editor.plugin.lock().unwrap());
        let mut success = true;
        match result {
            Ok(state) if state != editor.last_capture => {
                self.execute(Command::SetPluginState {
                    track_id: editor.target.clone(),
                    device_id: editor.device.clone(),
                    state: state.clone(),
                });
                if self
                    .editor_device(&editor)
                    .is_some_and(|d| d.plugin_state == state)
                {
                    editor.state = state.clone();
                    editor.last_capture = state;
                } else {
                    success = false;
                }
            }
            Err(error) => {
                self.status = format!("Cannot save VST3 settings: {error}");
                self.error = true;
                success = false;
            }
            _ => {}
        }
        editor.checked = Instant::now();
        editor.pending = false;
        self.plugin_editor = Some(editor);
        success
    }

    pub(super) fn poll_plugin_editor(&mut self, ctx: &egui::Context) {
        if let Some(rx) = &self.plugin_scan {
            let mut finished = false;
            loop {
                match rx.try_recv() {
                    Ok((path, role)) => {
                        self.plugin_roles.insert(path, role);
                    }
                    Err(mpsc::TryRecvError::Disconnected) => {
                        finished = true;
                        break;
                    }
                    Err(mpsc::TryRecvError::Empty) => break,
                }
            }
            if finished {
                self.plugin_scan = None;
            }
        }
        let Some(mut editor) = self.plugin_editor.take() else {
            return;
        };
        let Some(device) = self.editor_device(&editor).cloned() else {
            return;
        };
        if device.plugin_state != editor.state {
            // Undo/redo restores the open editor as well as the rendered instance.
            let result = if device.plugin_state.is_empty() {
                let state = editor.default_state.clone().map(Ok).unwrap_or_else(|| {
                    velvet_audio::plugins::load(&device, 48000, self.session.project.tempo.bpm)
                        .and_then(|p| Ok(p.save_state()?))
                });
                state.and_then(|state| {
                    editor.default_state = Some(state.clone());
                    Ok(editor.plugin.lock().unwrap().load_state(&state)?)
                })
            } else {
                editor
                    .plugin
                    .lock()
                    .unwrap()
                    .load_state(&device.plugin_state)
                    .map_err(anyhow::Error::from)
            };
            if let Err(error) = result {
                self.status = format!("{error:#}");
                self.error = true;
                return;
            }
            editor.state = device.plugin_state;
            match velvet_audio::plugins::snapshot(&mut editor.plugin.lock().unwrap()) {
                Ok(state) => editor.last_capture = state,
                Err(error) => {
                    self.status = format!("{error}");
                    self.error = true;
                    return;
                }
            }
        }
        if let Err(error) = editor.window.service_platform_events() {
            self.status = format!("VST3 editor error: {error}");
            self.error = true;
        }
        let closed = editor.window.closed_by_user();
        {
            let mut plugin = editor.plugin.lock().unwrap();
            plugin.service_run_loop();
            let revision = plugin.edit_revision();
            // Drain control feedback, never the parameter queue feeding the processor.
            plugin.take_parameter_edits();
            plugin.take_host_notifications();
            if revision != editor.revision {
                editor.revision = revision;
                editor.changed = Instant::now();
                editor.pending = true;
            }
        }
        let capture = closed
            || (editor.pending && (editor.changed.elapsed() >= Duration::from_millis(75)
                || editor.checked.elapsed() >= Duration::from_millis(250)))
            // Some plugins change private state without notifying their host.
            || editor.checked.elapsed() >= Duration::from_secs(2);
        self.plugin_editor = Some(editor);
        if capture && !self.capture_plugin_state() {
            return;
        }
        if closed {
            self.plugin_editor = None;
        } else {
            ctx.request_repaint_after(Duration::from_millis(25));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn plugin_card_controls_stay_inside_body() {
        let ctx = egui::Context::default();
        let mut app = Velvet::new(&ctx);
        let path = std::env::current_dir().unwrap().join("Test.vst3");
        let device = Device::new(&format!("vst3.effect:{}", path.display())).unwrap();
        let body = Rect::from_min_size(Pos2::new(10.0, 10.0), Vec2::new(236.0, 240.0));
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let result = ui.scope_builder(
                    egui::UiBuilder::new()
                        .max_rect(body)
                        .layout(egui::Layout::left_to_right(egui::Align::Min)),
                    |ui| app.plugin_controls(ui, "master", &device),
                );
                assert!(
                    result.response.rect.right() <= body.right() + 1.0
                        && result.response.rect.bottom() <= body.bottom() + 1.0,
                    "Plugin controls overflow: {:?}",
                    result.response.rect
                );
            });
        });
    }
    #[test]
    fn scope_checkbox_toggles_capture_without_editing_project() {
        let ctx = egui::Context::default();
        let mut app = Velvet::new(&ctx);
        let path = std::env::current_dir().unwrap().join("Test.vst3");
        let device = Device::new(&format!("vst3.effect:{}", path.display())).unwrap();
        let project = app.session.project.clone();
        let render = |app: &mut Velvet, events| {
            ctx.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1000.0, 700.0))),
                    events,
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        ui.scope_builder(
                            egui::UiBuilder::new().max_rect(Rect::from_min_size(
                                Pos2::new(10.0, 10.0),
                                Vec2::new(236.0, 240.0),
                            )),
                            |ui| app.plugin_controls(ui, "master", &device),
                        );
                    });
                },
            )
        };
        let _ = render(&mut app, vec![]);
        let output = render(&mut app, vec![]);
        let position = output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.text() == "Oscilloscope" => {
                    Some(text.pos + text.galley.size() * 0.5)
                }
                _ => None,
            })
            .unwrap();
        let click = |pressed| egui::Event::PointerButton {
            pos: position,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        let _ = render(
            &mut app,
            vec![egui::Event::PointerMoved(position), click(true)],
        );
        let _ = render(&mut app, vec![click(false)]);
        assert!(app.scope_enabled.contains(&device.id));
        assert!(app.live_dirty);
        app.scope_signals.insert(
            device.id.clone(),
            Arc::new(velvet_audio::ScopeSignal::new(&[[0.2, -0.3]; 1000], 48000)),
        );
        let _ = render(&mut app, vec![click(true)]);
        let _ = render(&mut app, vec![click(false)]);
        assert!(!app.scope_enabled.contains(&device.id));
        assert!(!app.scope_signals.contains_key(&device.id));
        assert_eq!(app.session.project, project);
    }
    #[test]
    fn vst_instrument_and_effect_render_in_the_rack_without_builtin_parameters() {
        let ctx = egui::Context::default();
        let mut app = Velvet::new(&ctx);
        app.session = Session::new(Project::new("Plugins"), PathBuf::new());
        app.job = None;
        app.execute(Command::AddMidiTrack {
            name: "MIDI".into(),
        });
        let track = app.session.project.tracks[0].id.clone();
        let path = std::env::current_dir().unwrap().join("Test.vst3");
        app.execute(Command::SetTrackInstrument {
            track_id: track.clone(),
            kind: Some(format!("vst3.instrument:{}", path.display())),
        });
        app.execute(Command::AddDevice {
            track_id: track.clone(),
            kind: format!("vst3.effect:{}", path.display()),
        });
        app.selected_track = Some(track);
        let _ = ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1440.0, 900.0))),
                ..Default::default()
            },
            |ctx| app.rack(ctx),
        );
        assert_eq!(app.session.project.tracks[0].devices.len(), 1);
        assert!(!app.error, "{}", app.status);
    }
}
