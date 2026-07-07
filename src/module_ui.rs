use super::*;

pub(super) fn module_tab(ui: &mut egui::Ui, app: &mut ChattyCogApp, module_id: &str) {
    let mf = app
        .module_registry
        .modules
        .iter()
        .find(|m| m.module_id == module_id)
        .cloned();
    let module_dir = mf.as_ref().map(|m| m.dir.clone());

    if mf.is_none() {
        ui.heading("Module");
        ui.separator();
        ui.label("Manifest not found (module was removed or rescanned).");
        return;
    }

    let hosted_visual = mf
        .as_ref()
        .and_then(|module| module.visual_load.clone())
        .filter(|visual| visual.hosts_native_window());

    if let (Some(dir), Some(visual)) = (module_dir.as_deref(), hosted_visual.as_ref()) {
        render_module_host_tab(ui, app, mf.as_ref(), module_id, dir, visual);
        return;
    }

    render_standard_module_tab(ui, app, mf.as_ref(), module_id, module_dir.as_deref());
}

fn render_standard_module_tab(
    ui: &mut egui::Ui,
    app: &mut ChattyCogApp,
    manifest: Option<&ModuleManifest>,
    module_id: &str,
    module_dir: Option<&Path>,
) {
    egui::ScrollArea::vertical()
        .id_salt(format!("module_tab_scroll_{module_id}"))
        .auto_shrink([false, false])
        .show(ui, |ui| {
            render_module_support_panels(ui, app, manifest, module_id, module_dir, true);
        });
}

fn render_module_host_tab(
    ui: &mut egui::Ui,
    app: &mut ChattyCogApp,
    manifest: Option<&ModuleManifest>,
    module_id: &str,
    module_dir: &Path,
    visual: &ModuleVisualLoad,
) {
    let host = app
        .module_hosts
        .entry(module_id.to_string())
        .or_insert_with(ModuleHostState::default);
    let running = host.is_running();

    ui.horizontal_wrapped(|ui| {
        if let Some(manifest) = manifest {
            ui.heading(&manifest.display_name);
            ui.separator();
            ui.small(&manifest.description);
            ui.separator();
        }

        if visual.build.is_some() && ui.button("Build UI").clicked() {
            if let Err(err) = host.start_build(module_dir, visual) {
                host.status = err;
            }
        }

        if running {
            if ui.button("Restart UI").clicked() {
                host.force_stop();
                if let Err(err) = host.launch(module_dir, visual) {
                    host.status = err;
                }
            }
            if ui.button("Close module app").clicked() {
                host.request_close(visual);
            }
        } else if ui.button("Launch in tab").clicked() {
            if let Err(err) = host.launch(module_dir, visual) {
                host.status = err;
            }
        }

        if ui.button("Open module folder").clicked() {
            open_path_in_explorer(module_dir);
        }

        ui.separator();
        ui.small(host.status.clone());
    });

    if !visual.notes.trim().is_empty() {
        ui.small(visual.notes.trim());
    }

    ui.add_space(6.0);
    egui::CollapsingHeader::new("ChattyCog bridge")
        .default_open(false)
        .show(ui, |ui| {
            ui.small(
                "Use this only for the compatibility loop: module-reported status, suspend rundown, and the optional ChattyCog-side helper. The hosted module keeps owning its own real UI/state.",
            );
            ui.add_space(6.0);
            render_module_support_panels(ui, app, manifest, module_id, Some(module_dir), true);
        });

    ui.add_space(8.0);
    let available = ui.available_size();
    let desired = egui::vec2(available.x.max(240.0), available.y.max(320.0));
    let (rect, _) = ui.allocate_exact_size(desired, egui::Sense::hover());
    ui.painter().rect_filled(rect, 0.0, egui::Color32::WHITE);
    ui.painter()
        .rect_stroke(rect, 0.0, egui::Stroke::new(1.0, egui::Color32::LIGHT_GRAY));

    app.set_module_host_target(module_id, rect, ui.ctx().pixels_per_point());

    let host = app
        .module_hosts
        .entry(module_id.to_string())
        .or_insert_with(ModuleHostState::default);
    let centered = rect.center();
    let message = if host.is_running() {
        if host.is_waiting_for_window() {
            if visual.kind.trim().eq_ignore_ascii_case("webview") {
                "Launching hosted webview..."
            } else {
                "Launching module window..."
            }
        } else {
            if visual.kind.trim().eq_ignore_ascii_case("webview") {
                "Module webview is hosted here."
            } else {
                "Module window is hosted here."
            }
        }
    } else {
        if visual.kind.trim().eq_ignore_ascii_case("webview") {
            "Hosted webview is not running yet."
        } else {
            "Native module UI is not running yet."
        }
    };
    ui.painter().text(
        centered,
        egui::Align2::CENTER_CENTER,
        message,
        egui::TextStyle::Body.resolve(ui.style()),
        egui::Color32::DARK_GRAY,
    );
}

pub(super) fn module_allows_network_feature(
    manifest: Option<&ModuleManifest>,
    feature: ModuleNetworkFeature,
) -> bool {
    manifest
        .and_then(|mf| mf.network_capabilities.as_ref())
        .map(|caps| caps.has(feature))
        .unwrap_or(true)
}

