use super::*;

struct CloudHealthFailureChip {
    label: &'static str,
    color: egui::Color32,
    focus: CloudModelEditorFocusField,
    reason: &'static str,
}

fn cloud_health_badge(
    ui: &mut egui::Ui,
    status: Option<&str>,
    checked_at_unix_ms: u64,
    is_running: bool,
) {
    let (label, color) = if is_running {
        ("[testing]", egui::Color32::from_rgb(180, 140, 40))
    } else if let Some(status) = status {
        let status = status.trim();
        if is_cloud_health_stale_success(status, checked_at_unix_ms) {
            ("[stale]", egui::Color32::from_rgb(150, 130, 80))
        } else if status.starts_with("Health check passed") {
            ("[ok]", egui::Color32::from_rgb(50, 140, 70))
        } else if status.starts_with("Health check failed") {
            ("[fail]", egui::Color32::from_rgb(170, 60, 60))
        } else {
            ("[info]", egui::Color32::from_rgb(90, 120, 160))
        }
    } else {
        ("[untested]", egui::Color32::GRAY)
    };
    ui.colored_label(color, label);
}

fn cloud_health_failure_chip(status: Option<&str>) -> Option<CloudHealthFailureChip> {
    let status = status?.trim();
    if !status.starts_with("Health check failed") {
        return None;
    }
    let lower = status.to_lowercase();
    if lower.contains("api key")
        || lower.contains("unauthorized")
        || lower.contains("authentication")
        || lower.contains("invalid x-api-key")
        || lower.contains("incorrect api key")
    {
        Some(CloudHealthFailureChip {
            label: "auth",
            color: egui::Color32::from_rgb(150, 70, 70),
            focus: CloudModelEditorFocusField::ApiKey,
            reason: "API key",
        })
    } else if lower.contains("model") && (lower.contains("not found") || lower.contains("unknown"))
    {
        Some(CloudHealthFailureChip {
            label: "model",
            color: egui::Color32::from_rgb(140, 90, 60),
            focus: CloudModelEditorFocusField::ChatModel,
            reason: "chat model name",
        })
    } else if lower.contains("base url")
        || lower.contains("404")
        || lower.contains("405")
        || lower.contains("messages")
        || lower.contains("chat/completions")
        || lower.contains("/embeddings")
    {
        Some(CloudHealthFailureChip {
            label: "endpoint",
            color: egui::Color32::from_rgb(140, 100, 60),
            focus: CloudModelEditorFocusField::BaseUrl,
            reason: "base URL / endpoint",
        })
    } else if lower.contains("timed out")
        || lower.contains("dns")
        || lower.contains("connection")
        || lower.contains("tls")
        || lower.contains("socket")
        || lower.contains("transport")
    {
        Some(CloudHealthFailureChip {
            label: "network",
            color: egui::Color32::from_rgb(120, 100, 60),
            focus: CloudModelEditorFocusField::BaseUrl,
            reason: "base URL / network path",
        })
    } else if lower.contains("embedding") {
        Some(CloudHealthFailureChip {
            label: "embed",
            color: egui::Color32::from_rgb(110, 90, 120),
            focus: CloudModelEditorFocusField::EmbeddingsModel,
            reason: "embeddings model name",
        })
    } else if lower.contains("chat test failed") {
        Some(CloudHealthFailureChip {
            label: "chat",
            color: egui::Color32::from_rgb(110, 90, 120),
            focus: CloudModelEditorFocusField::ChatModel,
            reason: "chat model / provider lane",
        })
    } else {
        Some(CloudHealthFailureChip {
            label: "error",
            color: egui::Color32::from_rgb(120, 120, 120),
            focus: CloudModelEditorFocusField::BaseUrl,
            reason: "provider settings",
        })
    }
}

