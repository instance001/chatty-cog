use super::*;

fn render_models_prefs_header(ui: &mut egui::Ui, app: &mut ChattyCogApp) {
    ui.horizontal_wrapped(|ui| {
        ui.label(format!("Prefs file: {}", app.prefs_path.display()));
        if ui.button("Reload").clicked() {
            match preferences::load_prefs(&app.prefs_path) {
                Ok(p) => {
                    app.prefs = p;
                    app.ensure_persisted_network_identity();
                    app.apply_prefs_to_runtime_settings();
                    app.sync_capsule_selection_from_prefs();
                    app.networking
                        .set_device_name(&app.prefs.network_device_name);
                    app.networking
                        .set_allow_unknown_devices(app.prefs.network_allow_unknown_devices);
                    let blocked = app
                        .prefs
                        .network_blocked_devices
                        .iter()
                        .map(|peer| BlockedPeer {
                            device_id: peer.device_id.clone(),
                            device_name: peer.device_name.clone(),
                            address: String::new(),
                            last_seen_secs_ago: None,
                        })
                        .collect::<Vec<_>>();
                    app.networking.replace_blocked_peers(&blocked);
                    let trusted = app
                        .prefs
                        .network_trusted_devices
                        .iter()
                        .map(|peer| TrustedPeer {
                            device_id: peer.device_id.clone(),
                            device_name: peer.device_name.clone(),
                            address: String::new(),
                            last_seen_secs_ago: None,
                        })
                        .collect::<Vec<_>>();
                    app.networking.replace_trusted_peers(&trusted);
                    app.networking_device_name_input =
                        app.networking.snapshot().device_name.clone();
                    app.prefs_status = "Reloaded preferences.".to_string();
                }
                Err(e) => app.prefs_status = format!("Reload failed: {e}"),
            }
        }
        if ui.button("Save").clicked() {
            app.ensure_persisted_network_identity();
            match preferences::save_prefs(&app.prefs_path, &app.prefs) {
                Ok(()) => app.prefs_status = "Saved preferences.".to_string(),
                Err(e) => app.prefs_status = format!("Save failed: {e}"),
            }
        }
    });

    if !app.prefs_status.trim().is_empty() {
        ui.small(app.prefs_status.clone());
    }
}

fn render_models_orchestrator_prefs(ui: &mut egui::Ui, app: &mut ChattyCogApp) {
    ui.group(|ui| {
        ui.heading("Orchestrator (Chat)");
        let mut live_changed = false;
        ui.horizontal(|ui| {
            if ui.button("Copy from current").clicked() {
                app.prefs.orchestrator.temp = app.orch_temp;
                app.prefs.orchestrator.top_p = app.orch_top_p;
                app.prefs.orchestrator.top_k = app.orch_top_k;
                app.prefs.orchestrator.max_tokens = app.orch_max_tokens;
                app.prefs_status = "Copied orchestrator settings.".to_string();
            }
            if ui.button("Apply to current").clicked() {
                app.orch_temp = app.prefs.orchestrator.temp;
                app.orch_top_p = app.prefs.orchestrator.top_p;
                app.orch_top_k = app.prefs.orchestrator.top_k;
                app.orch_max_tokens = app.prefs.orchestrator.max_tokens;
                app.prefs_status = "Applied orchestrator settings.".to_string();
            }
        });
        add_presets_prefs_orchestrator(ui, &mut app.prefs.orchestrator);
        live_changed |= ui
            .add(egui::Slider::new(&mut app.prefs.orchestrator.temp, 0.0..=2.0).text("temp"))
            .changed();
        live_changed |= ui
            .add(egui::Slider::new(&mut app.prefs.orchestrator.top_p, 0.0..=1.0).text("top_p"))
            .changed();
        live_changed |= ui
            .add(egui::Slider::new(&mut app.prefs.orchestrator.top_k, 0..=200).text("top_k"))
            .changed();
        live_changed |= ui
            .add(
                egui::Slider::new(&mut app.prefs.orchestrator.max_tokens, 1..=4096)
                    .text("max_tokens"),
            )
            .changed();
        if live_changed {
            app.apply_live_orchestrator_prefs();
            app.prefs_status = format!(
                "Live chat settings updated. Chat max tokens now {}.",
                app.orch_max_tokens
            );
        }
    });
}

