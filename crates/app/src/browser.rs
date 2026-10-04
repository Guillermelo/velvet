use super::*;

impl Velvet {
    fn browse_folder(&mut self, folder: PathBuf) -> bool {
        match folder_entries(&folder) {
            Ok(files) => {
                self.browser_files = files;
                self.browser_folder = Some(folder);
                true
            }
            Err(e) => {
                self.report(Err(e.into()), "");
                false
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
                let result = save_samples_folder(&path, &folder);
                self.report(result, "Samples folder saved · Sorted by name");
            }
        }
    }
    pub(super) fn browser(&mut self, ctx: &egui::Context) {
        egui::SidePanel::left("browser")
            .default_width(264.0)
            .width_range(230.0..=400.0)
            .frame(egui::Frame::new().fill(PANEL).inner_margin(12.0))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    let response = ui.add(
                        egui::TextEdit::singleline(&mut self.search)
                            .id(egui::Id::new("library_search"))
                            .hint_text("Search (Ctrl+F)")
                            .desired_width((ui.available_width() - 36.0).max(100.0)),
                    );
                    if ctx.input(|i| i.modifiers.ctrl && i.key_pressed(egui::Key::F)) {
                        response.request_focus();
                    }
                    if ui.small_button("×").on_hover_text("Clear search").clicked() {
                        self.search.clear();
                    }
                });
                ui.add_space(8.0);
                ui.separator();
                ui.add_space(8.0);
                eyebrow(ui, "LIBRARY");
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    for (category, label) in
                        [(0, "All"), (1, "Samples"), (2, "Effects"), (3, "Project")]
                    {
                        if ui
                            .selectable_label(self.browser_category == category, label)
                            .clicked()
                        {
                            self.browser_category = category;
                        }
                    }
                });
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    ui.add_enabled_ui(self.job.is_none(), |ui| {
                        if ui.button("Import file…").clicked() {
                            if let Some(path) = rfd::FileDialog::new()
                                .add_filter("Audio", &["wav", "flac"])
                                .pick_file()
                            {
                                self.import(
                                    path,
                                    None,
                                    self.session.transport.seconds * self.session.project.tempo.bpm
                                        / 60.0,
                                );
                            }
                        }
                    });
                    if ui
                        .button("Samples folder…")
                        .on_hover_text("Choose and remember your samples folder")
                        .clicked()
                    {
                        if let Some(path) = rfd::FileDialog::new().pick_folder() {
                            self.select_samples_folder(path);
                        }
                    }
                });
                let query = self.search.trim().to_lowercase();
                ui.add_space(12.0);
                eyebrow(
                    ui,
                    if query.is_empty() {
                        "CONTENT"
                    } else {
                        "SEARCH RESULTS"
                    },
                );
                ui.add_space(6.0);
                if self.browser_category != 2 && self.browser_category != 3 {
                    if let Some(folder) = self.browser_folder.clone() {
                        ui.horizontal(|ui| {
                            if ui
                                .add_enabled(
                                    self.samples_root.as_ref() != Some(&folder),
                                    egui::Button::new("Up"),
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
                                    folder.file_name().unwrap_or_default().to_string_lossy(),
                                )
                                .truncate(),
                            )
                            .on_hover_text(folder.display().to_string());
                            ui.label(egui::RichText::new("A–Z").small().color(MUTED));
                        });
                    }
                }
                let mut count = 0;
                let list_height = (ui.available_height() - 140.0).max(80.0);
                egui::ScrollArea::vertical()
                    .id_salt("library_content")
                    .max_height(list_height)
                    .show(ui, |ui| {
                        if self.browser_category != 1 && self.browser_category != 3 {
                            for &(name, kind, description) in velvet_core::BUILTIN_DEVICES {
                                if !format!("{name} {description} {kind}")
                                    .to_lowercase()
                                    .contains(&query)
                                {
                                    continue;
                                }
                                count += 1;
                                ui.horizontal(|ui| {
                                    ui.colored_label(ACCENT, "FX");
                                    ui.add_enabled_ui(
                                        self.selected_track.as_ref().is_some_and(|id| {
                                            self.session.project.devices(id).is_ok()
                                        }) && self.job.is_none(),
                                        |ui| {
                                            if ui
                                                .selectable_label(false, name)
                                                .on_hover_text("Add to selected track")
                                                .clicked()
                                            {
                                                self.execute(Command::AddDevice {
                                                    track_id: self.selected_track.clone().unwrap(),
                                                    kind: kind.into(),
                                                });
                                            }
                                        },
                                    );
                                });
                                ui.label(egui::RichText::new(description).small().color(MUTED));
                                ui.add_space(5.0);
                            }
                        }
                        if self.browser_category != 2 {
                            let files =
                                if self.browser_folder.is_some() && self.browser_category != 3 {
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
                            if !files.is_empty() {
                                ui.add_space(8.0);
                                ui.separator();
                            }
                            for path in files {
                                let name = path.file_name().unwrap_or_default().to_string_lossy();
                                if !name.to_lowercase().contains(&query) {
                                    continue;
                                }
                                count += 1;
                                let directory = path.is_dir();
                                ui.horizontal(|ui| {
                                    ui.label(
                                        egui::RichText::new(if directory {
                                            "DIR".into()
                                        } else {
                                            path.extension()
                                                .unwrap_or_default()
                                                .to_string_lossy()
                                                .to_uppercase()
                                        })
                                        .small()
                                        .color(MUTED),
                                    );
                                    let response = ui
                                        .add(
                                            egui::Button::new(name.as_ref())
                                                .frame(false)
                                                .truncate(),
                                        )
                                        .on_hover_text(path.display().to_string());
                                    if directory && response.clicked() {
                                        self.browse_folder(path.clone());
                                    }
                                    if !directory && response.double_clicked() && self.job.is_none()
                                    {
                                        self.import(
                                            path.clone(),
                                            None,
                                            self.session.transport.seconds
                                                * self.session.project.tempo.bpm
                                                / 60.0,
                                        );
                                    }
                                    response.context_menu(|ui| {
                                        if !directory
                                            && ui
                                                .add_enabled(
                                                    self.job.is_none(),
                                                    egui::Button::new("Import to selected track"),
                                                )
                                                .clicked()
                                        {
                                            self.import(
                                                path.clone(),
                                                None,
                                                self.session.transport.seconds
                                                    * self.session.project.tempo.bpm
                                                    / 60.0,
                                            );
                                            ui.close_menu();
                                        }
                                    });
                                });
                            }
                        }
                        if count == 0 {
                            ui.add_space(12.0);
                            ui.label(
                                egui::RichText::new(if query.is_empty() {
                                    "Add a folder or import audio to browse samples."
                                } else {
                                    "No matching audio or effects."
                                })
                                .color(MUTED),
                            );
                        }
                    });
                ui.add_space(10.0);
                ui.label(
                    egui::RichText::new("Double-click a sample to import.")
                        .small()
                        .color(MUTED),
                );
                ui.with_layout(egui::Layout::bottom_up(egui::Align::LEFT), |ui| {
                    ui.label(
                        egui::RichText::new("External files stay in place.")
                            .small()
                            .color(MUTED),
                    );
                    let missing = self.session.project.missing(&self.session.root).len();
                    if missing > 0 {
                        ui.colored_label(
                            Color32::from_rgb(215, 166, 117),
                            format!("{missing} missing source(s)"),
                        );
                    }
                    if let Some(root) = self.samples_root.clone() {
                        if ui
                            .selectable_label(
                                self.browser_category == 1,
                                format!(
                                    "Samples: {}",
                                    root.file_name().unwrap_or_default().to_string_lossy()
                                ),
                            )
                            .on_hover_text(root.display().to_string())
                            .clicked()
                        {
                            self.browse_folder(root);
                            self.browser_category = 1;
                        }
                    }
                    eyebrow(ui, "PLACES");
                    ui.separator();
                });
            });
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
