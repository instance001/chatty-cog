use super::*;

pub(super) fn left_sidebar_settings(ui: &mut egui::Ui, _app: &mut ChattyCogApp) {
    ui.heading("Settings");
    ui.separator();
    ui.add(
        egui::Label::new("Global appearance and default behavior live in Preferences for now.")
            .wrap(),
    );
}

pub(super) fn left_sidebar_sandbox(ui: &mut egui::Ui, app: &mut ChattyCogApp) {
    ui.heading("Chatty Sandbox");
    ui.separator();

    if let Some(dir) = app.sandbox_dir.clone() {
        ui.label(format!("Folder: {}", dir.display()));
        ui.small(format!(
            "Scratchpad: Chatty_Sandbox/{}",
            DEFAULT_SANDBOX_SCRATCHPAD_REL_PATH
        ));
        ui.small(format!(
            "Task ledger: Chatty_Sandbox/{}",
            DEFAULT_SANDBOX_TASK_LEDGER_REL_PATH
        ));
        ui.horizontal(|ui| {
            if ui.button("Open Folder").clicked() {
                let _ = std::process::Command::new("explorer.exe").arg(&dir).spawn();
            }
            if ui.button("Open Scratchpad").clicked() {
                app.open_default_sandbox_scratchpad();
            }
            if ui.button("Open Task Ledger").clicked() {
                app.open_default_sandbox_task_ledger();
            }
            if ui.button("Refresh").clicked() {
                app.ensure_default_sandbox_scratchpad();
                app.ensure_default_sandbox_task_ledger();
                app.sandbox_status = "Refreshed".to_string();
            }
        });
    } else {
        ui.label("Folder: (not found)");
        if ui.button("Locate/Create").clicked() {
            app.sandbox_dir = find_or_create_sandbox_dir();
            app.ensure_default_sandbox_scratchpad();
            app.ensure_default_sandbox_task_ledger();
        }
    }

    if !app.sandbox_status.trim().is_empty() {
        ui.add_space(6.0);
        ui.label(app.sandbox_status.clone());
    }
}

pub(super) fn left_sidebar_about(ui: &mut egui::Ui, _app: &mut ChattyCogApp) {
    ui.heading("About");
    ui.separator();
    ui.label("ChattyCog • Rust GUI");
}

