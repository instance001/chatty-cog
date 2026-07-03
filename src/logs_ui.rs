use super::*;

pub(super) fn left_sidebar_logs(ui: &mut egui::Ui, app: &mut ChattyCogApp) {
    ui.heading("Logs");
    ui.separator();

    ui.heading("Luke Warm");
    ui.add(egui::Label::new("Rolling summary (auto-updated)").wrap());
    ui.group(|ui| {
        let text = if app.lukewarm_summary.trim().is_empty() {
            "(no summary yet)".to_string()
        } else {
            app.lukewarm_summary.clone()
        };
        egui::ScrollArea::vertical()
            .id_salt("logs_lukewarm_scroll")
            .max_height(140.0)
            .auto_shrink([true, false])
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.add(egui::Label::new(text).wrap());
            });
    });

    ui.separator();
    if let Some(dir) = &app.logs_dir {
        ui.label(format!("Folder: {}", dir.display()));
        if ui.button("Open Folder").clicked() {
            let _ = std::process::Command::new("explorer.exe").arg(dir).spawn();
        }
    } else {
        ui.label("Folder: (not found)");
    }

    ui.separator();
    ui.heading("Bookkeeper (CPU)");
    ui.label("Model");

    if ui.button("Refresh models").clicked() {
        app.models_cache = scan_ggufs(app.models_dir.as_deref());
    }
    if app.models_cache.is_empty() {
        app.models_cache = scan_ggufs(app.models_dir.as_deref());
    }

    egui::ComboBox::from_id_salt("bookkeeper_model_combo")
        .selected_text(
            app.bookkeeper_model_path
                .as_ref()
                .and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_string()))
                .unwrap_or_else(|| "(none)".to_string()),
        )
        .show_ui(ui, |ui| {
            if ui
                .selectable_value(&mut app.bookkeeper_model_path, None, "(none)")
                .changed()
            {
                app.bookkeeper_restart_due = Some(Instant::now() + Duration::from_millis(600));
            }
            for p in &app.models_cache {
                let label = p
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string();
                if ui
                    .selectable_value(&mut app.bookkeeper_model_path, Some(p.clone()), label)
                    .changed()
                {
                    app.bookkeeper_restart_due = Some(Instant::now() + Duration::from_millis(600));
                }
            }
        });

    if ui.button("Pick model...").clicked() {
        let mut dialog = rfd::FileDialog::new().add_filter("GGUF", &["gguf"]);
        if let Some(dir) = &app.models_dir {
            dialog = dialog.set_directory(dir);
        }
        if let Some(path) = dialog.pick_file() {
            app.bookkeeper_model_path = Some(path);
            app.bookkeeper_restart_due = Some(Instant::now() + Duration::from_millis(600));
        }
    }

    ui.separator();
    ui.heading("Params");
    add_presets_bookkeeper(ui, app);
    let mut changed = false;
    changed |= ui
        .add(egui::Slider::new(&mut app.bookkeeper_temp, 0.0..=2.0).text("temp"))
        .changed();
    changed |= ui
        .add(egui::Slider::new(&mut app.bookkeeper_top_p, 0.0..=1.0).text("top_p"))
        .changed();
    changed |= ui
        .add(egui::Slider::new(&mut app.bookkeeper_top_k, 0..=200).text("top_k"))
        .changed();
    changed |= ui
        .add(egui::Slider::new(&mut app.bookkeeper_max_tokens, 1..=4096).text("max_tokens"))
        .changed();
    if changed {
        app.bookkeeper_restart_due = Some(Instant::now() + Duration::from_millis(600));
    }

    ui.horizontal(|ui| {
        if ui.button("Start/Restart").clicked() {
            if let Some(bk) = &app.bookkeeper {
                bk.shutdown();
            }
            app.bookkeeper =
                start_bookkeeper(app.bookkeeper_model_path.clone(), app.logs_dir.clone());
        }
        if ui.button("Stop").clicked() {
            if let Some(bk) = &app.bookkeeper {
                bk.shutdown();
            }
            app.bookkeeper = None;
            app.bookkeeper_restart_due = None;
        }
    });
}

