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
    ui.heading("Bookkeeper");
    ui.label("Model");

    if ui.button("Refresh models").clicked() {
        app.models_cache = scan_ggufs(app.models_dir.as_deref());
    }
    if app.models_cache.is_empty() {
        app.models_cache = scan_ggufs(app.models_dir.as_deref());
    }
    let model_opts = build_hybrid_model_options_for_lane(
        app.models_dir.as_deref(),
        app.modules_dir.as_deref(),
        &app.prefs.cloud_models,
        ModelLane::Bookkeeper,
    );
    let selected_hint = app.current_bookkeeper_selection();
    let selected_label = selected_model_option_label(
        &model_opts,
        selected_hint.as_deref(),
        app.bookkeeper_model_path.as_ref().map(|p| {
            p.file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| p.display().to_string())
        }),
    );

    if let Some(picked) = show_grouped_model_option_combo(
        ui,
        "bookkeeper_model_combo",
        selected_label,
        &model_opts,
        selected_hint.as_deref(),
    ) {
        app.set_bookkeeper_model_selection(picked);
    }

    if ui.button("Pick model...").clicked() {
        let mut dialog = rfd::FileDialog::new().add_filter("GGUF", &["gguf"]);
        if let Some(dir) = &app.models_dir {
            dialog = dialog.set_directory(dir);
        }
        if let Some(path) = dialog.pick_file() {
            app.set_bookkeeper_model_selection(
                app.portable_model_hint(Some(&path)).map(local_selection_id),
            );
        }
    }
    ui.small("Local-first by default. Cloud entries are optional and can also power search/summaries when configured with embeddings.");

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
        app.prefs.bookkeeper = GenParams {
            temp: app.bookkeeper_temp,
            top_p: app.bookkeeper_top_p,
            top_k: app.bookkeeper_top_k,
            max_tokens: app.bookkeeper_max_tokens,
        };
        app.bookkeeper_restart_due = Some(Instant::now() + Duration::from_millis(600));
    }

    ui.horizontal(|ui| {
        if ui.button("Start/Restart").clicked() {
            if let Some(bk) = &app.bookkeeper {
                bk.shutdown();
            }
            app.bookkeeper = start_bookkeeper(
                app.current_bookkeeper_selection(),
                app.logs_dir.clone(),
                app.models_dir.clone(),
                app.modules_dir.clone(),
                app.prefs.cloud_models.clone(),
                GenParams {
                    temp: app.bookkeeper_temp,
                    top_p: app.bookkeeper_top_p,
                    top_k: app.bookkeeper_top_k,
                    max_tokens: app.bookkeeper_max_tokens,
                },
            );
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

fn show_atlas_lineage_record_side(
    ui: &mut egui::Ui,
    app: &mut ChattyCogApp,
    label: &str,
    atlas_id: &str,
) {
    let Some(record) = app
        .cold_atlas_records
        .iter()
        .find(|record| record.atlas_id == atlas_id)
        .cloned()
    else {
        ui.small(format!("{label}: {atlas_id} [missing record]"));
        return;
    };

    ui.separator();
    ui.horizontal_wrapped(|ui| {
        ui.label(format!("{label}: {}", record.title));
        ui.small(format!("[{}] {}", record.atlas_id, record.status.as_str()));
    });
    ui.small(format!("summary: {}", record.summary));
    if record.evidence_refs.is_empty() {
        ui.small("evidence: none");
        return;
    }

    ui.horizontal_wrapped(|ui| {
        ui.small("Evidence:");
        for (evidence_index, evidence_ref) in record.evidence_refs.iter().enumerate() {
            let inspection_id = format!("{}:{}", record.atlas_id, evidence_index);
            let inspected = app.cold_atlas_inspections.contains_key(&inspection_id);
            let button_label = if inspected {
                format!("Hide {}", evidence_index + 1)
            } else {
                format!("Inspect {}", evidence_index + 1)
            };
            if ui
                .button(button_label)
                .on_hover_text(format!(
                    "Hydrate exact bounded cold-log context for events {}..{} without recalling it into Main AI.",
                    evidence_ref.start_event_index, evidence_ref.end_event_index
                ))
                .clicked()
            {
                app.inspect_cold_atlas_evidence(&record.atlas_id, evidence_index);
            }
        }
    });

    for evidence_index in 0..record.evidence_refs.len() {
        let inspection_id = format!("{}:{}", record.atlas_id, evidence_index);
        if let Some(payload) = app.cold_atlas_inspections.get(&inspection_id) {
            let mut payload = payload.clone();
            egui::ScrollArea::vertical()
                .id_salt(("cold_atlas_lineage_evidence", inspection_id))
                .max_height(180.0)
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    ui.add(
                        egui::TextEdit::multiline(&mut payload)
                            .desired_rows(8)
                            .code_editor()
                            .interactive(false),
                    );
                });
        }
    }
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

    render_semantic_sort_controls(ui, app);
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

fn render_semantic_sort_controls(ui: &mut egui::Ui, app: &mut ChattyCogApp) {
    ui.group(|ui| {
        ui.horizontal_wrapped(|ui| {
            ui.label("Semantic Sort");
            ui.small("Prepare bounded cold-log exchanges, inspect the prompt, then commit validated sorter JSON.");
        });
        ui.horizontal_wrapped(|ui| {
            if ui
                .button("Create starter bucket map")
                .on_hover_text("Create memory/cold_semantic/bucket_map.json only when no map exists.")
                .clicked()
            {
                app.create_default_semantic_bucket_map();
            }
            if ui
                .button("Refresh bucket map")
                .on_hover_text("Inspect the active frozen semantic bucket plan.")
                .clicked()
            {
                app.refresh_semantic_bucket_map_view();
            }
            if ui
                .button("Refresh placements")
                .on_hover_text("Inspect current semantic bucket placements without hydrating transcript text.")
                .clicked()
            {
                app.refresh_semantic_bucket_placement_view();
            }
            if ui
                .button("Review queue")
                .on_hover_text(
                    "Load proposed and needs-review semantic placements for user approval or rejection.",
                )
                .clicked()
            {
                app.refresh_semantic_placement_review();
            }
            if ui
                .button("Refresh receipts")
                .on_hover_text(
                    "Inspect semantic sort/review/sift, Remember recall/attachment/feedback, and cold atlas receipts.",
                )
                .clicked()
            {
                app.refresh_semantic_receipts_view();
            }
            if ui
                .button("Refresh atlas")
                .on_hover_text("Inspect broad cold-atlas shape records without injecting context.")
                .clicked()
            {
                app.refresh_cold_atlas_view();
            }
        });
        ui.collapsing("Recall route preview", |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.add_sized(
                    [220.0, 24.0],
                    egui::TextEdit::singleline(&mut app.recall_route_preview_text)
                        .hint_text("Preview Remember routes"),
                );
                ui.checkbox(&mut app.recall_route_preview_include_misc, "Include misc")
                    .on_hover_text("Include misc semantic bucket hits in the route preview.");
                ui.checkbox(
                    &mut app.recall_route_preview_include_superseded,
                    "Include superseded",
                )
                .on_hover_text("Opt into superseded atlas lineage hits for this preview.");
                if ui
                    .button("Preview")
                    .on_hover_text("Preview atlas, semantic, and direct cold-log lanes without attaching context.")
                    .clicked()
                {
                    app.preview_recall_routes();
                }
                if ui.button("Clear").clicked() {
                    app.recall_route_preview.clear();
                }
            });
            if !app.recall_route_preview.trim().is_empty() {
                let mut preview = app.recall_route_preview.clone();
                egui::ScrollArea::vertical()
                    .id_salt("recall_route_preview_scroll")
                    .max_height(260.0)
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        ui.add(
                            egui::TextEdit::multiline(&mut preview)
                                .desired_rows(12)
                                .code_editor()
                                .interactive(false),
                        );
                    });
            }
        });
        if !app.semantic_bucket_map_view.trim().is_empty() {
            ui.collapsing("Bucket map", |ui| {
                let mut bucket_map_view = app.semantic_bucket_map_view.clone();
                egui::ScrollArea::vertical()
                    .id_salt("semantic_bucket_map_scroll")
                    .max_height(180.0)
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        ui.add(
                            egui::TextEdit::multiline(&mut bucket_map_view)
                                .desired_rows(8)
                                .code_editor()
                                .interactive(false),
                        );
                    });
            });
        }
        if !app.semantic_bucket_placement_view.trim().is_empty() {
            ui.collapsing("Bucket placements", |ui| {
                ui.collapsing("Semantic query preview", |ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.add_sized(
                            [220.0, 24.0],
                            egui::TextEdit::singleline(&mut app.semantic_query_preview_text)
                                .hint_text("Preview bucket recall phrase"),
                        );
                        ui.checkbox(&mut app.semantic_query_preview_include_misc, "Include misc")
                            .on_hover_text("Include the misc / weak-fit bucket. Useful for fuzzy recall, but noisier.");
                        if ui
                            .button("Preview")
                            .on_hover_text("Run the semantic bucket query policy without hydrating, injecting, or promoting context.")
                            .clicked()
                        {
                            app.preview_semantic_bucket_query();
                        }
                        if ui.button("Clear").clicked() {
                            app.semantic_query_preview.clear();
                        }
                    });
                    if !app.semantic_query_preview.trim().is_empty() {
                        let mut preview = app.semantic_query_preview.clone();
                        egui::ScrollArea::vertical()
                            .id_salt("semantic_bucket_query_preview_scroll")
                            .max_height(220.0)
                            .auto_shrink([false, true])
                            .show(ui, |ui| {
                                ui.add(
                                    egui::TextEdit::multiline(&mut preview)
                                        .desired_rows(10)
                                        .code_editor()
                                        .interactive(false),
                                );
                            });
                    }
                });
                ui.separator();
                let mut placement_view = app.semantic_bucket_placement_view.clone();
                egui::ScrollArea::vertical()
                    .id_salt("semantic_bucket_placement_scroll")
                    .max_height(220.0)
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        ui.add(
                            egui::TextEdit::multiline(&mut placement_view)
                                .desired_rows(10)
                                .code_editor()
                                .interactive(false),
                        );
                    });
            });
        }
        if !app.cold_atlas_view.trim().is_empty() {
            ui.collapsing("Cold atlas", |ui| {
                if !app.cold_atlas_records.is_empty() {
                    ui.small("Atlas status marks are user-governed. They write compact receipts and do not inject or promote context.");
                    ui.collapsing("Atlas query preview", |ui| {
                        ui.horizontal_wrapped(|ui| {
                            ui.add_sized(
                                [220.0, 24.0],
                                egui::TextEdit::singleline(&mut app.cold_atlas_query_text)
                                    .hint_text("Preview atlas recall phrase"),
                            );
                            ui.checkbox(
                                &mut app.cold_atlas_query_include_superseded,
                                "Include superseded",
                            )
                            .on_hover_text("Opt into lineage-style lookup. Normal Remember atlas recall excludes superseded records.");
                            if ui
                                .button("Preview")
                                .on_hover_text("Run the atlas query policy without hydrating, injecting, or promoting context.")
                                .clicked()
                            {
                                app.preview_cold_atlas_query();
                            }
                            if ui.button("Clear").clicked() {
                                app.cold_atlas_query_preview.clear();
                            }
                        });
                        if !app.cold_atlas_query_preview.trim().is_empty() {
                            let mut preview = app.cold_atlas_query_preview.clone();
                            egui::ScrollArea::vertical()
                                .id_salt("cold_atlas_query_preview_scroll")
                                .max_height(220.0)
                                .auto_shrink([false, true])
                                .show(ui, |ui| {
                                    ui.add(
                                        egui::TextEdit::multiline(&mut preview)
                                            .desired_rows(10)
                                            .code_editor()
                                            .interactive(false),
                                    );
                                });
                        }
                    });
                    ui.separator();
                    egui::ScrollArea::vertical()
                        .id_salt("cold_atlas_record_controls_scroll")
                        .max_height(180.0)
                        .auto_shrink([false, true])
                        .show(ui, |ui| {
                            for record in app.cold_atlas_records.clone() {
                                ui.group(|ui| {
                                    ui.horizontal_wrapped(|ui| {
                                        ui.label(record.title.clone());
                                        ui.small(format!(
                                            "[{}] {} confidence={:.2} evidence_refs={}",
                                            record.atlas_id,
                                            record.status.as_str(),
                                            record.confidence.score,
                                            record.evidence_refs.len()
                                        ));
                                    });
                                    ui.horizontal_wrapped(|ui| {
                                        if ui
                                            .add_enabled(
                                                record.status != ColdAtlasRecordStatus::NeedsReview,
                                                egui::Button::new("Needs review"),
                                            )
                                            .on_hover_text("Mark this broad atlas shape for user review.")
                                            .clicked()
                                        {
                                            app.mark_cold_atlas_record_status(
                                                &record.atlas_id,
                                                ColdAtlasRecordStatus::NeedsReview,
                                            );
                                        }
                                        if ui
                                            .add_enabled(
                                                record.status != ColdAtlasRecordStatus::Stale,
                                                egui::Button::new("Stale"),
                                            )
                                            .on_hover_text("Mark this atlas shape as historically useful but no longer current enough to trust.")
                                            .clicked()
                                        {
                                            app.mark_cold_atlas_record_status(
                                                &record.atlas_id,
                                                ColdAtlasRecordStatus::Stale,
                                            );
                                        }
                                        if ui
                                            .add_enabled(
                                                record.status != ColdAtlasRecordStatus::Retired,
                                                egui::Button::new("Retire"),
                                            )
                                            .on_hover_text("Retire this atlas shape from recall queries.")
                                            .clicked()
                                        {
                                            app.mark_cold_atlas_record_status(
                                                &record.atlas_id,
                                                ColdAtlasRecordStatus::Retired,
                                            );
                                        }
                                        if ui
                                            .add_enabled(
                                                record.status
                                                    != ColdAtlasRecordStatus::HistoricalShape,
                                                egui::Button::new("Restore shape"),
                                            )
                                            .on_hover_text("Restore this record as broad historical shape memory.")
                                            .clicked()
                                        {
                                            app.mark_cold_atlas_record_status(
                                                &record.atlas_id,
                                                ColdAtlasRecordStatus::HistoricalShape,
                                            );
                                        }
                                        if ui
                                            .button("Revise")
                                            .on_hover_text("Stage this atlas record for a before/after JSON revision review.")
                                            .clicked()
                                        {
                                            app.stage_cold_atlas_revision(&record.atlas_id);
                                        }
                                        if ui
                                            .add_enabled(
                                                record.status != ColdAtlasRecordStatus::Superseded,
                                                egui::Button::new("Supersede"),
                                            )
                                            .on_hover_text("Stage a replacement atlas record and mark this one superseded after preview.")
                                            .clicked()
                                        {
                                            app.stage_cold_atlas_supersede(&record.atlas_id);
                                        }
                                    });
                                    if !record.evidence_refs.is_empty() {
                                        ui.horizontal_wrapped(|ui| {
                                            ui.small("Evidence:");
                                            for (evidence_index, evidence_ref) in
                                                record.evidence_refs.iter().enumerate()
                                            {
                                                let inspection_id =
                                                    format!("{}:{}", record.atlas_id, evidence_index);
                                                let inspected = app
                                                    .cold_atlas_inspections
                                                    .contains_key(&inspection_id);
                                                let label = if inspected {
                                                    format!("Hide {}", evidence_index + 1)
                                                } else {
                                                    format!("Inspect {}", evidence_index + 1)
                                                };
                                                if ui
                                                    .button(label)
                                                    .on_hover_text(format!(
                                                        "Hydrate exact bounded cold-log context for events {}..{} without revising or injecting.",
                                                        evidence_ref.start_event_index,
                                                        evidence_ref.end_event_index
                                                    ))
                                                    .clicked()
                                                {
                                                    app.inspect_cold_atlas_evidence(
                                                        &record.atlas_id,
                                                        evidence_index,
                                                    );
                                                }
                                            }
                                        });
                                        for evidence_index in 0..record.evidence_refs.len() {
                                            let inspection_id =
                                                format!("{}:{}", record.atlas_id, evidence_index);
                                            if let Some(payload) =
                                                app.cold_atlas_inspections.get(&inspection_id)
                                            {
                                                let mut payload = payload.clone();
                                                egui::ScrollArea::vertical()
                                                    .id_salt((
                                                        "cold_atlas_evidence_inspection",
                                                        inspection_id,
                                                    ))
                                                    .max_height(220.0)
                                                    .auto_shrink([false, true])
                                                    .show(ui, |ui| {
                                                        ui.add(
                                                            egui::TextEdit::multiline(&mut payload)
                                                                .desired_rows(10)
                                                                .code_editor()
                                                                .interactive(false),
                                                        );
                                                    });
                                            }
                                        }
                                    }
                                });
                            }
                    });
                    ui.separator();
                }
                if !app.cold_atlas_lineage_view.trim().is_empty() {
                    ui.collapsing("Atlas lineage", |ui| {
                        ui.small("Lineage rows are built from linked receipts. Inspecting evidence here is read-only and does not recall it into Main AI.");
                        if app.cold_atlas_lineage_links.is_empty() {
                            let mut lineage_view = app.cold_atlas_lineage_view.clone();
                            ui.add(
                                egui::TextEdit::multiline(&mut lineage_view)
                                    .desired_rows(4)
                                    .code_editor()
                                    .interactive(false),
                            );
                        } else {
                            egui::ScrollArea::vertical()
                                .id_salt("cold_atlas_lineage_rows_scroll")
                                .max_height(260.0)
                                .auto_shrink([false, true])
                                .show(ui, |ui| {
                                    for link in app.cold_atlas_lineage_links.clone() {
                                        ui.group(|ui| {
                                            ui.small(format!(
                                                "linked at {}",
                                                link.recorded_at_unix_ms
                                            ));
                                            show_atlas_lineage_record_side(
                                                ui,
                                                app,
                                                "Old",
                                                &link.old_atlas_id,
                                            );
                                            show_atlas_lineage_record_side(
                                                ui,
                                                app,
                                                "Replacement",
                                                &link.replacement_atlas_id,
                                            );
                                        });
                                    }
                                });
                        }
                    });
                    ui.separator();
                }
                let mut atlas_view = app.cold_atlas_view.clone();
                egui::ScrollArea::vertical()
                    .id_salt("cold_atlas_scroll")
                    .max_height(220.0)
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        ui.add(
                            egui::TextEdit::multiline(&mut atlas_view)
                                .desired_rows(10)
                                .code_editor()
                                .interactive(false),
                        );
                    });
            });
        }
        ui.collapsing("Cold atlas draft", |ui| {
            if let Some(target_id) = app.cold_atlas_revision_target_id.as_deref() {
                ui.small(format!(
                    "Revision target: {target_id}. Preview shows before/after; commit replaces that atlas record and writes a revised receipt."
                ));
            }
            if let Some(target_id) = app.cold_atlas_supersede_target_id.as_deref() {
                ui.small(format!(
                    "Supersede target: {target_id}. Preview shows the old record to mark superseded and the replacement record to create."
                ));
            }
            ui.horizontal_wrapped(|ui| {
                ui.label("Max refs:");
                ui.add(
                    egui::DragValue::new(&mut app.cold_atlas_max_items)
                        .range(1..=64)
                        .speed(1),
                );
                if ui
                    .button("Prepare atlas")
                    .on_hover_text("Prepare atlas draft evidence from approved semantic placements.")
                    .clicked()
                {
                    app.prepare_cold_atlas_from_approved_placements();
                }
                if ui
                    .add_enabled(
                        app.cold_atlas_prepared.is_some() && app.cold_atlas_draft_rx.is_none(),
                        egui::Button::new("Ask Bookkeeper"),
                    )
                    .on_hover_text("Ask Bookkeeper to draft atlas JSON into the editable field.")
                    .clicked()
                {
                    app.draft_prepared_cold_atlas_with_bookkeeper();
                }
                if ui
                    .add_enabled(
                        app.cold_atlas_prepared.is_some()
                            && !app.cold_atlas_output_json.trim().is_empty(),
                        egui::Button::new("Preview atlas JSON"),
                    )
                    .on_hover_text("Validate the pasted atlas JSON without writing records.")
                    .clicked()
                {
                    app.preview_cold_atlas_json();
                }
                if ui
                    .add_enabled(
                        app.cold_atlas_prepared.is_some()
                            && app.cold_atlas_current_json_is_reviewed(),
                        egui::Button::new("Commit atlas"),
                    )
                    .on_hover_text("Write only the reviewed atlas records and compact receipts.")
                    .clicked()
                {
                    app.commit_cold_atlas_from_json();
                }
            });
            if let Some(prepared) = &app.cold_atlas_prepared {
                if app.cold_atlas_draft_rx.is_some() {
                    ui.small("Bookkeeper is drafting cold atlas JSON...");
                }
                ui.small(format!(
                    "Prepared {} evidence ref(s) from {}. Atlas records orient recall and do not inject context.",
                    prepared.evidence.len(),
                    prepared.source
                ));
                ui.collapsing("Atlas prompt", |ui| {
                    let mut prompt = prepared.prompt.clone();
                    egui::ScrollArea::vertical()
                        .id_salt("cold_atlas_prompt_scroll")
                        .max_height(220.0)
                        .auto_shrink([false, true])
                        .show(ui, |ui| {
                            ui.add(
                                egui::TextEdit::multiline(&mut prompt)
                                    .desired_rows(10)
                                    .code_editor()
                                    .interactive(false),
                            );
                        });
                });
                ui.label("Atlas JSON");
                let output_response = ui.add(
                    egui::TextEdit::multiline(&mut app.cold_atlas_output_json)
                        .desired_rows(8)
                        .code_editor(),
                );
                if output_response.changed() {
                    app.mark_cold_atlas_json_edited();
                }
                if !app.cold_atlas_output_json.trim().is_empty()
                    && !app.cold_atlas_current_json_is_reviewed()
                {
                    ui.small("Atlas JSON changed after preview; preview again before commit.");
                }
                if !app.cold_atlas_review_preview.trim().is_empty() {
                    ui.collapsing("Atlas review preview", |ui| {
                        let mut review_preview = app.cold_atlas_review_preview.clone();
                        egui::ScrollArea::vertical()
                            .id_salt("cold_atlas_review_preview_scroll")
                            .max_height(220.0)
                            .auto_shrink([false, true])
                            .show(ui, |ui| {
                                ui.add(
                                    egui::TextEdit::multiline(&mut review_preview)
                                        .desired_rows(10)
                                        .code_editor()
                                        .interactive(false),
                                );
                            });
                    });
                }
            }
        });
        if !app.semantic_receipts_view.trim().is_empty() {
            ui.collapsing("Governance receipts", |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.label("Filter:");
                    ui.add_sized(
                        [260.0, 24.0],
                        egui::TextEdit::singleline(&mut app.semantic_receipts_filter)
                            .hint_text("recall id, outcome, bucket, query..."),
                    );
                    if ui
                        .button("Clear")
                        .on_hover_text("Clear the receipt view filter without changing receipts.")
                        .clicked()
                    {
                        app.semantic_receipts_filter.clear();
                    }
                });
                let mut receipts_view = filter_governance_receipts_view(
                    &app.semantic_receipts_view,
                    &app.semantic_receipts_filter,
                );
                egui::ScrollArea::vertical()
                    .id_salt("semantic_receipts_scroll")
                    .max_height(220.0)
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        ui.add(
                            egui::TextEdit::multiline(&mut receipts_view)
                                .desired_rows(10)
                                .code_editor()
                                .interactive(false),
                        );
                    });
            });
        }
        ui.collapsing("Bucket sifting", |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label("Bucket:");
                ui.add_sized(
                    [190.0, 24.0],
                    egui::TextEdit::singleline(&mut app.semantic_sift_bucket_id)
                        .hint_text("bucket id"),
                );
                if let Some(map) = app.semantic_review_bucket_map.clone() {
                    egui::ComboBox::from_id_salt("semantic_sift_bucket_picker")
                        .selected_text(if app.semantic_sift_bucket_id.trim().is_empty() {
                            "choose"
                        } else {
                            app.semantic_sift_bucket_id.as_str()
                        })
                        .show_ui(ui, |ui| {
                            for bucket in map.buckets {
                                ui.selectable_value(
                                    &mut app.semantic_sift_bucket_id,
                                    bucket.bucket_id.clone(),
                                    bucket.label,
                                );
                            }
                        });
                }
                if ui
                    .button("Propose sift")
                    .on_hover_text("Find weak-fit, duplicate, or feedback-marked placements in this bucket without applying anything.")
                    .clicked()
                {
                    app.propose_semantic_bucket_sift();
                }
                if ui
                    .add_enabled(
                        app.semantic_sift_proposal
                            .as_ref()
                            .is_some_and(|proposal| {
                                proposal.changes.iter().any(|change| change.selected_for_apply)
                            }),
                        egui::Button::new("Apply selected sift"),
                    )
                    .on_hover_text("Apply only selected sift changes and write a sift receipt.")
                    .clicked()
                {
                    app.apply_selected_semantic_sift_changes();
                }
            });
            if let Some(proposal) = app.semantic_sift_proposal.clone() {
                ui.small(format!(
                    "{} change(s) proposed for {}",
                    proposal.changes.len(),
                    proposal.bucket_id
                ));
                ui.small(format!("rationale: {}", proposal.rationale));
                egui::ScrollArea::vertical()
                    .id_salt("semantic_sift_proposal_scroll")
                    .max_height(260.0)
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        for change in proposal.changes {
                            ui.group(|ui| {
                                ui.horizontal_wrapped(|ui| {
                                    let mut selected = change.selected_for_apply;
                                    if ui.checkbox(&mut selected, "Apply").changed() {
                                        app.set_semantic_sift_change_selected(
                                            &change.change_id,
                                            selected,
                                        );
                                    }
                                    ui.label(format!(
                                        "{:?} {} -> {}",
                                        change.action,
                                        change.from_bucket_id.as_deref().unwrap_or("-"),
                                        change.to_bucket_id.as_deref().unwrap_or("-")
                                    ));
                                    let inspected =
                                        app.semantic_sift_inspections.contains_key(&change.change_id);
                                    let inspect_label =
                                        if inspected { "Hide context" } else { "Inspect context" };
                                    if ui
                                        .button(inspect_label)
                                        .on_hover_text("Hydrate the exact bounded cold-log context for this sift change without applying it.")
                                        .clicked()
                                    {
                                        app.inspect_semantic_sift_change(&change.change_id);
                                    }
                                });
                                ui.small(format!("before: {}", change.before));
                                ui.small(format!("after: {}", change.after));
                                ui.small(format!("rationale: {}", change.rationale));
                                ui.small(format!("id: {}", change.change_id));
                                if let Some(payload) =
                                    app.semantic_sift_inspections.get(&change.change_id)
                                {
                                    let mut payload = payload.clone();
                                    egui::ScrollArea::vertical()
                                        .id_salt(("semantic_sift_inspection", change.change_id))
                                        .max_height(220.0)
                                        .auto_shrink([false, true])
                                        .show(ui, |ui| {
                                            ui.add(
                                                egui::TextEdit::multiline(&mut payload)
                                                    .desired_rows(10)
                                                    .code_editor()
                                                    .interactive(false),
                                            );
                                        });
                                }
                            });
                        }
                    });
            }
        });
        if !app.semantic_review_proposed.is_empty() {
            ui.collapsing("Placement review", |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.label("Bucket:");
                    ui.add_sized(
                        [150.0, 24.0],
                        egui::TextEdit::singleline(&mut app.semantic_review_bucket_filter)
                            .hint_text("any"),
                    );
                    ui.label("Fit:");
                    egui::ComboBox::from_id_salt("semantic_review_fit_filter")
                        .selected_text(if app.semantic_review_fit_filter.trim().is_empty() {
                            "any"
                        } else {
                            app.semantic_review_fit_filter.as_str()
                        })
                        .show_ui(ui, |ui| {
                            ui.selectable_value(
                                &mut app.semantic_review_fit_filter,
                                String::new(),
                                "any",
                            );
                            ui.selectable_value(
                                &mut app.semantic_review_fit_filter,
                                "strong".to_string(),
                                "strong",
                            );
                            ui.selectable_value(
                                &mut app.semantic_review_fit_filter,
                                "medium".to_string(),
                                "medium",
                            );
                            ui.selectable_value(
                                &mut app.semantic_review_fit_filter,
                                "weak".to_string(),
                                "weak",
                            );
                            ui.selectable_value(
                                &mut app.semantic_review_fit_filter,
                                "overflow".to_string(),
                                "overflow",
                            );
                        });
                    if ui.button("Clear filters").clicked() {
                        app.semantic_review_bucket_filter.clear();
                        app.semantic_review_fit_filter.clear();
                    }
                });
                let filtered_review_placements = filter_semantic_review_placements(
                    &app.semantic_review_proposed,
                    &app.semantic_review_bucket_filter,
                    &app.semantic_review_fit_filter,
                );
                ui.horizontal_wrapped(|ui| {
                    if ui
                        .add_enabled(
                            !app.semantic_review_approve_ids.is_empty()
                                || !app.semantic_review_reject_ids.is_empty(),
                            egui::Button::new("Apply selected reviews"),
                        )
                        .on_hover_text("Approve or reject only the selected proposed or needs-review placements.")
                        .clicked()
                    {
                        app.apply_semantic_placement_review();
                    }
                    ui.small(format!(
                        "{} shown, {} reviewable, {} approve, {} reject",
                        filtered_review_placements.len(),
                        app.semantic_review_proposed.len(),
                        app.semantic_review_approve_ids.len(),
                        app.semantic_review_reject_ids.len()
                    ));
                });
                egui::ScrollArea::vertical()
                    .id_salt("semantic_placement_review_scroll")
                    .max_height(260.0)
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        for placement in filtered_review_placements {
                            ui.group(|ui| {
                                let bucket = app
                                    .semantic_review_bucket_map
                                    .as_ref()
                                    .and_then(|map| map.bucket(&placement.bucket_id));
                                let bucket_label = bucket
                                    .map(|bucket| bucket.label.clone())
                                    .unwrap_or_else(|| placement.bucket_id.clone());
                                let bucket_detail = bucket
                                    .map(|bucket| {
                                        format!(
                                            "bucket: {} | {}",
                                            bucket.bucket_id, bucket.description
                                        )
                                    })
                                    .unwrap_or_else(|| format!("bucket: {}", placement.bucket_id));
                                ui.horizontal_wrapped(|ui| {
                                    let mut approve = app
                                        .semantic_review_approve_ids
                                        .contains(&placement.placement_id);
                                    if ui.checkbox(&mut approve, "Approve").changed() {
                                        app.set_semantic_review_approve(
                                            &placement.placement_id,
                                            approve,
                                        );
                                    }
                                    let mut reject = app
                                        .semantic_review_reject_ids
                                        .contains(&placement.placement_id);
                                    if ui.checkbox(&mut reject, "Reject").changed() {
                                        app.set_semantic_review_reject(
                                            &placement.placement_id,
                                            reject,
                                        );
                                    }
                                    ui.label(format!(
                                        "{} / {} / {} events {}..{}",
                                        bucket_label,
                                        placement.review_status.as_str(),
                                        placement.fit.as_str(),
                                        placement.evidence_ref.start_event_index,
                                        placement.evidence_ref.end_event_index
                                    ));
                                    let inspected = app
                                        .semantic_review_inspections
                                        .contains_key(&placement.placement_id);
                                    let inspect_label =
                                        if inspected { "Hide context" } else { "Inspect context" };
                                    if ui
                                        .button(inspect_label)
                                        .on_hover_text("Hydrate the exact bounded cold-log context for this placement without approving or injecting it.")
                                        .clicked()
                                    {
                                        app.inspect_semantic_review_placement(
                                            &placement.placement_id,
                                        );
                                    }
                                });
                                ui.small(bucket_detail);
                                ui.small(format!("reason: {}", placement.reason));
                                ui.small(format!("id: {}", placement.placement_id));
                                if let Some(payload) =
                                    app.semantic_review_inspections.get(&placement.placement_id)
                                {
                                    let mut payload = payload.clone();
                                    egui::ScrollArea::vertical()
                                        .id_salt((
                                            "semantic_placement_inspection",
                                            placement.placement_id.clone(),
                                        ))
                                        .max_height(220.0)
                                        .auto_shrink([false, true])
                                        .show(ui, |ui| {
                                            ui.add(
                                                egui::TextEdit::multiline(&mut payload)
                                                    .desired_rows(10)
                                                    .code_editor()
                                                    .interactive(false),
                                            );
                                        });
                                }
                            });
                        }
                    });
            });
        }
        ui.horizontal_wrapped(|ui| {
            ui.label("Requested queue batch:");
            ui.add(
                egui::DragValue::new(&mut app.semantic_sort_max_items)
                    .range(1..=64)
                    .speed(1),
            );
            if ui
                .button("Prepare sorter prompt")
                .on_hover_text("Collect new interaction exchanges after the latest semantic sort cursor. The host deliberately prepares one compact exchange per model draft so smaller local models stay reliable.")
                .clicked()
            {
                app.prepare_semantic_sort_from_logs();
            }
            if ui
                .add_enabled(
                    app.semantic_sort_prepared.is_some() && app.semantic_sort_draft_rx.is_none(),
                    egui::Button::new("Ask Bookkeeper"),
                )
                .on_hover_text("Draft sorter JSON into the editable box without committing anything.")
                .clicked()
            {
                app.draft_prepared_semantic_sort_with_bookkeeper();
            }
            if ui
                .add_enabled(
                    app.semantic_sort_prepared.is_some()
                        && !app.semantic_sort_output_json.trim().is_empty(),
                    egui::Button::new("Preview JSON"),
                )
                .on_hover_text("Validate the sorter JSON and show the proposed placements before committing.")
                .clicked()
            {
                app.preview_prepared_semantic_sort_json();
            }
            if ui
                .add_enabled(
                    app.semantic_sort_prepared.is_some()
                        && app.semantic_sort_current_json_is_reviewed(),
                    egui::Button::new("Commit pasted JSON"),
                )
                .on_hover_text("Commit only the exact sorter JSON that was successfully previewed.")
                .clicked()
            {
                app.commit_prepared_semantic_sort_from_json();
            }
            ui.label("Reject / skip reason:");
            ui.add(
                egui::TextEdit::singleline(&mut app.semantic_sort_skip_reason)
                    .desired_width(260.0)
                    .hint_text("Why this draft should not be filed"),
            );
            if ui
                .add_enabled(
                    app.semantic_sort_prepared.is_some()
                        && app.semantic_sort_current_json_is_reviewed()
                        && !app.semantic_sort_skip_reason.trim().is_empty(),
                    egui::Button::new("Reject / skip evidence"),
                )
                .on_hover_text("Writes an audit receipt and advances the cursor for this reviewed evidence. It creates no semantic placement. Clear leaves no receipt and does not advance the cursor.")
                .clicked()
            {
                app.skip_prepared_semantic_sort_from_review();
            }
            if ui.button("Clear").clicked() {
                app.clear_semantic_sort_panel();
            }
        });
        ui.small("Host policy: one compact exchange per Bookkeeper draft, 512 output tokens. Commit files reviewed placements; Reject / skip writes only a receipt and advances the cursor; Clear writes nothing.");
        if !app.semantic_sort_status.trim().is_empty() {
            ui.small(app.semantic_sort_status.clone());
        }

        if let Some(prepared) = &app.semantic_sort_prepared {
            ui.horizontal_wrapped(|ui| {
                ui.small(format!(
                    "Prepared {} evidence item(s) from {} after cursor {:?}.",
                    prepared.evidence.len(),
                    prepared.source_log_path,
                    prepared.after_event_index
                ));
            });
            ui.collapsing("Sorter prompt", |ui| {
                let mut prompt = prepared.prompt.clone();
                egui::ScrollArea::vertical()
                    .id_salt("semantic_sort_prompt_scroll")
                    .max_height(220.0)
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        ui.add(
                            egui::TextEdit::multiline(&mut prompt)
                                .desired_rows(10)
                                .code_editor()
                                .interactive(false),
                        );
                    });
            });
            ui.label("Sorter JSON:");
            let json_response = ui.add(
                egui::TextEdit::multiline(&mut app.semantic_sort_output_json)
                    .desired_rows(6)
                    .hint_text("{\"items\":[...]}")
                    .code_editor(),
            );
            if json_response.changed() {
                app.mark_semantic_sort_json_edited();
            }
            if !app.semantic_sort_output_json.trim().is_empty()
                && !app.semantic_sort_current_json_is_reviewed()
            {
                ui.small("Preview the current JSON before commit.");
            }
            if !app.semantic_sort_review_preview.trim().is_empty() {
                ui.collapsing("Review preview", |ui| {
                    let mut review_preview = app.semantic_sort_review_preview.clone();
                    egui::ScrollArea::vertical()
                        .id_salt("semantic_sort_review_preview_scroll")
                        .max_height(240.0)
                        .auto_shrink([false, true])
                        .show(ui, |ui| {
                            ui.add(
                                egui::TextEdit::multiline(&mut review_preview)
                                    .desired_rows(10)
                                    .code_editor()
                                    .interactive(false),
                            );
                        });
                });
            }
        }
    });
}