fn render_module_support_panels(
    ui: &mut egui::Ui,
    app: &mut ChattyCogApp,
    manifest: Option<&ModuleManifest>,
    module_id: &str,
    module_dir: Option<&Path>,
    include_surface: bool,
) {
    if include_surface {
        if let Some(dir) = module_dir {
            render_module_surface(ui, app, manifest, module_id, dir);
        }
        ui.add_space(8.0);
    }

    if let Some(dir) = module_dir {
        let network_caps = manifest.and_then(|mf| mf.network_capabilities.as_ref());
        egui::CollapsingHeader::new("Declared network capabilities")
            .default_open(false)
            .show(ui, |ui| {
                if let Some(caps) = network_caps {
                    ui.small(
                        "Optional contract: this tells ChattyCog which network lanes the module intentionally supports, so future sharing stays predictable and portable.",
                    );
                    if !caps.features.is_empty() {
                        ui.horizontal_wrapped(|ui| {
                            for feature in &caps.features {
                                ui.label(
                                    egui::RichText::new(feature.label())
                                        .small()
                                        .monospace(),
                                );
                            }
                        });
                    }
                    if !caps.asset_lanes.is_empty() {
                        ui.add_space(6.0);
                        ui.label("Declared asset lanes");
                        for lane in &caps.asset_lanes {
                            ui.group(|ui| {
                                ui.horizontal_wrapped(|ui| {
                                    ui.strong(lane.label.trim());
                                    ui.small(format!(
                                        "[{} | {} | {}]",
                                        lane.lane_id,
                                        lane.direction.label(),
                                        lane.delivery_mode.label()
                                    ));
                                });
                                let mut summary_bits = Vec::new();
                                if !lane.artifact_kinds.is_empty() {
                                    summary_bits
                                        .push(format!("Kinds: {}", lane.artifact_kinds.join(", ")));
                                }
                                if !lane.accepted_content_types.is_empty() {
                                    summary_bits.push(format!(
                                        "Content: {}",
                                        lane.accepted_content_types.join(", ")
                                    ));
                                }
                                if let Some(max_bytes) = lane.max_bytes {
                                    summary_bits
                                        .push(format!("Max: {}", format_network_transfer_size(max_bytes)));
                                }
                                summary_bits.push(if lane.replayable {
                                    "Replayable".to_string()
                                } else {
                                    "Not replayable".to_string()
                                });
                                ui.small(summary_bits.join(" | "));
                                for note in &lane.notes {
                                    ui.small(format!("Note: {}", note));
                                }
                            });
                        }
                    }
                    for note in &caps.notes {
                        ui.small(format!("Note: {}", note));
                    }
                    if caps.features.is_empty() && caps.asset_lanes.is_empty() && caps.notes.is_empty() {
                        ui.small("This module's capability block is present but currently empty.");
                    }
                } else {
                    ui.small(
                        "No `network_capabilities.json` declared yet. ChattyCog will keep falling back to bridge-file presence and safe manual controls.",
                    );
                }
            });

        let room_capable = manifest
            .and_then(|mf| mf.network_capabilities.as_ref())
            .map(|caps| {
                caps.has(ModuleNetworkFeature::RoomAware)
                    || caps.has(ModuleNetworkFeature::Multiplayer)
            })
            .unwrap_or(false);
        if room_capable {
            egui::CollapsingHeader::new("Shared room lane")
                .default_open(false)
                .show(ui, |ui| {
                    let multiplayer = manifest
                        .and_then(|mf| mf.network_capabilities.as_ref())
                        .map(|caps| caps.has(ModuleNetworkFeature::Multiplayer))
                        .unwrap_or(false);
                    if app.shared_chat_scope_matches_module(module_id) {
                        ui.small(format!(
                            "The shared room is currently focused on this module: {}.",
                            app.shared_chat_scope_label()
                        ));
                    } else {
                        ui.small(
                            "This module can opt into the shared-room lane. Use the buttons below when you want the room policy to follow this module cleanly.",
                        );
                    }
                    ui.horizontal_wrapped(|ui| {
                        let button_label = if multiplayer {
                            "Use this module as multiplayer room"
                        } else {
                            "Use this module in shared room"
                        };
                        if ui.button(button_label).clicked() {
                            let module_name = manifest
                                .map(|mf| mf.display_name.clone())
                                .unwrap_or_else(|| module_id.to_string());
                            app.set_shared_chat_scope_module(
                                module_id.to_string(),
                                module_name,
                                multiplayer,
                            );
                            app.broadcast_shared_chat_policy(
                                "Room scope moved to this module.",
                            );
                        }
                        if ui.button("Return room to general").clicked() {
                            app.set_shared_chat_scope_general();
                            app.broadcast_shared_chat_policy(
                                "Room scope returned to the general lane.",
                            );
                        }
                        if ui.button("Open shared room controls").clicked() {
                            set_active_tab(app, Tab::Networking, "Networking");
                            app.focus_networking_section(NetworkingFocusSection::SharedRoom);
                        }
                        if app.shared_chat_scope_matches_module(module_id)
                            && !app.networking_shared_chat_policy.session_active
                            && ui.button("Start room session now").clicked()
                        {
                            if let Some(module_name) = app.begin_shared_chat_module_session() {
                                app.broadcast_shared_chat_policy(&format!(
                                    "Started host-guided module session for {module_name}."
                                ));
                            }
                        } else if app.shared_chat_scope_matches_module(module_id)
                            && app.networking_shared_chat_policy.session_active
                            && ui.button("End room session now").clicked()
                        {
                            let label = app
                                .networking_shared_chat_policy
                                .session_label
                                .trim()
                                .to_string();
                            app.end_shared_chat_module_session();
                            app.broadcast_shared_chat_policy(&format!(
                                "Ended {}.",
                                if label.is_empty() {
                                    "the module session".to_string()
                                } else {
                                    label
                                }
                            ));
                        }
                    });
                });
        }

        egui::CollapsingHeader::new("Module-reported status (portable bridge)")
            .default_open(false)
            .show(ui, |ui| {
                let status_path = bridge_status_path(dir);
                let log_sources_path = bridge_log_sources_path(dir);
                let shared_state_path = bridge_shared_state_path(dir);
                let incoming_shared_state_path = bridge_incoming_shared_state_path(dir);
                let incoming_assets_dir = bridge_incoming_assets_dir(dir);
                let shared_room_state_path = bridge_shared_room_state_path(dir);
                let shared_room_events_path = bridge_shared_room_events_path(dir);
                let outgoing_room_events_path = bridge_outgoing_room_events_path(dir);
                ui.small(
                    "Optional plug: the module stays standalone and only reports summary/snapshot here when it wants ChattyCog context handoff. If `log_sources.json` exists, ChattyCog can also tail declared module-local logs for auto-rundown context.",
                );
                ui.horizontal(|ui| {
                    if ui.button("Open bridge folder").clicked() {
                        open_path_in_explorer(status_path.parent().unwrap_or(dir));
                    }
                    if status_path.is_file() && ui.button("Open status.json").clicked() {
                        open_path_in_explorer(&status_path);
                    }
                    if log_sources_path.is_file() && ui.button("Open log_sources.json").clicked() {
                        open_path_in_explorer(&log_sources_path);
                    }
                    if shared_state_path.is_file() && ui.button("Open shared_state.json").clicked() {
                        open_path_in_explorer(&shared_state_path);
                    }
                    if incoming_shared_state_path.is_file()
                        && ui.button("Open incoming_shared_state.json").clicked()
                    {
                        open_path_in_explorer(&incoming_shared_state_path);
                    }
                    if shared_room_state_path.is_file()
                        && ui.button("Open shared_room_state.json").clicked()
                    {
                        open_path_in_explorer(&shared_room_state_path);
                    }
                    if shared_room_events_path.is_file()
                        && ui.button("Open shared_room_events.json").clicked()
                    {
                        open_path_in_explorer(&shared_room_events_path);
                    }
                    if outgoing_room_events_path.is_file()
                        && ui.button("Open outgoing_room_events.json").clicked()
                    {
                        open_path_in_explorer(&outgoing_room_events_path);
                    }
                    if incoming_assets_dir.is_dir() && ui.button("Open incoming assets").clicked() {
                        open_path_in_explorer(&incoming_assets_dir);
                    }
                });

                match app.read_module_bridge_status(module_id, dir) {
                    Some(status) => {
                        if status.updated_at_unix_ms > 0 {
                            ui.small(format!(
                                "Last update: {}",
                                status.updated_at_unix_ms
                            ));
                        }
                        if !status.tags.is_empty() {
                            ui.small(format!("Tags: {}", status.tags.join(", ")));
                        }
                        if !status.summary.trim().is_empty() {
                            ui.label("Summary");
                            ui.group(|ui| {
                                ui.label(status.summary.trim());
                            });
                        }
                        if !status.snapshot.trim().is_empty() {
                            ui.add_space(6.0);
                            ui.label("Snapshot");
                            let mut snapshot = status.snapshot.clone();
                            egui::ScrollArea::vertical()
                                .id_salt(format!("module_bridge_snapshot_{module_id}"))
                                .max_height(180.0)
                                .show(ui, |ui| {
                                    ui.add(
                                        egui::TextEdit::multiline(&mut snapshot)
                                            .desired_rows(8)
                                            .interactive(false),
                                    );
                                });
                        }
                    }
                    None => {
                        ui.small(
                            "No bridge status yet. The module is still standalone; it just has not reported a rundown for ChattyCog to read.",
                        );
                    }
                }

                if room_capable {
                    ui.add_space(6.0);
                    ui.label("Shared room state");
                    match read_bridge_shared_room_state(dir) {
                        Ok(Some(room_state)) => {
                            ui.small(format!(
                                "Last room-state update: {}",
                                room_state.updated_at_unix_ms
                            ));
                            ui.small(format!(
                                "Scope: {}",
                                if room_state.scope_kind.trim() == "module"
                                    && !room_state.scope_module_name.trim().is_empty()
                                {
                                    if room_state.scope_multiplayer {
                                        format!(
                                            "{} (multiplayer)",
                                            room_state.scope_module_name.trim()
                                        )
                                    } else {
                                        format!(
                                            "{} (module)",
                                            room_state.scope_module_name.trim()
                                        )
                                    }
                                } else {
                                    "General room".to_string()
                                }
                            ));
                            ui.small(format!(
                                "Active for this module: {}",
                                if room_state.active_for_module {
                                    "yes"
                                } else {
                                    "no"
                                }
                            ));
                            ui.small(format!(
                                "Turn mode: {} | AI mode: {}",
                                if room_state.turn_mode.trim().is_empty() {
                                    "(unset)"
                                } else {
                                    room_state.turn_mode.trim()
                                },
                                if room_state.ai_mode.trim().is_empty() {
                                    "(unset)"
                                } else {
                                    room_state.ai_mode.trim()
                                }
                            ));
                            if room_state.session_active {
                                ui.small(format!(
                                    "Session: {} | revision {}{}",
                                    if room_state.session_label.trim().is_empty() {
                                        if room_state.session_id.trim().is_empty() {
                                            "(unnamed session)"
                                        } else {
                                            room_state.session_id.trim()
                                        }
                                    } else {
                                        room_state.session_label.trim()
                                    },
                                    room_state.session_revision.max(1),
                                    if room_state.host_authoritative {
                                        " | host-authoritative"
                                    } else {
                                        ""
                                    }
                                ));
                            } else {
                                ui.small("Session: inactive");
                            }
                            if !room_state.host_device_name.trim().is_empty() {
                                ui.small(format!(
                                    "Host: {}",
                                    room_state.host_device_name.trim()
                                ));
                            }
                            if !room_state.turn_holder_device_name.trim().is_empty() {
                                ui.small(format!(
                                    "Turn holder: {}",
                                    room_state.turn_holder_device_name.trim()
                                ));
                            }
                            ui.small(format!(
                                "Connected peers in room: {}",
                                room_state.connected_peer_count
                            ));
                            ui.small(format!(
                                "Participants visible to module: {}",
                                room_state.participant_count
                            ));
                            if !room_state.participants.is_empty() {
                                ui.horizontal_wrapped(|ui| {
                                    for participant in room_state.participants.iter().take(8) {
                                        let label = if participant.device_name.trim().is_empty() {
                                            participant.device_id.trim()
                                        } else {
                                            participant.device_name.trim()
                                        };
                                        ui.small(if participant.is_local {
                                            format!("(local) {label}")
                                        } else {
                                            label.to_string()
                                        });
                                    }
                                });
                            }
                            if !room_state.summary.trim().is_empty() {
                                ui.group(|ui| {
                                    ui.label(room_state.summary.trim());
                                });
                            }
                        }
                        Ok(None) => {
                            ui.small(
                                "No shared_room_state.json yet. Once the shared-room lane is active, ChattyCog will mirror that room policy here for room-aware or multiplayer modules.",
                            );
                        }
                        Err(err) => {
                            ui.small(format!(
                                "Could not read shared_room_state.json: {err}"
                            ));
                        }
                    }
                }

                if room_capable {
                    ui.add_space(6.0);
                    ui.label("Recent shared room events");
                    match read_bridge_shared_room_events(dir) {
                        Ok(Some(events)) => {
                            ui.small(format!(
                                "Last event sync: {} | {} event(s)",
                                events.updated_at_unix_ms,
                                events.events.len()
                            ));
                            for event in events.events.iter().rev().take(8) {
                                ui.group(|ui| {
                                    ui.horizontal_wrapped(|ui| {
                                        ui.strong(if event.label.trim().is_empty() {
                                            event.event_type.trim()
                                        } else {
                                            event.label.trim()
                                        });
                                        ui.small(format!(
                                            "{} | {}",
                                            if event.from_device_name.trim().is_empty() {
                                                "(unknown sender)"
                                            } else {
                                                event.from_device_name.trim()
                                            },
                                            event.received_at_unix_ms
                                        ));
                                    });
                                    if !event.payload_text.trim().is_empty() {
                                        ui.label(event.payload_text.trim());
                                    } else {
                                        ui.small("(no text payload)");
                                    }
                                });
                            }
                        }
                        Ok(None) => {
                            ui.small(
                                "No shared_room_events.json yet. Room-aware modules can read a recent event feed here once peers start emitting lightweight room events.",
                            );
                        }
                        Err(err) => {
                            ui.small(format!(
                                "Could not read shared_room_events.json: {err}"
                            ));
                        }
                    }
                    match read_bridge_outgoing_room_events(dir) {
                        Ok(events) if !events.is_empty() => {
                            ui.small(format!(
                                "Queued outgoing room events: {}",
                                events.len()
                            ));
                        }
                        Ok(_) => {}
                        Err(err) => {
                            ui.small(format!(
                                "Could not read outgoing_room_events.json: {err}"
                            ));
                        }
                    }
                }

                ui.add_space(6.0);
                ui.label("Shared session state");
                if let Some(shared_state) = app.read_module_bridge_shared_state(module_id, dir) {
                    let can_publish_shared_state = module_allows_network_feature(
                        manifest,
                        ModuleNetworkFeature::SharedStatePublish,
                    );
                    let can_receive_shared_state = module_allows_network_feature(
                        manifest,
                        ModuleNetworkFeature::SharedStateReceive,
                    );
                    let tracker = app.module_session_trackers.get(module_id).cloned();
                    if shared_state.updated_at_unix_ms > 0 {
                        ui.small(format!(
                            "Last shared-state update: {}",
                            shared_state.updated_at_unix_ms
                        ));
                    }
                    if let Some(tracker) = &tracker {
                        ui.small(format!(
                            "Current shared session: {} | revision {}",
                            tracker.session_id, tracker.last_revision
                        ));
                    } else if !shared_state.session_id.trim().is_empty() {
                        ui.small(format!(
                            "Current shared session: {} | revision {}",
                            shared_state.session_id, shared_state.session_revision
                        ));
                    }
                    if !shared_state.summary.trim().is_empty() {
                        ui.group(|ui| {
                            ui.label(shared_state.summary.trim());
                        });
                    } else {
                        ui.small("This module published shared state without a human summary.");
                    }

                    if !shared_state.payload.is_null() {
                        let mut payload =
                            serde_json::to_string_pretty(&shared_state.payload).unwrap_or_default();
                        egui::ScrollArea::vertical()
                            .id_salt(format!("module_bridge_shared_state_{module_id}"))
                            .max_height(140.0)
                            .show(ui, |ui| {
                                ui.add(
                                    egui::TextEdit::multiline(&mut payload)
                                        .desired_rows(6)
                                        .interactive(false),
                                );
                            });
                    }

                    let selected_connections = app.selected_network_connection_ids();
                    ui.horizontal_wrapped(|ui| {
                        if ui.button("Start new shared session").clicked() {
                            app.reset_module_shared_session(module_id);
                            app.networking_status = format!(
                                "Networking: reset the shared session for {}.",
                                manifest
                                    .map(|mf| mf.display_name.clone())
                                    .unwrap_or_else(|| module_id.to_string())
                            );
                        }
                        if selected_connections.is_empty() {
                            ui.small(
                                "Select one or more connected peers in Networking to share this module state.",
                            );
                        } else if !can_publish_shared_state {
                            ui.small(
                                "This module has not declared `shared_state_publish` support yet.",
                            );
                        } else if ui.button("Share to selected peers").clicked() {
                            let prepared = app.prepare_outgoing_module_shared_state(module_id, &shared_state);
                            match serde_json::to_string_pretty(&prepared) {
                            Ok(text) => {
                                app.remember_recoverable_module_shared_state(
                                    module_id,
                                    &prepared,
                                    &text,
                                );
                                let label = manifest
                                    .map(|mf| format!("{} shared state", mf.display_name))
                                    .unwrap_or_else(|| format!("{module_id} shared state"));
                                let summary = if prepared.summary.trim().is_empty() {
                                    manifest
                                        .map(|mf| {
                                            format!(
                                                "Shared workflow state for {}",
                                                mf.display_name.trim()
                                            )
                                        })
                                        .unwrap_or_else(|| {
                                            format!("Shared workflow state for {module_id}")
                                        })
                                } else {
                                    prepared.summary.trim().to_string()
                                };
                                let file_name =
                                    format!("{}_shared_state.json", slugify_filename(module_id, "module"));
                                for connection_id in &selected_connections {
                                    app.networking.send_artifact(
                                        connection_id,
                                        "module_shared_state_json",
                                        &label,
                                        Some(module_id),
                                        &summary,
                                        &file_name,
                                        &text,
                                    );
                                }
                                let module_label = manifest
                                    .map(|mf| mf.display_name.clone())
                                    .unwrap_or_else(|| module_id.to_string());
                                app.networking_status = format!(
                                    "Networking: shared {} session {} revision {} with {} selected peer(s).",
                                    module_label,
                                    prepared.session_id,
                                    prepared.session_revision,
                                    selected_connections.len()
                                );
                            }
                            Err(err) => {
                                app.networking_status = format!(
                                    "Networking: could not serialize shared state for {}: {}",
                                    module_id, err
                                );
                            }
                        }
                        }
                    });
                    if !can_receive_shared_state {
                        ui.small(
                            "This module has not declared `shared_state_receive` support yet, so incoming workflow applies stay disabled.",
                        );
                    }
                } else {
                    ui.small(
                        "No shared_state.json yet. Add the optional shared-state plug if you want this module to sync a ready-to-use workflow state across the LAN.",
                    );
                }

                let has_pending_workflows = app
                    .received_workflow_inbox
                    .iter()
                    .any(|item| item.record.module_id.trim() == module_id);
                if has_pending_workflows {
                    ui.add_space(6.0);
                    app.render_received_workflow_inbox(
                        ui,
                        "Received workflow inbox",
                        Some(module_id),
                    );
                }

                if let Some(incoming) = app.read_module_bridge_incoming_shared_state(module_id, dir) {
                    ui.add_space(6.0);
                    ui.label("Incoming shared state");
                    ui.small(format!(
                        "Most recent network state came from {} [{}].",
                        if incoming.from_device_name.trim().is_empty() {
                            "(unknown device)"
                        } else {
                            incoming.from_device_name.trim()
                        },
                        incoming.from_device_id.trim()
                    ));
                    if !incoming.session_id.trim().is_empty() {
                        ui.small(format!(
                            "Session {} | revision {}{}",
                            incoming.session_id,
                            incoming.session_revision,
                            if incoming.host_authoritative {
                                " | host-authoritative"
                            } else {
                                ""
                            }
                        ));
                    }
                    if !incoming.summary.trim().is_empty() {
                        ui.group(|ui| {
                            ui.label(incoming.summary.trim());
                        });
                    }
                    if !incoming.payload.is_null() {
                        let mut payload =
                            serde_json::to_string_pretty(&incoming.payload).unwrap_or_default();
                        egui::ScrollArea::vertical()
                            .id_salt(format!("module_bridge_incoming_state_{module_id}"))
                            .max_height(120.0)
                            .show(ui, |ui| {
                                ui.add(
                                    egui::TextEdit::multiline(&mut payload)
                                        .desired_rows(5)
                                        .interactive(false),
                                );
                            });
                    }
                }

                let incoming_asset_lanes = manifest
                    .and_then(|manifest| manifest.network_capabilities.as_ref())
                    .map(|caps| caps.asset_lanes.clone())
                    .unwrap_or_default();
                if !incoming_asset_lanes.is_empty() {
                    ui.add_space(6.0);
                    ui.label("Incoming asset lanes");
                    for lane in incoming_asset_lanes {
                        let incoming_assets =
                            app.read_module_bridge_incoming_assets(module_id, dir, Some(&lane.lane_id));
                        ui.group(|ui| {
                            ui.horizontal_wrapped(|ui| {
                                ui.strong(lane.label.trim());
                                ui.small(format!(
                                    "[{} | {} waiting]",
                                    lane.lane_id,
                                    incoming_assets.len()
                                ));
                            });
                            ui.small(format!(
                                "{} | {}{}",
                                lane.direction.label(),
                                lane.delivery_mode.label(),
                                if lane.replayable { " | replayable" } else { "" }
                            ));
                            ui.horizontal_wrapped(|ui| {
                                if ui.button("Open lane folder").clicked() {
                                    open_path_in_explorer(&bridge_incoming_asset_lane_dir(
                                        dir,
                                        &lane.lane_id,
                                    ));
                                }
                                if !incoming_assets.is_empty() {
                                    ui.small("Modules can consume these from the bridge when ready.");
                                }
                            });
                            if incoming_assets.is_empty() {
                                ui.small("No assets are waiting in this lane right now.");
                            } else {
                                for asset in incoming_assets.iter().take(4) {
                                    ui.small(format!(
                                        "{} | {} | {}",
                                        if asset.label.trim().is_empty() {
                                            asset.kind.trim()
                                        } else {
                                            asset.label.trim()
                                        },
                                        if asset.from_device_name.trim().is_empty() {
                                            asset.from_device_id.trim()
                                        } else {
                                            asset.from_device_name.trim()
                                        },
                                        format_network_transfer_meta(
                                            &asset.content_type,
                                            &asset.transfer_encoding,
                                            asset.byte_len,
                                            asset.chunk_count,
                                        )
                                    ));
                                }
                            }
                            for note in &lane.notes {
                                ui.small(format!("Note: {}", note));
                            }
                        });
                    }
                }

                let receipts = app.module_session_receipts_for(module_id);
                if !receipts.is_empty() {
                    ui.add_space(6.0);
                    ui.label("Recent session apply receipts");
                    egui::ScrollArea::vertical()
                        .id_salt(format!("module_session_receipts_{module_id}"))
                        .max_height(120.0)
                        .show(ui, |ui| {
                            for receipt in receipts.iter().take(8) {
                                ui.group(|ui| {
                                    ui.horizontal_wrapped(|ui| {
                                        ui.strong(if receipt.from_device_name.trim().is_empty() {
                                            receipt.from_device_id.trim()
                                        } else {
                                            receipt.from_device_name.trim()
                                        });
                                        ui.small(format!(
                                            "session {} | revision {} | {}",
                                            receipt.session_id,
                                            receipt.session_revision,
                                            if receipt.applied {
                                                "applied"
                                            } else if receipt.stale {
                                                "stale"
                                            } else {
                                                "not applied"
                                            }
                                        ));
                                    });
                                    if !receipt.message.trim().is_empty() {
                                        ui.small(receipt.message.trim());
                                    }
                                });
                                ui.add_space(4.0);
                            }
                        });
                }
            });

        ui.add_space(8.0);
    }

    egui::CollapsingHeader::new("Suspend rundown (what Orchestrator sees)")
        .default_open(false)
        .show(ui, |ui| {
            ui.label("Short status used for the Bookkeeper debrief when you leave this tab.");
            ui.horizontal(|ui| {
                let running = app.module_rundown_jobs.contains_key(module_id);
                ui.add_enabled_ui(!running, |ui| {
                    if ui.button("Auto-generate (Bookkeeper)").clicked() {
                        app.start_module_rundown_job(module_id, true, false);
                    }
                });
                if running {
                    ui.small("Generating...");
                }
                ui.separator();
                ui.small(format!(
                    "Auto-generate on tab leave: {} (set in Preferences)",
                    if app.prefs.auto_generate_module_suspend_rundown {
                        "ON"
                    } else {
                        "OFF"
                    }
                ));
                if ui.button("Clear").clicked() {
                    let notes = app
                        .module_state_notes
                        .entry(module_id.to_string())
                        .or_insert_with(String::new);
                    notes.clear();
                }
            });
            let notes = app
                .module_state_notes
                .entry(module_id.to_string())
                .or_insert_with(String::new);
            egui::ScrollArea::vertical()
                .id_salt(format!("module_notes_{module_id}"))
                .show(ui, |ui| {
                    ui.add(
                        egui::TextEdit::multiline(notes)
                            .desired_rows(10)
                            .hint_text("One paragraph max. What changed? What's next?"),
                    );
                });
        });

    ui.add_space(8.0);

    if let Some(mf) = manifest {
        if mf.ai_enabled {
            ui.separator();
            ui.heading("Module AI");
            ui.label("This module can run its own local model while the orchestrator is paused.");

            let models_dir = app.models_dir.clone();
            let modules_dir = app.modules_dir.clone();
            let model_opts = build_model_options(app.models_dir.as_deref(), app.modules_dir.as_deref());
            let preferred_model_hint = app
                .prefs
                .modules
                .get(module_id)
                .and_then(|p| p.preferred_model.as_ref())
                .filter(|s| !s.trim().is_empty())
                .cloned()
                .or_else(|| mf.default_model.clone());
            let preferred_model_path = app.resolve_portable_model_hint(preferred_model_hint.as_deref());

            let st = app
                .module_ai
                .entry(module_id.to_string())
                .or_insert_with(ModuleAiState::default);

            if !st.initialized {
                if let Some(p) = app.prefs.modules.get(module_id) {
                    st.temp = p.params.temp;
                    st.top_p = p.params.top_p;
                    st.top_k = p.params.top_k;
                    st.max_tokens = p.params.max_tokens;
                }
                st.initialized = true;
            }
            if st.model_path.is_none() {
                st.model_path = preferred_model_path;
            }

            ui.horizontal(|ui| {
                if ui.button("Refresh models").clicked() {
                    st.models_cache = scan_ggufs(app.models_dir.as_deref());
                    app.models_cache = scan_ggufs(app.models_dir.as_deref());
                }
                if ui.button("Stop").clicked() {
                    if let Some(c) = &st.cancel {
                        c.store(true, Ordering::Relaxed);
                    }
                }
                if !st.status.trim().is_empty() {
                    ui.label(st.status.clone());
                }
            });

            let selected_hint = portable_model_hint_for_dirs(
                models_dir.as_deref(),
                modules_dir.as_deref(),
                st.model_path.as_deref(),
            );
            let selected_label = selected_model_option_label(
                &model_opts,
                selected_hint.as_deref(),
                st.model_path.as_ref().map(|p| {
                    p.file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_else(|| p.display().to_string())
                }),
            );
            let mut picked_model: Option<Option<String>> = None;
            ui.horizontal(|ui| {
                ui.label("Module model");
                picked_model = show_grouped_model_option_combo(
                    ui,
                    ("module_model_combo", module_id),
                    selected_label,
                    &model_opts,
                    selected_hint.as_deref(),
                );
            });
            if let Some(picked) = picked_model {
                st.model_path = resolve_portable_model_hint_for_dirs(
                    models_dir.as_deref(),
                    modules_dir.as_deref(),
                    picked.as_deref(),
                );
            }

            ui.horizontal(|ui| {
                ui.add(egui::Slider::new(&mut st.temp, 0.0..=2.0).text("temp"));
                ui.add(egui::Slider::new(&mut st.top_p, 0.0..=1.0).text("top_p"));
                ui.add(egui::Slider::new(&mut st.top_k, 0..=200).text("top_k"));
                ui.add(egui::Slider::new(&mut st.max_tokens, 1..=2048).text("max_tokens"));
            });

            let events = st
                .rx
                .as_ref()
                .map(|rx| rx.try_iter().collect::<Vec<_>>())
                .unwrap_or_default();
            let mut close_after = false;
            for ev in events {
                match ev {
                    GenEvent::Token(t) => st.output.push_str(&t),
                    GenEvent::Info(s) => {
                        st.status = format!("Runtime: {}", truncate_for_ui(&s, 120))
                    }
                    GenEvent::Error(e) => {
                        st.status = format!("Error: {e}");
                        st.is_running = false;
                        st.cancel = None;
                        st.rx = None;
                        if app.close_pending_modules.contains(module_id) {
                            close_after = true;
                        }
                    }
                    GenEvent::Done => {
                        st.is_running = false;
                        st.cancel = None;
                        st.rx = None;
                        st.status = "Done.".to_string();
                        if app.close_pending_modules.contains(module_id) {
                            close_after = true;
                        }
                    }
                }
            }
            if close_after {
                let module_id = module_id.to_string();
                let _ = st;
                close_module_tab(app, &module_id);
                return;
            }

            ui.add_space(6.0);
            ui.label("Task input:");
            ui.add(
                egui::TextEdit::multiline(&mut st.user_input)
                    .desired_rows(4)
                    .hint_text("Describe the task this department should handle..."),
            );

            ui.horizontal(|ui| {
                ui.add_enabled_ui(!st.is_running, |ui| {
                    if ui.button("Run").clicked() {
                        let Some(model_path) = st.model_path.clone() else {
                            st.status = "Pick a module model first.".to_string();
                            return;
                        };
                        let runtime_dir = match find_runtime_windows_dir() {
                            Ok(p) => p,
                            Err(e) => {
                                st.status = format!("{e:#}");
                                return;
                            }
                        };

                        let input = st.user_input.trim().to_string();
                        if input.is_empty() {
                            st.status = "Enter a task first.".to_string();
                            return;
                        }

                        let module_name = mf.display_name.clone();
                        let module_description = mf.description.clone();
                        let system = format!(
                            "You are the {module_name} department inside ChattyCog.\n\
Module purpose: {module_description}\n\
Help with the task using the current module state as context.\n\
Keep the reply practical and concise.\n"
                        );

                        let (tx, rx) = crossbeam_channel::unbounded::<GenEvent>();
                        let cancel = Arc::new(AtomicBool::new(false));
                        let cancel_for_thread = Arc::clone(&cancel);
                        let temp = st.temp;
                        let top_p = st.top_p;
                        let top_k = st.top_k;
                        let max_tokens = st.max_tokens.max(1) as usize;

                        st.output.clear();
                        st.status = "Running...".to_string();
                        st.is_running = true;
                        st.cancel = Some(cancel);
                        st.rx = Some(rx);

                        std::thread::spawn(move || {
                            let llama = match llama_dyn::Llama::load(&runtime_dir) {
                                Ok(l) => l,
                                Err(e) => {
                                    let _ = tx.send(GenEvent::Error(format!("{e:#}")));
                                    let _ = tx.send(GenEvent::Done);
                                    return;
                                }
                            };
                            let info = llama.system_info();
                            if !info.is_empty() {
                                let _ = tx.send(GenEvent::Info(info));
                            }
                            let res = llama.generate_chat(
                                &model_path,
                                &system,
                                &input,
                                max_tokens,
                                temp,
                                top_p,
                                top_k,
                                &cancel_for_thread,
                                |tok| {
                                    let _ = tx.send(GenEvent::Token(tok.to_string()));
                                },
                            );
                            if let Err(e) = res {
                                let _ = tx.send(GenEvent::Error(format!("{e:#}")));
                            }
                            let _ = tx.send(GenEvent::Done);
                        });
                    }
                });

                if ui.button("Copy output -> suspend rundown").clicked() {
                    let block = st.output.trim();
                    if !block.is_empty() {
                        let entry = app
                            .module_state_notes
                            .entry(module_id.to_string())
                            .or_insert_with(String::new);
                        if !entry.trim().is_empty() {
                            entry.push_str("\n\n");
                        }
                        entry.push_str(block);
                    }
                }
            });

            ui.add_space(6.0);
            ui.label("Output:");
            egui::ScrollArea::vertical()
                .id_salt(format!("module_ai_out_{module_id}"))
                .show(ui, |ui| {
                    ui.add(
                        egui::TextEdit::multiline(&mut st.output)
                            .desired_rows(12)
                            .code_editor(),
                    );
                });
        }
    }
}