fn render_models_bookkeeper_prefs(ui: &mut egui::Ui, app: &mut ChattyCogApp) {
    ui.group(|ui| {
        ui.heading("Bookkeeper (CPU)");
        ui.horizontal(|ui| {
            if ui.button("Copy from current").clicked() {
                app.prefs.bookkeeper.temp = app.bookkeeper_temp;
                app.prefs.bookkeeper.top_p = app.bookkeeper_top_p;
                app.prefs.bookkeeper.top_k = app.bookkeeper_top_k;
                app.prefs.bookkeeper.max_tokens = app.bookkeeper_max_tokens;
                app.prefs_status = "Copied bookkeeper settings.".to_string();
            }
            if ui.button("Apply to current").clicked() {
                app.bookkeeper_temp = app.prefs.bookkeeper.temp;
                app.bookkeeper_top_p = app.prefs.bookkeeper.top_p;
                app.bookkeeper_top_k = app.prefs.bookkeeper.top_k;
                app.bookkeeper_max_tokens = app.prefs.bookkeeper.max_tokens;
                app.bookkeeper_restart_due = Some(Instant::now() + Duration::from_millis(200));
                app.prefs_status = "Applied bookkeeper settings (restart pending).".to_string();
            }
        });
        add_presets_prefs_bookkeeper(ui, &mut app.prefs.bookkeeper);
        ui.add(egui::Slider::new(&mut app.prefs.bookkeeper.temp, 0.0..=2.0).text("temp"));
        ui.add(egui::Slider::new(&mut app.prefs.bookkeeper.top_p, 0.0..=1.0).text("top_p"));
        ui.add(egui::Slider::new(&mut app.prefs.bookkeeper.top_k, 0..=200).text("top_k"));
        ui.add(
            egui::Slider::new(&mut app.prefs.bookkeeper.max_tokens, 1..=4096).text("max_tokens"),
        );
    });
}

fn render_models_access_tools_prefs(ui: &mut egui::Ui, app: &mut ChattyCogApp) {
    ui.group(|ui| {
        ui.heading("Access / Tools");
        ui.checkbox(
            &mut app.prefs.allow_sandbox_tool_requests,
            "Allow sandbox tool requests (user-approved)",
        );
        ui.small("If disabled, Chat tab won't parse tool JSON requests and will hide the approval panel.");
        ui.add_space(6.0);
        ui.checkbox(
            &mut app.prefs.auto_generate_module_suspend_rundown,
            "Auto-generate module suspend rundown on tab leave (Bookkeeper)",
        );
        ui.small(
            "If enabled, leaving a module tab will auto-write a short department update into cold logs for cross-module awareness.",
        );
    });
}

fn render_models_module_prefs(ui: &mut egui::Ui, app: &mut ChattyCogApp) {
    ui.group(|ui| {
        ui.heading("Per-module preferences");
        ui.small("Defaults for modules that have AI enabled (or future module runners).");

        let model_opts = build_model_options(app.models_dir.as_deref(), app.modules_dir.as_deref());
        let modules = app.module_registry.modules.clone();
        if modules.is_empty() {
            ui.label("(no modules discovered)");
            return;
        }

        for m in modules {
            ui.push_id(&m.module_id, |ui| {
                let entry = app
                    .prefs
                    .modules
                    .entry(m.module_id.clone())
                    .or_insert_with(ModulePreferences::default);

                ui.separator();
                ui.label(format!("{} ({})", m.display_name, m.module_id));

                let selected = entry.preferred_model.clone().unwrap_or_default();
                let selected_label = selected_model_option_label(
                    &model_opts,
                    entry.preferred_model.as_deref(),
                    if selected.is_empty() { None } else { Some(selected.clone()) },
                );

                if let Some(picked) = show_grouped_model_option_combo(
                    ui,
                    ("preferred_model", m.module_id.as_str()),
                    selected_label,
                    &model_opts,
                    entry.preferred_model.as_deref(),
                ) {
                    entry.preferred_model = picked;
                }

                add_presets_prefs_orchestrator(ui, &mut entry.params);
                ui.add(egui::Slider::new(&mut entry.params.temp, 0.0..=2.0).text("temp"));
                ui.add(egui::Slider::new(&mut entry.params.top_p, 0.0..=1.0).text("top_p"));
                ui.add(egui::Slider::new(&mut entry.params.top_k, 0..=200).text("top_k"));
                ui.add(
                    egui::Slider::new(&mut entry.params.max_tokens, 1..=4096).text("max_tokens"),
                );
                ui.checkbox(
                    &mut entry.allow_receive_lukewarm_context,
                    "Allow luke warm context",
                );
            });
        }
    });
}