pub(super) fn sandbox_tab(ui: &mut egui::Ui, app: &mut ChattyCogApp) {
    ui.heading("Sandbox");
    ui.separator();

    let Some(dir) = app.sandbox_dir.clone() else {
        ui.label("Sandbox folder not found. Create `Chatty_Sandbox/` inside the app folder.");
        return;
    };

    if let Some(p) = app.sandbox_editor_path.clone() {
        if ensure_save_path_within_dir(&dir, &p).is_err() {
            app.sandbox_editor_path = None;
            app.sandbox_status = "Blocked unsafe sandbox path.".to_string();
        }
    }

    ui.group(|ui| {
        ui.heading("Scratchpad");
        ui.small(
            "Persistent working notes for ChattyCog. The chat prompt can see this file, and the model can request writes/appends to it through the sandbox tool flow.",
        );
        ui.horizontal(|ui| {
            if ui.button("Open default scratchpad").clicked() {
                app.open_default_sandbox_scratchpad();
            }
            if ui.button("Append hot memory snapshot").clicked() {
                let snapshot = if app.hot_memory.is_empty() {
                    String::new()
                } else {
                    let mut text = format!("# Hot memory snapshot ({})\n", now_unix_ms().max(0));
                    for item in &app.hot_memory {
                        text.push_str("- ");
                        text.push_str(item);
                        text.push('\n');
                    }
                    text.push('\n');
                    text
                };
                if snapshot.trim().is_empty() {
                    app.sandbox_status = "Hot memory is empty.".to_string();
                } else {
                    match sandbox_append(&dir, DEFAULT_SANDBOX_SCRATCHPAD_REL_PATH, &snapshot) {
                        Ok(path) => {
                            app.sandbox_status = format!("Appended hot memory to {}", path.display());
                            app.open_sandbox_file_in_editor(&path);
                        }
                        Err(err) => {
                            app.sandbox_status =
                                format!("Could not append hot memory snapshot: {err}");
                        }
                    }
                }
            }
        });
    });

    ui.add_space(8.0);

    ui.group(|ui| {
        ui.heading("Task Ledger");
        ui.small(
            "Structured durable state for longer tasks: current task, next step, open questions, and files touched.",
        );
        ui.horizontal(|ui| {
            if ui.button("Open task ledger").clicked() {
                app.open_default_sandbox_task_ledger();
            }
            if ui.button("Seed from current context").clicked() {
                app.seed_default_sandbox_task_ledger_from_context();
            }
        });
    });

    ui.add_space(8.0);

    ui.columns(2, |cols| {
        let image_preview_path = app
            .sandbox_editor_path
            .as_ref()
            .filter(|path| path_uses_inline_image_preview(path))
            .cloned();
        cols[0].heading("Files");
        egui::ScrollArea::vertical()
            .id_salt("sandbox_files_scroll")
            .show(&mut cols[0], |ui| {
                for p in list_sandbox_files(&dir) {
                    let name = p
                        .strip_prefix(&dir)
                        .unwrap_or(&p)
                        .to_string_lossy()
                        .replace('\\', "/");
                    if ui
                        .selectable_label(app.sandbox_selected.as_ref() == Some(&p), name)
                        .clicked()
                    {
                        app.open_sandbox_file_in_editor(&p);
                    }
                }
            });

        cols[1].heading("Editor");
        let ledger_summary = read_task_ledger_summary(&dir);
        cols[1].group(|ui| {
            ui.horizontal_wrapped(|ui| {
                ui.heading("Task Ledger Snapshot");
                if let Some(summary) = ledger_summary.as_ref() {
                    if !summary.status.trim().is_empty() {
                        ui.small(format!("Status: {}", summary.status.trim()));
                    }
                }
                if ui.button("Open ledger").clicked() {
                    app.open_default_sandbox_task_ledger();
                }
            });
            if let Some(summary) = ledger_summary.as_ref() {
                ui.small(
                    "Read-only summary of the structured task ledger. Use the ledger itself for edits.",
                );
                ui.add_space(4.0);
                ui.label(format!(
                    "Current task: {}",
                    if summary.current_task.trim().is_empty() {
                        "(not set)"
                    } else {
                        summary.current_task.trim()
                    }
                ));
                ui.label(format!(
                    "Next step: {}",
                    if summary.next_step.trim().is_empty() {
                        "(not set)"
                    } else {
                        summary.next_step.trim()
                    }
                ));
                ui.horizontal_wrapped(|ui| {
                    ui.small(format!("Open questions: {}", summary.open_questions.len()));
                    ui.small(format!("Files touched: {}", summary.files_touched.len()));
                    ui.small(format!("Working notes: {}", summary.notes.len()));
                });

                if !summary.open_questions.is_empty() {
                    ui.add_space(4.0);
                    ui.small("Open questions:");
                    for item in summary.open_questions.iter().take(3) {
                        ui.small(format!("- {}", truncate_for_ui(item, 120)));
                    }
                }
                if !summary.files_touched.is_empty() {
                    ui.add_space(4.0);
                    ui.small(format!(
                        "Recent files: {}",
                        truncate_for_ui(&summary.files_touched.join(", "), 180)
                    ));
                }
            } else {
                ui.small("Task ledger not available yet.");
            }
        });
        cols[1].add_space(8.0);
        cols[1].horizontal_wrapped(|ui| {
            ui.add_enabled_ui(image_preview_path.is_none(), |ui| {
                if ui.button("New scratch").clicked() {
                    app.sandbox_editor_path = None;
                    app.sandbox_editor_text.clear();
                    app.sandbox_status = "New scratch buffer".to_string();
                }
                if ui.button("Append summary to hot memory").clicked() {
                    app.append_editor_summary_to_hot_memory();
                }
                if ui.button("Use as current task").clicked() {
                    app.set_task_ledger_field_from_editor(true);
                }
                if ui.button("Use as next step").clicked() {
                    app.set_task_ledger_field_from_editor(false);
                }
                if ui.button("Promote to scratchpad").clicked() {
                    app.promote_editor_text_to_scratchpad();
                }
                if ui.button("Promote to ledger notes").clicked() {
                    app.promote_editor_text_to_ledger_notes();
                }
                if ui.button("Save as...").clicked() {
                    if let Some(path) = rfd::FileDialog::new().set_directory(&dir).save_file() {
                        match ensure_save_path_within_dir(&dir, &path).and_then(|pp| {
                            std::fs::write(&pp, &app.sandbox_editor_text)
                                .with_context(|| format!("write {}", pp.display()))?;
                            Ok(pp)
                        }) {
                            Ok(pp) => {
                                app.sandbox_editor_path = Some(pp.clone());
                                app.sandbox_status = format!("Saved {}", pp.display());
                            }
                            Err(e) => app.sandbox_status = format!("Save blocked/failed: {e}"),
                        }
                    }
                }
                if ui.button("Save").clicked() {
                    if let Some(path) = &app.sandbox_editor_path {
                        match ensure_save_path_within_dir(&dir, path).and_then(|pp| {
                            std::fs::write(&pp, &app.sandbox_editor_text)
                                .with_context(|| format!("write {}", pp.display()))?;
                            Ok(pp)
                        }) {
                            Ok(pp) => {
                                app.sandbox_editor_path = Some(pp.clone());
                                app.sandbox_status = format!("Saved {}", pp.display());
                            }
                            Err(e) => app.sandbox_status = format!("Save blocked/failed: {e}"),
                        }
                    } else {
                        app.sandbox_status = "No file path. Use Save as...".to_string();
                    }
                }
            });
            if image_preview_path.is_some() {
                ui.small("Read-only image preview");
            }
        });

        egui::ScrollArea::vertical()
            .id_salt("sandbox_editor_scroll")
            .show(&mut cols[1], |ui| {
                if let Some(image_path) = image_preview_path.as_ref() {
                    if let Some(texture) = load_local_png_texture(
                        ui.ctx(),
                        image_path,
                        &format!("sandbox_preview_{}", image_path.display()),
                    ) {
                        ui.small("PNG preview");
                        ui.add_space(6.0);
                        let size = texture.size_vec2();
                        let max = egui::vec2(720.0, 520.0);
                        let scale = (max.x / size.x).min(max.y / size.y).min(1.0);
                        ui.image((texture.id(), size * scale));
                    } else {
                        ui.small("Could not preview this image file.");
                    }
                } else {
                    ui.add(
                        egui::TextEdit::multiline(&mut app.sandbox_editor_text)
                            .desired_rows(24)
                            .code_editor(),
                    );
                }
            });
    });
}