pub(super) fn open_path_in_explorer(path: &Path) {
    let _ = std::process::Command::new("explorer.exe").arg(path).spawn();
}

fn value_is_meaningful(value: &ModuleFieldValue) -> bool {
    match value {
        ModuleFieldValue::Str(s) => !s.trim().is_empty(),
        ModuleFieldValue::Bool(b) => *b,
        ModuleFieldValue::Num(n) => n.abs() > f64::EPSILON,
    }
}

fn filled_field_count(spec: &ModuleUiSpec, values: &HashMap<String, ModuleFieldValue>) -> usize {
    spec.fields
        .iter()
        .filter(|f| values.get(f.id.trim()).is_some_and(value_is_meaningful))
        .count()
}

fn humanize_section_id(section: &str) -> String {
    let cleaned = section.trim().replace(['_', '-'], " ");
    if cleaned.is_empty() {
        return "Workspace".to_string();
    }
    cleaned
        .split_whitespace()
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn module_surface_path(root: &Path, relative: &str) -> Option<PathBuf> {
    let trimmed = relative.trim();
    if trimmed.is_empty() || trimmed == "." {
        return Some(root.to_path_buf());
    }

    let rel_path = Path::new(trimmed);
    if rel_path.is_absolute() {
        return None;
    }
    if rel_path.components().any(|component| {
        matches!(
            component,
            std::path::Component::ParentDir
                | std::path::Component::RootDir
                | std::path::Component::Prefix(_)
        )
    }) {
        return None;
    }

    Some(root.join(rel_path))
}

fn module_field_label(spec: &ModuleUiSpec, field_id: &str) -> String {
    spec.fields
        .iter()
        .find(|field| field.id.trim() == field_id.trim())
        .map(|field| field.label.clone())
        .unwrap_or_else(|| humanize_section_id(field_id))
}

fn module_field_spec<'a>(spec: &'a ModuleUiSpec, field_id: &str) -> Option<&'a ModuleUiField> {
    spec.fields
        .iter()
        .find(|field| field.id.trim() == field_id.trim())
}

fn module_field_value_as_text(
    values: &HashMap<String, ModuleFieldValue>,
    field_id: &str,
) -> Option<String> {
    match values.get(field_id.trim()) {
        Some(ModuleFieldValue::Str(value)) => {
            let trimmed = value.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        }
        Some(ModuleFieldValue::Bool(value)) => Some(if *value { "Yes" } else { "No" }.to_string()),
        Some(ModuleFieldValue::Num(value)) => Some(
            format!("{value:.2}")
                .trim_end_matches('0')
                .trim_end_matches('.')
                .to_string(),
        ),
        None => None,
    }
}

fn module_field_value_as_number(
    values: &HashMap<String, ModuleFieldValue>,
    field_id: &str,
) -> Option<f64> {
    match values.get(field_id.trim()) {
        Some(ModuleFieldValue::Num(value)) => Some(*value),
        Some(ModuleFieldValue::Str(value)) => value.trim().parse::<f64>().ok(),
        Some(ModuleFieldValue::Bool(value)) => Some(if *value { 1.0 } else { 0.0 }),
        None => None,
    }
}

fn module_field_candidate_paths(
    values: &HashMap<String, ModuleFieldValue>,
    field_id: &str,
) -> Vec<String> {
    let Some(text) = module_field_value_as_text(values, field_id) else {
        return Vec::new();
    };

    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(|line| {
            line.trim_start_matches("- ")
                .trim_start_matches("* ")
                .trim()
        })
        .map(|line| line.trim_matches('"').trim_matches('\'').to_string())
        .filter(|line| !line.is_empty())
        .collect()
}

fn resolve_module_block_paths(
    module_dir: &Path,
    values: &HashMap<String, ModuleFieldValue>,
    explicit_path: &str,
    field_id: &str,
) -> Vec<PathBuf> {
    let mut candidates = Vec::new();

    if let Some(path) = module_surface_path(module_dir, explicit_path) {
        candidates.push(path);
    }

    if !field_id.trim().is_empty() {
        for relative in module_field_candidate_paths(values, field_id) {
            if let Some(path) = module_surface_path(module_dir, &relative) {
                candidates.push(path);
            }
        }
    }

    let mut deduped = Vec::new();
    let mut seen = HashSet::new();
    for path in candidates {
        let key = path.to_string_lossy().to_string();
        if seen.insert(key) {
            deduped.push(path);
        }
    }
    deduped
}

fn looks_like_text_file(path: &Path) -> bool {
    match path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_ascii_lowercase())
        .as_deref()
    {
        Some(
            "md" | "txt" | "json" | "jsonl" | "toml" | "yaml" | "yml" | "rs" | "py" | "js" | "ts"
            | "tsx" | "jsx" | "csv" | "log" | "ini" | "cfg" | "html" | "css" | "xml",
        ) => true,
        _ => false,
    }
}

fn preview_text_file(path: &Path, max_lines: usize) -> Option<String> {
    let bytes = std::fs::read(path).ok()?;
    let limit = bytes.len().min(16 * 1024);
    let preview = String::from_utf8_lossy(&bytes[..limit]).to_string();
    let mut lines = preview.lines().take(max_lines.max(1)).collect::<Vec<_>>();
    if lines.is_empty() && !preview.trim().is_empty() {
        lines.push(preview.trim());
    }
    let mut text = lines.join("\n");
    if bytes.len() > limit || preview.lines().count() > max_lines {
        if !text.is_empty() {
            text.push_str("\n…");
        } else {
            text.push('…');
        }
    }
    Some(text)
}

fn read_module_text_source(
    module_dir: &Path,
    values: &HashMap<String, ModuleFieldValue>,
    explicit_path: &str,
    field_id: &str,
) -> Option<String> {
    if !field_id.trim().is_empty() {
        if let Some(content) = module_field_value_as_text(values, field_id) {
            return Some(content);
        }
    }

    let candidates = resolve_module_block_paths(module_dir, values, explicit_path, field_id);
    let path = candidates.into_iter().find(|path| path.is_file())?;
    if !looks_like_text_file(&path) {
        return None;
    }
    std::fs::read_to_string(path).ok()
}

fn read_module_table_source(
    module_dir: &Path,
    values: &HashMap<String, ModuleFieldValue>,
    explicit_path: &str,
    field_id: &str,
) -> Option<String> {
    let content = read_module_text_source(module_dir, values, explicit_path, field_id)?;
    if content.contains('\n')
        || content.contains('|')
        || content.contains('\t')
        || content.contains(',')
        || content.contains(';')
    {
        Some(content)
    } else {
        None
    }
}

fn split_table_row(line: &str, delimiter: char) -> Vec<String> {
    if delimiter == '|' {
        line.trim()
            .trim_matches('|')
            .split('|')
            .map(|cell| cell.trim().to_string())
            .collect()
    } else {
        line.split(delimiter)
            .map(|cell| cell.trim().to_string())
            .collect()
    }
}

fn parse_lightweight_table(
    content: &str,
    has_header: bool,
) -> Option<(Vec<String>, Vec<Vec<String>>)> {
    let lines = content
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>();
    if lines.is_empty() {
        return None;
    }

    let delimiters = ['|', '\t', ',', ';'];
    let delimiter = delimiters.into_iter().find(|delimiter| {
        let matching = lines
            .iter()
            .take(6)
            .filter(|line| line.contains(*delimiter))
            .count();
        matching >= 1
    })?;

    let mut rows = lines
        .iter()
        .map(|line| split_table_row(line, delimiter))
        .filter(|row| row.len() >= 2)
        .collect::<Vec<_>>();
    if rows.is_empty() {
        return None;
    }

    let width = rows.iter().map(|row| row.len()).max().unwrap_or(0);
    if width == 0 {
        return None;
    }
    for row in &mut rows {
        while row.len() < width {
            row.push(String::new());
        }
    }

    let (header, body_start) = if has_header && rows.len() >= 2 {
        (rows[0].clone(), 1usize)
    } else {
        (
            (1..=width)
                .map(|idx| format!("Column {idx}"))
                .collect::<Vec<_>>(),
            0usize,
        )
    };

    let body = rows.into_iter().skip(body_start).collect::<Vec<_>>();
    Some((header, body))
}

#[derive(Debug, Clone, Copy)]
enum ChecklistState {
    Pending,
    InProgress,
    Done,
    Note,
}

#[derive(Debug, Clone)]
struct KanbanCard {
    lane: String,
    text: String,
}

#[derive(Debug, Clone)]
struct DependencyNodeView {
    name: String,
    depends_on: Vec<String>,
    unlocks: Vec<String>,
    stage: usize,
}