fn render_models_capsule_library(ui: &mut egui::Ui, app: &mut ChattyCogApp) {
    ui.group(|ui| {
        ui.heading("Capsule Library");
        ui.small(
            "Save reusable personality or behavior injections here, then activate one when a task needs a different tone, role, or voice.",
        );
        ui.add_space(6.0);

        let capsule_names = app
            .prefs
            .orchestrator_capsules
            .iter()
            .map(|capsule| capsule.name.clone())
            .collect::<Vec<_>>();
        let active_label = app
            .prefs
            .active_orchestrator_capsule
            .as_deref()
            .filter(|name| !name.trim().is_empty())
            .unwrap_or("(none)")
            .to_string();

        ui.horizontal(|ui| {
            ui.label("Active capsule");
            egui::ComboBox::from_id_salt("active_orchestrator_capsule")
                .selected_text(active_label)
                .show_ui(ui, |ui| {
                    if ui
                        .selectable_label(
                            app.prefs.active_orchestrator_capsule.is_none(),
                            "(none)",
                        )
                        .clicked()
                    {
                        app.prefs.active_orchestrator_capsule = None;
                    }
                    for name in &capsule_names {
                        if ui
                            .selectable_label(
                                app.prefs.active_orchestrator_capsule.as_deref()
                                    == Some(name.as_str()),
                                name,
                            )
                            .clicked()
                        {
                            app.prefs.active_orchestrator_capsule = Some(name.clone());
                            app.capsule_selected_name = Some(name.clone());
                            if let Some(capsule) = app
                                .prefs
                                .orchestrator_capsules
                                .iter()
                                .find(|capsule| capsule.name == *name)
                            {
                                app.capsule_editor_name = capsule.name.clone();
                                app.capsule_editor_text = capsule.text.clone();
                            }
                        }
                    }
                });
            if ui.button("Use native voice").clicked() {
                app.prefs.active_orchestrator_capsule = None;
                app.prefs_status =
                    "Capsule deselected. ChattyCog native voice restored.".to_string();
            }
        });
        ui.small(
            "Choose '(none)' or 'Use native voice' to fall back to ChattyCog's built-in personality.",
        );

        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.label("Saved capsules");
            if ui.button("Deselect editor").clicked() {
                app.capsule_selected_name = None;
                app.capsule_editor_name.clear();
                app.capsule_editor_text.clear();
                app.prefs_status = "Capsule editor cleared.".to_string();
            }
        });
        egui::ScrollArea::vertical()
            .id_salt("orchestrator_capsule_list")
            .max_height(180.0)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                if capsule_names.is_empty() {
                    ui.small("No capsules saved yet.");
                }
                let nothing_selected = app.capsule_selected_name.is_none()
                    && app.capsule_editor_name.trim().is_empty()
                    && app.capsule_editor_text.trim().is_empty();
                if ui
                    .selectable_label(nothing_selected, "(deselect editor)")
                    .clicked()
                {
                    app.capsule_selected_name = None;
                    app.capsule_editor_name.clear();
                    app.capsule_editor_text.clear();
                }
                for name in &capsule_names {
                    let selected = app.capsule_selected_name.as_deref() == Some(name.as_str());
                    if ui.selectable_label(selected, name).clicked() {
                        app.capsule_selected_name = Some(name.clone());
                        if let Some(capsule) = app
                            .prefs
                            .orchestrator_capsules
                            .iter()
                            .find(|capsule| capsule.name == *name)
                        {
                            app.capsule_editor_name = capsule.name.clone();
                            app.capsule_editor_text = capsule.text.clone();
                        }
                    }
                }
            });

        ui.add_space(8.0);
        ui.label("Capsule name");
        ui.text_edit_singleline(&mut app.capsule_editor_name);
        ui.add_space(4.0);
        ui.label("Capsule instructions");
        ui.add(
            egui::TextEdit::multiline(&mut app.capsule_editor_text)
                .desired_rows(16)
                .hint_text(
                    "Your nickname is Barry. You are a scriptwriter. Answer verbosely and keep a cinematic tone.",
                ),
        );

        ui.add_space(8.0);
        ui.horizontal_wrapped(|ui| {
            if ui.button("New").clicked() {
                app.capsule_selected_name = None;
                app.capsule_editor_name.clear();
                app.capsule_editor_text.clear();
            }

            if ui.button("Save capsule").clicked() {
                let name = app.capsule_editor_name.trim().to_string();
                let text = app.capsule_editor_text.trim().to_string();
                if name.is_empty() || text.is_empty() {
                    app.prefs_status = "Capsule needs both a name and instructions.".to_string();
                } else if let Some(existing) = app
                    .prefs
                    .orchestrator_capsules
                    .iter_mut()
                    .find(|capsule| capsule.name == name)
                {
                    existing.text = text;
                    app.capsule_selected_name = Some(name.clone());
                    app.prefs_status = format!("Updated capsule '{name}'.");
                } else {
                    app.prefs.orchestrator_capsules.push(PromptCapsule {
                        name: name.clone(),
                        text,
                    });
                    app.prefs.orchestrator_capsules.sort_by(|a, b| {
                        a.name.to_lowercase().cmp(&b.name.to_lowercase())
                    });
                    app.capsule_selected_name = Some(name.clone());
                    app.prefs_status = format!("Saved capsule '{name}'.");
                }
            }

            if ui.button("Set active").clicked() {
                let name = app.capsule_editor_name.trim().to_string();
                if name.is_empty() {
                    app.prefs_status = "Choose or save a capsule first.".to_string();
                } else if app
                    .prefs
                    .orchestrator_capsules
                    .iter()
                    .any(|capsule| capsule.name == name)
                {
                    app.prefs.active_orchestrator_capsule = Some(name.clone());
                    app.capsule_selected_name = Some(name.clone());
                    app.prefs_status = format!("Activated capsule '{name}'.");
                } else {
                    app.prefs_status = "Save the capsule before making it active.".to_string();
                }
            }

            let delete_target = app
                .capsule_selected_name
                .clone()
                .filter(|name| !name.trim().is_empty())
                .or_else(|| {
                    let editor_name = app.capsule_editor_name.trim().to_string();
                    if editor_name.is_empty() {
                        None
                    } else {
                        Some(editor_name)
                    }
                });
            if ui
                .add_enabled(delete_target.is_some(), egui::Button::new("Delete"))
                .clicked()
            {
                if let Some(target) = delete_target {
                    app.prefs
                        .orchestrator_capsules
                        .retain(|capsule| capsule.name != target);
                    if app.prefs.active_orchestrator_capsule.as_deref()
                        == Some(target.as_str())
                    {
                        app.prefs.active_orchestrator_capsule = None;
                    }
                    app.capsule_selected_name = None;
                    app.capsule_editor_name.clear();
                    app.capsule_editor_text.clear();
                    app.sync_capsule_selection_from_prefs();
                    app.prefs_status = format!("Deleted capsule '{target}'.");
                }
            }
        });
    });
}

pub(super) fn models_tab(ui: &mut egui::Ui, app: &mut ChattyCogApp) {
    ui.heading("Preferences");
    ui.separator();

    egui::ScrollArea::vertical()
        .id_salt("prefs_scroll_v2")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            render_models_prefs_header(ui, app);

            ui.add_space(8.0);
            ui.columns(2, |columns| {
                let left = &mut columns[0];
                render_models_orchestrator_prefs(left, app);

                left.add_space(8.0);
                render_models_bookkeeper_prefs(left, app);

                left.add_space(8.0);
                render_models_access_tools_prefs(left, app);

                left.add_space(8.0);
                render_models_module_prefs(left, app);

                let right = &mut columns[1];
                render_models_capsule_library(right, app);
            });
        });
}