pub(super) fn settings_tab(ui: &mut egui::Ui, _app: &mut ChattyCogApp) {
    ui.heading("Settings");
    ui.separator();
    ui.label("Planned:");
    ui.label("- Keyboard shortcuts");
    ui.label("- Appearance (old-school theme presets)");
    ui.label("- Default model folder");
}

pub(super) fn about_tab(ui: &mut egui::Ui) {
    ui.heading("ChattyCog");
    ui.separator();
    ui.label("Old-school, tabbed desktop UI for local-first, cloud-optional AI work.");
    ui.add_space(8.0);
    ui.label("Status: local llama.cpp runtime wired, with optional BYO cloud lanes.");
    ui.add_space(12.0);
    ui.group(|ui| {
        ui.heading("Project Identity");
        ui.small("Compact stewardship surface for the host shell.");
        ui.add_space(4.0);
        ui.label("Publisher / steward: Fractal Media Infrastructure");
        ui.label("GitHub: instance001");
        ui.label("License: GNU Affero General Public License v3.0");
        ui.label("ChattyCog is published under FMI, a small independent R&D umbrella for open-source AI tooling, cognitive scaffolding experiments, and local-first research systems.");
        ui.add_space(6.0);
        ui.horizontal_wrapped(|ui| {
            ui.hyperlink_to(
                "Repository: instance001/chatty-cog",
                "https://github.com/instance001/chatty-cog",
            );
            ui.separator();
            ui.hyperlink_to(
                "Publisher GitHub: instance001",
                "https://github.com/instance001",
            );
        });
    });
}