fn title_case_words(s: &str) -> String {
    s.split_whitespace()
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                Some(first) => {
                    let mut out = String::new();
                    out.extend(first.to_uppercase());
                    out.push_str(&chars.as_str().to_ascii_lowercase());
                    out
                }
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn canonical_lane_name(raw: &str) -> String {
    let compact = raw
        .trim()
        .replace('_', " ")
        .replace('-', " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let lowered = compact.to_ascii_lowercase();
    match lowered.as_str() {
        "todo" | "to do" | "backlog" | "queued" | "queue" => "To Do".to_string(),
        "doing" | "in progress" | "inprogress" | "active" | "working" => "Doing".to_string(),
        "review" | "qa" | "verify" | "verification" => "Review".to_string(),
        "blocked" | "waiting" | "stalled" => "Blocked".to_string(),
        "done" | "complete" | "completed" | "finished" => "Done".to_string(),
        "note" | "notes" => "Notes".to_string(),
        "inbox" => "Inbox".to_string(),
        _ if compact.is_empty() => "Inbox".to_string(),
        _ => title_case_words(&compact),
    }
}

fn lane_accent_color(lane: &str) -> egui::Color32 {
    match canonical_lane_name(lane).to_ascii_lowercase().as_str() {
        "to do" | "inbox" => egui::Color32::from_rgb(30, 80, 180),
        "doing" | "review" => egui::Color32::from_rgb(180, 110, 10),
        "blocked" => egui::Color32::from_rgb(180, 40, 40),
        "done" => egui::Color32::from_rgb(20, 120, 60),
        _ => egui::Color32::from_gray(120),
    }
}

fn parse_checklist_line(line: &str) -> Option<(ChecklistState, String)> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return None;
    }

    let candidates = [
        ("- [x] ", ChecklistState::Done),
        ("* [x] ", ChecklistState::Done),
        ("[x] ", ChecklistState::Done),
        ("- [X] ", ChecklistState::Done),
        ("* [X] ", ChecklistState::Done),
        ("[X] ", ChecklistState::Done),
        ("- [ ] ", ChecklistState::Pending),
        ("* [ ] ", ChecklistState::Pending),
        ("[ ] ", ChecklistState::Pending),
        ("- [-] ", ChecklistState::InProgress),
        ("* [-] ", ChecklistState::InProgress),
        ("[-] ", ChecklistState::InProgress),
        ("- [~] ", ChecklistState::InProgress),
        ("* [~] ", ChecklistState::InProgress),
        ("[~] ", ChecklistState::InProgress),
    ];

    for (prefix, state) in candidates {
        if let Some(rest) = trimmed.strip_prefix(prefix) {
            let text = rest.trim();
            if !text.is_empty() {
                return Some((state, text.to_string()));
            }
        }
    }

    if let Some(rest) = trimmed
        .strip_prefix("- ")
        .or_else(|| trimmed.strip_prefix("* "))
    {
        let text = rest.trim();
        if !text.is_empty() {
            return Some((ChecklistState::Pending, text.to_string()));
        }
    }

    if let Some((prefix, rest)) = trimmed.split_once(". ") {
        if prefix.chars().all(|ch| ch.is_ascii_digit()) {
            let text = rest.trim();
            if !text.is_empty() {
                return Some((ChecklistState::Pending, text.to_string()));
            }
        }
    }

    Some((ChecklistState::Note, trimmed.to_string()))
}

fn parse_timeline_line(line: &str) -> Option<(String, String)> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return None;
    }

    if let Some(rest) = trimmed
        .strip_prefix("- ")
        .or_else(|| trimmed.strip_prefix("* "))
    {
        return parse_timeline_line(rest);
    }

    if trimmed.starts_with('[') {
        if let Some(end) = trimmed.find(']') {
            let stamp = trimmed[1..end].trim();
            let rest = trimmed[end + 1..]
                .trim_start_matches([' ', '-', '—', '|'])
                .trim();
            if !stamp.is_empty() && !rest.is_empty() {
                return Some((stamp.to_string(), rest.to_string()));
            }
        }
    }

    for sep in [" | ", " — ", " - "] {
        if let Some((left, right)) = trimmed.split_once(sep) {
            let left = left.trim();
            let right = right.trim();
            if !left.is_empty() && !right.is_empty() {
                return Some((left.to_string(), right.to_string()));
            }
        }
    }

    Some((String::new(), trimmed.to_string()))
}

fn parse_kanban_line(line: &str) -> Option<(String, String)> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return None;
    }

    if let Some(rest) = trimmed
        .strip_prefix("- ")
        .or_else(|| trimmed.strip_prefix("* "))
    {
        if let Some((lane, text)) = parse_kanban_line(rest) {
            return Some((lane, text));
        }
    }

    if let Some((state, text)) = parse_checklist_line(trimmed) {
        let lane = match state {
            ChecklistState::Pending => "To Do",
            ChecklistState::InProgress => "Doing",
            ChecklistState::Done => "Done",
            ChecklistState::Note => "Notes",
        };
        return Some((lane.to_string(), text));
    }

    if trimmed.starts_with('[') {
        if let Some(end) = trimmed.find(']') {
            let lane = canonical_lane_name(&trimmed[1..end]);
            let rest = trimmed[end + 1..]
                .trim_start_matches([' ', '-', ':', '|'])
                .trim();
            if !rest.is_empty() {
                return Some((lane, rest.to_string()));
            }
        }
    }

    if let Some((left, right)) = trimmed.split_once(" | ") {
        let lane = canonical_lane_name(left);
        let text = right.trim();
        if !text.is_empty() {
            return Some((lane, text.to_string()));
        }
    }

    if let Some((left, right)) = trimmed.split_once(": ") {
        let left = left.trim();
        let right = right.trim();
        if !left.is_empty() && !right.is_empty() && left.len() <= 24 {
            return Some((canonical_lane_name(left), right.to_string()));
        }
    }

    Some(("Inbox".to_string(), trimmed.to_string()))
}

fn parse_kanban_content(
    content: &str,
    preferred_lanes: &[String],
) -> (Vec<String>, Vec<KanbanCard>) {
    let mut lanes = Vec::new();
    let mut seen_lanes = HashSet::new();
    for lane in preferred_lanes {
        let lane = canonical_lane_name(lane);
        if seen_lanes.insert(lane.clone()) {
            lanes.push(lane);
        }
    }

    let mut cards = Vec::new();
    for (lane, text) in content.lines().filter_map(parse_kanban_line) {
        let lane = canonical_lane_name(&lane);
        if seen_lanes.insert(lane.clone()) {
            lanes.push(lane.clone());
        }
        cards.push(KanbanCard { lane, text });
    }

    (lanes, cards)
}

fn normalize_dependency_segments(line: &str) -> Vec<String> {
    line.replace("=>", "->")
        .replace('→', "->")
        .split("->")
        .map(|part| {
            part.trim()
                .trim_start_matches("- ")
                .trim_start_matches("* ")
                .trim()
                .to_string()
        })
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
}

fn parse_dependency_graph_content(content: &str) -> Vec<DependencyNodeView> {
    let mut node_order = Vec::new();
    let mut seen_nodes = HashSet::new();
    let mut edges = Vec::new();
    let mut seen_edges = HashSet::new();

    let mut remember_node = |node: String| {
        if seen_nodes.insert(node.clone()) {
            node_order.push(node);
        }
    };

    for raw_line in content.lines() {
        let trimmed = raw_line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let segments = normalize_dependency_segments(trimmed);
        if segments.len() >= 2 {
            for segment in &segments {
                remember_node(segment.clone());
            }
            for pair in segments.windows(2) {
                let edge = (pair[0].clone(), pair[1].clone());
                let key = format!("{}->{}", edge.0, edge.1);
                if seen_edges.insert(key) {
                    edges.push(edge);
                }
            }
        } else if let Some(node) = segments.first() {
            remember_node(node.clone());
        } else {
            remember_node(trimmed.to_string());
        }
    }

    if node_order.is_empty() {
        return Vec::new();
    }

    let mut depends_on: HashMap<String, Vec<String>> = HashMap::new();
    let mut unlocks: HashMap<String, Vec<String>> = HashMap::new();
    let mut indegree: HashMap<String, usize> = HashMap::new();
    let mut levels: HashMap<String, usize> = HashMap::new();

    for node in &node_order {
        indegree.insert(node.clone(), 0);
    }

    for (from, to) in &edges {
        unlocks.entry(from.clone()).or_default().push(to.clone());
        depends_on.entry(to.clone()).or_default().push(from.clone());
        *indegree.entry(to.clone()).or_default() += 1;
        indegree.entry(from.clone()).or_default();
    }

    let mut queue = VecDeque::new();
    for node in &node_order {
        if indegree.get(node).copied().unwrap_or(0) == 0 {
            queue.push_back(node.clone());
            levels.entry(node.clone()).or_insert(0);
        }
    }

    let mut processed = HashSet::new();
    while let Some(node) = queue.pop_front() {
        if !processed.insert(node.clone()) {
            continue;
        }
        let node_level = levels.get(&node).copied().unwrap_or(0);
        if let Some(children) = unlocks.get(&node) {
            for child in children {
                let next_level = node_level + 1;
                if next_level > levels.get(child).copied().unwrap_or(0) {
                    levels.insert(child.clone(), next_level);
                }
                if let Some(entry) = indegree.get_mut(child) {
                    *entry = entry.saturating_sub(1);
                    if *entry == 0 {
                        queue.push_back(child.clone());
                    }
                }
            }
        }
    }

    if processed.len() < node_order.len() {
        let fallback_stage = levels.values().copied().max().unwrap_or(0) + 1;
        for node in &node_order {
            if !processed.contains(node) {
                levels.entry(node.clone()).or_insert(fallback_stage);
            }
        }
    }

    node_order
        .into_iter()
        .map(|name| DependencyNodeView {
            stage: levels.get(&name).copied().unwrap_or(0),
            depends_on: depends_on.remove(&name).unwrap_or_default(),
            unlocks: unlocks.remove(&name).unwrap_or_default(),
            name,
        })
        .collect()
}

fn text_matches_filter(haystack: &str, query: &str) -> bool {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return true;
    }
    let haystack = haystack.to_ascii_lowercase();
    trimmed
        .split_whitespace()
        .all(|term| haystack.contains(&term.to_ascii_lowercase()))
}

fn module_block_filter_query(
    ui: &mut egui::Ui,
    module_id: &str,
    ui_id: &str,
    searchable: bool,
    item_count: usize,
    custom_placeholder: &str,
    default_placeholder: &str,
    presets: &[ModuleUiFilterPreset],
) -> String {
    if !searchable {
        return String::new();
    }

    let filter_id = egui::Id::new(("module_surface_filter", module_id, ui_id));
    let mut query = ui
        .ctx()
        .data_mut(|data| data.get_temp::<String>(filter_id))
        .unwrap_or_default();
    let should_show = item_count > 4 || !query.trim().is_empty() || !presets.is_empty();
    if !should_show {
        return query;
    }

    let placeholder = if custom_placeholder.trim().is_empty() {
        default_placeholder.to_string()
    } else {
        custom_placeholder.trim().to_string()
    };

    if !presets.is_empty() {
        ui.horizontal_wrapped(|ui| {
            let all_selected = query.trim().is_empty();
            if ui.selectable_label(all_selected, "All").clicked() {
                query.clear();
            }
            for preset in presets {
                let label = preset.label.trim();
                if label.is_empty() {
                    continue;
                }
                let selected = query.trim().eq_ignore_ascii_case(preset.query.trim());
                if ui.selectable_label(selected, label).clicked() {
                    query = preset.query.trim().to_string();
                }
            }
        });
        ui.add_space(4.0);
    }

    ui.horizontal(|ui| {
        ui.small("Filter");
        ui.add(
            egui::TextEdit::singleline(&mut query)
                .desired_width(180.0)
                .hint_text(placeholder),
        );
        if !query.trim().is_empty() && ui.small_button("Clear").clicked() {
            query.clear();
        }
    });
    ui.add_space(6.0);

    ui.ctx()
        .data_mut(|data| data.insert_temp(filter_id, query.clone()));
    query
}

fn module_layout_preset_selection(
    ui: &mut egui::Ui,
    module_id: &str,
    ui_id: &str,
    presets: &[ModuleUiViewPreset],
) -> Option<usize> {
    if presets.is_empty() {
        return None;
    }

    let preset_id = egui::Id::new(("module_layout_preset", module_id, ui_id));
    let mut active = ui
        .ctx()
        .data_mut(|data| data.get_temp::<usize>(preset_id))
        .unwrap_or(0);
    if active > presets.len() {
        active = 0;
    }

    ui.horizontal_wrapped(|ui| {
        if ui.selectable_label(active == 0, "Default").clicked() {
            active = 0;
        }
        for (idx, preset) in presets.iter().enumerate() {
            let label = preset.label.trim();
            if label.is_empty() {
                continue;
            }
            if ui.selectable_label(active == idx + 1, label).clicked() {
                active = idx + 1;
            }
        }
    });
    ui.add_space(6.0);
    ui.ctx()
        .data_mut(|data| data.insert_temp(preset_id, active));

    active.checked_sub(1)
}

fn module_layout_visible_panes<'a>(
    panes: &'a [ResolvedModuleUiPane],
    preset: Option<&ModuleUiViewPreset>,
) -> Vec<&'a ResolvedModuleUiPane> {
    let Some(preset) = preset else {
        return panes.iter().collect();
    };

    let pane_ids = preset
        .pane_ids
        .iter()
        .map(|pane_id| pane_id.trim())
        .filter(|pane_id| !pane_id.is_empty())
        .collect::<Vec<_>>();
    if pane_ids.is_empty() {
        return panes.iter().collect();
    }

    let mut visible = Vec::new();
    let mut seen = HashSet::new();
    for pane_id in pane_ids {
        if let Some(pane) = panes.iter().find(|pane| pane.id == pane_id) {
            if seen.insert(pane.id.clone()) {
                visible.push(pane);
            }
        }
    }

    if visible.is_empty() {
        panes.iter().collect()
    } else {
        visible
    }
}

fn normalized_block_id(block: &ModuleUiBlock, fallback: &str) -> String {
    if !block.id.trim().is_empty() {
        block.id.trim().to_string()
    } else if !block.title.trim().is_empty() {
        format!(
            "{fallback}:{}",
            block.title.trim().to_lowercase().replace(' ', "_")
        )
    } else if !block.field.trim().is_empty() {
        format!("{fallback}:{}", block.field.trim())
    } else {
        fallback.to_string()
    }
}

fn resolve_module_ui_container_blocks(
    spec: &ModuleUiSpec,
    blocks_cfg: &[ModuleUiBlock],
    field_ids: &[String],
    used: &mut HashSet<String>,
    container_key: &str,
) -> Vec<ResolvedModuleUiBlock> {
    let mut blocks = Vec::new();

    for (idx, block) in blocks_cfg.iter().enumerate() {
        let block_key = format!("{container_key}:block_{idx}");
        if let Some(block) = resolve_module_ui_block(spec, block, used, &block_key) {
            blocks.push(block);
        }
    }

    for field_id in field_ids {
        if let Some(field) = spec
            .fields
            .iter()
            .find(|field| field.id.trim() == field_id.trim())
        {
            if used.insert(field.id.clone()) {
                blocks.push(ResolvedModuleUiBlock::Field(field.clone()));
            }
        }
    }

    blocks
}

fn resolve_module_ui_panes(
    spec: &ModuleUiSpec,
    panes: &[ModuleUiPane],
    used: &mut HashSet<String>,
    container_key: &str,
) -> Vec<ResolvedModuleUiPane> {
    let mut resolved = Vec::new();

    for (idx, pane) in panes.iter().enumerate() {
        let pane_key = if !pane.id.trim().is_empty() {
            format!("{container_key}:{}", pane.id.trim())
        } else {
            format!("{container_key}:pane_{idx}")
        };
        let blocks =
            resolve_module_ui_container_blocks(spec, &pane.blocks, &pane.fields, used, &pane_key);
        if blocks.is_empty() {
            continue;
        }
        resolved.push(ResolvedModuleUiPane {
            id: if !pane.id.trim().is_empty() {
                pane.id.trim().to_string()
            } else {
                format!("pane_{idx}")
            },
            title: pane.title.clone(),
            description: pane.description.clone(),
            summary: pane.summary.clone(),
            summary_field: pane.summary_field.clone(),
            blocks,
            weight: pane.weight.unwrap_or(1.0).max(0.1),
            default_open: pane.default_open.unwrap_or(false),
        });
    }

    resolved
}