fn cloud_health_checked_label(checked_at_unix_ms: u64) -> Option<String> {
    if checked_at_unix_ms == 0 {
        return None;
    }
    let now = now_unix_ms().max(0) as u64;
    let age_ms = now.saturating_sub(checked_at_unix_ms);
    let age_secs = age_ms / 1_000;
    let age = if age_secs < 5 {
        "just now".to_string()
    } else if age_secs < 60 {
        format!("{age_secs}s ago")
    } else if age_secs < 3_600 {
        format!("{}m ago", age_secs / 60)
    } else if age_secs < 86_400 {
        format!("{}h ago", age_secs / 3_600)
    } else {
        format!("{}d ago", age_secs / 86_400)
    };
    let stale_suffix = if is_cloud_health_stale_success("Health check passed", checked_at_unix_ms)
    {
        " (stale)"
    } else {
        ""
    };
    Some(format!("Checked {age}{stale_suffix}"))
}

fn relative_time_label(unix_ms: u64) -> Option<String> {
    if unix_ms == 0 {
        return None;
    }
    let now = now_unix_ms().max(0) as u64;
    let age_secs = now.saturating_sub(unix_ms) / 1_000;
    Some(if age_secs < 5 {
        "just now".to_string()
    } else if age_secs < 60 {
        format!("{age_secs}s ago")
    } else if age_secs < 3_600 {
        format!("{}m ago", age_secs / 60)
    } else if age_secs < 86_400 {
        format!("{}h ago", age_secs / 3_600)
    } else {
        format!("{}d ago", age_secs / 86_400)
    })
}

fn cloud_entry_is_unhealthy(entry: &preferences::CloudModelEntry) -> bool {
    is_cloud_health_failed(&entry.last_health_status)
        || is_cloud_health_stale_success(
            &entry.last_health_status,
            entry.last_health_checked_at_unix_ms,
        )
}

fn cloud_entry_sort_rank(entry: &preferences::CloudModelEntry) -> u8 {
    if is_cloud_health_failed(&entry.last_health_status) {
        0
    } else if is_cloud_health_stale_success(
        &entry.last_health_status,
        entry.last_health_checked_at_unix_ms,
    ) {
        1
    } else {
        2
    }
}

fn cloud_repair_action_label(action: CloudModelEditorRepairAction) -> &'static str {
    match action {
        CloudModelEditorRepairAction::RestoreProviderDefaultBaseUrl => {
            "Restore provider default base URL"
        }
        CloudModelEditorRepairAction::UseGeminiDefaultEmbeddingsModel => {
            "Use Gemini embeddings default"
        }
        CloudModelEditorRepairAction::UseExampleChatModel => "Use example chat model",
        CloudModelEditorRepairAction::UseExampleEmbeddingsModel => "Use example embeddings model",
        CloudModelEditorRepairAction::UseLastVerifiedChatModel => "Use last verified chat model",
        CloudModelEditorRepairAction::UseLastVerifiedEmbeddingsModel => {
            "Use last verified embeddings model"
        }
    }
}

fn cloud_provider_example_chat_action(
    kind: preferences::CloudProviderKind,
) -> Option<CloudModelEditorRepairAction> {
    match kind {
        preferences::CloudProviderKind::OpenAi
        | preferences::CloudProviderKind::Anthropic
        | preferences::CloudProviderKind::Gemini => Some(CloudModelEditorRepairAction::UseExampleChatModel),
        preferences::CloudProviderKind::OpenAiCompatible => None,
    }
}

fn cloud_provider_example_embedding_action(
    kind: preferences::CloudProviderKind,
) -> Option<CloudModelEditorRepairAction> {
    match kind {
        preferences::CloudProviderKind::OpenAi
        | preferences::CloudProviderKind::Anthropic
        | preferences::CloudProviderKind::Gemini => {
            Some(CloudModelEditorRepairAction::UseExampleEmbeddingsModel)
        }
        preferences::CloudProviderKind::OpenAiCompatible => None,
    }
}

fn cloud_provider_label(kind: preferences::CloudProviderKind) -> &'static str {
    match kind {
        preferences::CloudProviderKind::OpenAi => "OpenAI",
        preferences::CloudProviderKind::OpenAiCompatible => "OpenAI-compatible",
        preferences::CloudProviderKind::Anthropic => "Anthropic",
        preferences::CloudProviderKind::Gemini => "Gemini",
    }
}

