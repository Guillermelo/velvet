use super::*;

pub(super) struct EffectDrag {
    pub kind: String,
}
pub(super) struct SampleDrag {
    pub path: PathBuf,
}

#[derive(Clone, Copy)]
enum BrowserIcon {
    Folder,
    Audio,
    Instrument,
    Effects,
    Project,
    All,
}
const BROWSER_SELECTION: Color32 = Color32::from_rgb(160, 214, 230);

impl Velvet {
    fn load_browser_folder(&mut self, folder: PathBuf) -> bool {
        match folder_entries(&folder) {
            Ok(files) => {
                self.browser_files = files;
                self.browser_folder = Some(folder);
                self.browser_selected = None;
                self.browser_children.clear();
                true
            }
            Err(e) => {
                self.report(Err(e.into()), "");
                false
            }
        }
    }
    fn browse_folder(&mut self, folder: PathBuf) -> bool {
        if !self.load_browser_folder(folder.clone()) {
            return false;
        }
        if self.browser_history.get(self.browser_history_index) != Some(&folder) {
            self.browser_history
                .truncate(self.browser_history_index + 1);
            self.browser_history.push(folder);
            self.browser_history_index = self.browser_history.len() - 1;
        }
        true
    }
    fn browser_history_step(&mut self, forward: bool) {
        let index = if forward {
            self.browser_history_index + 1
        } else {
            self.browser_history_index.saturating_sub(1)
        };
        if let Some(folder) = self.browser_history.get(index).cloned() {
            if self.load_browser_folder(folder) {
                self.browser_history_index = index;
            }
        }
    }
    pub(super) fn restore_samples_folder(&mut self) {
        if let Some(folder) = samples_settings_file().and_then(|path| read_saved_folder(&path)) {
            if self.browse_folder(folder.clone()) {
                self.samples_root = Some(folder);
                self.browser_category = 1;
            }
        }
    }
    fn select_samples_folder(&mut self, folder: PathBuf) {
        if self.browse_folder(folder.clone()) {
            self.samples_root = Some(folder.clone());
            self.browser_category = 1;
            if let Some(path) = samples_settings_file() {
                self.report(
                    save_samples_folder(&path, &folder),
                    "Samples folder saved · Sorted by name",
                );
            }
        }
    }
    fn pick_samples_folder(&mut self) {
        if let Some(path) = rfd::FileDialog::new().pick_folder() {
            self.select_samples_folder(path);
        }
    }
    fn import_browser_file(&mut self, path: PathBuf) {
        self.import(
            path,
            None,
            self.session.transport.seconds * self.session.project.tempo.bpm / 60.0,
        );
    }
    fn preview_sample(&mut self, path: PathBuf) {
        self.sample_preview = None;
        let (tx, rx) = mpsc::channel();
        self.sample_preview_job = Some(rx);
        std::thread::spawn(move || {
            let result = (|| {
                let rate = Player::output_rate()?;
                let data = velvet_audio::decode(&path)?;
                let ratio = data.sample_rate as f64 / rate as f64;
                let count = (data.frames.len() as f64 / ratio).ceil() as usize;
                let frames = (0..count).map(|i| {
                    let position = i as f64 * ratio;
                    let index = position as usize;
                    let a = data.frames[index.min(data.frames.len() - 1)];
                    let b = data.frames[(index + 1).min(data.frames.len() - 1)];
                    let blend = position.fract() as f32;
                    [a[0] + (b[0] - a[0]) * blend, a[1] + (b[1] - a[1]) * blend]
                }).collect();
                Ok(Mix { sample_rate: rate, frames, missing: vec![], sources: vec![path],
                    peak: 0.0, device_signals: Default::default() })
            })();
            let _ = tx.send(result);
        });
    }
    pub(super) fn poll_sample_preview(&mut self) {
        if let Some(rx) = &self.sample_preview_job {
            match rx.try_recv() {
                Ok(result) => {
                    self.sample_preview_job = None;
                    match result.and_then(|mix| Player::new(Arc::new(mix), 0.0)) {
                        Ok(player) => {
                            player.set_monitor_gain(self.monitor_volume, false);
                            player.play();
                            self.sample_preview = Some(player);
                        }
                        Err(error) => self.report(Err(error), ""),
                    }
                }
                Err(mpsc::TryRecvError::Disconnected) => self.sample_preview_job = None,
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
        if self.sample_preview.as_ref().is_some_and(|p| !p.playing()) { self.sample_preview = None; }
    }
    pub(super) fn browser(&mut self, ctx: &egui::Context) {
        egui::SidePanel::left("browser")
            .default_width(420.0)
            .width_range(340.0..=660.0)
            .frame(egui::Frame::new().fill(PANEL).inner_margin(6.0))
            .show(ctx, |ui| {
                ui.spacing_mut().item_spacing = Vec2::new(2.0, 2.0);
                ui.spacing_mut().button_padding = Vec2::new(4.0, 2.0);
                ui.spacing_mut().interact_size = Vec2::new(20.0, 20.0);
                egui::Frame::new()
                    .fill(Color32::from_rgb(26, 29, 35))
                    .stroke(Stroke::new(0.5_f32, LINE))
                    .inner_margin(3.0)
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            if ui
                                .add_enabled(
                                    self.browser_history_index > 0,
                                    egui::Button::new("‹").frame(false),
                                )
                                .on_hover_text("Back")
                                .clicked()
                            {
                                self.browser_history_step(false);
                            }
                            if ui
                                .add_enabled(
                                    self.browser_history_index + 1 < self.browser_history.len(),
                                    egui::Button::new("›").frame(false),
                                )
                                .on_hover_text("Forward")
                                .clicked()
                            {
                                self.browser_history_step(true);
                            }
                            let search = ui.add(
                                egui::TextEdit::singleline(&mut self.search)
                                    .font(FontId::monospace(10.0))
                                    .id(egui::Id::new("library_search"))
                                    .hint_text("Search (Ctrl+F)")
                                    .desired_width((ui.available_width() - 54.0).max(90.0)),
                            );
                            if ctx.input(|i| i.modifiers.ctrl && i.key_pressed(egui::Key::F)) {
                                search.request_focus();
                            }
                            if ui.small_button("×").on_hover_text("Clear search").clicked() {
                                self.search.clear();
                            }
                            ui.menu_button("=", |ui| {
                                for (category, label) in [
                                    (0, "All"),
                                    (1, "Samples"),
                                    (2, "Audio Effects"),
                                    (4, "Instruments"),
                                    (3, "Current Project"),
                                ] {
                                    if ui
                                        .selectable_label(self.browser_category == category, label)
                                        .clicked()
                                    {
                                        self.browser_category = category;
                                        self.browser_selected = None;
                                        ui.close_menu();
                                    }
                                }
                            })
                            .response
                            .on_hover_text("Filter content");
                        });
                    });
                let body_height = ui.available_height();
                let body_width = ui.available_width();
                ui.horizontal_top(|ui| {
                    ui.spacing_mut().item_spacing.x = 0.0;
                    let navigation_width = (body_width * 0.35).clamp(118.0, 166.0);
                    ui.allocate_ui_with_layout(
                        Vec2::new(navigation_width, body_height),
                        egui::Layout::top_down(egui::Align::LEFT),
                        |ui| {
                            ui.set_width(navigation_width);
                            egui::ScrollArea::vertical()
                                .id_salt("library_navigation")
                                .auto_shrink([false, false])
                                .show(ui, |ui| {
                                    ui.add_space(5.0);
                                    eyebrow(ui, "LIBRARY");
                                    ui.add_space(5.0);
                                    for (category, label, icon) in [
                                        (0, "All", BrowserIcon::All),
                                        (2, "Audio Effects", BrowserIcon::Effects),
                                        (4, "Instruments", BrowserIcon::Instrument),
                                        (1, "Samples", BrowserIcon::Audio),
                                        (3, "Clips", BrowserIcon::Project),
                                    ] {
                                        if browser_row(
                                            ui,
                                            label,
                                            icon,
                                            self.browser_category == category,
                                            0.0,
                                            None,
                                        )
                                        .clicked()
                                        {
                                            self.browser_category = category;
                                            self.browser_selected = None;
                                        }
                                    }
                                    ui.add_space(28.0);
                                    eyebrow(ui, "PLACES");
                                    ui.add_space(5.0);
                                    if browser_row(
                                        ui,
                                        "Current Project",
                                        BrowserIcon::Project,
                                        self.browser_category == 3,
                                        0.0,
                                        None,
                                    )
                                    .clicked()
                                    {
                                        self.browser_category = 3;
                                        self.browser_selected = None;
                                    }
                                    if let Some(root) = self.samples_root.clone() {
                                        let name =
                                            root.file_name().unwrap_or_default().to_string_lossy();
                                        if browser_row(
                                            ui,
                                            &name,
                                            BrowserIcon::Folder,
                                            self.browser_category != 2
                                                && self.browser_category != 4
                                                && self.browser_category != 3
                                                && self.browser_folder.is_some(),
                                            0.0,
                                            None,
                                        )
                                        .on_hover_text(root.display().to_string())
                                        .clicked()
                                        {
                                            self.browse_folder(root);
                                            self.browser_category = 1;
                                        }
                                    }
                                    if browser_row(
                                        ui,
                                        "Add Folder…",
                                        BrowserIcon::Folder,
                                        false,
                                        0.0,
                                        None,
                                    )
                                    .on_hover_text("Choose and remember your samples folder")
                                    .clicked()
                                    {
                                        self.pick_samples_folder();
                                    }
                                    ui.add_space(12.0);
                                    if ui
                                        .add_enabled(
                                            self.job.is_none(),
                                            egui::Button::new("Import file…").frame(false),
                                        )
                                        .clicked()
                                    {
                                        if let Some(path) = rfd::FileDialog::new()
                                            .add_filter("Audio", &["wav", "flac"])
                                            .pick_file()
                                        {
                                            self.import_browser_file(path);
                                        }
                                    }
                                });
                        },
                    );
                    let separator_x = ui.cursor().left();
                    ui.painter().line_segment(
                        [
                            Pos2::new(separator_x, ui.cursor().top()),
                            Pos2::new(separator_x, ui.cursor().top() + body_height),
                        ],
                        Stroke::new(1.0_f32, LINE),
                    );
                    ui.add_space(1.0);
                    ui.allocate_ui_with_layout(
                        Vec2::new(body_width - navigation_width - 1.0, body_height),
                        egui::Layout::top_down(egui::Align::LEFT),
                        |ui| {
                            ui.set_width(body_width - navigation_width - 1.0);
                            self.browser_content(ui, body_height);
                        },
                    );
                });
            });
    }
    fn browser_content(&mut self, ui: &mut egui::Ui, height: f32) {
        egui::Frame::new()
            .fill(Color32::from_rgb(35, 39, 47))
            .inner_margin(4.0)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("Name").monospace().size(10.0).strong());
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.menu_button("···", |ui| {
                            if ui.button("Refresh files").clicked() {
                                if let Some(folder) = self.browser_folder.clone() {
                                    self.load_browser_folder(folder);
                                }
                                ui.close_menu();
                            }
                            if ui.button("Samples folder…").clicked() {
                                self.pick_samples_folder();
                                ui.close_menu();
                            }
                        });
                        ui.label(egui::RichText::new("A–Z").small().color(MUTED))
                            .on_hover_text("Folders first · A–Z");
                    });
                });
            });
        if self.browser_category != 2 && self.browser_category != 3 && self.browser_category != 4 {
            if let Some(folder) = self.browser_folder.clone() {
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(
                            self.samples_root.as_ref() != Some(&folder),
                            egui::Button::new("↑").frame(false),
                        )
                        .on_hover_text("Parent folder")
                        .clicked()
                    {
                        if let Some(parent) = folder.parent() {
                            self.browse_folder(parent.to_path_buf());
                        }
                    }
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(
                                folder.file_name().unwrap_or_default().to_string_lossy(),
                            )
                            .small()
                            .color(MUTED),
                        )
                        .truncate(),
                    )
                    .on_hover_text(folder.display().to_string());
                });
            }
        }
        if matches!(self.browser_category, 0 | 2 | 4) {
            ui.menu_button("VST3", |ui| {
                if ui.small_button("Refresh VST3").clicked() {
                    self.plugin_paths = velvet_audio::plugins::scan();
                    self.scan_plugin_roles();
                }
                if ui.small_button("Add VST3…").clicked() {
                    if let Some(path) = rfd::FileDialog::new()
                        .add_filter("VST3", &["vst3"])
                        .pick_file()
                    {
                        if !self.plugin_paths.contains(&path) {
                            self.plugin_paths.push(path);
                            self.scan_plugin_roles();
                        }
                    }
                }
                if ui.small_button("VST3 folder…").clicked() {
                    if let Some(folder) = rfd::FileDialog::new().pick_folder() {
                        self.plugin_paths
                            .extend(velvet_audio::plugins::scan_folder(&folder));
                        self.plugin_paths.sort();
                        self.plugin_paths.dedup();
                        self.scan_plugin_roles();
                    }
                }
            });
        }
        let query = self.search.trim().to_lowercase();
        let list_height = (height - 94.0).max(60.0);
        let mut count = 0;
        egui::ScrollArea::vertical()
            .id_salt("library_content")
            .auto_shrink([false, false])
            .max_height(list_height)
            .min_scrolled_height(list_height)
            .show(ui, |ui| {
                if self.browser_category == 0 || self.browser_category == 2 {
                    for &(name, kind, description) in velvet_core::BUILTIN_DEVICES {
                        if !format!("{name} {description} {kind}")
                            .to_lowercase()
                            .contains(&query)
                        {
                            continue;
                        }
                        count += 1;
                        ui.add_enabled_ui(
                            self.job.is_none()
                                && self
                                    .selected_track
                                    .as_deref()
                                    .is_some_and(|id| self.session.project.devices(id).is_ok()),
                            |ui| {
                                let response =
                                    browser_row(ui, name, BrowserIcon::Effects, false, 0.0, None)
                                        .on_hover_text(format!(
                                        "{description}\nDrag into the device chain · Click to add"
                                    ));
                                response.dnd_set_drag_payload(EffectDrag { kind: kind.into() });
                                if response.clicked() {
                                    self.execute(Command::AddDevice {
                                        track_id: self.selected_track.clone().unwrap(),
                                        kind: kind.into(),
                                    });
                                }
                            },
                        );
                    }
                }
                if (self.browser_category == 0 || self.browser_category == 4)
                    && "dot native polyphonic synthesizer builtin.dot".contains(&query)
                {
                    count += 1;
                    let target = self.selected_track.clone().filter(|id| {
                        self.session
                            .project
                            .track(id)
                            .is_ok_and(|t| matches!(t.kind, velvet_core::TrackKind::Midi))
                    });
                    ui.add_enabled_ui(self.job.is_none(), |ui| {
                        let response = browser_row(
                            ui,
                            "Dot",
                            BrowserIcon::Instrument,
                            false,
                            0.0,
                            None,
                        )
                        .interact(Sense::click_and_drag())
                        .on_hover_text(
                            "Drag onto a MIDI track · Click to assign to selected MIDI track",
                        );
                        response.dnd_set_drag_payload(crate::synth::InstrumentDrag {
                            kind: "builtin.dot".into(),
                        });
                        if response.clicked() {
                            if let Some(track_id) = target {
                                self.execute(Command::SetTrackInstrument {
                                    track_id,
                                    kind: Some("builtin.dot".into()),
                                });
                            } else {
                                self.status = "Select a MIDI track or drag Dot onto one".into();
                            }
                        }
                    });
                    if self.browser_category == 4 {
                        ui.label(
                            egui::RichText::new("Drag Dot onto a MIDI track")
                                .small()
                                .color(MUTED),
                        );
                    }
                }
                if matches!(self.browser_category, 0 | 2 | 4) {
                    for path in self.plugin_paths.clone() {
                        let name = path
                            .file_stem()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .into_owned();
                        if !name.to_lowercase().contains(&query) {
                            continue;
                        }
                        let known_role = self.plugin_roles.get(&path).copied();
                        if (self.browser_category == 4 && known_role == Some(false))
                            || (self.browser_category == 2 && known_role == Some(true))
                        {
                            continue;
                        }
                        count += 1;
                        let instrument = known_role.unwrap_or(self.browser_category == 4);
                        let kind = format!(
                            "vst3.{}:{}",
                            if instrument { "instrument" } else { "effect" },
                            path.display()
                        );
                        let response = browser_row(
                            ui,
                            &format!("{name} / VST3"),
                            if instrument {
                                BrowserIcon::Instrument
                            } else {
                                BrowserIcon::Effects
                            },
                            false,
                            0.0,
                            None,
                        )
                        .on_hover_text(format!(
                            "{}\nUse Instruments for MIDI, Effects for audio",
                            path.display()
                        ));
                        if instrument {
                            response.dnd_set_drag_payload(crate::synth::InstrumentDrag {
                                kind: kind.clone(),
                            });
                        } else {
                            response.dnd_set_drag_payload(EffectDrag { kind: kind.clone() });
                        }
                        if response.clicked() && self.job.is_none() {
                            if let Some(track_id) = self.selected_track.clone() {
                                if instrument {
                                    self.execute(Command::SetTrackInstrument {
                                        track_id,
                                        kind: Some(kind),
                                    });
                                } else {
                                    self.execute(Command::AddDevice { track_id, kind });
                                }
                            }
                        }
                    }
                }
                if self.browser_category != 2 && self.browser_category != 4 {
                    let files = if self.browser_folder.is_some() && self.browser_category != 3 {
                        self.browser_files.clone()
                    } else {
                        let mut files: Vec<_> = self
                            .session
                            .project
                            .tracks
                            .iter()
                            .flat_map(|t| &t.clips)
                            .map(|c| {
                                self.session
                                    .project
                                    .source_path(&self.session.root, &c.source)
                            })
                            .collect();
                        files.sort();
                        files.dedup();
                        sort_files(&mut files);
                        files
                    };
                    count += self.browser_tree(ui, &files, &query, 0);
                }
                if count == 0 {
                    ui.add_space(12.0);
                    ui.label(
                        egui::RichText::new(if query.is_empty() {
                            "Add a folder to browse your audio."
                        } else {
                            "No matching audio or effects."
                        })
                        .small()
                        .color(MUTED),
                    );
                }
            });
        ui.add_space(4.0);
        ui.separator();
        ui.add_space(4.0);
        let selection = self
            .browser_selected
            .as_ref()
            .map(|p| p.file_name().unwrap_or_default().to_string_lossy());
        ui.add(
            egui::Label::new(
                egui::RichText::new(selection.as_deref().unwrap_or(
                    if self.browser_category == 4 {
                        "VELVET / NATIVE INSTRUMENTS"
                    } else {
                        "WAV / FLAC"
                    },
                ))
                .small()
                .color(MUTED),
            )
            .truncate(),
        );
        ui.horizontal(|ui| {
            let (dot, _) = ui.allocate_exact_size(Vec2::splat(10.0), Sense::hover());
            ui.painter()
                .circle_stroke(dot.center(), 3.0, Stroke::new(1.0_f32, CYAN));
            ui.label(
                egui::RichText::new(if selection.is_some() {
                    "1 item selected"
                } else if self.browser_category == 4 {
                    "Drag onto MIDI track · Click to assign"
                } else {
                    "Drag onto arrangement · Double-click to import"
                })
                .small()
                .color(MUTED),
            );
        });
    }
    fn browser_tree(
        &mut self,
        ui: &mut egui::Ui,
        files: &[PathBuf],
        query: &str,
        depth: usize,
    ) -> usize {
        let mut count = 0;
        for path in files {
            let directory = path.is_dir();
            let name = path.file_name().unwrap_or_default().to_string_lossy();
            // Keep folder paths visible so filtering does not hide navigation.
            if !directory && !name.to_lowercase().contains(query) {
                continue;
            }
            let expanded = self.browser_expanded.contains(path);
            let response = browser_row(
                ui,
                &name,
                if directory {
                    BrowserIcon::Folder
                } else {
                    BrowserIcon::Audio
                },
                self.browser_selected.as_ref() == Some(path),
                depth as f32 * 12.0,
                directory.then_some(expanded),
            )
            .on_hover_text(path.display().to_string());
            if !directory && self.job.is_none() {
                response.dnd_set_drag_payload(SampleDrag { path: path.clone() });
            }
            if response.clicked() {
                self.browser_selected = Some(path.clone());
                if !directory { self.preview_sample(path.clone()); }
                if directory
                    && response
                        .interact_pointer_pos()
                        .is_some_and(|p| p.x < response.rect.left() + depth as f32 * 12.0 + 17.0)
                {
                    if expanded {
                        self.browser_expanded.remove(path);
                    } else {
                        self.browser_expanded.insert(path.clone());
                    }
                }
            }
            if response.double_clicked() {
                if directory {
                    self.browse_folder(path.clone());
                } else if self.job.is_none() {
                    self.import_browser_file(path.clone());
                }
            }
            response.context_menu(|ui| {
                if directory {
                    if ui.button("Open folder").clicked() {
                        self.browse_folder(path.clone());
                        ui.close_menu();
                    }
                } else if ui
                    .add_enabled(
                        self.job.is_none(),
                        egui::Button::new("Import to selected track"),
                    )
                    .clicked()
                {
                    self.import_browser_file(path.clone());
                    ui.close_menu();
                }
            });
            count += usize::from(directory || name.to_lowercase().contains(query));
            if directory && self.browser_expanded.contains(path) && depth < 32 && !path.is_symlink()
            {
                if !self.browser_children.contains_key(path) {
                    match folder_entries(path) {
                        Ok(children) => {
                            self.browser_children.insert(path.clone(), children);
                        }
                        Err(error) => {
                            ui.label(egui::RichText::new(error.to_string()).small().color(ROSE));
                            self.browser_expanded.remove(path);
                        }
                    }
                }
                if let Some(children) = self.browser_children.get(path).cloned() {
                    count += self.browser_tree(ui, &children, query, depth + 1);
                }
            }
        }
        count
    }
}