fn render_markdownish(ui: &mut egui::Ui, text: &str) {
    let mut in_code = false;
    let mut code_lines = Vec::<String>::new();

    let flush_code = |ui: &mut egui::Ui, code_lines: &mut Vec<String>| {
        if code_lines.is_empty() {
            return;
        }
        let code = code_lines.join("\n");
        egui::Frame::group(ui.style())
            .inner_margin(egui::Margin::same(8.0))
            .show(ui, |ui| {
                ui.add(egui::Label::new(egui::RichText::new(code).monospace()).wrap());
            });
        code_lines.clear();
    };

    for raw_line in text.lines() {
        let line = raw_line.trim_end();
        let trimmed = line.trim();

        if trimmed.starts_with("```") {
            if in_code {
                flush_code(ui, &mut code_lines);
            }
            in_code = !in_code;
            continue;
        }

        if in_code {
            code_lines.push(line.to_string());
            continue;
        }

        if trimmed.is_empty() {
            ui.add_space(4.0);
            continue;
        }

        if let Some(rest) = trimmed.strip_prefix("### ") {
            ui.label(egui::RichText::new(rest).strong());
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("## ") {
            ui.label(egui::RichText::new(rest).heading());
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("# ") {
            ui.label(egui::RichText::new(rest).heading().strong());
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("> ") {
            ui.label(egui::RichText::new(rest).italics());
            continue;
        }
        if let Some(rest) = trimmed
            .strip_prefix("- ")
            .or_else(|| trimmed.strip_prefix("* "))
        {
            ui.label(format!("• {rest}"));
            continue;
        }
        if let Some((prefix, rest)) = trimmed.split_once(". ") {
            if !prefix.is_empty() && prefix.chars().all(|ch| ch.is_ascii_digit()) {
                ui.label(format!("{prefix}. {rest}"));
                continue;
            }
        }

        ui.label(trimmed);
    }

    if in_code {
        flush_code(ui, &mut code_lines);
    }
}

fn resolve_module_ui_block(
    spec: &ModuleUiSpec,
    block: &ModuleUiBlock,
    used: &mut HashSet<String>,
    container_key: &str,
) -> Option<ResolvedModuleUiBlock> {
    let kind = block.kind.trim().to_lowercase();
    match kind.as_str() {
        "" | "field" => {
            let field_id = block.field.trim();
            let field = spec
                .fields
                .iter()
                .find(|field| field.id.trim() == field_id)?;
            if used.insert(field.id.clone()) {
                Some(ResolvedModuleUiBlock::Field(field.clone()))
            } else {
                None
            }
        }
        "text" => {
            let text = block.text.trim().to_string();
            let title = block.title.trim().to_string();
            if text.is_empty() && title.is_empty() {
                None
            } else {
                Some(ResolvedModuleUiBlock::Text { title, text })
            }
        }
        "markdown" => {
            let field_id = block.field.trim().to_string();
            let text = block.text.trim().to_string();
            let title = block.title.trim().to_string();
            if text.is_empty() && field_id.is_empty() && title.is_empty() {
                None
            } else {
                Some(ResolvedModuleUiBlock::Markdown {
                    title,
                    text,
                    field_id,
                    empty: block.empty.trim().to_string(),
                })
            }
        }
        "callout" => {
            let text = block.text.trim().to_string();
            let title = block.title.trim().to_string();
            if text.is_empty() && title.is_empty() {
                None
            } else {
                Some(ResolvedModuleUiBlock::Callout {
                    title,
                    text,
                    tone: block.tone.trim().to_string(),
                })
            }
        }
        "stat" => {
            let field_id = block.field.trim().to_string();
            if field_id.is_empty() {
                return None;
            }
            Some(ResolvedModuleUiBlock::Stat {
                label: if block.label.trim().is_empty() {
                    module_field_label(spec, &field_id)
                } else {
                    block.label.trim().to_string()
                },
                field_id,
                empty: block.empty.trim().to_string(),
            })
        }
        "actions" => {
            if block.actions.is_empty() {
                None
            } else {
                Some(ResolvedModuleUiBlock::Actions {
                    actions: block.actions.clone(),
                })
            }
        }
        "progress" => {
            let field_id = block.field.trim().to_string();
            if field_id.is_empty() {
                return None;
            }
            let field_spec = module_field_spec(spec, &field_id);
            Some(ResolvedModuleUiBlock::Progress {
                label: if block.label.trim().is_empty() {
                    module_field_label(spec, &field_id)
                } else {
                    block.label.trim().to_string()
                },
                field_id,
                min: block.min.or_else(|| field_spec.and_then(|field| field.min)),
                max: block.max.or_else(|| field_spec.and_then(|field| field.max)),
                empty: block.empty.trim().to_string(),
            })
        }
        "record" | "key_value" | "kv" => {
            let ui_id = normalized_block_id(block, &format!("{container_key}:record"));
            let mut field_ids = Vec::new();
            for field_id in &block.fields {
                let trimmed = field_id.trim();
                if !trimmed.is_empty() && module_field_spec(spec, trimmed).is_some() {
                    field_ids.push(trimmed.to_string());
                }
            }
            if field_ids.is_empty() {
                None
            } else {
                Some(ResolvedModuleUiBlock::Record {
                    title: block.title.trim().to_string(),
                    ui_id,
                    field_ids,
                    empty: block.empty.trim().to_string(),
                })
            }
        }
        "checklist" => {
            let ui_id = normalized_block_id(block, &format!("{container_key}:checklist"));
            let field_id = block.field.trim().to_string();
            let path = block.path.trim().to_string();
            if field_id.is_empty() && path.is_empty() {
                None
            } else {
                Some(ResolvedModuleUiBlock::Checklist {
                    title: block.title.trim().to_string(),
                    ui_id,
                    field_id,
                    path,
                    empty: block.empty.trim().to_string(),
                    max_rows: block.max_rows.unwrap_or(12).clamp(1, 100),
                    searchable: block.searchable.unwrap_or(true),
                    filter_placeholder: block.filter_placeholder.trim().to_string(),
                    filter_presets: block.filter_presets.clone(),
                })
            }
        }
        "timeline" => {
            let ui_id = normalized_block_id(block, &format!("{container_key}:timeline"));
            let field_id = block.field.trim().to_string();
            let path = block.path.trim().to_string();
            if field_id.is_empty() && path.is_empty() {
                None
            } else {
                Some(ResolvedModuleUiBlock::Timeline {
                    title: block.title.trim().to_string(),
                    ui_id,
                    field_id,
                    path,
                    empty: block.empty.trim().to_string(),
                    max_rows: block.max_rows.unwrap_or(10).clamp(1, 100),
                    searchable: block.searchable.unwrap_or(true),
                    filter_placeholder: block.filter_placeholder.trim().to_string(),
                    filter_presets: block.filter_presets.clone(),
                })
            }
        }
        "kanban" | "board" => {
            let ui_id = normalized_block_id(block, &format!("{container_key}:kanban"));
            let field_id = block.field.trim().to_string();
            let path = block.path.trim().to_string();
            if field_id.is_empty() && path.is_empty() {
                None
            } else {
                Some(ResolvedModuleUiBlock::Kanban {
                    title: block.title.trim().to_string(),
                    ui_id,
                    field_id,
                    path,
                    empty: block.empty.trim().to_string(),
                    max_rows: block.max_rows.unwrap_or(18).clamp(1, 200),
                    lanes: block
                        .lanes
                        .iter()
                        .map(|lane| lane.trim().to_string())
                        .filter(|lane| !lane.is_empty())
                        .collect(),
                    searchable: block.searchable.unwrap_or(true),
                    filter_placeholder: block.filter_placeholder.trim().to_string(),
                    filter_presets: block.filter_presets.clone(),
                })
            }
        }
        "table" => {
            let ui_id = normalized_block_id(block, &format!("{container_key}:table"));
            let field_id = block.field.trim().to_string();
            let path = block.path.trim().to_string();
            if field_id.is_empty() && path.is_empty() {
                None
            } else {
                Some(ResolvedModuleUiBlock::Table {
                    title: block.title.trim().to_string(),
                    ui_id,
                    field_id,
                    path,
                    empty: block.empty.trim().to_string(),
                    max_rows: block.max_rows.unwrap_or(8).clamp(1, 50),
                    has_header: block.has_header.unwrap_or(true),
                    searchable: block.searchable.unwrap_or(true),
                    filter_placeholder: block.filter_placeholder.trim().to_string(),
                    filter_presets: block.filter_presets.clone(),
                })
            }
        }
        "dependency_graph" | "graph" | "dependencies" => {
            let ui_id = normalized_block_id(block, &format!("{container_key}:dependency_graph"));
            let field_id = block.field.trim().to_string();
            let path = block.path.trim().to_string();
            if field_id.is_empty() && path.is_empty() {
                None
            } else {
                Some(ResolvedModuleUiBlock::DependencyGraph {
                    title: block.title.trim().to_string(),
                    ui_id,
                    field_id,
                    path,
                    empty: block.empty.trim().to_string(),
                    max_rows: block.max_rows.unwrap_or(16).clamp(1, 200),
                    searchable: block.searchable.unwrap_or(true),
                    filter_placeholder: block.filter_placeholder.trim().to_string(),
                    filter_presets: block.filter_presets.clone(),
                })
            }
        }
        "bar_chart" | "bars" => {
            let mut field_ids = Vec::new();
            for field_id in &block.fields {
                let trimmed = field_id.trim();
                if !trimmed.is_empty() && module_field_spec(spec, trimmed).is_some() {
                    field_ids.push(trimmed.to_string());
                }
            }
            if field_ids.is_empty() {
                return None;
            }
            Some(ResolvedModuleUiBlock::BarChart {
                title: block.title.trim().to_string(),
                field_ids,
                min: block.min,
                max: block.max,
                empty: block.empty.trim().to_string(),
            })
        }
        "tabs" => {
            let ui_id = normalized_block_id(block, &format!("{container_key}:tabs"));
            let panes = resolve_module_ui_panes(spec, &block.tabs, used, &ui_id);
            if panes.is_empty() {
                None
            } else {
                Some(ResolvedModuleUiBlock::Tabs {
                    title: block.title.trim().to_string(),
                    ui_id,
                    panes,
                    view_presets: block.view_presets.clone(),
                })
            }
        }
        "split" | "columns" => {
            let ui_id = normalized_block_id(block, &format!("{container_key}:split"));
            let panes = resolve_module_ui_panes(spec, &block.columns, used, &ui_id);
            if panes.is_empty() {
                None
            } else {
                Some(ResolvedModuleUiBlock::Split {
                    title: block.title.trim().to_string(),
                    ui_id,
                    direction: if block.direction.trim().is_empty() {
                        "horizontal".to_string()
                    } else {
                        block.direction.trim().to_string()
                    },
                    panes,
                    view_presets: block.view_presets.clone(),
                })
            }
        }
        "accordion" | "inspector" => {
            let inspector_style = kind == "inspector";
            let ui_id = normalized_block_id(
                block,
                &format!(
                    "{container_key}:{}",
                    if inspector_style {
                        "inspector"
                    } else {
                        "accordion"
                    }
                ),
            );
            let panes = resolve_module_ui_panes(spec, &block.panes, used, &ui_id);
            if panes.is_empty() {
                None
            } else {
                Some(ResolvedModuleUiBlock::Accordion {
                    title: block.title.trim().to_string(),
                    ui_id,
                    panes,
                    inspector_style,
                    view_presets: block.view_presets.clone(),
                })
            }
        }
        "file_list" => Some(ResolvedModuleUiBlock::FileList {
            title: block.title.trim().to_string(),
            ui_id: normalized_block_id(block, &format!("{container_key}:file_list")),
            path: block.path.trim().to_string(),
            empty: block.empty.trim().to_string(),
            max_entries: block.max_entries.unwrap_or(8).clamp(1, 50),
            searchable: block.searchable.unwrap_or(true),
            filter_placeholder: block.filter_placeholder.trim().to_string(),
            filter_presets: block.filter_presets.clone(),
        }),
        "artifact_preview" | "file_preview" => Some(ResolvedModuleUiBlock::ArtifactPreview {
            title: block.title.trim().to_string(),
            path: block.path.trim().to_string(),
            field_id: block.field.trim().to_string(),
            empty: block.empty.trim().to_string(),
            max_lines: block.max_lines.unwrap_or(16).clamp(4, 80),
        }),
        "separator" => Some(ResolvedModuleUiBlock::Separator),
        "spacer" => Some(ResolvedModuleUiBlock::Spacer(
            block.points.unwrap_or(8.0).clamp(0.0, 64.0),
        )),
        _ => None,
    }
}

fn resolve_module_ui_sections(spec: &ModuleUiSpec) -> Vec<ResolvedModuleUiSection> {
    let mut resolved = Vec::new();
    let mut used = HashSet::new();

    if !spec.sections.is_empty() {
        for (idx, section) in spec.sections.iter().enumerate() {
            let section_key = if !section.id.trim().is_empty() {
                format!("section:{}", section.id.trim())
            } else {
                format!("section_{idx}")
            };
            let mut blocks = resolve_module_ui_container_blocks(
                spec,
                &section.blocks,
                &section.fields,
                &mut used,
                &section_key,
            );

            if !section.id.trim().is_empty() {
                for field in &spec.fields {
                    if !used.contains(&field.id) && field.section.trim() == section.id.trim() {
                        used.insert(field.id.clone());
                        blocks.push(ResolvedModuleUiBlock::Field(field.clone()));
                    }
                }
            }

            if !blocks.is_empty() {
                resolved.push(ResolvedModuleUiSection {
                    title: section.title.clone(),
                    description: section.description.clone(),
                    blocks,
                    sidebar: section.sidebar,
                });
            }
        }
    }

    let mut grouped_keys = Vec::<String>::new();
    let mut grouped = HashMap::<String, Vec<ModuleUiField>>::new();
    for field in &spec.fields {
        if used.contains(&field.id) {
            continue;
        }
        let key = if !field.section.trim().is_empty() {
            field.section.trim().to_string()
        } else {
            "workspace".to_string()
        };
        if !grouped.contains_key(&key) {
            grouped_keys.push(key.clone());
        }
        grouped.entry(key).or_default().push(field.clone());
    }

    for key in grouped_keys {
        if let Some(fields) = grouped.remove(&key) {
            if !fields.is_empty() {
                resolved.push(ResolvedModuleUiSection {
                    title: humanize_section_id(&key),
                    description: String::new(),
                    blocks: fields
                        .into_iter()
                        .map(ResolvedModuleUiBlock::Field)
                        .collect(),
                    sidebar: false,
                });
            }
        }
    }

    resolved
}

fn field_str_mut<'a>(
    values: &'a mut HashMap<String, ModuleFieldValue>,
    id: &str,
) -> &'a mut String {
    let v = values
        .entry(id.to_string())
        .or_insert(ModuleFieldValue::Str(String::new()));
    match v {
        ModuleFieldValue::Str(s) => s,
        _ => {
            *v = ModuleFieldValue::Str(String::new());
            match v {
                ModuleFieldValue::Str(s) => s,
                _ => unreachable!(),
            }
        }
    }
}

fn field_bool_mut<'a>(values: &'a mut HashMap<String, ModuleFieldValue>, id: &str) -> &'a mut bool {
    let v = values
        .entry(id.to_string())
        .or_insert(ModuleFieldValue::Bool(false));
    match v {
        ModuleFieldValue::Bool(b) => b,
        _ => {
            *v = ModuleFieldValue::Bool(false);
            match v {
                ModuleFieldValue::Bool(b) => b,
                _ => unreachable!(),
            }
        }
    }
}