fn cloud_provider_default_base_url(kind: preferences::CloudProviderKind) -> &'static str {
    match kind {
        preferences::CloudProviderKind::OpenAi => "https://api.openai.com/v1",
        preferences::CloudProviderKind::OpenAiCompatible => "https://api.openai.com/v1",
        preferences::CloudProviderKind::Anthropic => "https://api.anthropic.com/v1",
        preferences::CloudProviderKind::Gemini => {
            "https://generativelanguage.googleapis.com/v1beta/openai"
        }
    }
}

fn cloud_provider_example_chat_model(kind: preferences::CloudProviderKind) -> &'static str {
    match kind {
        preferences::CloudProviderKind::OpenAi => "gpt-4.1-mini",
        preferences::CloudProviderKind::OpenAiCompatible => "whatever your host exposes",
        preferences::CloudProviderKind::Anthropic => "claude-sonnet-5",
        preferences::CloudProviderKind::Gemini => "gemini-3.5-flash",
    }
}

fn cloud_provider_example_embedding_model(kind: preferences::CloudProviderKind) -> &'static str {
    match kind {
        preferences::CloudProviderKind::OpenAi => "text-embedding-3-small",
        preferences::CloudProviderKind::OpenAiCompatible => "whatever your host exposes",
        preferences::CloudProviderKind::Anthropic => "(leave blank for now)",
        preferences::CloudProviderKind::Gemini => "gemini-embedding-2-preview",
    }
}

fn cloud_verification_scope_label(scope: &CloudModelVerificationScope) -> &'static str {
    match scope {
        CloudModelVerificationScope::None => "not yet verified",
        CloudModelVerificationScope::ChatOnly => "chat ready only",
        CloudModelVerificationScope::ChatAndEmbeddings => {
            "chat + Bookkeeper ready"
        }
    }
}

fn cloud_bookkeeper_ready_badge(ui: &mut egui::Ui, scope: &CloudModelVerificationScope) {
    match scope {
        CloudModelVerificationScope::ChatAndEmbeddings => {
            ui.colored_label(
                egui::Color32::from_rgb(50, 140, 70),
                "[bookkeeper-ready]",
            );
        }
        CloudModelVerificationScope::ChatOnly => {
            ui.colored_label(
                egui::Color32::from_rgb(150, 130, 80),
                "[chat-ready]",
            );
        }
        CloudModelVerificationScope::None => {}
    }
}

fn should_refresh_cloud_base_url(current: &str) -> bool {
    let current = current.trim().trim_end_matches('/');
    current.is_empty()
        || current == "https://api.openai.com/v1"
        || current == "https://api.anthropic.com/v1"
        || current == "https://generativelanguage.googleapis.com/v1beta/openai"
}