fn browser_row(
    ui: &mut egui::Ui,
    label: &str,
    icon: BrowserIcon,
    selected: bool,
    indent: f32,
    expanded: Option<bool>,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(
        Vec2::new(ui.available_width(), 21.0),
        Sense::click_and_drag(),
    );
    response.widget_info(|| {
        egui::WidgetInfo::selected(
            egui::WidgetType::SelectableLabel,
            ui.is_enabled(),
            selected,
            label,
        )
    });
    let painter = ui.painter().with_clip_rect(rect);
    let color = if selected {
        Color32::from_rgb(17, 32, 39)
    } else {
        TEXT
    };
    if selected {
        painter.rect_filled(rect, 0.0, BROWSER_SELECTION);
    } else if response.hovered() {
        painter.rect_filled(rect, 0.0, Color32::from_rgb(28, 34, 42));
    }
    let mut x = rect.left() + indent + 5.0;
    if let Some(expanded) = expanded {
        let center = Pos2::new(x + 5.0, rect.center().y);
        let points = if expanded {
            vec![
                center + Vec2::new(-3.0, -2.0),
                center + Vec2::new(3.0, -2.0),
                center + Vec2::new(0.0, 3.0),
            ]
        } else {
            vec![
                center + Vec2::new(-2.0, -3.0),
                center + Vec2::new(3.0, 0.0),
                center + Vec2::new(-2.0, 3.0),
            ]
        };
        painter.add(egui::Shape::convex_polygon(
            points,
            color.gamma_multiply(0.75),
            Stroke::NONE,
        ));
        x += 13.0;
    } else if matches!(icon, BrowserIcon::Audio) {
        x += 13.0;
    }
    let icon_rect =
        Rect::from_center_size(Pos2::new(x + 6.0, rect.center().y), Vec2::new(12.0, 11.0));
    browser_icon(&painter, icon_rect, icon, color);
    let text_rect = Rect::from_min_max(
        Pos2::new(x + 17.0, rect.top()),
        rect.max - Vec2::new(4.0, 0.0),
    );
    let mut job = egui::text::LayoutJob::simple(
        label.into(),
        FontId::monospace(10.0),
        color,
        text_rect.width().max(1.0),
    );
    job.wrap.max_rows = 1;
    job.wrap.break_anywhere = true;
    let galley = ui.fonts(|fonts| fonts.layout_job(job));
    painter.galley(
        Pos2::new(text_rect.left(), rect.center().y - galley.size().y / 2.0),
        galley,
        color,
    );
    response
}
fn browser_icon(painter: &egui::Painter, rect: Rect, icon: BrowserIcon, color: Color32) {
    let stroke = Stroke::new(0.8_f32, color.gamma_multiply(0.85));
    match icon {
        BrowserIcon::Folder => {
            painter.rect_stroke(rect, 0.0, stroke, egui::StrokeKind::Inside);
            painter.line_segment(
                [
                    rect.left_top() + Vec2::new(0.0, -2.0),
                    rect.left_top() + Vec2::new(5.0, -2.0),
                ],
                stroke,
            );
            painter.line_segment(
                [rect.left_top() + Vec2::new(0.0, -2.0), rect.left_top()],
                stroke,
            );
        }
        BrowserIcon::Audio | BrowserIcon::Instrument => {
            painter.rect_stroke(rect, 0.0, stroke, egui::StrokeKind::Inside);
            for (i, height) in [3.0, 7.0, 5.0, 2.0].into_iter().enumerate() {
                let x = rect.left() + 3.0 + i as f32 * 2.0;
                painter.line_segment(
                    [
                        Pos2::new(x, rect.center().y - height / 2.0),
                        Pos2::new(x, rect.center().y + height / 2.0),
                    ],
                    stroke,
                );
            }
        }
        BrowserIcon::Effects => {
            for i in 0..3 {
                let x = rect.left() + 2.0 + i as f32 * 4.0;
                painter.line_segment(
                    [Pos2::new(x, rect.top()), Pos2::new(x, rect.bottom())],
                    stroke,
                );
                painter.line_segment(
                    [
                        Pos2::new(x - 2.0, rect.top() + 3.0 + i as f32 * 2.0),
                        Pos2::new(x + 2.0, rect.top() + 3.0 + i as f32 * 2.0),
                    ],
                    stroke,
                );
            }
        }
        BrowserIcon::Project => {
            painter.rect_stroke(rect, 0.0, stroke, egui::StrokeKind::Inside);
            for y in [3.0, 6.0, 9.0] {
                painter.line_segment(
                    [
                        rect.left_top() + Vec2::new(2.0, y),
                        rect.left_top() + Vec2::new(10.0, y),
                    ],
                    stroke,
                );
            }
        }
        BrowserIcon::All => {
            for i in 0..5 {
                let x = rect.left() + i as f32 * 2.5;
                painter.line_segment(
                    [Pos2::new(x, rect.top()), Pos2::new(x, rect.bottom())],
                    stroke,
                );
            }
        }
    }
}