fn field_num_mut<'a>(
    values: &'a mut HashMap<String, ModuleFieldValue>,
    id: &str,
    default: f64,
) -> &'a mut f64 {
    let v = values
        .entry(id.to_string())
        .or_insert(ModuleFieldValue::Num(default));
    match v {
        ModuleFieldValue::Num(n) => n,
        _ => {
            *v = ModuleFieldValue::Num(default);
            match v {
                ModuleFieldValue::Num(n) => n,
                _ => unreachable!(),
            }
        }
    }
}

fn render_module_field(
    ui: &mut egui::Ui,
    st: &mut ModuleFormState,
    module_id: &str,
    f: &ModuleUiField,
) {
    let id = f.id.trim();
    if id.is_empty() {
        return;
    }

    let kind = f.kind.trim().to_lowercase();
    ui.vertical(|ui| {
        if kind == "bool" {
            let value = field_bool_mut(&mut st.values, id);
            ui.checkbox(value, &f.label);
            if !f.help.trim().is_empty() {
                ui.small(f.help.clone());
            }
            return;
        }

        ui.label(egui::RichText::new(&f.label).strong());
        if !f.help.trim().is_empty() {
            ui.small(f.help.clone());
        }

        match kind.as_str() {
            "number" => {
                let value = field_num_mut(&mut st.values, id, f.min.unwrap_or(0.0));
                if let (Some(min), Some(max)) = (f.min, f.max) {
                    ui.add(egui::Slider::new(value, min..=max).show_value(true));
                } else {
                    ui.add(egui::DragValue::new(value).speed(0.1));
                }
            }
            "choice" => {
                let value = field_str_mut(&mut st.values, id);
                let compact = f.options.len() <= 4 && f.options.iter().all(|opt| opt.len() <= 18);
                if compact {
                    ui.horizontal_wrapped(|ui| {
                        for opt in &f.options {
                            let selected = value == opt;
                            if ui.selectable_label(selected, opt).clicked() {
                                value.clear();
                                value.push_str(opt);
                            }
                        }
                    });
                } else {
                    let selected = if value.trim().is_empty() {
                        "(none)".to_string()
                    } else {
                        value.clone()
                    };
                    egui::ComboBox::from_id_salt((module_id, id))
                        .selected_text(selected)
                        .show_ui(ui, |ui| {
                            for opt in &f.options {
                                ui.selectable_value(value, opt.clone(), opt.clone());
                            }
                        });
                }
            }
            "singleline" => {
                let value = field_str_mut(&mut st.values, id);
                ui.add_sized(
                    [ui.available_width(), 28.0],
                    egui::TextEdit::singleline(value).hint_text(&f.placeholder),
                );
            }
            _ => {
                let value = field_str_mut(&mut st.values, id);
                let rows = f.rows.unwrap_or(4).clamp(2, 24);
                ui.add(
                    egui::TextEdit::multiline(value)
                        .desired_rows(rows)
                        .hint_text(&f.placeholder),
                );
            }
        }
    });
}

fn render_module_builtin_actions(
    ui: &mut egui::Ui,
    st: &mut ModuleFormState,
    module_dir: &Path,
    state_path: &Path,
    actions: &[String],
) {
    ui.horizontal_wrapped(|ui| {
        for action in actions {
            let normalized = action.trim().to_lowercase();
            match normalized.as_str() {
                "save" => {
                    if ui.button("Save").clicked() {
                        st.save();
                    }
                }
                "reload" => {
                    if ui.button("Reload").clicked() {
                        st.reload();
                    }
                }
                "open_folder" | "open_module" | "open_module_folder" => {
                    if ui.button("Open Folder").clicked() {
                        open_path_in_explorer(module_dir);
                    }
                }
                "open_readme" => {
                    let path = module_dir.join("README.md");
                    if path.is_file() && ui.button("README").clicked() {
                        open_path_in_explorer(&path);
                    }
                }
                "open_manual" => {
                    let path = module_dir.join("USER_MANUAL.md");
                    if path.is_file() && ui.button("Manual").clicked() {
                        open_path_in_explorer(&path);
                    }
                }
                "open_handshake" => {
                    let path = module_dir.join("HANDSHAKE.md");
                    if path.is_file() && ui.button("Handshake").clicked() {
                        open_path_in_explorer(&path);
                    }
                }
                "open_state" => {
                    if state_path.is_file() && ui.button("State JSON").clicked() {
                        open_path_in_explorer(state_path);
                    }
                }
                "open_manifest" => {
                    let path = module_dir.join("manifest.json");
                    if path.is_file() && ui.button("Manifest").clicked() {
                        open_path_in_explorer(&path);
                    }
                }
                _ => {}
            }
        }
    });
}

fn render_module_file_list(
    ui: &mut egui::Ui,
    module_id: &str,
    module_dir: &Path,
    title: &str,
    ui_id: &str,
    relative_path: &str,
    empty: &str,
    max_entries: usize,
    searchable: bool,
    filter_placeholder: &str,
    filter_presets: &[ModuleUiFilterPreset],
) {
    if !title.trim().is_empty() {
        ui.label(egui::RichText::new(title).strong());
    }

    let Some(target_dir) = module_surface_path(module_dir, relative_path) else {
        ui.small(
            "This file list path is not allowed. Use a relative path inside the module folder.",
        );
        return;
    };

    if !target_dir.exists() {
        ui.small(if empty.trim().is_empty() {
            "Nothing here yet."
        } else {
            empty
        });
        return;
    }

    let Ok(read_dir) = std::fs::read_dir(&target_dir) else {
        ui.small("Couldn't read this folder.");
        return;
    };

    let mut entries = read_dir.flatten().collect::<Vec<_>>();
    entries.sort_by(|a, b| {
        let a_dir = a.path().is_dir();
        let b_dir = b.path().is_dir();
        b_dir.cmp(&a_dir).then_with(|| {
            a.file_name()
                .to_string_lossy()
                .to_lowercase()
                .cmp(&b.file_name().to_string_lossy().to_lowercase())
        })
    });

    if entries.is_empty() {
        ui.small(if empty.trim().is_empty() {
            "Nothing here yet."
        } else {
            empty
        });
        return;
    }

    let total_entries = entries.len();
    let query = module_block_filter_query(
        ui,
        module_id,
        ui_id,
        searchable,
        total_entries,
        filter_placeholder,
        "Search files...",
        filter_presets,
    );
    let filtered_entries = entries
        .into_iter()
        .filter(|entry| {
            if query.trim().is_empty() {
                true
            } else {
                text_matches_filter(&entry.file_name().to_string_lossy(), &query)
            }
        })
        .take(max_entries)
        .collect::<Vec<_>>();

    if filtered_entries.is_empty() {
        ui.small("No files match the current filter.");
        return;
    }

    if !query.trim().is_empty() {
        ui.small(format!(
            "Showing {}/{}",
            filtered_entries.len(),
            total_entries
        ));
        ui.add_space(4.0);
    }

    for entry in filtered_entries {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        let label = if path.is_dir() {
            format!("📁 {name}")
        } else {
            format!("📄 {name}")
        };
        if ui.button(label).clicked() {
            open_path_in_explorer(&path);
        }
    }
}

fn render_module_artifact_preview(
    ui: &mut egui::Ui,
    module_dir: &Path,
    values: &HashMap<String, ModuleFieldValue>,
    title: &str,
    explicit_path: &str,
    field_id: &str,
    empty: &str,
    max_lines: usize,
) {
    if !title.trim().is_empty() {
        ui.label(egui::RichText::new(title).strong());
    }

    let candidates = resolve_module_block_paths(module_dir, values, explicit_path, field_id);
    let Some(path) = candidates.into_iter().find(|path| path.exists()) else {
        ui.small(if empty.trim().is_empty() {
            "No artifact available yet."
        } else {
            empty
        });
        return;
    };

    ui.horizontal_wrapped(|ui| {
        ui.small(format!("Artifact: {}", path.display()));
        if ui.button("Open").clicked() {
            open_path_in_explorer(&path);
        }
    });

    if path.is_dir() {
        render_module_file_list(
            ui,
            "",
            &path,
            "",
            "artifact_preview_dir",
            ".",
            "This folder is empty.",
            max_lines.clamp(1, 20),
            true,
            "",
            &[],
        );
        return;
    }

    let metadata = std::fs::metadata(&path).ok();
    if let Some(metadata) = metadata {
        ui.small(format!("Size: {} bytes", metadata.len()));
    }

    if looks_like_text_file(&path) {
        if let Some(preview) = preview_text_file(&path, max_lines) {
            if path
                .extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(|ext| ext.eq_ignore_ascii_case("md"))
            {
                render_markdownish(ui, &preview);
            } else {
                egui::Frame::group(ui.style())
                    .inner_margin(egui::Margin::same(8.0))
                    .show(ui, |ui| {
                        ui.add(egui::Label::new(egui::RichText::new(preview).monospace()).wrap());
                    });
            }
        } else {
            ui.small("Couldn't preview this file.");
        }
    } else {
        ui.small("Preview unavailable for this file type. Use Open to inspect it directly.");
    }
}

fn render_module_record_block(
    ui: &mut egui::Ui,
    spec: &ModuleUiSpec,
    values: &HashMap<String, ModuleFieldValue>,
    title: &str,
    ui_id: &str,
    field_ids: &[String],
    empty: &str,
) {
    if !title.trim().is_empty() {
        ui.label(egui::RichText::new(title).strong());
    }

    let rows = field_ids
        .iter()
        .filter_map(|field_id| {
            module_field_value_as_text(values, field_id)
                .map(|value| (module_field_label(spec, field_id), value))
        })
        .collect::<Vec<_>>();

    if rows.is_empty() {
        ui.small(if empty.trim().is_empty() {
            "No values to show yet."
        } else {
            empty
        });
        return;
    }

    egui::Grid::new(("module_record", ui_id))
        .num_columns(2)
        .spacing([12.0, 6.0])
        .striped(true)
        .show(ui, |ui| {
            for (label, value) in rows {
                ui.small(label);
                ui.label(value);
                ui.end_row();
            }
        });
}

fn render_module_table_block(
    ui: &mut egui::Ui,
    module_id: &str,
    module_dir: &Path,
    values: &HashMap<String, ModuleFieldValue>,
    title: &str,
    ui_id: &str,
    field_id: &str,
    path: &str,
    empty: &str,
    max_rows: usize,
    has_header: bool,
    searchable: bool,
    filter_placeholder: &str,
    filter_presets: &[ModuleUiFilterPreset],
) {
    if !title.trim().is_empty() {
        ui.label(egui::RichText::new(title).strong());
    }

    let Some(content) = read_module_table_source(module_dir, values, path, field_id) else {
        ui.small(if empty.trim().is_empty() {
            "No table data available yet."
        } else {
            empty
        });
        return;
    };
    let Some((header, rows)) = parse_lightweight_table(&content, has_header) else {
        ui.small(if empty.trim().is_empty() {
            "Couldn't parse table data. Use CSV, TSV, semicolon, or pipe-delimited rows."
        } else {
            empty
        });
        return;
    };
    if rows.is_empty() {
        ui.small(if empty.trim().is_empty() {
            "No table rows available yet."
        } else {
            empty
        });
        return;
    }

    let total_rows = rows.len();
    let query = module_block_filter_query(
        ui,
        module_id,
        ui_id,
        searchable,
        total_rows,
        filter_placeholder,
        "Filter rows...",
        filter_presets,
    );
    let filtered_rows = rows
        .into_iter()
        .filter(|row| {
            if query.trim().is_empty() {
                true
            } else {
                text_matches_filter(&row.join(" "), &query)
            }
        })
        .take(max_rows)
        .collect::<Vec<_>>();

    if filtered_rows.is_empty() {
        ui.small("No table rows match the current filter.");
        return;
    }

    if !query.trim().is_empty() {
        ui.small(format!("Showing {}/{}", filtered_rows.len(), total_rows));
        ui.add_space(4.0);
    }

    egui::ScrollArea::horizontal()
        .id_salt(("module_table_scroll", ui_id))
        .show(ui, |ui| {
            egui::Grid::new(("module_table", ui_id))
                .num_columns(header.len().max(1))
                .spacing([12.0, 6.0])
                .striped(true)
                .show(ui, |ui| {
                    for column in &header {
                        ui.label(egui::RichText::new(column).strong());
                    }
                    ui.end_row();

                    for row in filtered_rows {
                        for cell in row {
                            ui.label(cell);
                        }
                        ui.end_row();
                    }
                });
        });
}

fn render_module_checklist_block(
    ui: &mut egui::Ui,
    module_id: &str,
    module_dir: &Path,
    values: &HashMap<String, ModuleFieldValue>,
    title: &str,
    ui_id: &str,
    field_id: &str,
    path: &str,
    empty: &str,
    max_rows: usize,
    searchable: bool,
    filter_placeholder: &str,
    filter_presets: &[ModuleUiFilterPreset],
) {
    if !title.trim().is_empty() {
        ui.label(egui::RichText::new(title).strong());
    }

    let Some(content) = read_module_text_source(module_dir, values, path, field_id) else {
        ui.small(if empty.trim().is_empty() {
            "No checklist available yet."
        } else {
            empty
        });
        return;
    };

    let items = content
        .lines()
        .filter_map(parse_checklist_line)
        .collect::<Vec<_>>();

    if items.is_empty() {
        ui.small(if empty.trim().is_empty() {
            "No checklist items available yet."
        } else {
            empty
        });
        return;
    }

    let total_items = items.len();
    let query = module_block_filter_query(
        ui,
        module_id,
        ui_id,
        searchable,
        total_items,
        filter_placeholder,
        "Filter checklist...",
        filter_presets,
    );
    let filtered_items = items
        .into_iter()
        .filter(|(_, text)| text_matches_filter(text, &query))
        .take(max_rows)
        .collect::<Vec<_>>();

    if filtered_items.is_empty() {
        ui.small("No checklist items match the current filter.");
        return;
    }

    if !query.trim().is_empty() {
        ui.small(format!("Showing {}/{}", filtered_items.len(), total_items));
        ui.add_space(4.0);
    }

    for (state, text) in filtered_items {
        ui.horizontal_wrapped(|ui| {
            let (marker, color) = match state {
                ChecklistState::Done => ("[x]", egui::Color32::from_rgb(20, 120, 60)),
                ChecklistState::InProgress => ("[~]", egui::Color32::from_rgb(180, 110, 10)),
                ChecklistState::Pending => ("[ ]", egui::Color32::from_gray(120)),
                ChecklistState::Note => ("•", egui::Color32::from_gray(120)),
            };
            ui.colored_label(color, marker);
            ui.label(text);
        });
    }
}

fn render_module_timeline_block(
    ui: &mut egui::Ui,
    module_id: &str,
    module_dir: &Path,
    values: &HashMap<String, ModuleFieldValue>,
    title: &str,
    ui_id: &str,
    field_id: &str,
    path: &str,
    empty: &str,
    max_rows: usize,
    searchable: bool,
    filter_placeholder: &str,
    filter_presets: &[ModuleUiFilterPreset],
) {
    if !title.trim().is_empty() {
        ui.label(egui::RichText::new(title).strong());
    }

    let Some(content) = read_module_text_source(module_dir, values, path, field_id) else {
        ui.small(if empty.trim().is_empty() {
            "No timeline available yet."
        } else {
            empty
        });
        return;
    };

    let items = content
        .lines()
        .filter_map(parse_timeline_line)
        .collect::<Vec<_>>();

    if items.is_empty() {
        ui.small(if empty.trim().is_empty() {
            "No timeline entries available yet."
        } else {
            empty
        });
        return;
    }

    let total_items = items.len();
    let query = module_block_filter_query(
        ui,
        module_id,
        ui_id,
        searchable,
        total_items,
        filter_placeholder,
        "Filter timeline...",
        filter_presets,
    );
    let filtered_items = items
        .into_iter()
        .filter(|(stamp, text)| text_matches_filter(&format!("{stamp} {text}"), &query))
        .take(max_rows)
        .collect::<Vec<_>>();

    if filtered_items.is_empty() {
        ui.small("No timeline entries match the current filter.");
        return;
    }

    if !query.trim().is_empty() {
        ui.small(format!("Showing {}/{}", filtered_items.len(), total_items));
        ui.add_space(4.0);
    }

    let total = filtered_items.len();
    egui::Frame::group(ui.style())
        .inner_margin(egui::Margin::same(8.0))
        .show(ui, |ui| {
            for (idx, (stamp, text)) in filtered_items.into_iter().enumerate() {
                ui.horizontal_wrapped(|ui| {
                    ui.colored_label(egui::Color32::from_rgb(30, 80, 180), "•");
                    if !stamp.trim().is_empty() {
                        ui.small(egui::RichText::new(stamp).strong());
                    }
                    ui.label(text);
                });
                if idx + 1 < total {
                    ui.add_space(4.0);
                }
            }
        });
}