pub(super) fn logs_tab(ui: &mut egui::Ui, app: &mut ChattyCogApp) {
    ui.heading("Logs");
    ui.separator();

    ui.horizontal_wrapped(|ui| {
        ui.label("Filter module:");
        ui.add_sized(
            [180.0, 24.0],
            egui::TextEdit::singleline(&mut app.logs_filter_module).hint_text("any"),
        );
        ui.label("Tag:");
        ui.add_sized(
            [140.0, 24.0],
            egui::TextEdit::singleline(&mut app.logs_filter_tag).hint_text("any"),
        );
        if ui.button("Clear filters").clicked() {
            app.logs_filter_module.clear();
            app.logs_filter_tag.clear();
        }
    });

    ui.horizontal_wrapped(|ui| {
        ui.label("Semantic:");
        let semantic_width = (ui.available_width() - 64.0).max(220.0);
        ui.add_sized(
            [semantic_width, 28.0],
            egui::TextEdit::singleline(&mut app.logs_query_semantic).hint_text("Ask bookkeeper..."),
        );
        if ui.button("Search").clicked() {
            if let Some(bk) = &app.bookkeeper {
                app.logs_results_semantic = bk
                    .search(
                        app.logs_query_semantic.clone(),
                        Some(app.logs_filter_module.clone()),
                        Some(app.logs_filter_tag.clone()),
                        16,
                    )
                    .ok()
                    .unwrap_or_default();
            } else {
                app.logs_results_semantic.clear();
                app.logs_results_keyword =
                    vec!["Bookkeeper not running. Use Logs sidebar to Start.".to_string()];
            }
        }
    });

    ui.horizontal_wrapped(|ui| {
        ui.label("Keyword:");
        let keyword_width = (ui.available_width() - 48.0).max(220.0);
        ui.add_sized(
            [keyword_width, 28.0],
            egui::TextEdit::singleline(&mut app.logs_query_keyword)
                .hint_text("Search cold_log.jsonl..."),
        );
        if ui.button("Find").clicked() {
            app.logs_results_keyword =
                keyword_search_cold_log(app.logs_dir.as_deref(), &app.logs_query_keyword, 50);
        }
    });

    ui.add_space(8.0);

    ui.group(|ui| {
        ui.add(egui::Label::new("New cold-log event (schemaless)").wrap());
        ui.horizontal_wrapped(|ui| {
            ui.label("Module/Dept:");
            ui.add_sized(
                [160.0, 24.0],
                egui::TextEdit::singleline(&mut app.logs_new_module).hint_text("general"),
            );
            ui.label("Type:");
            ui.add_sized(
                [120.0, 24.0],
                egui::TextEdit::singleline(&mut app.logs_new_event_type).hint_text("note"),
            );
            ui.label("Tags:");
            let tags_width = ui.available_width().max(180.0);
            ui.add_sized(
                [tags_width, 24.0],
                egui::TextEdit::singleline(&mut app.logs_new_tags).hint_text("comma,separated"),
            );
        });
        ui.label("Summary:");
        ui.add(
            egui::TextEdit::multiline(&mut app.logs_new_summary)
                .desired_rows(2)
                .hint_text("Short human summary..."),
        );
        ui.label("Payload JSON (optional):");
        ui.add(
            egui::TextEdit::multiline(&mut app.logs_new_payload_json)
                .desired_rows(3)
                .hint_text("{\"anything\": \"goes\"}"),
        );
        if ui.button("Append to cold log").clicked() {
            let summary = app.logs_new_summary.trim().to_string();
            if !summary.is_empty() {
                let tags = app
                    .logs_new_tags
                    .split(',')
                    .map(|s| s.trim())
                    .filter(|s| !s.is_empty())
                    .map(|s| s.to_string())
                    .collect::<Vec<_>>();
                let payload = app.logs_new_payload_json.trim().to_string();
                let payload = if payload.is_empty() {
                    None
                } else {
                    Some(payload)
                };
                if let Some(bk) = &app.bookkeeper {
                    bk.append_module_event(
                        app.logs_new_module.trim().to_string(),
                        app.logs_new_event_type.trim().to_string(),
                        summary,
                        tags,
                        payload,
                    );
                }
                app.logs_new_summary.clear();
            }
        }
    });

    ui.columns(2, |cols| {
        cols[0].heading("Results");
        egui::ScrollArea::vertical()
            .id_salt("logs_results_scroll")
            .show(&mut cols[0], |ui| {
                if !app.logs_results_semantic.is_empty() {
                    ui.label("Semantic hits:");
                    for h in &app.logs_results_semantic {
                        let module = h.module.clone().unwrap_or_else(|| "-".to_string());
                        let ty = h.event_type.clone().unwrap_or_else(|| "-".to_string());
                        let tags = if h.tags.is_empty() {
                            String::new()
                        } else {
                            format!(" tags=[{}]", h.tags.join(", "))
                        };
                        ui.label(format!(
                            "{:.3} [{}] ({}/{}){} {}",
                            h.score, h.source, module, ty, tags, h.text
                        ));
                    }
                    ui.separator();
                }
                if !app.logs_results_keyword.is_empty() {
                    ui.label("Keyword hits:");
                    for l in &app.logs_results_keyword {
                        ui.label(l);
                    }
                }
                if app.logs_results_semantic.is_empty() && app.logs_results_keyword.is_empty() {
                    ui.label("No results yet.");
                }
            });

        cols[1].heading("Log Folder");
        egui::ScrollArea::vertical()
            .id_salt("logs_folder_scroll")
            .show(&mut cols[1], |ui| {
                let Some(dir) = &app.logs_dir else {
                    ui.label("No logs dir.");
                    return;
                };
                for p in list_dir_files(dir) {
                    let name = p
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .to_string();
                    if ui
                        .selectable_label(app.logs_selected.as_ref() == Some(&p), name)
                        .clicked()
                    {
                        app.logs_selected = Some(p.clone());
                        app.logs_view =
                            read_text_file(&p, 200_000).unwrap_or_else(|e| format!("Error: {e:#}"));
                    }
                }
            });
    });

    ui.separator();
    ui.heading("Preview");
    egui::ScrollArea::vertical()
        .id_salt("logs_preview_scroll")
        .show(ui, |ui| {
            ui.add(
                egui::TextEdit::multiline(&mut app.logs_view)
                    .desired_rows(12)
                    .code_editor(),
            );
        });
}