fn render_models_prefs_header(ui: &mut egui::Ui, app: &mut ChattyCogApp) {
    ui.horizontal_wrapped(|ui| {
        ui.label(format!("Prefs file: {}", app.prefs_path.display()));
        if ui.button("Reload").clicked() {
            match preferences::load_prefs(&app.prefs_path) {
                Ok(p) => {
                    app.prefs = p;
                    app.cloud_model_list_filter = if app.prefs.cloud_models_unhealthy_only {
                        CloudModelListFilter::UnhealthyOnly
                    } else {
                        CloudModelListFilter::All
                    };
                    app.sync_cloud_model_health_cache_from_prefs();
                    app.ensure_persisted_network_identity();
                    app.apply_prefs_to_runtime_settings();
                    if let Some(selection) = app.current_orchestrator_selection() {
                        app.set_active_chat_model_selection(Some(selection));
                    }
                    if let Some(selection) = app.current_bookkeeper_selection() {
                        app.set_bookkeeper_model_selection(Some(selection));
                    }
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
        let model_opts = build_hybrid_model_options_for_lane(
            app.models_dir.as_deref(),
            app.modules_dir.as_deref(),
            &app.prefs.cloud_models,
            ModelLane::Orchestrator,
        );
        let selected = app.current_orchestrator_selection();
        let selected_label = selected_model_option_label(
            &model_opts,
            selected.as_deref(),
            app.gguf_path.as_ref().map(|path| {
                path.file_name()
                    .map(|name| name.to_string_lossy().to_string())
                    .unwrap_or_else(|| path.display().to_string())
            }),
        );
        ui.horizontal(|ui| {
            ui.label("Active model");
            if let Some(picked) = show_grouped_model_option_combo(
                ui,
                "models_tab_orchestrator_model",
                selected_label,
                &model_opts,
                selected.as_deref(),
            ) {
                app.set_active_chat_model_selection(picked);
                app.prefs_status = "Updated orchestrator model selection.".to_string();
            }
        });
        ui.add_space(6.0);
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
        ui.heading("Bookkeeper");
        let model_opts = build_hybrid_model_options_for_lane(
            app.models_dir.as_deref(),
            app.modules_dir.as_deref(),
            &app.prefs.cloud_models,
            ModelLane::Bookkeeper,
        );
        let selected = app.current_bookkeeper_selection();
        let selected_label = selected_model_option_label(
            &model_opts,
            selected.as_deref(),
            app.bookkeeper_model_path.as_ref().map(|path| {
                path.file_name()
                    .map(|name| name.to_string_lossy().to_string())
                    .unwrap_or_else(|| path.display().to_string())
            }),
        );
        ui.horizontal(|ui| {
            ui.label("Active model");
            if let Some(picked) = show_grouped_model_option_combo(
                ui,
                "models_tab_bookkeeper_model",
                selected_label,
                &model_opts,
                selected.as_deref(),
            ) {
                app.set_bookkeeper_model_selection(picked);
                app.prefs_status = "Updated bookkeeper model selection.".to_string();
            }
        });
        ui.small(
            "Cloud bookkeeper entries should also include an embeddings model name so semantic search can keep working.",
        );
        ui.add_space(6.0);
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
            app.prefs_status = "Updated bookkeeper settings (restart pending).".to_string();
        }
    });
}

fn render_models_cloud_registry(ui: &mut egui::Ui, app: &mut ChattyCogApp) {
    ui.group(|ui| {
        ui.heading("Cloud Models");
        ui.small(
            "Add your own provider entries here. They appear beside local GGUFs in the same orchestrator and Bookkeeper pickers, so local-first stays the baseline and cloud stays optional.",
        );
        ui.add_space(6.0);

        ui.horizontal(|ui| {
            ui.label("Provider");
            egui::ComboBox::from_id_salt("cloud_model_provider_kind")
                .selected_text(cloud_provider_label(
                    app.cloud_model_editor.provider_kind.clone(),
                ))
                .show_ui(ui, |ui| {
                    let mut choose = |kind: preferences::CloudProviderKind, label: &str| {
                        if ui
                            .selectable_label(app.cloud_model_editor.provider_kind == kind, label)
                            .clicked()
                        {
                            app.cloud_model_editor.provider_kind = kind.clone();
                            if should_refresh_cloud_base_url(&app.cloud_model_editor.base_url) {
                                app.cloud_model_editor.base_url =
                                    cloud_provider_default_base_url(kind).to_string();
                            }
                        }
                    };
                    choose(preferences::CloudProviderKind::OpenAi, "OpenAI");
                    choose(
                        preferences::CloudProviderKind::OpenAiCompatible,
                        "OpenAI-compatible",
                    );
                    choose(preferences::CloudProviderKind::Anthropic, "Anthropic");
                    choose(preferences::CloudProviderKind::Gemini, "Gemini");
                });
        });
        ui.horizontal(|ui| {
            ui.label("Display name");
            ui.text_edit_singleline(&mut app.cloud_model_editor.display_name);
        });
        ui.horizontal(|ui| {
            ui.label("Base URL");
            let response = ui.text_edit_singleline(&mut app.cloud_model_editor.base_url);
            if app.cloud_model_editor.pending_focus == Some(CloudModelEditorFocusField::BaseUrl) {
                response.request_focus();
                app.cloud_model_editor.pending_focus = None;
            }
            if ui.button("Use provider default").clicked() {
                app.cloud_model_editor.base_url =
                    cloud_provider_default_base_url(app.cloud_model_editor.provider_kind.clone())
                        .to_string();
            }
        });
        ui.horizontal(|ui| {
            ui.label("Chat model");
            let response = ui.text_edit_singleline(&mut app.cloud_model_editor.model_name);
            if app.cloud_model_editor.pending_focus == Some(CloudModelEditorFocusField::ChatModel)
            {
                response.request_focus();
                app.cloud_model_editor.pending_focus = None;
            }
        });
        ui.horizontal(|ui| {
            ui.label("Embeddings model");
            let response = ui.text_edit_singleline(&mut app.cloud_model_editor.embedding_model_name);
            if app.cloud_model_editor.pending_focus
                == Some(CloudModelEditorFocusField::EmbeddingsModel)
            {
                response.request_focus();
                app.cloud_model_editor.pending_focus = None;
            }
        });
        match app.cloud_model_editor.provider_kind {
            preferences::CloudProviderKind::OpenAi
            | preferences::CloudProviderKind::OpenAiCompatible => {
                ui.small("Supports chat now. Add an embeddings model too if you want this entry available for the Bookkeeper lane.");
            }
            preferences::CloudProviderKind::Anthropic => {
                ui.small("Anthropic chat is wired for the orchestrator lane. The Bookkeeper lane still needs embeddings, so Anthropic entries will not appear there.");
            }
            preferences::CloudProviderKind::Gemini => {
                ui.small("Gemini chat and embeddings are wired through its current OpenAI-compatible endpoint family. Add an embeddings model if you want this entry available for the Bookkeeper lane.");
            }
        }
        ui.small(format!(
            "Suggested base URL: {}",
            cloud_provider_default_base_url(app.cloud_model_editor.provider_kind.clone())
        ));
        ui.horizontal_wrapped(|ui| {
            ui.small(format!(
                "Example chat model: {}",
                cloud_provider_example_chat_model(app.cloud_model_editor.provider_kind.clone())
            ));
            if let Some(action) =
                cloud_provider_example_chat_action(app.cloud_model_editor.provider_kind.clone())
            {
                if ui.button(cloud_repair_action_label(action)).clicked() {
                    app.cloud_model_editor.repair_action = Some(action);
                    app.apply_cloud_model_editor_repair_action();
                }
            }
        });
        if !app
            .cloud_model_editor
            .last_verified_chat_model_name
            .trim()
            .is_empty()
        {
            ui.horizontal_wrapped(|ui| {
                ui.small(format!(
                    "Last verified chat model: {}",
                    app.cloud_model_editor.last_verified_chat_model_name.trim()
                ));
                if ui
                    .button(cloud_repair_action_label(
                        CloudModelEditorRepairAction::UseLastVerifiedChatModel,
                    ))
                    .clicked()
                {
                    app.cloud_model_editor.repair_action =
                        Some(CloudModelEditorRepairAction::UseLastVerifiedChatModel);
                    app.apply_cloud_model_editor_repair_action();
                }
            });
            ui.small(format!(
                "Last verified scope: {}",
                cloud_verification_scope_label(&app.cloud_model_editor.last_verified_scope)
            ));
        }
        ui.horizontal_wrapped(|ui| {
            ui.small(format!(
                "Example embeddings model: {}",
                cloud_provider_example_embedding_model(app.cloud_model_editor.provider_kind.clone())
            ));
            if let Some(action) = cloud_provider_example_embedding_action(
                app.cloud_model_editor.provider_kind.clone(),
            ) {
                if ui.button(cloud_repair_action_label(action)).clicked() {
                    app.cloud_model_editor.repair_action = Some(action);
                    app.apply_cloud_model_editor_repair_action();
                }
            }
        });
        if !app
            .cloud_model_editor
            .last_verified_embedding_model_name
            .trim()
            .is_empty()
        {
            ui.horizontal_wrapped(|ui| {
                ui.small(format!(
                    "Last verified embeddings model: {}",
                    app.cloud_model_editor
                        .last_verified_embedding_model_name
                        .trim()
                ));
                if ui
                    .button(cloud_repair_action_label(
                        CloudModelEditorRepairAction::UseLastVerifiedEmbeddingsModel,
                    ))
                    .clicked()
                {
                    app.cloud_model_editor.repair_action =
                        Some(CloudModelEditorRepairAction::UseLastVerifiedEmbeddingsModel);
                    app.apply_cloud_model_editor_repair_action();
                }
            });
        }
        if !app.cloud_model_editor.repair_hint.trim().is_empty() {
            ui.horizontal_wrapped(|ui| {
                ui.small(format!(
                    "Repair hint: {}",
                    app.cloud_model_editor.repair_hint.trim()
                ));
                if let Some(action) = app.cloud_model_editor.repair_action {
                    if ui.button(cloud_repair_action_label(action)).clicked() {
                        app.apply_cloud_model_editor_repair_action();
                    }
                }
            });
        }
        ui.horizontal(|ui| {
            ui.label("API key");
            let response = ui.add(
                egui::TextEdit::singleline(&mut app.cloud_model_editor.api_key).password(true),
            );
            if app.cloud_model_editor.pending_focus == Some(CloudModelEditorFocusField::ApiKey) {
                response.request_focus();
                app.cloud_model_editor.pending_focus = None;
            }
        });
        ui.checkbox(&mut app.cloud_model_editor.enabled, "Enabled");
        ui.horizontal(|ui| {
            let button_label = if app.cloud_model_editor.edit_id.is_some() {
                "Update entry"
            } else {
                "Add entry"
            };
            let test_label = if app.cloud_model_editor.health_check_running {
                "Testing..."
            } else {
                "Test connection"
            };
            if ui
                .add_enabled(
                    !app.cloud_model_editor.health_check_running,
                    egui::Button::new(test_label),
                )
                .clicked()
            {
                app.start_cloud_model_health_check();
            }
            if ui.button(button_label).clicked() {
                match app.upsert_cloud_model_from_editor() {
                    Ok(()) => {
                        app.prefs_status = "Saved cloud model entry.".to_string();
                        app.clear_cloud_model_editor();
                    }
                    Err(err) => {
                        app.prefs_status = format!("Cloud model save failed: {err}");
                    }
                }
            }
            if ui.button("Clear editor").clicked() {
                app.clear_cloud_model_editor();
            }
        });
        if !app.cloud_model_editor.health_status.trim().is_empty() {
            ui.small(app.cloud_model_editor.health_status.clone());
        }

        ui.separator();
        if app.prefs.cloud_models.is_empty() {
            ui.small("No cloud model entries yet.");
            return;
        }

        let stale_count = app
            .prefs
            .cloud_models
            .iter()
            .filter(|entry| {
                is_cloud_health_stale_success(
                    &entry.last_health_status,
                    entry.last_health_checked_at_unix_ms,
                )
            })
            .count();
        let failed_count = app
            .prefs
            .cloud_models
            .iter()
            .filter(|entry| is_cloud_health_failed(&entry.last_health_status))
            .count();
        let unhealthy_count = app
            .prefs
            .cloud_models
            .iter()
            .filter(|entry| cloud_entry_is_unhealthy(entry))
            .count();
        let any_saved_health_running = app.cloud_model_health_running_id.is_some();
        let current_filter_label = match app.cloud_model_list_filter {
            CloudModelListFilter::All => "all",
            CloudModelListFilter::UnhealthyOnly => "unhealthy only",
        };
        let disclosure_label = format!(
            "{} Advanced status / maintenance ({} stale, {} failed, filter: {})",
            if app.prefs.cloud_models_advanced_open {
                "▼"
            } else {
                "▶"
            },
            stale_count,
            failed_count,
            current_filter_label
        );
        if ui.button(disclosure_label).clicked() {
            app.prefs.cloud_models_advanced_open = !app.prefs.cloud_models_advanced_open;
            app.save_prefs_quietly();
        }
        if app.prefs.cloud_models_advanced_open {
            if let Some(when) = relative_time_label(app.prefs.cloud_models_last_sweep_ran_at_unix_ms)
            {
                let kind = if app.prefs.cloud_models_last_sweep_kind.trim().is_empty() {
                    "maintenance"
                } else {
                    app.prefs.cloud_models_last_sweep_kind.trim()
                };
                ui.small(format!("Last {kind} sweep ran {when}."));
                ui.add_space(4.0);
            }
            if let Some(when) =
                relative_time_label(app.prefs.cloud_models_last_unhealthy_sweep_ran_at_unix_ms)
            {
                ui.small(format!("Last all-unhealthy sweep ran {when}."));
                ui.add_space(4.0);
            }
            ui.horizontal_wrapped(|ui| {
                ui.small("Legend:");
                ui.colored_label(egui::Color32::GRAY, "[untested]");
                ui.small("not checked yet");
                ui.colored_label(egui::Color32::from_rgb(180, 140, 40), "[testing]");
                ui.small("health check running");
                ui.colored_label(egui::Color32::from_rgb(50, 140, 70), "[ok]");
                ui.small("fresh success");
                ui.colored_label(egui::Color32::from_rgb(150, 130, 80), "[stale]");
                ui.small("old success");
                ui.colored_label(egui::Color32::from_rgb(170, 60, 60), "[fail]");
                ui.small("last health check failed");
                ui.colored_label(
                    egui::Color32::from_rgb(50, 140, 70),
                    "[bookkeeper-ready]",
                );
                ui.small("chat + embeddings verified");
                ui.colored_label(egui::Color32::from_rgb(150, 130, 80), "[chat-ready]");
                ui.small("chat works, Bookkeeper not ready yet");
            });
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                let unhealthy_sweep_label =
                    if app.cloud_model_health_sweep_active
                        && app.cloud_model_health_sweep_kind == Some("unhealthy")
                    {
                        "Retesting all unhealthy..."
                    } else {
                        "Retest all unhealthy"
                    };
                if ui
                    .add_enabled(
                        unhealthy_count > 0
                            && !any_saved_health_running
                            && !app.cloud_model_health_sweep_active,
                        egui::Button::new(unhealthy_sweep_label),
                    )
                    .clicked()
                {
                    app.start_saved_cloud_model_unhealthy_sweep();
                }
                ui.small(format!(
                    "{unhealthy_count} unhealthy entr{}",
                    if unhealthy_count == 1 { "y" } else { "ies" }
                ));
            });
            ui.horizontal(|ui| {
                let stale_sweep_label =
                    if app.cloud_model_health_sweep_active
                        && app.cloud_model_health_sweep_kind == Some("stale")
                    {
                        "Retesting stale..."
                    } else {
                        "Retest stale"
                    };
                if ui
                    .add_enabled(
                        stale_count > 0
                            && !any_saved_health_running
                            && !app.cloud_model_health_sweep_active,
                        egui::Button::new(stale_sweep_label),
                    )
                    .clicked()
                {
                    app.start_saved_cloud_model_stale_sweep();
                }
                ui.small(format!(
                    "{stale_count} stale entr{}",
                    if stale_count == 1 { "y" } else { "ies" }
                ));
            });
            ui.horizontal(|ui| {
                let failed_sweep_label =
                    if app.cloud_model_health_sweep_active
                        && app.cloud_model_health_sweep_kind == Some("failed")
                    {
                        "Retesting failed..."
                    } else {
                        "Retest failed"
                    };
                if ui
                    .add_enabled(
                        failed_count > 0
                            && !any_saved_health_running
                            && !app.cloud_model_health_sweep_active,
                        egui::Button::new(failed_sweep_label),
                    )
                    .clicked()
                {
                    app.start_saved_cloud_model_failed_sweep();
                }
                ui.small(format!(
                    "{failed_count} failed entr{}",
                    if failed_count == 1 { "y" } else { "ies" }
                ));
            });
            ui.horizontal(|ui| {
                ui.label("List filter");
                let before = app.cloud_model_list_filter;
                ui.selectable_value(
                    &mut app.cloud_model_list_filter,
                    CloudModelListFilter::All,
                    "All",
                );
                ui.selectable_value(
                    &mut app.cloud_model_list_filter,
                    CloudModelListFilter::UnhealthyOnly,
                    "Unhealthy only",
                );
                if app.cloud_model_list_filter != before {
                    app.prefs.cloud_models_unhealthy_only =
                        app.cloud_model_list_filter == CloudModelListFilter::UnhealthyOnly;
                    app.save_prefs_quietly();
                }
            });
        }
        ui.add_space(4.0);

        let entries = app
            .prefs
            .cloud_models
            .iter()
            .filter(|entry| match app.cloud_model_list_filter {
                CloudModelListFilter::All => true,
                CloudModelListFilter::UnhealthyOnly => cloud_entry_is_unhealthy(entry),
            })
            .cloned()
            .collect::<Vec<_>>();
        let mut entries = entries;
        if app.cloud_model_list_filter == CloudModelListFilter::All {
            entries.sort_by(|left, right| {
                cloud_entry_sort_rank(left)
                    .cmp(&cloud_entry_sort_rank(right))
                    .then_with(|| {
                        left.display_name
                            .to_lowercase()
                            .cmp(&right.display_name.to_lowercase())
                    })
            });
        }
        if entries.is_empty() {
            match app.cloud_model_list_filter {
                CloudModelListFilter::All => ui.small("No cloud model entries yet."),
                CloudModelListFilter::UnhealthyOnly => {
                    ui.small("No stale or failed cloud model entries right now.")
                }
            };
            return;
        }
        for entry in entries {
            ui.horizontal_wrapped(|ui| {
                let is_retesting =
                    app.cloud_model_health_running_id.as_deref() == Some(entry.id.as_str());
                let status = app.cloud_model_health_statuses.get(&entry.id).map(|s| s.as_str());
                cloud_health_badge(
                    ui,
                    status,
                    entry.last_health_checked_at_unix_ms,
                    is_retesting,
                );
                if let Some(chip) = cloud_health_failure_chip(status) {
                    let response = ui
                        .colored_label(chip.color, format!("[{}]", chip.label))
                        .on_hover_text(format!("Load into editor and focus {}.", chip.reason));
                    if response.clicked() {
                        app.load_cloud_model_into_editor_with_focus(&entry, chip.focus, chip.reason);
                    }
                }
                ui.label(format!(
                    "{} [{}]",
                    entry.display_name,
                    if entry.enabled { "enabled" } else { "disabled" }
                ));
                if entry.last_verified_scope != CloudModelVerificationScope::None {
                    cloud_bookkeeper_ready_badge(ui, &entry.last_verified_scope);
                    ui.small(format!(
                        "status: {}",
                        cloud_verification_scope_label(&entry.last_verified_scope)
                    ));
                }
                ui.small(format!("chat: {}", entry.model_name));
                if !entry.embedding_model_name.trim().is_empty() {
                    ui.small(format!("embed: {}", entry.embedding_model_name));
                }
                if ui.button("Edit").clicked() {
                    app.load_cloud_model_into_editor(&entry);
                }
                let retest_label = if is_retesting { "Retesting..." } else { "Retest" };
                if ui
                    .add_enabled(
                        !any_saved_health_running,
                        egui::Button::new(retest_label),
                    )
                    .clicked()
                {
                    app.start_saved_cloud_model_health_check(&entry);
                }
                if ui.button("Delete").clicked() {
                    app.prefs.cloud_models.retain(|item| item.id != entry.id);
                    app.cloud_model_health_statuses.remove(&entry.id);
                    app.cloud_model_health_queue.retain(|queued_id| queued_id != &entry.id);
                    if app.cloud_model_health_running_id.as_deref() == Some(entry.id.as_str()) {
                        app.cloud_model_health_running_id = None;
                        app.cloud_model_health_rx = None;
                    }
                    if app.current_orchestrator_selection().as_deref()
                        == Some(cloud_selection_id(&entry.id).as_str())
                    {
                        app.set_active_chat_model_selection(None);
                    }
                    if app.current_bookkeeper_selection().as_deref()
                        == Some(cloud_selection_id(&entry.id).as_str())
                    {
                        app.set_bookkeeper_model_selection(None);
                    }
                    app.save_prefs_quietly();
                    app.prefs_status = format!("Deleted cloud model '{}'.", entry.display_name);
                }
            });
            if let Some(checked_label) =
                cloud_health_checked_label(entry.last_health_checked_at_unix_ms)
            {
                ui.small(checked_label);
            }
            if let Some(status) = app.cloud_model_health_statuses.get(&entry.id) {
                ui.small(status);
            }
        }
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
                render_models_cloud_registry(left, app);

                left.add_space(8.0);
                render_models_access_tools_prefs(left, app);

                left.add_space(8.0);
                render_models_module_prefs(left, app);

                let right = &mut columns[1];
                render_models_capsule_library(right, app);
            });
        });
}