fn render_module_kanban_block(
    ui: &mut egui::Ui,
    module_id: &str,
    module_dir: &Path,
    values: &HashMap<String, ModuleFieldValue>,
    title: &str,
    ui_id: &str,
    field_id: &str,
    path: &str,
    empty: &str,
    max_rows: usize,
    lanes: &[String],
    searchable: bool,
    filter_placeholder: &str,
    filter_presets: &[ModuleUiFilterPreset],
) {
    if !title.trim().is_empty() {
        ui.label(egui::RichText::new(title).strong());
    }

    let Some(content) = read_module_text_source(module_dir, values, path, field_id) else {
        ui.small(if empty.trim().is_empty() {
            "No kanban board available yet."
        } else {
            empty
        });
        return;
    };

    let (lane_order, cards) = parse_kanban_content(&content, lanes);
    if cards.is_empty() {
        ui.small(if empty.trim().is_empty() {
            "No kanban cards available yet."
        } else {
            empty
        });
        return;
    }

    let total_cards = cards.len();
    let query = module_block_filter_query(
        ui,
        module_id,
        ui_id,
        searchable,
        total_cards,
        filter_placeholder,
        "Filter board...",
        filter_presets,
    );
    let filtered_cards = cards
        .into_iter()
        .filter(|card| text_matches_filter(&format!("{} {}", card.lane, card.text), &query))
        .take(max_rows)
        .collect::<Vec<_>>();

    if filtered_cards.is_empty() {
        ui.small("No kanban cards match the current filter.");
        return;
    }

    if !query.trim().is_empty() {
        ui.small(format!("Showing {}/{}", filtered_cards.len(), total_cards));
        ui.add_space(4.0);
    }

    egui::ScrollArea::horizontal()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.horizontal_top(|ui| {
                for (idx, lane) in lane_order.iter().enumerate() {
                    let lane_cards = filtered_cards
                        .iter()
                        .filter(|card| card.lane == *lane)
                        .collect::<Vec<_>>();
                    let accent = lane_accent_color(lane);

                    egui::Frame::group(ui.style())
                        .inner_margin(egui::Margin::same(8.0))
                        .show(ui, |ui| {
                            ui.set_min_width(220.0);
                            ui.colored_label(
                                accent,
                                egui::RichText::new(format!("{} ({})", lane, lane_cards.len()))
                                    .strong(),
                            );
                            ui.add_space(6.0);

                            if lane_cards.is_empty() {
                                ui.small("No cards in this lane yet.");
                            } else {
                                for (card_idx, card) in lane_cards.iter().enumerate() {
                                    egui::Frame::group(ui.style())
                                        .inner_margin(egui::Margin::same(6.0))
                                        .show(ui, |ui| {
                                            ui.label(card.text.clone());
                                        });
                                    if card_idx + 1 < lane_cards.len() {
                                        ui.add_space(6.0);
                                    }
                                }
                            }
                        });

                    if idx + 1 < lane_order.len() {
                        ui.add_space(8.0);
                    }
                }
            });
        });
}

fn render_module_dependency_graph_block(
    ui: &mut egui::Ui,
    module_id: &str,
    module_dir: &Path,
    values: &HashMap<String, ModuleFieldValue>,
    title: &str,
    ui_id: &str,
    field_id: &str,
    path: &str,
    empty: &str,
    max_rows: usize,
    searchable: bool,
    filter_placeholder: &str,
    filter_presets: &[ModuleUiFilterPreset],
) {
    if !title.trim().is_empty() {
        ui.label(egui::RichText::new(title).strong());
    }

    let Some(content) = read_module_text_source(module_dir, values, path, field_id) else {
        ui.small(if empty.trim().is_empty() {
            "No dependency graph available yet."
        } else {
            empty
        });
        return;
    };

    let nodes = parse_dependency_graph_content(&content);
    if nodes.is_empty() {
        ui.small(if empty.trim().is_empty() {
            "No dependency graph nodes available yet."
        } else {
            empty
        });
        return;
    }

    let total_nodes = nodes.len();
    let query = module_block_filter_query(
        ui,
        module_id,
        ui_id,
        searchable,
        total_nodes,
        filter_placeholder,
        "Filter graph...",
        filter_presets,
    );
    let filtered_nodes = nodes
        .into_iter()
        .filter(|node| {
            text_matches_filter(
                &format!(
                    "{} {} {}",
                    node.name,
                    node.depends_on.join(" "),
                    node.unlocks.join(" ")
                ),
                &query,
            )
        })
        .take(max_rows)
        .collect::<Vec<_>>();

    if filtered_nodes.is_empty() {
        ui.small("No dependency graph nodes match the current filter.");
        return;
    }

    if !query.trim().is_empty() {
        ui.small(format!("Showing {}/{}", filtered_nodes.len(), total_nodes));
        ui.add_space(4.0);
    }

    let max_stage = filtered_nodes
        .iter()
        .map(|node| node.stage)
        .max()
        .unwrap_or(0);
    egui::ScrollArea::horizontal()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.horizontal_top(|ui| {
                for stage in 0..=max_stage {
                    let stage_nodes = filtered_nodes
                        .iter()
                        .filter(|node| node.stage == stage)
                        .collect::<Vec<_>>();

                    egui::Frame::group(ui.style())
                        .inner_margin(egui::Margin::same(8.0))
                        .show(ui, |ui| {
                            ui.set_min_width(230.0);
                            ui.label(egui::RichText::new(format!("Stage {}", stage + 1)).strong());
                            ui.add_space(6.0);

                            if stage_nodes.is_empty() {
                                ui.small("No nodes in this stage.");
                            } else {
                                for (idx, node) in stage_nodes.iter().enumerate() {
                                    egui::Frame::group(ui.style())
                                        .inner_margin(egui::Margin::same(6.0))
                                        .show(ui, |ui| {
                                            ui.label(egui::RichText::new(&node.name).strong());
                                            if !node.depends_on.is_empty() {
                                                ui.small(format!(
                                                    "Depends on: {}",
                                                    truncate_for_ui(
                                                        &node.depends_on.join(", "),
                                                        120
                                                    )
                                                ));
                                            }
                                            if !node.unlocks.is_empty() {
                                                ui.small(format!(
                                                    "Unblocks: {}",
                                                    truncate_for_ui(&node.unlocks.join(", "), 120)
                                                ));
                                            }
                                        });
                                    if idx + 1 < stage_nodes.len() {
                                        ui.add_space(6.0);
                                    }
                                }
                            }
                        });

                    if stage < max_stage {
                        ui.add_space(8.0);
                    }
                }
            });
        });
}

fn render_module_pane(
    ui: &mut egui::Ui,
    st: &mut ModuleFormState,
    spec: &ModuleUiSpec,
    module_id: &str,
    module_dir: &Path,
    pane: &ResolvedModuleUiPane,
    framed: bool,
) {
    let mut render_contents = |ui: &mut egui::Ui| {
        if !pane.title.trim().is_empty() {
            ui.label(egui::RichText::new(&pane.title).strong());
        }
        if !pane.description.trim().is_empty() {
            ui.small(pane.description.clone());
            ui.add_space(6.0);
        }
        for (idx, block) in pane.blocks.iter().enumerate() {
            render_module_block(ui, st, spec, module_id, module_dir, block);
            if idx + 1 < pane.blocks.len() {
                ui.add_space(8.0);
                ui.separator();
                ui.add_space(8.0);
            }
        }
    };

    if framed {
        egui::Frame::group(ui.style())
            .inner_margin(egui::Margin::same(8.0))
            .show(ui, render_contents);
    } else {
        render_contents(ui);
    }
}

fn render_module_pane_summary(
    values: &HashMap<String, ModuleFieldValue>,
    pane: &ResolvedModuleUiPane,
) -> String {
    if !pane.summary_field.trim().is_empty() {
        if let Some(value) = module_field_value_as_text(values, &pane.summary_field) {
            return truncate_for_ui(&value.replace('\n', " "), 80);
        }
    }
    if !pane.summary.trim().is_empty() {
        return pane.summary.trim().to_string();
    }
    String::new()
}

fn render_module_tabs_block(
    ui: &mut egui::Ui,
    st: &mut ModuleFormState,
    spec: &ModuleUiSpec,
    module_id: &str,
    module_dir: &Path,
    title: &str,
    ui_id: &str,
    panes: &[ResolvedModuleUiPane],
    view_presets: &[ModuleUiViewPreset],
) {
    if panes.is_empty() {
        return;
    }
    if !title.trim().is_empty() {
        ui.label(egui::RichText::new(title).strong());
        ui.add_space(4.0);
    }

    let preset_idx = module_layout_preset_selection(ui, module_id, ui_id, view_presets);
    let visible_panes =
        module_layout_visible_panes(panes, preset_idx.and_then(|idx| view_presets.get(idx)));
    if visible_panes.is_empty() {
        ui.small("No panes available for this view.");
        return;
    }

    let tabs_id = egui::Id::new(("module_surface_tabs", module_id, ui_id));
    let mut active = ui
        .ctx()
        .data_mut(|data| data.get_temp::<usize>(tabs_id))
        .unwrap_or(0);
    if active >= visible_panes.len() {
        active = 0;
    }

    ui.horizontal_wrapped(|ui| {
        for (idx, pane) in visible_panes.iter().enumerate() {
            let selected = idx == active;
            if ui.selectable_label(selected, &pane.title).clicked() {
                active = idx;
            }
        }
    });
    ui.ctx().data_mut(|data| data.insert_temp(tabs_id, active));
    ui.add_space(8.0);

    render_module_pane(
        ui,
        st,
        spec,
        module_id,
        module_dir,
        visible_panes[active],
        true,
    );
}

fn render_module_accordion_block(
    ui: &mut egui::Ui,
    st: &mut ModuleFormState,
    spec: &ModuleUiSpec,
    module_id: &str,
    module_dir: &Path,
    title: &str,
    ui_id: &str,
    panes: &[ResolvedModuleUiPane],
    inspector_style: bool,
    view_presets: &[ModuleUiViewPreset],
) {
    if panes.is_empty() {
        return;
    }

    let preset_idx = module_layout_preset_selection(ui, module_id, ui_id, view_presets);
    let visible_panes =
        module_layout_visible_panes(panes, preset_idx.and_then(|idx| view_presets.get(idx)));
    if visible_panes.is_empty() {
        ui.small("No panes available for this view.");
        return;
    }

    let render_stack = |ui: &mut egui::Ui,
                        st: &mut ModuleFormState,
                        spec: &ModuleUiSpec,
                        module_id: &str,
                        module_dir: &Path| {
        for (idx, pane) in visible_panes.iter().enumerate() {
            let summary = render_module_pane_summary(&st.values, pane);
            let header = if summary.is_empty() {
                pane.title.clone()
            } else {
                format!("{} — {}", pane.title, summary)
            };
            egui::CollapsingHeader::new(header)
                .id_salt((module_id, ui_id, pane.id.as_str()))
                .default_open(pane.default_open || preset_idx.is_some())
                .show(ui, |ui| {
                    if !pane.description.trim().is_empty() {
                        ui.small(pane.description.clone());
                        ui.add_space(6.0);
                    }
                    for (block_idx, block) in pane.blocks.iter().enumerate() {
                        render_module_block(ui, st, spec, module_id, module_dir, block);
                        if block_idx + 1 < pane.blocks.len() {
                            ui.add_space(8.0);
                            ui.separator();
                            ui.add_space(8.0);
                        }
                    }
                });

            if idx + 1 < visible_panes.len() {
                ui.add_space(if inspector_style { 4.0 } else { 8.0 });
            }
        }
    };

    if inspector_style {
        egui::Frame::group(ui.style())
            .inner_margin(egui::Margin::same(8.0))
            .show(ui, |ui| {
                if !title.trim().is_empty() {
                    ui.label(egui::RichText::new(title).strong());
                    ui.add_space(4.0);
                }
                render_stack(ui, st, spec, module_id, module_dir);
            });
    } else {
        if !title.trim().is_empty() {
            ui.label(egui::RichText::new(title).strong());
            ui.add_space(4.0);
        }
        render_stack(ui, st, spec, module_id, module_dir);
    }
}

fn render_module_split_block(
    ui: &mut egui::Ui,
    st: &mut ModuleFormState,
    spec: &ModuleUiSpec,
    module_id: &str,
    module_dir: &Path,
    title: &str,
    ui_id: &str,
    direction: &str,
    panes: &[ResolvedModuleUiPane],
    view_presets: &[ModuleUiViewPreset],
) {
    if panes.is_empty() {
        return;
    }
    if !title.trim().is_empty() {
        ui.label(egui::RichText::new(title).strong());
        ui.add_space(4.0);
    }

    let preset_idx = module_layout_preset_selection(ui, module_id, ui_id, view_presets);
    let visible_panes =
        module_layout_visible_panes(panes, preset_idx.and_then(|idx| view_presets.get(idx)));
    if visible_panes.is_empty() {
        ui.small("No panes available for this view.");
        return;
    }

    let horizontal =
        !direction.trim().eq_ignore_ascii_case("vertical") && ui.available_width() >= 720.0;
    if horizontal && visible_panes.len() > 1 {
        let total_width = ui.available_width();
        let spacing = ui.spacing().item_spacing.x;
        let total_spacing = spacing * (visible_panes.len().saturating_sub(1) as f32);
        let total_weight: f32 = visible_panes.iter().map(|pane| pane.weight.max(0.1)).sum();
        ui.horizontal_top(|ui| {
            for (idx, pane) in visible_panes.iter().enumerate() {
                let width = ((total_width - total_spacing).max(200.0)
                    * (pane.weight.max(0.1) / total_weight))
                    .max(180.0);
                ui.allocate_ui_with_layout(
                    egui::vec2(width, 0.0),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| render_module_pane(ui, st, spec, module_id, module_dir, pane, true),
                );
                if idx + 1 < visible_panes.len() {
                    ui.add_space(spacing);
                }
            }
        });
    } else {
        for (idx, pane) in visible_panes.iter().enumerate() {
            render_module_pane(ui, st, spec, module_id, module_dir, pane, true);
            if idx + 1 < visible_panes.len() {
                ui.add_space(8.0);
            }
        }
    }
}