fn sort_files(files: &mut [PathBuf]) {
    files.sort_by_cached_key(|p| {
        (
            !p.is_dir(),
            p.file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_lowercase(),
            p.clone(),
        )
    });
}
fn folder_entries(folder: &std::path::Path) -> std::io::Result<Vec<PathBuf>> {
    let mut files = std::fs::read_dir(folder)?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.is_dir() || is_audio(p))
        .collect::<Vec<_>>();
    sort_files(&mut files);
    Ok(files)
}
fn samples_settings_file() -> Option<PathBuf> {
    let base = if cfg!(target_os = "windows") {
        std::env::var_os("APPDATA").map(PathBuf::from)
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
    };
    base.map(|p| p.join("Velvet").join("samples-folder.json"))
}
fn read_saved_folder(path: &std::path::Path) -> Option<PathBuf> {
    serde_json::from_slice(&std::fs::read(path).ok()?).ok()
}
fn save_samples_folder(path: &std::path::Path, folder: &std::path::Path) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, serde_json::to_vec(folder)?)?;
    Ok(())
}
fn is_audio(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("wav") || e.eq_ignore_ascii_case("flac"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tree_selection_filtering_and_navigation_preserve_transport() {
        let dir = tempfile::tempdir().unwrap();
        let vocals = dir.path().join("Vocals");
        let drums = dir.path().join("Drums");
        std::fs::create_dir(&vocals).unwrap();
        std::fs::create_dir(&drums).unwrap();
        let voice = vocals.join("voice.wav");
        std::fs::write(&voice, []).unwrap();
        std::fs::write(vocals.join("other.flac"), []).unwrap();
        let ctx = egui::Context::default();
        let mut app = Velvet::new(&ctx);
        app.job = None;
        app.browser_history.clear();
        app.browser_history_index = 0;
        app.session.transport.playing = true;
        app.session.transport.seconds = 4.0;
        assert!(app.browse_folder(dir.path().to_path_buf()));
        assert!(app.browse_folder(vocals.clone()));
        app.browser_history_step(false);
        assert_eq!(app.browser_folder.as_deref(), Some(dir.path()));
        app.browser_history_step(true);
        assert_eq!(app.browser_folder, Some(vocals.clone()));
        app.browser_history_step(false);
        assert!(app.browse_folder(drums.clone()));
        assert_eq!(app.browser_history, [dir.path().to_path_buf(), drums]);
        app.browser_history_step(false);
        app.browser_expanded.insert(vocals);
        app.search = "voice".into();
        let input = |events| egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1000.0, 650.0))),
            events,
            ..Default::default()
        };
        let output = ctx.run(input(vec![]), |ctx| app.browser(ctx));
        let voice_rect = output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.job.text == "voice.wav" => {
                    Some(Rect::from_min_size(text.pos, text.galley.size()))
                }
                _ => None,
            })
            .expect("Expanded folder did not show its matching audio");
        assert!(!output.shapes.iter().any(|shape| matches!(&shape.shape, egui::Shape::Text(text) if text.galley.job.text == "other.flac")));
        let pos = voice_rect.center();
        let button = |pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        let _ = ctx.run(
            input(vec![egui::Event::PointerMoved(pos), button(true)]),
            |ctx| app.browser(ctx),
        );
        let _ = ctx.run(input(vec![button(false)]), |ctx| app.browser(ctx));
        assert_eq!(app.browser_selected, Some(voice));
        assert!(app.sample_preview_job.is_some(), "Single click must request a preview");
        assert!(app.session.transport.playing);
        assert_eq!(app.session.transport.seconds, 4.0);
        assert!(app.job.is_none(), "Single selection imported audio");
    }
    #[test]
    fn selected_folder_restores_and_lists_only_sorted_audio_and_subfolders() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("Mis samples á");
        std::fs::create_dir(&root).unwrap();
        for name in ["Zebra.WAV", "bass.flac", "Alpha.wav", "ignore.png"] {
            std::fs::write(root.join(name), []).unwrap();
        }
        std::fs::create_dir(root.join("Drums")).unwrap();
        let settings = dir.path().join("settings").join("samples-folder.json");
        save_samples_folder(&settings, &root).unwrap();
        let restored = read_saved_folder(&settings).unwrap();
        assert_eq!(restored, root);
        let entries = folder_entries(&restored).unwrap();
        let names: Vec<_> = entries
            .iter()
            .map(|p| p.file_name().unwrap().to_str().unwrap())
            .collect();
        assert_eq!(names, ["Drums", "Alpha.wav", "bass.flac", "Zebra.WAV"]);
        assert!(folder_entries(&root.join("missing")).is_err());
    }
}