fn render_module_block(
    ui: &mut egui::Ui,
    st: &mut ModuleFormState,
    spec: &ModuleUiSpec,
    module_id: &str,
    module_dir: &Path,
    block: &ResolvedModuleUiBlock,
) {
    match block {
        ResolvedModuleUiBlock::Field(field) => render_module_field(ui, st, module_id, field),
        ResolvedModuleUiBlock::Text { title, text } => {
            if !title.trim().is_empty() {
                ui.label(egui::RichText::new(title).strong());
            }
            if !text.trim().is_empty() {
                ui.label(text.clone());
            }
        }
        ResolvedModuleUiBlock::Markdown {
            title,
            text,
            field_id,
            empty,
        } => {
            if !title.trim().is_empty() {
                ui.label(egui::RichText::new(title).strong());
            }
            let content = if !field_id.trim().is_empty() {
                module_field_value_as_text(&st.values, field_id)
            } else if !text.trim().is_empty() {
                Some(text.clone())
            } else {
                None
            };

            if let Some(content) = content {
                render_markdownish(ui, &content);
            } else {
                ui.small(if empty.trim().is_empty() {
                    "Nothing to preview yet."
                } else {
                    empty
                });
            }
        }
        ResolvedModuleUiBlock::Callout { title, text, tone } => {
            let accent = match tone.trim().to_lowercase().as_str() {
                "success" => egui::Color32::from_rgb(20, 120, 60),
                "warning" | "warn" => egui::Color32::from_rgb(180, 110, 10),
                "error" | "danger" => egui::Color32::from_rgb(180, 40, 40),
                "info" => egui::Color32::from_rgb(30, 80, 180),
                _ => egui::Color32::from_gray(120),
            };
            egui::Frame::group(ui.style())
                .inner_margin(egui::Margin::same(8.0))
                .show(ui, |ui| {
                    if !title.trim().is_empty() {
                        ui.colored_label(accent, egui::RichText::new(title).strong());
                    }
                    if !text.trim().is_empty() {
                        ui.label(text.clone());
                    }
                });
        }
        ResolvedModuleUiBlock::Stat {
            label,
            field_id,
            empty,
        } => {
            egui::Frame::group(ui.style())
                .inner_margin(egui::Margin::same(8.0))
                .show(ui, |ui| {
                    ui.small(label.clone());
                    let value =
                        module_field_value_as_text(&st.values, field_id).unwrap_or_else(|| {
                            if empty.trim().is_empty() {
                                "(empty)".to_string()
                            } else {
                                empty.clone()
                            }
                        });
                    ui.label(egui::RichText::new(value).strong());
                });
        }
        ResolvedModuleUiBlock::Actions { actions } => {
            let state_path = st.state_path.clone();
            render_module_builtin_actions(ui, st, module_dir, &state_path, actions);
        }
        ResolvedModuleUiBlock::Progress {
            label,
            field_id,
            min,
            max,
            empty,
        } => {
            egui::Frame::group(ui.style())
                .inner_margin(egui::Margin::same(8.0))
                .show(ui, |ui| {
                    ui.small(label.clone());
                    let Some(value) = module_field_value_as_number(&st.values, field_id) else {
                        ui.label(if empty.trim().is_empty() {
                            "(empty)".to_string()
                        } else {
                            empty.clone()
                        });
                        return;
                    };

                    let field_spec = module_field_spec(spec, field_id);
                    let min = min
                        .or_else(|| field_spec.and_then(|field| field.min))
                        .unwrap_or(0.0);
                    let max = max
                        .or_else(|| field_spec.and_then(|field| field.max))
                        .unwrap_or(100.0);
                    let denom = (max - min).abs().max(f64::EPSILON);
                    let progress = ((value - min) / denom).clamp(0.0, 1.0) as f32;
                    ui.add(egui::ProgressBar::new(progress).text(format!("{value:.2} / {max:.2}")));
                });
        }
        ResolvedModuleUiBlock::Record {
            title,
            ui_id,
            field_ids,
            empty,
        } => {
            render_module_record_block(ui, spec, &st.values, title, ui_id, field_ids, empty);
        }
        ResolvedModuleUiBlock::Table {
            title,
            ui_id,
            field_id,
            path,
            empty,
            max_rows,
            has_header,
            searchable,
            filter_placeholder,
            filter_presets,
        } => {
            render_module_table_block(
                ui,
                module_id,
                module_dir,
                &st.values,
                title,
                ui_id,
                field_id,
                path,
                empty,
                *max_rows,
                *has_header,
                *searchable,
                filter_placeholder,
                filter_presets,
            );
        }
        ResolvedModuleUiBlock::Checklist {
            title,
            ui_id,
            field_id,
            path,
            empty,
            max_rows,
            searchable,
            filter_placeholder,
            filter_presets,
        } => {
            render_module_checklist_block(
                ui,
                module_id,
                module_dir,
                &st.values,
                title,
                ui_id,
                field_id,
                path,
                empty,
                *max_rows,
                *searchable,
                filter_placeholder,
                filter_presets,
            );
        }
        ResolvedModuleUiBlock::Timeline {
            title,
            ui_id,
            field_id,
            path,
            empty,
            max_rows,
            searchable,
            filter_placeholder,
            filter_presets,
        } => {
            render_module_timeline_block(
                ui,
                module_id,
                module_dir,
                &st.values,
                title,
                ui_id,
                field_id,
                path,
                empty,
                *max_rows,
                *searchable,
                filter_placeholder,
                filter_presets,
            );
        }
        ResolvedModuleUiBlock::Kanban {
            title,
            ui_id,
            field_id,
            path,
            empty,
            max_rows,
            lanes,
            searchable,
            filter_placeholder,
            filter_presets,
        } => {
            render_module_kanban_block(
                ui,
                module_id,
                module_dir,
                &st.values,
                title,
                ui_id,
                field_id,
                path,
                empty,
                *max_rows,
                lanes,
                *searchable,
                filter_placeholder,
                filter_presets,
            );
        }
        ResolvedModuleUiBlock::BarChart {
            title,
            field_ids,
            min,
            max,
            empty,
        } => {
            if !title.trim().is_empty() {
                ui.label(egui::RichText::new(title).strong());
            }

            let mut rendered = 0usize;
            for field_id in field_ids {
                let Some(value) = module_field_value_as_number(&st.values, field_id) else {
                    continue;
                };
                let field_spec = module_field_spec(spec, field_id);
                let field_min = min
                    .or_else(|| field_spec.and_then(|field| field.min))
                    .unwrap_or(0.0);
                let field_max = max
                    .or_else(|| field_spec.and_then(|field| field.max))
                    .unwrap_or(100.0);
                let denom = (field_max - field_min).abs().max(f64::EPSILON);
                let progress = ((value - field_min) / denom).clamp(0.0, 1.0) as f32;
                ui.small(module_field_label(spec, field_id));
                ui.add(egui::ProgressBar::new(progress).text(format!("{value:.2}")));
                rendered += 1;
            }

            if rendered == 0 {
                ui.small(if empty.trim().is_empty() {
                    "No chart values available yet."
                } else {
                    empty
                });
            }
        }
        ResolvedModuleUiBlock::DependencyGraph {
            title,
            ui_id,
            field_id,
            path,
            empty,
            max_rows,
            searchable,
            filter_placeholder,
            filter_presets,
        } => {
            render_module_dependency_graph_block(
                ui,
                module_id,
                module_dir,
                &st.values,
                title,
                ui_id,
                field_id,
                path,
                empty,
                *max_rows,
                *searchable,
                filter_placeholder,
                filter_presets,
            );
        }
        ResolvedModuleUiBlock::Tabs {
            title,
            ui_id,
            panes,
            view_presets,
        } => {
            render_module_tabs_block(
                ui,
                st,
                spec,
                module_id,
                module_dir,
                title,
                ui_id,
                panes,
                view_presets,
            );
        }
        ResolvedModuleUiBlock::Split {
            title,
            ui_id,
            direction,
            panes,
            view_presets,
        } => {
            render_module_split_block(
                ui,
                st,
                spec,
                module_id,
                module_dir,
                title,
                ui_id,
                direction,
                panes,
                view_presets,
            );
        }
        ResolvedModuleUiBlock::Accordion {
            title,
            ui_id,
            panes,
            inspector_style,
            view_presets,
        } => {
            render_module_accordion_block(
                ui,
                st,
                spec,
                module_id,
                module_dir,
                title,
                ui_id,
                panes,
                *inspector_style,
                view_presets,
            );
        }
        ResolvedModuleUiBlock::FileList {
            title,
            ui_id,
            path,
            empty,
            max_entries,
            searchable,
            filter_placeholder,
            filter_presets,
        } => {
            render_module_file_list(
                ui,
                module_id,
                module_dir,
                title,
                ui_id,
                path,
                empty,
                *max_entries,
                *searchable,
                filter_placeholder,
                filter_presets,
            );
        }
        ResolvedModuleUiBlock::ArtifactPreview {
            title,
            path,
            field_id,
            empty,
            max_lines,
        } => {
            render_module_artifact_preview(
                ui, module_dir, &st.values, title, path, field_id, empty, *max_lines,
            );
        }
        ResolvedModuleUiBlock::Separator => {
            ui.separator();
        }
        ResolvedModuleUiBlock::Spacer(points) => {
            ui.add_space(*points);
        }
    }
}

fn render_module_section(
    ui: &mut egui::Ui,
    st: &mut ModuleFormState,
    spec: &ModuleUiSpec,
    module_id: &str,
    module_dir: &Path,
    section: &ResolvedModuleUiSection,
) {
    egui::Frame::group(ui.style())
        .inner_margin(egui::Margin::same(10.0))
        .show(ui, |ui| {
            ui.label(egui::RichText::new(&section.title).heading());
            if !section.description.trim().is_empty() {
                ui.label(section.description.clone());
                ui.add_space(6.0);
            }

            for (idx, block) in section.blocks.iter().enumerate() {
                render_module_block(ui, st, spec, module_id, module_dir, block);
                if idx + 1 < section.blocks.len() {
                    ui.add_space(8.0);
                    ui.separator();
                    ui.add_space(8.0);
                }
            }
        });
}

fn render_module_tools_card(
    ui: &mut egui::Ui,
    module_dir: &Path,
    state_path: &Path,
    spec_path: &Path,
    filled_fields: usize,
    total_fields: usize,
    status: &str,
) {
    egui::Frame::group(ui.style())
        .inner_margin(egui::Margin::same(10.0))
        .show(ui, |ui| {
            ui.label(egui::RichText::new("Module Tools").heading());
            ui.small(format!(
                "{} / {} fields populated",
                filled_fields, total_fields
            ));
            ui.small(format!("UI: {}", spec_path.display()));
            ui.small(format!("State: {}", state_path.display()));
            if !status.trim().is_empty() {
                ui.add_space(4.0);
                ui.small(status.to_string());
            }
            ui.add_space(8.0);

            ui.horizontal_wrapped(|ui| {
                if ui.button("Open Folder").clicked() {
                    open_path_in_explorer(module_dir);
                }
                let candidates = [
                    ("README", module_dir.join("README.md")),
                    ("Manual", module_dir.join("USER_MANUAL.md")),
                    ("Handshake", module_dir.join("HANDSHAKE.md")),
                    ("State JSON", state_path.to_path_buf()),
                ];
                for (label, path) in candidates {
                    if path.is_file() && ui.button(label).clicked() {
                        open_path_in_explorer(&path);
                    }
                }
            });
        });
}

fn render_module_surface(
    ui: &mut egui::Ui,
    app: &mut ChattyCogApp,
    manifest: Option<&ModuleManifest>,
    module_id: &str,
    module_dir: &Path,
) {
    // Prefer a declarative form UI if the module provides `ui.json`.
    let spec_path = module_dir.join("ui.json");
    if spec_path.is_file() {
        let st = app
            .module_forms
            .entry(module_id.to_string())
            .or_insert_with(|| ModuleFormState::new(module_dir));
        st.ensure_loaded();

        if let Some(spec) = st.spec.clone() {
            let title = spec
                .title
                .clone()
                .or_else(|| manifest.map(|m| m.display_name.clone()))
                .unwrap_or_else(|| "Module Workspace".to_string());
            let description = spec
                .description
                .clone()
                .or_else(|| manifest.map(|m| m.description.clone()))
                .unwrap_or_default();

            egui::Frame::group(ui.style())
                .inner_margin(egui::Margin::same(10.0))
                .show(ui, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.vertical(|ui| {
                            ui.label(egui::RichText::new(title).heading());
                            if !description.trim().is_empty() {
                                ui.label(description);
                            }
                            if let Some(mf) = manifest {
                                ui.small(format!("Module ID: {}", mf.module_id));
                            } else {
                                ui.small(format!("Module ID: {module_id}"));
                            }
                        });
                    });

                    ui.add_space(8.0);
                    ui.horizontal_wrapped(|ui| {
                        if ui.button("Reload UI").clicked() {
                            st.reload();
                        }
                        if ui.button("Save").clicked() {
                            st.save();
                        }
                        if ui.button("Open Folder").clicked() {
                            open_path_in_explorer(module_dir);
                        }
                    });
                });

            ui.add_space(10.0);
            let sections = resolve_module_ui_sections(&spec);
            let mut main_sections = Vec::new();
            let mut sidebar_sections = Vec::new();
            for section in sections {
                if section.sidebar {
                    sidebar_sections.push(section);
                } else {
                    main_sections.push(section);
                }
            }

            let filled = filled_field_count(&spec, &st.values);
            let total = spec.fields.len();
            let status = st.status.clone();

            if !sidebar_sections.is_empty() && ui.available_width() >= 980.0 {
                let total_width = ui.available_width();
                let sidebar_width = total_width.clamp(260.0, 330.0);
                let main_width = (total_width - sidebar_width - 12.0).max(320.0);

                ui.horizontal_top(|ui| {
                    ui.allocate_ui_with_layout(
                        egui::vec2(main_width, 0.0),
                        egui::Layout::top_down(egui::Align::Min),
                        |ui| {
                            for (idx, section) in main_sections.iter().enumerate() {
                                render_module_section(
                                    ui, st, &spec, module_id, module_dir, section,
                                );
                                if idx + 1 < main_sections.len() {
                                    ui.add_space(10.0);
                                }
                            }
                        },
                    );

                    ui.add_space(12.0);

                    ui.allocate_ui_with_layout(
                        egui::vec2(sidebar_width, 0.0),
                        egui::Layout::top_down(egui::Align::Min),
                        |ui| {
                            render_module_tools_card(
                                ui,
                                module_dir,
                                &st.state_path,
                                &spec_path,
                                filled,
                                total,
                                &status,
                            );
                            ui.add_space(10.0);
                            for (idx, section) in sidebar_sections.iter().enumerate() {
                                render_module_section(
                                    ui, st, &spec, module_id, module_dir, section,
                                );
                                if idx + 1 < sidebar_sections.len() {
                                    ui.add_space(10.0);
                                }
                            }
                        },
                    );
                });
            } else {
                render_module_tools_card(
                    ui,
                    module_dir,
                    &st.state_path,
                    &spec_path,
                    filled,
                    total,
                    &status,
                );
                ui.add_space(10.0);

                for section in &main_sections {
                    render_module_section(ui, st, &spec, module_id, module_dir, section);
                    ui.add_space(10.0);
                }
                for section in &sidebar_sections {
                    render_module_section(ui, st, &spec, module_id, module_dir, section);
                    ui.add_space(10.0);
                }
            }
        }

        return;
    }

    // Fallback: module-provided template-backed workspace text.
    let ws = app
        .module_workspaces
        .entry(module_id.to_string())
        .or_insert_with(|| ModuleWorkspaceState::new(module_dir));
    ws.ensure_loaded();

    ui.heading("Workspace");
    ui.horizontal(|ui| {
        if ui.button("Reload").clicked() {
            ws.reload();
        }
        if ui.button("Load template").clicked() {
            ws.load_template();
        }
        if ui.button("Save").clicked() {
            ws.save();
        }
        if ui.button("Open Folder").clicked() {
            let _ = std::process::Command::new("explorer.exe")
                .arg(module_dir)
                .spawn();
        }
        if !ws.status.trim().is_empty() {
            ui.label(ws.status.clone());
        }
    });

    egui::ScrollArea::vertical()
        .id_salt(format!("module_workspace_{module_id}"))
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.add(
                egui::TextEdit::multiline(&mut ws.text)
                    .desired_rows(18)
                    .hint_text("Module workspace...")
                    .code_editor(),
            );
        });
}
