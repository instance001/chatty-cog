use super::*;


fn networking_section_heading(
    ui: &mut egui::Ui,
    icon: &str,
    color: egui::Color32,
    title: &str,
) {
    ui.horizontal_wrapped(|ui| {
        ui.label(egui::RichText::new(icon).color(color).strong());
        ui.heading(title);
    });
}

fn render_networking_shared_room_section(
    ui: &mut egui::Ui,
    app: &mut ChattyCogApp,
    snapshot: &NetworkSnapshot,
    shared_room_connection_ids: &[String],
    shared_room_highlight: bool,
) -> egui::Response {
    let shared_room = egui::Frame::group(ui.style())
        .fill(if shared_room_highlight {
            egui::Color32::from_rgb(245, 246, 255)
        } else {
            ui.visuals().panel_fill
        })
        .stroke(if shared_room_highlight {
            egui::Stroke::new(1.5, egui::Color32::from_rgb(120, 90, 170))
        } else {
            ui.visuals().widgets.noninteractive.bg_stroke
        })
        .show(ui, |ui| {
            networking_section_heading(
                ui,
                "[ROOM]",
                egui::Color32::from_rgb(120, 90, 170),
                "Shared room chat",
            );
            if shared_room_highlight {
                ui.small(
                    egui::RichText::new("Focused by Quick help")
                        .strong()
                        .color(egui::Color32::from_rgb(120, 90, 170)),
                );
            }
            ui.label(
                "Use this when multiple ChattyCog instances should share one turn-aware room. Main chat can mirror into this room, while hot memory stays local and only luke warm summaries move across the network.",
            );

            let capable_modules = app.shared_chat_capable_modules();
            let mut next_turn_mode = app.networking_shared_chat_policy.turn_mode;
            let mut next_ai_mode = app.networking_shared_chat_policy.ai_mode;
            let mut scope_selection = if app.networking_shared_chat_policy.scope_kind
                == SharedChatScopeKind::Module
            {
                app.networking_shared_chat_policy.scope_module_id.clone()
            } else {
                "__general__".to_string()
            };
            ui.horizontal_wrapped(|ui| {
                ui.label("Scope:");
                egui::ComboBox::from_id_salt("shared_room_scope")
                    .selected_text(app.shared_chat_scope_label())
                    .show_ui(ui, |ui| {
                        ui.selectable_value(
                            &mut scope_selection,
                            "__general__".to_string(),
                            "General room",
                        );
                        for (module_id, module_name, multiplayer) in &capable_modules {
                            let label = if *multiplayer {
                                format!("{module_name} (multiplayer)")
                            } else {
                                module_name.clone()
                            };
                            ui.selectable_value(&mut scope_selection, module_id.clone(), label);
                        }
                    });
                ui.separator();
                ui.label("Turn mode:");
                egui::ComboBox::from_id_salt("shared_room_turn_mode")
                    .selected_text(next_turn_mode.label())
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut next_turn_mode, SharedChatTurnMode::Open, "Open");
                        ui.selectable_value(
                            &mut next_turn_mode,
                            SharedChatTurnMode::TalkingStick,
                            "Talking stick",
                        );
                    });
                ui.separator();
                ui.label("AI mode:");
                egui::ComboBox::from_id_salt("shared_room_ai_mode")
                    .selected_text(next_ai_mode.label())
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut next_ai_mode, SharedChatAiMode::Off, "Off");
                        ui.selectable_value(
                            &mut next_ai_mode,
                            SharedChatAiMode::LocalAllowed,
                            "Local allowed",
                        );
                        ui.selectable_value(
                            &mut next_ai_mode,
                            SharedChatAiMode::HostOnly,
                            "Host only",
                        );
                    });
            });
            let current_scope_selection = if app.networking_shared_chat_policy.scope_kind
                == SharedChatScopeKind::Module
            {
                app.networking_shared_chat_policy.scope_module_id.clone()
            } else {
                "__general__".to_string()
            };
            if scope_selection != current_scope_selection {
                if scope_selection == "__general__" {
                    app.set_shared_chat_scope_general();
                } else if let Some((_, module_name, multiplayer)) = capable_modules
                    .iter()
                    .find(|(module_id, _, _)| module_id == &scope_selection)
                {
                    app.set_shared_chat_scope_module(
                        scope_selection.clone(),
                        module_name.clone(),
                        *multiplayer,
                    );
                }
                app.broadcast_shared_chat_policy("Room scope changed.");
            }
            if next_turn_mode != app.networking_shared_chat_policy.turn_mode {
                app.networking_shared_chat_policy.turn_mode = next_turn_mode;
                if next_turn_mode == SharedChatTurnMode::Open {
                    app.networking_shared_chat_policy.turn_holder_device_id.clear();
                    app.networking_shared_chat_policy.turn_holder_device_name.clear();
                }
                app.broadcast_shared_chat_policy("Turn mode changed.");
            }
            if next_ai_mode != app.networking_shared_chat_policy.ai_mode {
                app.networking_shared_chat_policy.ai_mode = next_ai_mode;
                app.broadcast_shared_chat_policy("AI mode changed.");
            }

            ui.horizontal_wrapped(|ui| {
                ui.checkbox(
                    &mut app.networking_shared_chat_mirror_main_chat,
                    "Mirror local main-chat messages into this room",
                );
                ui.small(format!(
                    "Host: {}",
                    if app
                        .networking_shared_chat_policy
                        .host_device_name
                        .trim()
                        .is_empty()
                    {
                        "(not set)"
                    } else {
                        app.networking_shared_chat_policy.host_device_name.trim()
                    }
                ));
                ui.small(format!("Turn holder: {}", app.shared_chat_turn_holder_label()));
                ui.small(format!(
                    "Connected peers in room: {}",
                    shared_room_connection_ids.len()
                ));
                if app.networking_shared_chat_policy.scope_kind == SharedChatScopeKind::Module {
                    ui.small(format!("Scoped to module: {}", app.shared_chat_scope_label()));
                }
                if let Some(session_summary) = app.shared_chat_session_summary() {
                    ui.small(format!("Session: {session_summary}"));
                }
            });

            if !app.networking_shared_chat_policy.session_active {
                if let Some(recoverable) = app.networking_recoverable_shared_chat_policy.clone() {
                    ui.group(|ui| {
                        ui.strong("Recovered host session available");
                        ui.small(format!(
                            "{} | scope {} | revision {}",
                            if recoverable.session_label.trim().is_empty() {
                                recoverable.session_id.trim()
                            } else {
                                recoverable.session_label.trim()
                            },
                            if recoverable.scope_kind == SharedChatScopeKind::Module
                                && !recoverable.scope_module_name.trim().is_empty()
                            {
                                recoverable.scope_module_name.trim()
                            } else {
                                recoverable.label.trim()
                            },
                            recoverable.session_revision.max(1)
                        ));
                        ui.horizontal_wrapped(|ui| {
                            if ui.button("Resume saved session").clicked() {
                                if let Err(err) = app.resume_recoverable_shared_chat_policy() {
                                    app.networking_status = format!("Networking: {err}");
                                }
                            }
                            if ui.button("Discard recovery").clicked() {
                                app.discard_recoverable_shared_chat_policy();
                                app.networking_status =
                                    "Networking: discarded the saved host-session recovery snapshot."
                                        .to_string();
                            }
                        });
                    });
                }
            } else if app.shared_chat_is_local_host()
                && app.networking_recoverable_shared_chat_policy.is_some()
            {
                ui.small(
                    "Recovery snapshot armed: if this host restarts, you can resume this session cleanly.",
                );
            }

            if let Some(recovery) = app.networking_recoverable_module_session.clone() {
                ui.group(|ui| {
                    ui.strong("Recoverable module session state");
                    ui.small(format!(
                        "{} | latest shared state: {} | cached assets: {}",
                        if recovery.scope_module_name.trim().is_empty() {
                            recovery.scope_module_id.trim()
                        } else {
                            recovery.scope_module_name.trim()
                        },
                        recovery
                            .latest_shared_state
                            .as_ref()
                            .map(|state| format!("revision {}", state.session_revision.max(1)))
                            .unwrap_or_else(|| "none yet".to_string()),
                        recovery.recent_assets.len()
                    ));
                    ui.small(
                        "Use this after a restart or host handoff to restore the module bridge locally, then re-share the last good session state or cached assets to selected peers (or everyone in the room if nothing is selected).",
                    );
                    ui.horizontal_wrapped(|ui| {
                        if ui.button("Restore state to bridge").clicked() {
                            if let Err(err) = app.restore_recoverable_module_shared_state_to_bridge()
                            {
                                app.networking_status = format!("Networking: {err}");
                            }
                        }
                        if ui.button("Re-share latest state").clicked() {
                            if let Err(err) = app.replay_recoverable_module_shared_state() {
                                app.networking_status = format!("Networking: {err}");
                            }
                        }
                        if ui
                            .add_enabled(
                                !recovery.recent_assets.is_empty(),
                                egui::Button::new("Replay cached assets"),
                            )
                            .clicked()
                        {
                            if let Err(err) = app.replay_recoverable_module_assets() {
                                app.networking_status = format!("Networking: {err}");
                            }
                        }
                        if ui.button("Open recovery folder").clicked() {
                            open_path_in_explorer(&app.network_recovery_dir());
                        }
                    });
                });
            }

            if app.networking_shared_chat_policy.session_active
                && app.shared_chat_host_appears_offline()
            {
                ui.group(|ui| {
                    ui.colored_label(
                        egui::Color32::from_rgb(180, 110, 70),
                        "Current room host appears offline.",
                    );
                    ui.small(
                        "You can wait for the host to return, or take over and rebroadcast this room session from here.",
                    );
                    if ui.button("Take over as host").clicked() {
                        if let Err(err) = app.take_over_shared_chat_host() {
                            app.networking_status = format!("Networking: {err}");
                        }
                    }
                });
            }

            let selected_connected_peers = snapshot
                .connected_peers
                .iter()
                .filter(|peer| {
                    let key = if peer.device_id.trim().is_empty() {
                        peer.connection_id.clone()
                    } else {
                        peer.device_id.clone()
                    };
                    app.networking_selected_devices.contains(&key)
                })
                .collect::<Vec<_>>();
            ui.horizontal_wrapped(|ui| {
                if ui.button("Broadcast current room policy").clicked() {
                    app.broadcast_shared_chat_policy("Manual policy refresh.");
                }
                if app.networking_shared_chat_policy.scope_kind == SharedChatScopeKind::Module {
                    if !app.networking_shared_chat_policy.session_active {
                        if ui.button("Start module session").clicked() {
                            if let Some(module_name) = app.begin_shared_chat_module_session() {
                                app.broadcast_shared_chat_policy(&format!(
                                    "Started host-guided module session for {module_name}."
                                ));
                            }
                        }
                    } else if ui.button("End module session").clicked() {
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
                }
                if ui.button("Take stick").clicked() {
                    let local = app.networking.snapshot().clone();
                    app.networking_shared_chat_policy.turn_mode = SharedChatTurnMode::TalkingStick;
                    app.networking_shared_chat_policy.turn_holder_device_id = local.device_id;
                    app.networking_shared_chat_policy.turn_holder_device_name = local.device_name;
                    app.broadcast_shared_chat_policy("Turn stick taken locally.");
                }
                if ui
                    .add_enabled(
                        app.shared_chat_is_local_host() && selected_connected_peers.len() == 1,
                        egui::Button::new("Hand off host to selected peer"),
                    )
                    .clicked()
                {
                    if let Some(peer) = selected_connected_peers.first() {
                        if let Err(err) = app.handoff_shared_chat_host_to_peer(
                            &peer.device_id,
                            &app.network_display_name(&peer.device_id, &peer.device_name),
                        ) {
                            app.networking_status = format!("Networking: {err}");
                        }
                    }
                }
                if ui
                    .add_enabled(
                        selected_connected_peers.len() == 1,
                        egui::Button::new("Pass stick to selected peer"),
                    )
                    .clicked()
                {
                    if let Some(peer) = selected_connected_peers.first() {
                        app.networking_shared_chat_policy.turn_mode = SharedChatTurnMode::TalkingStick;
                        app.networking_shared_chat_policy.turn_holder_device_id =
                            peer.device_id.clone();
                        app.networking_shared_chat_policy.turn_holder_device_name =
                            app.network_display_name(&peer.device_id, &peer.device_name);
                        app.broadcast_shared_chat_policy("Turn stick reassigned.");
                    }
                }
                if ui.button("Open room flow").clicked() {
                    app.networking_shared_chat_policy.turn_mode = SharedChatTurnMode::Open;
                    app.networking_shared_chat_policy.turn_holder_device_id.clear();
                    app.networking_shared_chat_policy.turn_holder_device_name.clear();
                    app.broadcast_shared_chat_policy("Talking stick cleared.");
                }
            });

            let room_hint = if shared_room_connection_ids.is_empty() {
                "Connect to one or more peers to turn the shared room into a live conversation lane."
                    .to_string()
            } else {
                match app.shared_chat_can_send_user_message() {
                    Ok(()) => "You can type here to send a room message, or mirror your normal Chat tab into this room.".to_string(),
                    Err(reason) => reason,
                }
            };
            ui.small(room_hint);

            egui::ScrollArea::vertical()
                .id_salt("shared_room_log")
                .max_height(200.0)
                .show(ui, |ui| {
                    if app.networking_shared_chat_log.is_empty() {
                        ui.label("(no shared room activity yet)");
                    } else {
                        for entry in app.networking_shared_chat_log.iter().rev().take(48).rev() {
                            ui.group(|ui| {
                                ui.horizontal_wrapped(|ui| {
                                    let tag_color = match entry.speaker_kind.as_str() {
                                        "assistant" => egui::Color32::from_rgb(50, 140, 90),
                                        "system" => egui::Color32::from_rgb(120, 90, 170),
                                        _ => egui::Color32::from_rgb(70, 110, 180),
                                    };
                                    ui.label(
                                        egui::RichText::new(format!(
                                            "[{}]",
                                            entry.speaker_kind.to_uppercase()
                                        ))
                                        .color(tag_color)
                                        .strong(),
                                    );
                                    ui.strong(if entry.speaker_label.trim().is_empty() {
                                        entry.from_device_name.trim()
                                    } else {
                                        entry.speaker_label.trim()
                                    });
                                    ui.small(format!(
                                        "{} | {}",
                                        entry.from_device_name, entry.sent_at_unix_ms
                                    ));
                                    if entry.scope_kind == SharedChatScopeKind::Module {
                                        let scope_name =
                                            if entry.scope_module_name.trim().is_empty() {
                                                entry.scope_module_id.trim()
                                            } else {
                                                entry.scope_module_name.trim()
                                            };
                                        ui.small(format!("scope: {scope_name}"));
                                    }
                                });
                                ui.label(entry.body.trim());
                            });
                            ui.add_space(4.0);
                        }
                    }
                });

            ui.horizontal_wrapped(|ui| {
                let shared_input_width = (ui.available_width() - 120.0).max(220.0);
                let input = ui.add(
                    egui::TextEdit::singleline(&mut app.networking_shared_chat_input)
                        .desired_width(shared_input_width)
                        .hint_text("Shared room message..."),
                );
                let send_enabled = !app.networking_shared_chat_input.trim().is_empty()
                    && !shared_room_connection_ids.is_empty()
                    && app.shared_chat_can_send_user_message().is_ok();
                if input.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    if send_enabled {
                        let body = app.networking_shared_chat_input.trim().to_string();
                        app.networking_shared_chat_input.clear();
                        app.broadcast_shared_chat_message("user", "You", &body);
                    }
                }
                if ui
                    .add_enabled(send_enabled, egui::Button::new("Send to room"))
                    .clicked()
                {
                    let body = app.networking_shared_chat_input.trim().to_string();
                    app.networking_shared_chat_input.clear();
                    app.broadcast_shared_chat_message("user", "You", &body);
                }
            });
        });

    shared_room.response
}

fn render_networking_handoff_section(
    ui: &mut egui::Ui,
    app: &mut ChattyCogApp,
    snapshot: &NetworkSnapshot,
) {
    networking_section_heading(
        ui,
        "[OUT]",
        egui::Color32::from_rgb(120, 90, 170),
        "Cross-instance handoff",
    );
    ui.label(
        "Pass a concise brief to another connected ChattyCog instance without leaving the local network.",
    );

    if snapshot.connected_peers.is_empty() {
        ui.label("Connect to another ChattyCog instance to send a handoff.");
    } else {
        let selected_label = snapshot
            .connected_peers
            .iter()
            .find(|peer| peer.connection_id == app.networking_handoff_target)
            .map(|peer| peer.device_name.clone())
            .unwrap_or_else(|| "(choose target)".to_string());

        egui::ComboBox::from_id_salt("network_handoff_target")
            .selected_text(selected_label)
            .show_ui(ui, |ui| {
                for peer in &snapshot.connected_peers {
                    ui.selectable_value(
                        &mut app.networking_handoff_target,
                        peer.connection_id.clone(),
                        peer.device_name.clone(),
                    );
                }
            });

        ui.add(
            egui::TextEdit::singleline(&mut app.networking_handoff_title)
                .hint_text("Short handoff title..."),
        );
        ui.add(
            egui::TextEdit::multiline(&mut app.networking_handoff_body)
                .desired_rows(5)
                .hint_text("What should the other instance know or pick up?"),
        );

        let send_enabled = !app.networking_handoff_target.trim().is_empty()
            && !app.networking_handoff_body.trim().is_empty();
        if ui
            .add_enabled(send_enabled, egui::Button::new("Send handoff"))
            .clicked()
        {
            app.networking.send_handoff(
                &app.networking_handoff_target,
                &app.networking_handoff_title,
                &app.networking_handoff_body,
            );
            app.networking_handoff_title.clear();
            app.networking_handoff_body.clear();
        }
    }
}

fn render_networking_received_handoffs(
    ui: &mut egui::Ui,
    app: &mut ChattyCogApp,
    snapshot: &NetworkSnapshot,
) {
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new("[IN]")
                .color(egui::Color32::from_rgb(120, 90, 170))
                .strong(),
        );
        ui.heading("Received handoffs");
        if !snapshot.received_handoffs.is_empty() && ui.button("Clear received").clicked() {
            app.networking.clear_received_handoffs();
            app.networking_seen_handoffs.clear();
        }
    });

    if snapshot.received_handoffs.is_empty() {
        ui.label("(none yet)");
    } else {
        for handoff in &snapshot.received_handoffs {
            ui.group(|ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.strong(if handoff.title.trim().is_empty() {
                        "(untitled handoff)"
                    } else {
                        handoff.title.trim()
                    });
                    ui.small(format!(
                        "from {} | {}s ago",
                        handoff.from_device_name, handoff.received_secs_ago
                    ));
                });
                ui.monospace(&handoff.from_device_id);
                ui.small(format!("Address: {}", handoff.from_address));
                ui.label(&handoff.body);
            });
            ui.add_space(6.0);
        }
    }
}

fn render_networking_received_transfers(
    ui: &mut egui::Ui,
    app: &mut ChattyCogApp,
    snapshot: &NetworkSnapshot,
    received_transfer_visible: &[&ReceivedArtifact],
) {
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new("[XFER]")
                .color(egui::Color32::from_rgb(70, 140, 90))
                .strong(),
        );
        ui.heading("Received transfers");
        if !snapshot.received_artifacts.is_empty() && ui.button("Clear transfers").clicked() {
            app.networking.clear_received_artifacts();
            app.networking_seen_artifacts.clear();
        }
    });

    if received_transfer_visible.is_empty() {
        ui.label("(no shared module states or other transfers yet)");
    } else {
        for artifact in received_transfer_visible {
            ui.group(|ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.strong(if artifact.label.trim().is_empty() {
                        "(untitled transfer)"
                    } else {
                        artifact.label.trim()
                    });
                    ui.small(format!(
                        "{} from {} | {}s ago",
                        artifact.kind.trim(),
                        artifact.from_device_name,
                        artifact.received_secs_ago
                    ));
                });
                ui.monospace(&artifact.from_device_id);
                if !artifact.module_id.trim().is_empty() {
                    ui.small(format!("Module: {}", artifact.module_id));
                }
                if !artifact.file_name.trim().is_empty() {
                    ui.small(format!("File: {}", artifact.file_name));
                }
                ui.small(format_network_transfer_meta(
                    &artifact.content_type,
                    &artifact.transfer_encoding,
                    artifact.byte_len,
                    artifact.chunk_count,
                ));
                if artifact.is_binary() {
                    ui.small("Payload: binary/file-style transfer");
                }
                if !artifact.summary.trim().is_empty() {
                    ui.label(artifact.summary.trim());
                }
                ui.small(format!("Address: {}", artifact.from_address));
            });
            ui.add_space(6.0);
        }
    }
}

fn render_networking_quick_help(ui: &mut egui::Ui, app: &mut ChattyCogApp, snapshot: &NetworkSnapshot) {
    egui::CollapsingHeader::new("Quick help")
        .id_salt("chattycog_networking_quick_help")
        .default_open(false)
        .show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label(egui::RichText::new("Presets").strong());
                if ui
                    .selectable_value(
                        &mut app.networking_help_mode,
                        NetworkingQuickHelpMode::Everyday,
                        "Everyday",
                    )
                    .clicked()
                {
                    app.focus_networking_section(NetworkingFocusSection::DeviceList);
                }
                if ui
                    .selectable_value(
                        &mut app.networking_help_mode,
                        NetworkingQuickHelpMode::HostSetup,
                        "Host setup",
                    )
                    .clicked()
                {
                    app.focus_networking_section(NetworkingFocusSection::Controls);
                }
                if ui
                    .selectable_value(
                        &mut app.networking_help_mode,
                        NetworkingQuickHelpMode::ApprovalFirst,
                        "Approval first",
                    )
                    .clicked()
                {
                    let target = if snapshot.pending_requests.is_empty() {
                        NetworkingFocusSection::Controls
                    } else {
                        NetworkingFocusSection::PendingRequests
                    };
                    app.focus_networking_section(target);
                }
            });
            ui.add_space(4.0);

            let help_rows = match app.networking_help_mode {
                NetworkingQuickHelpMode::Everyday => vec![
                    (
                        "[HOST]",
                        "Turn on `Make available for connectivity` on the machine that should be visible.",
                    ),
                    (
                        "[SCAN]",
                        "Click `Refresh discovery` on the other machine, then `Connect` when it appears.",
                    ),
                    (
                        "[NAME]",
                        "Click a device name to give it a friendlier local name, or click `+ Group` to tag it.",
                    ),
                    (
                        "[FIND]",
                        "Use `Find` to search by name, device ID, address, or group label.",
                    ),
                    (
                        "[FAST]",
                        "`Select Connected` is the quickest way to act on the peers that are live right now.",
                    ),
                    (
                        "[SETUP]",
                        "Use `Workflow bundle` when you want to share the whole ChattyCog setup; use handoffs for short notes and module shares for module-specific state.",
                    ),
                    (
                        "[ROOM]",
                        "Use `Shared room chat` when you want a lightweight cross-instance room with talking-stick and AI-on/off rules.",
                    ),
                ],
                NetworkingQuickHelpMode::HostSetup => vec![
                    (
                        "[HOST]",
                        "Use this when you are the visible machine and other nearby ChattyCog instances should connect to you.",
                    ),
                    (
                        "[CHECK]",
                        "Keep an eye on the status line, listener port, and connected section so you can tell whether hosting is actually up.",
                    ),
                    (
                        "[TRUST]",
                        "Leave `Allow unknown devices` on for a relaxed trusted-room setup, or switch it off if you want approval prompts.",
                    ),
                    (
                        "[LABEL]",
                        "Rename frequently used peers so they stay recognizable the next time they appear.",
                    ),
                    (
                        "[BND]",
                        "Send a workflow bundle when you want nearby peers to mirror this machine's current setup, prompts, and AI preferences without copying logs or cold memory.",
                    ),
                    (
                        "[ROOM]",
                        "If you want one orderly shared room instead of several separate local chats, use the `Shared room chat` controls just below.",
                    ),
                    (
                        "[DEBUG]",
                        "Use `Copy info` when you need a clean support/debug snapshot of name, ID, and address.",
                    ),
                    (
                        "[PAIR]",
                        "Use `Export trusted list` / `Import trusted list` for remembered pairings, and `Export blocked list` / `Import blocked list` when you want another ChattyCog machine to inherit the same deny rules.",
                    ),
                    (
                        "[SYNC]",
                        "If a nearby machine shows up but refuses to talk cleanly, check the `Compatibility note` line to spot protocol/version mismatch quickly.",
                    ),
                ],
                NetworkingQuickHelpMode::ApprovalFirst => vec![
                    (
                        "[LOCK]",
                        "Turn off `Allow unknown devices` if you want new peers to ask first instead of joining freely.",
                    ),
                    (
                        "[QUEUE]",
                        "Pending requests appear above the device list, where you can Allow, Deny, or Block them.",
                    ),
                    (
                        "[BLOCK]",
                        "`Block` disconnects the peer now and keeps it out until you unblock it later.",
                    ),
                    (
                        "[REVIEW]",
                        "Use `Copy ID` or `Copy info` before allowing a device if you need to confirm which machine it is.",
                    ),
                    (
                        "[INBOX]",
                        "Received workflow bundles land in `Received setup bundles`, where you can preview them calmly before applying anything.",
                    ),
                    (
                        "[ROOM]",
                        "Use `Broadcast current room policy` when you want every connected peer to see the same talking-stick and AI rules.",
                    ),
                    (
                        "[RESET]",
                        "Blocked devices live in their own section so you can review and unblock them deliberately.",
                    ),
                    (
                        "[PAIR]",
                        "Trusted and blocked lists are portable now, so you can export a known-good policy set and import it on another local machine instead of rebuilding it by hand.",
                    ),
                ],
            };
            for (tag, body) in help_rows {
                ui.horizontal_wrapped(|ui| {
                    ui.label(egui::RichText::new(tag).monospace().strong());
                    ui.small(body);
                });
            }
        });
}

fn render_networking_controls_section(
    ui: &mut egui::Ui,
    app: &mut ChattyCogApp,
    snapshot: &NetworkSnapshot,
    controls_highlight: bool,
) -> egui::Response {
    let controls = egui::Frame::group(ui.style())
        .fill(if controls_highlight {
            egui::Color32::from_rgb(246, 250, 255)
        } else {
            ui.visuals().panel_fill
        })
        .stroke(if controls_highlight {
            egui::Stroke::new(1.5, egui::Color32::from_rgb(70, 110, 180))
        } else {
            ui.visuals().widgets.noninteractive.bg_stroke
        })
        .show(ui, |ui| {
            networking_section_heading(
                ui,
                "[CTL]",
                egui::Color32::from_rgb(70, 110, 180),
                "Network controls",
            );
            if controls_highlight {
                ui.small(
                    egui::RichText::new("Focused by Quick help")
                        .strong()
                        .color(egui::Color32::from_rgb(70, 110, 180)),
                );
            }
            ui.horizontal_wrapped(|ui| {
                let mut available = snapshot.available_for_connectivity;
                if ui
                    .checkbox(&mut available, "Make available for connectivity")
                    .changed()
                {
                    app.networking.set_available(available);
                }
                let mut allow_unknown = snapshot.allow_unknown_devices;
                if ui
                    .checkbox(&mut allow_unknown, "Allow unknown devices")
                    .changed()
                {
                    app.networking.set_allow_unknown_devices(allow_unknown);
                    app.prefs.network_allow_unknown_devices = allow_unknown;
                    app.persist_network_prefs();
                }
                let mut allow_shared_lukewarm = app.prefs.network_allow_shared_lukewarm_context;
                if ui
                    .checkbox(
                        &mut allow_shared_lukewarm,
                        "Allow shared luke warm context",
                    )
                    .changed()
                {
                    app.prefs.network_allow_shared_lukewarm_context = allow_shared_lukewarm;
                    app.persist_network_prefs();
                }
                if ui.button("Refresh discovery").clicked() {
                    app.networking.refresh_discovery();
                }
            });
            ui.horizontal_wrapped(|ui| {
                let has_trusted = !app.prefs.network_trusted_devices.is_empty();
                let has_blocked = !app.prefs.network_blocked_devices.is_empty();
                if ui
                    .add_enabled(has_trusted, egui::Button::new("Export trusted list"))
                    .clicked()
                {
                    app.export_trusted_peer_list();
                }
                if ui.button("Import trusted list").clicked() {
                    app.import_trusted_peer_list();
                }
                if ui
                    .add_enabled(has_blocked, egui::Button::new("Export blocked list"))
                    .clicked()
                {
                    app.export_blocked_peer_list();
                }
                if ui.button("Import blocked list").clicked() {
                    app.import_blocked_peer_list();
                }
                if !has_trusted {
                    ui.small(
                        "Trust a few regular peers first if you want to export a reusable pairing list.",
                    );
                } else if !has_blocked {
                    ui.small(
                        "Blocked lists are handy when you want another machine to inherit the same deny rules.",
                    );
                }
            });
        });
    controls.response
}

fn render_networking_pending_requests_section(
    ui: &mut egui::Ui,
    app: &mut ChattyCogApp,
    snapshot: &NetworkSnapshot,
    pending_highlight: bool,
) -> egui::Response {
    let pending = egui::Frame::group(ui.style())
        .fill(if pending_highlight {
            egui::Color32::from_rgb(255, 248, 240)
        } else {
            ui.visuals().panel_fill
        })
        .stroke(if pending_highlight {
            egui::Stroke::new(1.5, egui::Color32::from_rgb(190, 110, 30))
        } else {
            ui.visuals().widgets.noninteractive.bg_stroke
        })
        .show(ui, |ui| {
            networking_section_heading(
                ui,
                "[REQ]",
                egui::Color32::from_rgb(190, 110, 30),
                "Pending device requests",
            );
            if pending_highlight {
                ui.small(
                    egui::RichText::new("Focused by Quick help")
                        .strong()
                        .color(egui::Color32::from_rgb(190, 110, 30)),
                );
            }
            for request in &snapshot.pending_requests {
                ui.horizontal_wrapped(|ui| {
                    ui.label(format!(
                        "Unknown device {} [{}] requesting connection.",
                        request.device_name, request.device_id
                    ));
                    ui.small(format!("{} | {}s ago", request.address, request.requested_secs_ago));
                });
                ui.horizontal(|ui| {
                    if ui.button("Allow").clicked() {
                        app.networking.allow_pending_peer(&request.device_id);
                    }
                    if ui.button("Trust").clicked() {
                        app.trust_network_peer(&request.device_id, &request.device_name);
                    }
                    if ui.button("Deny").clicked() {
                        app.networking.deny_pending_peer(&request.device_id);
                    }
                    if ui.button("Block").clicked() {
                        app.block_network_peer(&request.device_id, &request.device_name);
                    }
                });
                ui.separator();
            }
        });
    pending.response
}

fn render_networking_this_device_section(
    ui: &mut egui::Ui,
    app: &mut ChattyCogApp,
    snapshot: &NetworkSnapshot,
    local_connection_info: &str,
) {
    ui.group(|ui| {
        networking_section_heading(
            ui,
            "[ME]",
            egui::Color32::from_rgb(70, 110, 180),
            "This device",
        );
        ui.label(format!("Name: {}", snapshot.device_name));
        ui.horizontal(|ui| {
            ui.label("Device ID:");
            ui.monospace(&snapshot.device_id);
            if ui.button("Copy device ID").clicked() {
                ui.ctx().copy_text(snapshot.device_id.clone());
                app.networking_status = "Copied local device ID.".to_string();
            }
            if ui.button("Copy connection info").clicked() {
                ui.ctx().copy_text(local_connection_info.to_string());
                app.networking_status = "Copied local connection info.".to_string();
            }
        });
        ui.horizontal(|ui| {
            ui.label("Edit name:");
            ui.add(
                egui::TextEdit::singleline(&mut app.networking_device_name_input)
                    .desired_width(260.0)
                    .hint_text("e.g. Office PC"),
            );
        });
        ui.horizontal(|ui| {
            if ui.button("Save name").clicked() {
                let trimmed = app.networking_device_name_input.trim().to_string();
                app.networking.set_device_name(&trimmed);
                app.prefs.network_device_name = trimmed;
                match preferences::save_prefs(&app.prefs_path, &app.prefs) {
                    Ok(()) => app.prefs_status = "Saved networking device name.".to_string(),
                    Err(e) => app.prefs_status = format!("Save failed: {e}"),
                }
                app.networking_device_name_input = app.networking.snapshot().device_name.clone();
            }
            if ui.button("Reset default").clicked() {
                app.networking.set_device_name("");
                app.prefs.network_device_name.clear();
                match preferences::save_prefs(&app.prefs_path, &app.prefs) {
                    Ok(()) => app.prefs_status = "Reset networking device name.".to_string(),
                    Err(e) => app.prefs_status = format!("Save failed: {e}"),
                }
                app.networking_device_name_input = app.networking.snapshot().device_name.clone();
            }
        });
        ui.label(format!(
            "Visibility: {}",
            if snapshot.available_for_connectivity {
                "Available on local network"
            } else {
                "Hidden / client only"
            }
        ));
        if let Some(port) = snapshot.listener_port {
            ui.label(format!("Host port: {port}"));
        }
        if !snapshot.local_presence.active_tab.trim().is_empty() {
            ui.label(format!("Shared tab status: {}", snapshot.local_presence.active_tab));
        }
        if !snapshot.local_presence.runtime_status.trim().is_empty() {
            ui.label(format!(
                "Shared runtime status: {}",
                snapshot.local_presence.runtime_status
            ));
        }
        if !snapshot.status.is_empty() {
            ui.label(format!("Status: {}", snapshot.status));
        }
        if !snapshot.protocol_notice.trim().is_empty() {
            ui.colored_label(
                egui::Color32::from_rgb(190, 110, 30),
                format!("Compatibility note: {}", snapshot.protocol_notice),
            );
        }
        if !snapshot.last_error.is_empty() {
            ui.colored_label(
                egui::Color32::from_rgb(160, 32, 32),
                format!("Last error: {}", snapshot.last_error),
            );
        }
        if !app.networking_status.trim().is_empty() {
            ui.small(app.networking_status.clone());
        }
        if !app.prefs_status.trim().is_empty() {
            ui.small(app.prefs_status.clone());
        }
    });
}

fn render_networking_peer_actions_section(
    ui: &mut egui::Ui,
    app: &mut ChattyCogApp,
    connected_visible: &[&ConnectedPeer],
    available_visible: &[&DiscoveredPeer],
    blocked_visible: &[&BlockedPeer],
    device_list_highlight: bool,
) -> egui::Response {
    let device_list = egui::Frame::group(ui.style())
        .fill(if device_list_highlight {
            egui::Color32::from_rgb(244, 250, 244)
        } else {
            ui.visuals().panel_fill
        })
        .stroke(if device_list_highlight {
            egui::Stroke::new(1.5, egui::Color32::from_rgb(70, 140, 90))
        } else {
            ui.visuals().widgets.noninteractive.bg_stroke
        })
        .show(ui, |ui| {
            networking_section_heading(
                ui,
                "[ACT]",
                egui::Color32::from_rgb(70, 140, 90),
                "Peer actions",
            );
            if device_list_highlight {
                ui.small(
                    egui::RichText::new("Focused by Quick help")
                        .strong()
                        .color(egui::Color32::from_rgb(70, 140, 90)),
                );
            }
            ui.horizontal_wrapped(|ui| {
                let connected_keys = connected_visible
                    .iter()
                    .map(|peer| {
                        if peer.device_id.trim().is_empty() {
                            peer.connection_id.clone()
                        } else {
                            peer.device_id.clone()
                        }
                    })
                    .collect::<Vec<_>>();
                let available_keys = available_visible
                    .iter()
                    .map(|peer| peer.device_id.clone())
                    .collect::<Vec<_>>();
                let blocked_keys = blocked_visible
                    .iter()
                    .map(|peer| peer.device_id.clone())
                    .collect::<Vec<_>>();
                if ui.button("Select All").clicked() {
                    app.networking_selected_devices = connected_keys
                        .iter()
                        .chain(available_keys.iter())
                        .chain(blocked_keys.iter())
                        .cloned()
                        .collect();
                }
                if ui.button("Deselect All").clicked() {
                    app.networking_selected_devices.clear();
                }
                if ui.button("Select Connected").clicked() {
                    app.networking_selected_devices = connected_keys.iter().cloned().collect();
                }
                if ui.button("Select Available").clicked() {
                    app.networking_selected_devices = available_keys.iter().cloned().collect();
                }
                ui.separator();
                ui.label("Find:");
                ui.add(
                    egui::TextEdit::singleline(&mut app.networking_filter)
                        .desired_width(200.0)
                        .hint_text("find device"),
                );
            });
            let selected_count = app.networking_selected_devices.len();
            ui.horizontal_wrapped(|ui| {
                let selected_connections = connected_visible
                    .iter()
                    .filter(|peer| {
                        let key = if peer.device_id.trim().is_empty() {
                            peer.connection_id.clone()
                        } else {
                            peer.device_id.clone()
                        };
                        app.networking_selected_devices.contains(&key)
                    })
                    .collect::<Vec<_>>();
                if ui
                    .add_enabled(selected_count > 0, egui::Button::new("Connect Selected"))
                    .clicked()
                {
                    for peer in available_visible {
                        if app.networking_selected_devices.contains(&peer.device_id) {
                            app.networking.connect_peer(&peer.device_id);
                        }
                    }
                }
                if ui
                    .add_enabled(selected_count > 0, egui::Button::new("Disconnect Selected"))
                    .clicked()
                {
                    for peer in &selected_connections {
                        app.networking.disconnect_connection(&peer.connection_id);
                    }
                }
                if ui
                    .add_enabled(selected_count > 0, egui::Button::new("Block Selected"))
                    .clicked()
                {
                    let mut blocked_count = 0usize;
                    for peer in connected_visible {
                        let key = if peer.device_id.trim().is_empty() {
                            peer.connection_id.clone()
                        } else {
                            peer.device_id.clone()
                        };
                        if app.networking_selected_devices.contains(&key)
                            && !peer.device_id.trim().is_empty()
                        {
                            app.block_network_peer(&peer.device_id, &peer.device_name);
                            blocked_count += 1;
                        }
                    }
                    for peer in available_visible {
                        if app.networking_selected_devices.contains(&peer.device_id) {
                            app.block_network_peer(&peer.device_id, &peer.device_name);
                            blocked_count += 1;
                        }
                    }
                    if blocked_count > 0 {
                        app.networking_status =
                            format!("Blocked {} selected device(s).", blocked_count);
                    }
                }
            });
            ui.small(format!(
                "Connected: {} | Available: {} | Blocked: {} | Selected: {}",
                connected_visible.len(),
                available_visible.len(),
                blocked_visible.len(),
                app.networking_selected_devices.len()
            ));
            ui.small(
                "Tip: click a device name to rename it, and click the group chip to tag it for your own workflow.",
            );
    });
    device_list.response
}

fn render_networking_workflow_bundle_section(
    ui: &mut egui::Ui,
    app: &mut ChattyCogApp,
    selected_connections: &[String],
) {
    networking_section_heading(
        ui,
        "[BND]",
        egui::Color32::from_rgb(80, 120, 170),
        "Workflow bundle",
    );
    ui.label(
        "Capture the current ChattyCog setup into a portable bundle: system prompt, model hints, sampling settings, sandbox policy, and per-module AI preferences.",
    );
    ui.add(
        egui::TextEdit::singleline(&mut app.networking_bundle_label)
            .hint_text("Bundle title..."),
    );
    ui.add(
        egui::TextEdit::multiline(&mut app.networking_bundle_summary)
            .desired_rows(3)
            .hint_text("What is this setup for?"),
    );
    ui.horizontal_wrapped(|ui| {
        ui.small(format!(
            "Selected connected peers: {}",
            selected_connections.len()
        ));
        ui.small(format!("Module prefs included: {}", app.prefs.modules.len()));
        ui.small(format!(
            "System prompt: {} chars",
            app.current_system_prompt().chars().count()
        ));
    });
    if selected_connections.is_empty() {
        ui.small("Select one or more connected peers above before sending a workflow bundle.");
    } else if ui.button("Send current setup to selected peers").clicked() {
        let bundle = app.build_current_workflow_bundle();
        let summary = if bundle.summary.trim().is_empty() {
            format!(
                "ChattyCog setup with {} module preference(s)",
                bundle.module_preferences.len()
            )
        } else {
            bundle.summary.trim().to_string()
        };
        match serde_json::to_string_pretty(&bundle) {
            Ok(text) => {
                let label = if bundle.label.trim().is_empty() {
                    "ChattyCog setup".to_string()
                } else {
                    bundle.label.trim().to_string()
                };
                let file_name = format!(
                    "workflow_bundle_{}.json",
                    slugify_filename(&label, "workflow_bundle")
                );
                for connection_id in selected_connections {
                    app.networking.send_artifact(
                        connection_id,
                        "workflow_bundle_json",
                        &label,
                        None,
                        &summary,
                        &file_name,
                        &text,
                    );
                }
                app.networking_status = format!(
                    "Networking: sent workflow bundle to {} selected peer(s).",
                    selected_connections.len()
                );
            }
            Err(err) => {
                app.networking_status =
                    format!("Networking: could not serialize workflow bundle: {err}");
            }
        }
    }
}

fn render_networking_lukewarm_share_section(
    ui: &mut egui::Ui,
    app: &mut ChattyCogApp,
    selected_connections: &[String],
) {
    networking_section_heading(
        ui,
        "[MEM]",
        egui::Color32::from_rgb(130, 90, 170),
        "Shared luke warm memory",
    );
    ui.label(
        "Share summary-only recent context with selected peers. Hot memory stays local, and cold logs are not transferred.",
    );
    let local_lukewarm_share = app.build_current_lukewarm_share();
    let applied_lukewarm_count =
        load_applied_lukewarm_contexts(&app.applied_lukewarm_dir()).unwrap_or_default().len();
    ui.horizontal_wrapped(|ui| {
        ui.small(format!(
            "Selected connected peers: {}",
            selected_connections.len()
        ));
        ui.small(format!("Applied peer summaries: {}", applied_lukewarm_count));
        ui.small(if app.prefs.network_allow_shared_lukewarm_context {
            "Shared luke warm is allowed in prompts"
        } else {
            "Shared luke warm is stored but not injected into prompts"
        });
    });
    let local_lukewarm_preview = if local_lukewarm_share.context_text.trim().is_empty() {
        "(No local luke warm context is ready yet.)".to_string()
    } else {
        local_lukewarm_share.context_text.clone()
    };
    egui::ScrollArea::vertical()
        .id_salt("network_lukewarm_share_preview")
        .max_height(160.0)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.add(egui::Label::new(local_lukewarm_preview).wrap());
        });
    if selected_connections.is_empty() {
        ui.small("Select one or more connected peers above before sharing luke warm context.");
    } else if ui
        .add_enabled(
            !local_lukewarm_share.context_text.trim().is_empty(),
            egui::Button::new("Share current luke warm to selected peers"),
        )
        .clicked()
    {
        match serde_json::to_string_pretty(&local_lukewarm_share) {
            Ok(text) => {
                let file_name = format!(
                    "lukewarm_context_{}.json",
                    slugify_filename(&local_lukewarm_share.label, "lukewarm_context")
                );
                for connection_id in selected_connections {
                    app.networking.send_artifact(
                        connection_id,
                        "lukewarm_context_json",
                        &local_lukewarm_share.label,
                        None,
                        &local_lukewarm_share.summary,
                        &file_name,
                        &text,
                    );
                }
                app.networking_status = format!(
                    "Networking: shared luke warm context to {} selected peer(s).",
                    selected_connections.len()
                );
            }
            Err(err) => {
                app.networking_status =
                    format!("Networking: could not serialize luke warm context: {err}");
            }
        }
    }
    app.render_received_lukewarm_inbox(ui, "Received luke warm context");
}

fn render_networking_recent_events_section(
    ui: &mut egui::Ui,
    app: &mut ChattyCogApp,
    snapshot: &NetworkSnapshot,
) {
    networking_section_heading(
        ui,
        "[EVT]",
        egui::Color32::from_rgb(170, 110, 70),
        "Recent session events",
    );
    ui.label(
        "Low-latency room events are meant for lightweight module signals like turns, small moves, ready states, or other game/program session nudges.",
    );
    ui.horizontal_wrapped(|ui| {
        ui.small(format!(
            "Recent events cached: {}",
            snapshot.received_session_events.len()
        ));
        if !snapshot.received_session_events.is_empty()
            && ui.button("Clear recent events").clicked()
        {
            app.networking.clear_received_session_events();
        }
    });
    if snapshot.received_session_events.is_empty() {
        ui.label("(no recent session events yet)");
    } else {
        for event in snapshot.received_session_events.iter().rev().take(24) {
            render_networking_recent_event_card(ui, event);
            ui.add_space(4.0);
        }
    }
}

fn render_networking_recent_event_card(ui: &mut egui::Ui, event: &ReceivedSessionEvent) {
    ui.group(|ui| {
        ui.horizontal_wrapped(|ui| {
            ui.strong(if event.label.trim().is_empty() {
                event.event_type.trim()
            } else {
                event.label.trim()
            });
            ui.small(format!(
                "{} | {}s ago",
                if event.from_device_name.trim().is_empty() {
                    "(unknown sender)"
                } else {
                    event.from_device_name.trim()
                },
                event.received_secs_ago
            ));
            if !event.scope_module_id.trim().is_empty() {
                ui.small(format!("module: {}", event.scope_module_id.trim()));
            }
            if !event.session_id.trim().is_empty() {
                ui.small(format!("session: {}", event.session_id.trim()));
            }
            if !event.from_address.trim().is_empty() {
                ui.small(format!("addr: {}", event.from_address.trim()));
            }
            if !event.content_type.trim().is_empty() {
                ui.small(event.content_type.trim());
            }
        });
        if !event.payload_text.trim().is_empty() {
            ui.label(event.payload_text.trim());
        } else {
            ui.small("(no text payload)");
        }
    });
}

fn render_networking_delivery_status_section(
    ui: &mut egui::Ui,
    delivery_visible: &[&OutgoingArtifactDelivery],
) {
    networking_section_heading(
        ui,
        "[ACK]",
        egui::Color32::from_rgb(80, 120, 170),
        "Recent delivery status",
    );
    if delivery_visible.is_empty() {
        ui.label("(no recent outgoing transfers yet)");
    } else {
        for artifact in delivery_visible {
            ui.group(|ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.strong(if artifact.label.trim().is_empty() {
                        artifact.kind.trim()
                    } else {
                        artifact.label.trim()
                    });
                    ui.small(format!(
                        "{} | {} attempt(s) | {}s ago",
                        artifact.status.trim(),
                        artifact.attempts,
                        artifact.updated_secs_ago
                    ));
                    ui.small(if artifact.waiting_for_ack {
                        "Awaiting ack"
                    } else {
                        "Closed loop"
                    });
                });
                ui.monospace(&artifact.artifact_id);
                if !artifact.to_device_name.trim().is_empty() {
                    ui.small(format!("To: {}", artifact.to_device_name));
                }
                if !artifact.to_device_id.trim().is_empty() {
                    ui.monospace(&artifact.to_device_id);
                }
                if !artifact.to_address.trim().is_empty() {
                    ui.small(format!("Address: {}", artifact.to_address));
                }
                if !artifact.module_id.trim().is_empty() {
                    ui.small(format!("Module: {}", artifact.module_id));
                }
                if !artifact.file_name.trim().is_empty() {
                    ui.small(format!("File: {}", artifact.file_name));
                }
                ui.small(format_network_transfer_meta(
                    &artifact.content_type,
                    &artifact.transfer_encoding,
                    artifact.byte_len,
                    artifact.chunk_count,
                ));
                if !artifact.summary.trim().is_empty() {
                    ui.label(artifact.summary.trim());
                }
            });
            ui.add_space(6.0);
        }
    }
}

pub(super) fn networking_tab(ui: &mut egui::Ui, app: &mut ChattyCogApp) {
    let snapshot = app.networking.snapshot().clone();
    if app
        .networking_focus_flash_until
        .is_some_and(|until| Instant::now() >= until)
    {
        app.networking_focus_flash_until = None;
        app.networking_focus_section = None;
    }
    let pending_focus = app.networking_focus_pending.take();
    let highlighted_section = app.networking_focus_section;
    let highlight_until = app.networking_focus_flash_until;
    let highlight_active = |section: NetworkingFocusSection| {
        highlighted_section == Some(section)
            && highlight_until.is_some_and(|until| Instant::now() < until)
    };
    let blend_color = |base: egui::Color32, tint: egui::Color32, amount: f32| {
        let inverse = 1.0 - amount;
        egui::Color32::from_rgba_premultiplied(
            ((base.r() as f32 * inverse) + (tint.r() as f32 * amount)).round() as u8,
            ((base.g() as f32 * inverse) + (tint.g() as f32 * amount)).round() as u8,
            ((base.b() as f32 * inverse) + (tint.b() as f32 * amount)).round() as u8,
            base.a().max(244),
        )
    };
    let network_card_frame =
        |ui: &egui::Ui, tint: egui::Color32, accent: egui::Color32, selected: bool| {
            let fill_mix = if selected {
                if ui.visuals().dark_mode { 0.26 } else { 0.16 }
            } else if ui.visuals().dark_mode {
                0.18
            } else {
                0.10
            };
            let stroke_mix = if selected {
                if ui.visuals().dark_mode { 0.80 } else { 0.55 }
            } else if ui.visuals().dark_mode {
                0.55
            } else {
                0.35
            };
            let stroke_width = if selected { 1.6 } else { 1.0 };
            egui::Frame::group(ui.style())
                .fill(blend_color(ui.visuals().panel_fill, tint, fill_mix))
                .stroke(egui::Stroke::new(
                    stroke_width,
                    blend_color(
                        ui.visuals().widgets.noninteractive.bg_stroke.color,
                        accent,
                        stroke_mix,
                    ),
                ))
                .inner_margin(egui::Margin::same(8.0))
        };

    let local_connection_info = {
        let mut parts = vec![
            format!("Name: {}", snapshot.device_name),
            format!("Device ID: {}", snapshot.device_id),
        ];
        if let Some(port) = snapshot.listener_port {
            parts.push(format!("Listener port: {port}"));
        } else {
            parts.push("Listener: client only".to_string());
        }
        parts.push(format!(
            "Visibility: {}",
            if snapshot.available_for_connectivity {
                "available"
            } else {
                "hidden"
            }
        ));
        if !snapshot.local_presence.active_tab.trim().is_empty() {
            parts.push(format!(
                "Active tab: {}",
                snapshot.local_presence.active_tab
            ));
        }
        parts.join(" | ")
    };
    if snapshot
        .connected_peers
        .iter()
        .all(|peer| peer.connection_id != app.networking_handoff_target)
    {
        app.networking_handoff_target = snapshot
            .connected_peers
            .first()
            .map(|peer| peer.connection_id.clone())
            .unwrap_or_default();
    }

    let filter_text = app.networking_filter.trim().to_lowercase();
    let matches_filter = |name: &str, device_id: &str, address: &str, group: Option<String>| {
        if filter_text.is_empty() {
            return true;
        }
        let haystack = format!(
            "{} {} {} {}",
            name.to_lowercase(),
            device_id.to_lowercase(),
            address.to_lowercase(),
            group.unwrap_or_default().to_lowercase(),
        );
        haystack.contains(&filter_text)
    };
    let connected_visible = snapshot
        .connected_peers
        .iter()
        .filter(|peer| {
            matches_filter(
                &app.network_display_name(&peer.device_id, &peer.device_name),
                &peer.device_id,
                &peer.address,
                app.network_group_label(&peer.device_id),
            )
        })
        .collect::<Vec<_>>();
    let available_visible = snapshot
        .discovered_peers
        .iter()
        .filter(|peer| {
            peer.connected_connection_id.is_none()
                && matches_filter(
                    &app.network_display_name(&peer.device_id, &peer.device_name),
                    &peer.device_id,
                    &format!("{}:{}", peer.address, peer.host_port),
                    app.network_group_label(&peer.device_id),
                )
        })
        .collect::<Vec<_>>();
    let blocked_visible = snapshot
        .blocked_peers
        .iter()
        .filter(|peer| {
            matches_filter(
                &app.network_display_name(&peer.device_id, &peer.device_name),
                &peer.device_id,
                &peer.address,
                app.network_group_label(&peer.device_id),
            )
        })
        .collect::<Vec<_>>();
    let trusted_visible = snapshot
        .trusted_peers
        .iter()
        .filter(|peer| {
            matches_filter(
                &app.network_display_name(&peer.device_id, &peer.device_name),
                &peer.device_id,
                &peer.address,
                app.network_group_label(&peer.device_id),
            )
        })
        .collect::<Vec<_>>();
    let shared_room_connection_ids = snapshot
        .connected_peers
        .iter()
        .map(|peer| peer.connection_id.clone())
        .collect::<Vec<_>>();
    let delivery_visible = snapshot
        .outgoing_artifacts
        .iter()
        .filter(|artifact| {
            artifact.kind != "shared_chat_policy_json"
                && artifact.kind != "shared_chat_message_json"
        })
        .collect::<Vec<_>>();
    let received_transfer_visible = snapshot
        .received_artifacts
        .iter()
        .filter(|artifact| {
            artifact.kind != "shared_chat_policy_json"
                && artifact.kind != "shared_chat_message_json"
        })
        .collect::<Vec<_>>();
    let section_heading = |ui: &mut egui::Ui, icon: &str, color: egui::Color32, title: &str| {
        ui.horizontal_wrapped(|ui| {
            ui.label(egui::RichText::new(icon).color(color).strong());
            ui.heading(title);
        });
    };

    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.heading("Networking");
            ui.separator();
            ui.label(
                "Connect nearby ChattyCog instances over the local Wi-Fi / LAN. Turn one instance on as the host, then scan and connect from the others.",
            );
            render_networking_quick_help(ui, app, &snapshot);

            ui.add_space(8.0);
            let controls_highlight = highlight_active(NetworkingFocusSection::Controls);
            let controls =
                render_networking_controls_section(ui, app, &snapshot, controls_highlight);
            if pending_focus == Some(NetworkingFocusSection::Controls) {
                controls.scroll_to_me(Some(egui::Align::Center));
            }

            if !snapshot.pending_requests.is_empty() {
                ui.add_space(8.0);
                let pending_highlight = highlight_active(NetworkingFocusSection::PendingRequests);
                let pending = render_networking_pending_requests_section(
                    ui,
                    app,
                    &snapshot,
                    pending_highlight,
                );
                if pending_focus == Some(NetworkingFocusSection::PendingRequests) {
                    pending.scroll_to_me(Some(egui::Align::Center));
                }
            }

            ui.add_space(8.0);
            render_networking_this_device_section(ui, app, &snapshot, &local_connection_info);

            ui.add_space(12.0);
            let device_list_highlight = highlight_active(NetworkingFocusSection::DeviceList);
            let device_list = render_networking_peer_actions_section(
                ui,
                app,
                &connected_visible,
                &available_visible,
                &blocked_visible,
                device_list_highlight,
            );
            if pending_focus == Some(NetworkingFocusSection::DeviceList) {
                device_list.scroll_to_me(Some(egui::Align::Center));
            }

            ui.add_space(8.0);
            let selected_connected_count = connected_visible
                .iter()
                .filter(|peer| {
                    let key = if peer.device_id.trim().is_empty() {
                        peer.connection_id.clone()
                    } else {
                        peer.device_id.clone()
                    };
                    app.networking_selected_devices.contains(&key)
                })
                .count();
            let selected_available_count = available_visible
                .iter()
                .filter(|peer| app.networking_selected_devices.contains(&peer.device_id))
                .count();
            let selected_blocked_count = blocked_visible
                .iter()
                .filter(|peer| app.networking_selected_devices.contains(&peer.device_id))
                .count();
            let render_selection_chip =
                |ui: &mut egui::Ui, label: &str, count: usize, tint: egui::Color32| {
                    let fill = if count > 0 {
                        blend_color(ui.visuals().panel_fill, tint, 0.18)
                    } else {
                        blend_color(
                            ui.visuals().panel_fill,
                            ui.visuals().widgets.noninteractive.bg_stroke.color,
                            0.06,
                        )
                    };
                    let stroke = if count > 0 {
                        blend_color(
                            ui.visuals().widgets.noninteractive.bg_stroke.color,
                            tint,
                            0.55,
                        )
                    } else {
                        ui.visuals().widgets.noninteractive.bg_stroke.color
                    };
                    egui::Frame::group(ui.style())
                        .fill(fill)
                        .stroke(egui::Stroke::new(1.0, stroke))
                        .inner_margin(egui::Margin::symmetric(8.0, 4.0))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.small(egui::RichText::new(label).strong());
                                ui.small(count.to_string());
                            });
                        });
                };
            ui.horizontal_wrapped(|ui| {
                render_selection_chip(
                    ui,
                    "Selected",
                    app.networking_selected_devices.len(),
                    egui::Color32::from_rgb(70, 110, 180),
                );
                render_selection_chip(
                    ui,
                    "Connected",
                    selected_connected_count,
                    egui::Color32::from_rgb(70, 110, 180),
                );
                render_selection_chip(
                    ui,
                    "Available",
                    selected_available_count,
                    egui::Color32::from_rgb(70, 140, 90),
                );
                render_selection_chip(
                    ui,
                    "Blocked",
                    selected_blocked_count,
                    egui::Color32::from_rgb(160, 60, 60),
                );
                if !app.networking_selected_devices.is_empty() {
                    if ui.small_button("Clear selection").clicked() {
                        app.networking_selected_devices.clear();
                    }
                    ui.small("Bulk actions apply to the checked devices.");
                }
            });
            ui.add_space(6.0);
            ui.columns(2, |cols| {
                section_heading(
                    &mut cols[0],
                    "[AVL]",
                    egui::Color32::from_rgb(70, 140, 90),
                    &format!("Available ({})", available_visible.len()),
                );
                cols[0].label("Visible on the network but not currently connected.");
                cols[0].add_space(6.0);

                if available_visible.is_empty() {
                    cols[0].label("(none found yet)");
                } else {
                    egui::ScrollArea::vertical()
                        .id_salt("network_discovered_scroll")
                        .max_height(360.0)
                        .show(&mut cols[0], |ui| {
                            for peer in &available_visible {
                                let key = peer.device_id.clone();
                                let selected_initial =
                                    app.networking_selected_devices.contains(&key);
                                let card = network_card_frame(
                                    ui,
                                    egui::Color32::from_rgb(223, 241, 228),
                                    egui::Color32::from_rgb(70, 140, 90),
                                    selected_initial,
                                )
                                .show(ui, |ui| {
                                    let display_name =
                                        app.network_display_name(&peer.device_id, &peer.device_name);
                                    let can_persist_identity = !peer.device_id.trim().is_empty();
                                    let is_trusted = app.network_is_trusted(&peer.device_id);
                                    let alias_editing = app.networking_alias_edit_device.as_deref()
                                        == Some(peer.device_id.as_str());
                                    let group_editing = app.networking_group_edit_device.as_deref()
                                        == Some(peer.device_id.as_str());

                                    ui.horizontal_wrapped(|ui| {
                                        let mut selected =
                                            app.networking_selected_devices.contains(&key);
                                        if ui.checkbox(&mut selected, "").changed() {
                                            if selected {
                                                app.networking_selected_devices.insert(key.clone());
                                            } else {
                                                app.networking_selected_devices.remove(&key);
                                            }
                                        }
                                        if can_persist_identity {
                                            if ui
                                                .link(
                                                    egui::RichText::new(display_name.clone())
                                                        .strong(),
                                                )
                                                .clicked()
                                            {
                                                app.begin_network_alias_edit(
                                                    &peer.device_id,
                                                    &peer.device_name,
                                                );
                                            }
                                        } else {
                                            ui.strong(display_name.clone());
                                        }
                                        if is_trusted {
                                            ui.small(
                                                egui::RichText::new("Trusted")
                                                    .color(egui::Color32::from_rgb(110, 80, 170))
                                                    .strong(),
                                            );
                                        }
                                        ui.small(format!("{}:{}", peer.address, peer.host_port));
                                        if let Some(group) =
                                            app.network_group_label(&peer.device_id)
                                        {
                                            if ui.small_button(format!("Group: {group}")).clicked() {
                                                app.begin_network_group_edit(&peer.device_id);
                                            }
                                        } else if can_persist_identity
                                            && ui.small_button("+ Group").clicked()
                                        {
                                            app.begin_network_group_edit(&peer.device_id);
                                        }
                                    });
                                    if alias_editing {
                                        ui.horizontal_wrapped(|ui| {
                                            ui.label("Rename:");
                                            ui.add(
                                                egui::TextEdit::singleline(
                                                    &mut app.networking_alias_input,
                                                )
                                                .desired_width(220.0)
                                                .hint_text("Office PC West"),
                                            );
                                            if ui.button("Save").clicked() {
                                                app.save_network_alias_edit(
                                                    &peer.device_id,
                                                    &peer.device_name,
                                                );
                                            }
                                            if ui.button("Cancel").clicked() {
                                                app.cancel_network_alias_edit();
                                            }
                                        });
                                    }
                                    if group_editing {
                                        ui.horizontal_wrapped(|ui| {
                                            ui.label("Group:");
                                            ui.add(
                                                egui::TextEdit::singleline(
                                                    &mut app.networking_group_input,
                                                )
                                                .desired_width(180.0)
                                                .hint_text("e.g. Research bench"),
                                            );
                                            if ui.button("Save group").clicked() {
                                                app.save_network_group_edit(
                                                    &peer.device_id,
                                                    &peer.device_name,
                                                );
                                            }
                                            if ui.button("Clear").clicked() {
                                                app.networking_group_input.clear();
                                                app.save_network_group_edit(
                                                    &peer.device_id,
                                                    &peer.device_name,
                                                );
                                            }
                                            if ui.button("Cancel").clicked() {
                                                app.cancel_network_group_edit();
                                            }
                                        });
                                    }
                                    let mut status_parts =
                                        vec![format!("Seen {}s ago", peer.last_seen_secs_ago)];
                                    if is_trusted {
                                        status_parts.push("Trusted".to_string());
                                    }
                                    if let Some(group) = app.network_group_label(&peer.device_id) {
                                        status_parts.push(format!("Group: {group}"));
                                    }
                                    ui.small(status_parts.join(" | "));
                                    ui.horizontal_wrapped(|ui| {
                                        if ui.button("Connect").clicked() {
                                            app.networking.connect_peer(&peer.device_id);
                                        }
                                        if can_persist_identity {
                                            if is_trusted {
                                                if ui.button("Untrust").clicked() {
                                                    app.untrust_network_peer(
                                                        &peer.device_id,
                                                        &peer.device_name,
                                                    );
                                                }
                                            } else if ui.button("Trust").clicked() {
                                                app.trust_network_peer(
                                                    &peer.device_id,
                                                    &peer.device_name,
                                                );
                                            }
                                        }
                                        if ui.button("Block").clicked() {
                                            app.block_network_peer(&peer.device_id, &peer.device_name);
                                        }
                                        if ui.small_button("Copy ID").clicked() {
                                            ui.ctx().copy_text(peer.device_id.clone());
                                            app.networking_status = format!(
                                                "Copied device ID for {}.",
                                                display_name
                                            );
                                        }
                                        if ui.small_button("Copy info").clicked() {
                                            ui.ctx().copy_text(format!(
                                                "Name: {} | Device ID: {} | Address: {}:{} | Seen: {}s ago",
                                                display_name,
                                                peer.device_id,
                                                peer.address,
                                                peer.host_port,
                                                peer.last_seen_secs_ago
                                            ));
                                            app.networking_status = format!(
                                                "Copied connection info for {}.",
                                                display_name
                                            );
                                        }
                                    });
                                });
                                if card.response.hovered() {
                                    ui.painter().rect_stroke(
                                        card.response.rect.expand(1.0),
                                        6.0,
                                        egui::Stroke::new(
                                            if selected_initial { 1.9 } else { 1.35 },
                                            blend_color(
                                                ui.visuals().widgets.noninteractive.bg_stroke.color,
                                                egui::Color32::from_rgb(70, 140, 90),
                                                if selected_initial { 0.78 } else { 0.60 },
                                            ),
                                        ),
                                    );
                                }
                                ui.add_space(6.0);
                            }
                        });
                }

                section_heading(
                    &mut cols[1],
                    "[CON]",
                    egui::Color32::from_rgb(70, 110, 180),
                    &format!("Connected ({})", connected_visible.len()),
                );
                cols[1].label("Live TCP links between ChattyCog instances.");
                cols[1].add_space(6.0);

                if connected_visible.is_empty() {
                    cols[1].label("(no active connections)");
                } else {
                    egui::ScrollArea::vertical()
                        .id_salt("network_connected_scroll")
                        .max_height(360.0)
                        .show(&mut cols[1], |ui| {
                            for peer in &connected_visible {
                                let key = if peer.device_id.trim().is_empty() {
                                    peer.connection_id.clone()
                                } else {
                                    peer.device_id.clone()
                                };
                                let selected_initial =
                                    app.networking_selected_devices.contains(&key);
                                let card = network_card_frame(
                                    ui,
                                    egui::Color32::from_rgb(224, 234, 250),
                                    egui::Color32::from_rgb(70, 110, 180),
                                    selected_initial,
                                )
                                .show(ui, |ui| {
                                    let is_trusted = app.network_is_trusted(&peer.device_id);
                                    ui.horizontal_wrapped(|ui| {
                                        let mut selected =
                                            app.networking_selected_devices.contains(&key);
                                        if ui.checkbox(&mut selected, "").changed() {
                                            if selected {
                                                app.networking_selected_devices.insert(key.clone());
                                            } else {
                                                app.networking_selected_devices.remove(&key);
                                            }
                                        }
                                        let display_name = app
                                            .network_display_name(&peer.device_id, &peer.device_name);
                                        let can_persist_identity = !peer.device_id.trim().is_empty();
                                        if can_persist_identity {
                                            if ui
                                                .link(
                                                    egui::RichText::new(display_name.clone())
                                                        .strong(),
                                                )
                                                .clicked()
                                            {
                                                app.begin_network_alias_edit(
                                                    &peer.device_id,
                                                    &peer.device_name,
                                                );
                                            }
                                        } else {
                                            ui.strong(display_name.clone());
                                        }
                                        if is_trusted {
                                            ui.small(
                                                egui::RichText::new("Trusted")
                                                    .color(egui::Color32::from_rgb(110, 80, 170))
                                                    .strong(),
                                            );
                                        }
                                        ui.small(if peer.inbound { "Inbound" } else { "Outbound" });
                                        if let Some(group) =
                                            app.network_group_label(&peer.device_id)
                                        {
                                            if ui.small_button(format!("Group: {group}")).clicked() {
                                                app.begin_network_group_edit(&peer.device_id);
                                            }
                                        } else if can_persist_identity
                                            && ui.small_button("+ Group").clicked()
                                        {
                                            app.begin_network_group_edit(&peer.device_id);
                                        }
                                    });
                                    let display_name =
                                        app.network_display_name(&peer.device_id, &peer.device_name);
                                    let alias_editing = app.networking_alias_edit_device.as_deref()
                                        == Some(peer.device_id.as_str());
                                    let group_editing = app.networking_group_edit_device.as_deref()
                                        == Some(peer.device_id.as_str());
                                    if alias_editing {
                                        ui.horizontal_wrapped(|ui| {
                                            ui.label("Rename:");
                                            ui.add(
                                                egui::TextEdit::singleline(
                                                    &mut app.networking_alias_input,
                                                )
                                                .desired_width(220.0)
                                                .hint_text("Chatty Station 2"),
                                            );
                                            if ui.button("Save").clicked() {
                                                app.save_network_alias_edit(
                                                    &peer.device_id,
                                                    &peer.device_name,
                                                );
                                            }
                                            if ui.button("Cancel").clicked() {
                                                app.cancel_network_alias_edit();
                                            }
                                        });
                                    }
                                    if group_editing {
                                        ui.horizontal_wrapped(|ui| {
                                            ui.label("Group:");
                                            ui.add(
                                                egui::TextEdit::singleline(
                                                    &mut app.networking_group_input,
                                                )
                                                .desired_width(180.0)
                                                .hint_text("e.g. Writers room"),
                                            );
                                            if ui.button("Save group").clicked() {
                                                app.save_network_group_edit(
                                                    &peer.device_id,
                                                    &peer.device_name,
                                                );
                                            }
                                            if ui.button("Clear").clicked() {
                                                app.networking_group_input.clear();
                                                app.save_network_group_edit(
                                                    &peer.device_id,
                                                    &peer.device_name,
                                                );
                                            }
                                            if ui.button("Cancel").clicked() {
                                                app.cancel_network_group_edit();
                                            }
                                        });
                                    }
                                    ui.label(format!("Address: {}", peer.address));
                                    let mut status_parts = vec![peer.status_summary.clone()];
                                    if is_trusted {
                                        status_parts.push("Trusted".to_string());
                                    }
                                    if let Some(group) = app.network_group_label(&peer.device_id) {
                                        status_parts.push(format!("Group: {group}"));
                                    }
                                    ui.label(format!("Shared status: {}", status_parts.join(" | ")));
                                    if let Some(age) = peer.status_age_secs {
                                        ui.small(format!("Status updated {}s ago", age));
                                    }
                                    ui.small(format!("Connected for {}s", peer.connected_secs));
                                    ui.horizontal_wrapped(|ui| {
                                        if ui.button("Disconnect").clicked() {
                                            app.networking.disconnect_connection(&peer.connection_id);
                                        }
                                        if !peer.device_id.trim().is_empty() {
                                            if is_trusted {
                                                if ui.button("Untrust").clicked() {
                                                    app.untrust_network_peer(
                                                        &peer.device_id,
                                                        &peer.device_name,
                                                    );
                                                }
                                            } else if ui.button("Trust").clicked() {
                                                app.trust_network_peer(
                                                    &peer.device_id,
                                                    &peer.device_name,
                                                );
                                            }
                                        }
                                        if ui.button("Block").clicked()
                                            && !peer.device_id.trim().is_empty()
                                        {
                                            app.block_network_peer(&peer.device_id, &peer.device_name);
                                        }
                                        if !peer.device_id.trim().is_empty()
                                            && ui.small_button("Copy ID").clicked()
                                        {
                                            ui.ctx().copy_text(peer.device_id.clone());
                                            app.networking_status = format!(
                                                "Copied device ID for {}.",
                                                display_name
                                            );
                                        }
                                        if ui.small_button("Copy info").clicked() {
                                            ui.ctx().copy_text(format!(
                                                "Name: {} | Device ID: {} | Address: {} | Direction: {} | Connected: {}s",
                                                display_name,
                                                peer.device_id,
                                                peer.address,
                                                if peer.inbound {
                                                    "inbound"
                                                } else {
                                                    "outbound"
                                                },
                                                peer.connected_secs
                                            ));
                                            app.networking_status = format!(
                                                "Copied connection info for {}.",
                                                display_name
                                            );
                                        }
                                    });
                                });
                                if card.response.hovered() {
                                    ui.painter().rect_stroke(
                                        card.response.rect.expand(1.0),
                                        6.0,
                                        egui::Stroke::new(
                                            if selected_initial { 1.9 } else { 1.35 },
                                            blend_color(
                                                ui.visuals().widgets.noninteractive.bg_stroke.color,
                                                egui::Color32::from_rgb(70, 110, 180),
                                                if selected_initial { 0.78 } else { 0.60 },
                                            ),
                                        ),
                                    );
                                }
                                ui.add_space(6.0);
                            }
                        });
                }
            });

            ui.add_space(8.0);
            egui::CollapsingHeader::new(
                egui::RichText::new(format!("[TRU] Trusted ({})", trusted_visible.len()))
                    .color(egui::Color32::from_rgb(110, 80, 170))
                    .strong(),
            )
            .id_salt("network_trusted_section")
            .default_open(false)
            .show(ui, |ui| {
                egui::ScrollArea::vertical()
                    .id_salt("network_trusted_scroll")
                    .max_height(220.0)
                    .show(ui, |ui| {
                        if trusted_visible.is_empty() {
                            ui.label("(none)");
                        } else {
                            for peer in &trusted_visible {
                                let display_name =
                                    app.network_display_name(&peer.device_id, &peer.device_name);
                                let card = network_card_frame(
                                    ui,
                                    egui::Color32::from_rgb(235, 230, 247),
                                    egui::Color32::from_rgb(110, 80, 170),
                                    false,
                                )
                                .show(ui, |ui| {
                                    ui.horizontal_wrapped(|ui| {
                                        ui.label(
                                            egui::RichText::new(display_name.clone()).strong(),
                                        );
                                        ui.small(
                                            egui::RichText::new("Trusted")
                                                .color(egui::Color32::from_rgb(110, 80, 170))
                                                .strong(),
                                        );
                                        if let Some(group) =
                                            app.network_group_label(&peer.device_id)
                                        {
                                            ui.small(format!("Group: {group}"));
                                        }
                                    });
                                    let mut detail_parts = Vec::new();
                                    if !peer.address.trim().is_empty() {
                                        detail_parts.push(format!("Address: {}", peer.address));
                                    }
                                    if let Some(age) = peer.last_seen_secs_ago {
                                        detail_parts.push(format!("Last seen {}s ago", age));
                                    } else {
                                        detail_parts.push("Not seen recently".to_string());
                                    }
                                    ui.small(detail_parts.join(" | "));
                                    ui.horizontal_wrapped(|ui| {
                                        if ui.button("Untrust").clicked() {
                                            app.untrust_network_peer(
                                                &peer.device_id,
                                                &peer.device_name,
                                            );
                                        }
                                        if ui.small_button("Copy ID").clicked() {
                                            ui.ctx().copy_text(peer.device_id.clone());
                                            app.networking_status = format!(
                                                "Copied device ID for {}.",
                                                display_name
                                            );
                                        }
                                        if ui.small_button("Copy info").clicked() {
                                            ui.ctx().copy_text(format!(
                                                "Name: {} | Device ID: {} | Address: {} | State: trusted",
                                                display_name, peer.device_id, peer.address
                                            ));
                                            app.networking_status = format!(
                                                "Copied connection info for {}.",
                                                display_name
                                            );
                                        }
                                    });
                                });
                                if card.response.hovered() {
                                    ui.painter().rect_stroke(
                                        card.response.rect.expand(1.0),
                                        6.0,
                                        egui::Stroke::new(
                                            1.35,
                                            blend_color(
                                                ui.visuals().widgets.noninteractive.bg_stroke.color,
                                                egui::Color32::from_rgb(110, 80, 170),
                                                0.60,
                                            ),
                                        ),
                                    );
                                }
                                ui.add_space(6.0);
                            }
                        }
                    });
            });

            ui.add_space(8.0);
            egui::CollapsingHeader::new(
                egui::RichText::new(format!("[BLK] Blocked ({})", blocked_visible.len()))
                    .color(egui::Color32::from_rgb(160, 60, 60))
                    .strong(),
            )
                .id_salt("network_blocked_section")
                .default_open(false)
                .show(ui, |ui| {
                    egui::ScrollArea::vertical()
                        .id_salt("network_blocked_scroll")
                        .max_height(220.0)
                        .show(ui, |ui| {
                            if blocked_visible.is_empty() {
                                ui.label("(none)");
                            } else {
                                for peer in &blocked_visible {
                                    let key = peer.device_id.clone();
                                    let selected_initial =
                                        app.networking_selected_devices.contains(&key);
                                    let card = network_card_frame(
                                        ui,
                                        egui::Color32::from_rgb(248, 228, 228),
                                        egui::Color32::from_rgb(160, 60, 60),
                                        selected_initial,
                                    )
                                    .show(ui, |ui| {
                                        let display_name =
                                            app.network_display_name(&peer.device_id, &peer.device_name);
                                        let alias_editing = app.networking_alias_edit_device.as_deref()
                                            == Some(peer.device_id.as_str());
                                        let group_editing = app.networking_group_edit_device.as_deref()
                                            == Some(peer.device_id.as_str());

                                        ui.horizontal_wrapped(|ui| {
                                            let mut selected =
                                                app.networking_selected_devices.contains(&key);
                                            if ui.checkbox(&mut selected, "").changed() {
                                                if selected {
                                                    app.networking_selected_devices.insert(key.clone());
                                                } else {
                                                    app.networking_selected_devices.remove(&key);
                                                }
                                            }
                                            if ui
                                                .link(
                                                    egui::RichText::new(display_name.clone())
                                                        .strong(),
                                                )
                                                .clicked()
                                            {
                                                app.begin_network_alias_edit(
                                                    &peer.device_id,
                                                    &peer.device_name,
                                                );
                                            }
                                            if let Some(group) =
                                                app.network_group_label(&peer.device_id)
                                            {
                                                if ui.small_button(format!("Group: {group}")).clicked() {
                                                    app.begin_network_group_edit(&peer.device_id);
                                                }
                                            } else if ui.small_button("+ Group").clicked() {
                                                app.begin_network_group_edit(&peer.device_id);
                                            }
                                        });
                                        if alias_editing {
                                            ui.horizontal_wrapped(|ui| {
                                                ui.label("Rename:");
                                                ui.add(
                                                    egui::TextEdit::singleline(
                                                        &mut app.networking_alias_input,
                                                    )
                                                    .desired_width(220.0)
                                                    .hint_text("Archive laptop"),
                                                );
                                                if ui.button("Save").clicked() {
                                                    app.save_network_alias_edit(
                                                        &peer.device_id,
                                                        &peer.device_name,
                                                    );
                                                }
                                                if ui.button("Cancel").clicked() {
                                                    app.cancel_network_alias_edit();
                                                }
                                            });
                                        }
                                        if group_editing {
                                            ui.horizontal_wrapped(|ui| {
                                                ui.label("Group:");
                                                ui.add(
                                                    egui::TextEdit::singleline(
                                                        &mut app.networking_group_input,
                                                    )
                                                    .desired_width(180.0)
                                                    .hint_text("e.g. Spare pool"),
                                                );
                                                if ui.button("Save group").clicked() {
                                                    app.save_network_group_edit(
                                                        &peer.device_id,
                                                        &peer.device_name,
                                                    );
                                                }
                                                if ui.button("Clear").clicked() {
                                                    app.networking_group_input.clear();
                                                    app.save_network_group_edit(
                                                        &peer.device_id,
                                                        &peer.device_name,
                                                    );
                                                }
                                                if ui.button("Cancel").clicked() {
                                                    app.cancel_network_group_edit();
                                                }
                                            });
                                        }
                                        if let Some(age) = peer.last_seen_secs_ago {
                                            if let Some(group) = app.network_group_label(&peer.device_id) {
                                                ui.small(format!("Blocked | Group: {} | Seen {}s ago", group, age));
                                            } else {
                                                ui.small(format!("Blocked | Seen {}s ago", age));
                                            }
                                        } else {
                                            if let Some(group) = app.network_group_label(&peer.device_id) {
                                                ui.small(format!("Blocked | Group: {group}"));
                                            } else {
                                                ui.small("Blocked");
                                            }
                                        }
                                        if !peer.address.trim().is_empty() {
                                            ui.small(format!("Address: {}", peer.address));
                                        }
                                        ui.horizontal_wrapped(|ui| {
                                            if ui.button("Unblock").clicked() {
                                                app.unblock_network_peer(
                                                    &peer.device_id,
                                                    &peer.device_name,
                                                );
                                            }
                                            if ui.small_button("Copy ID").clicked() {
                                                ui.ctx().copy_text(peer.device_id.clone());
                                                app.networking_status =
                                                    format!("Copied device ID for {}.", display_name);
                                            }
                                            if ui.small_button("Copy info").clicked() {
                                                ui.ctx().copy_text(format!(
                                                    "Name: {} | Device ID: {} | Address: {} | State: blocked",
                                                    display_name, peer.device_id, peer.address
                                                ));
                                                app.networking_status = format!(
                                                    "Copied connection info for {}.",
                                                    display_name
                                                );
                                            }
                                        });
                                    });
                                    if card.response.hovered() {
                                        ui.painter().rect_stroke(
                                            card.response.rect.expand(1.0),
                                            6.0,
                                            egui::Stroke::new(
                                                if selected_initial { 1.9 } else { 1.35 },
                                                blend_color(
                                                    ui.visuals()
                                                        .widgets
                                                        .noninteractive
                                                        .bg_stroke
                                                        .color,
                                                    egui::Color32::from_rgb(160, 60, 60),
                                                    if selected_initial { 0.78 } else { 0.60 },
                                                ),
                                            ),
                                        );
                                    }
                                    ui.add_space(6.0);
                                }
                            }
                        });
                });

            ui.add_space(12.0);
            ui.separator();
            let selected_connections = app.selected_network_connection_ids();
            render_networking_workflow_bundle_section(ui, app, &selected_connections);

            ui.add_space(12.0);
            ui.separator();
            render_networking_lukewarm_share_section(ui, app, &selected_connections);

            ui.add_space(12.0);
            ui.separator();
            let shared_room_highlight = highlight_active(NetworkingFocusSection::SharedRoom);
            let shared_room = render_networking_shared_room_section(
                ui,
                app,
                &snapshot,
                &shared_room_connection_ids,
                shared_room_highlight,
            );
            if pending_focus == Some(NetworkingFocusSection::SharedRoom) {
                shared_room.scroll_to_me(Some(egui::Align::Center));
            }

            ui.add_space(12.0);
            ui.separator();
            render_networking_recent_events_section(ui, app, &snapshot);

            ui.add_space(12.0);
            ui.separator();
            render_networking_delivery_status_section(ui, &delivery_visible);

            ui.add_space(12.0);
            ui.separator();
            render_networking_handoff_section(ui, app, &snapshot);

            ui.add_space(12.0);
            ui.separator();
            render_networking_received_handoffs(ui, app, &snapshot);

            ui.add_space(12.0);
            ui.separator();
            render_networking_received_transfers(
                ui,
                app,
                &snapshot,
                &received_transfer_visible,
            );

            ui.add_space(12.0);
            ui.separator();
            section_heading(
                ui,
                "[FIL]",
                egui::Color32::from_rgb(110, 130, 80),
                "Received file-style transfers",
            );
            app.render_received_generic_transfer_inbox(ui, "Received file transfers");

            ui.add_space(12.0);
            ui.separator();
            section_heading(
                ui,
                "[WRK]",
                egui::Color32::from_rgb(70, 140, 90),
                "Workflow inbox",
            );
            app.render_received_workflow_inbox(ui, "Received workflows", None);

            ui.add_space(12.0);
            ui.separator();
            section_heading(
                ui,
                "[SET]",
                egui::Color32::from_rgb(90, 110, 170),
                "Received setup bundles",
            );
            app.render_received_workflow_bundle_inbox(ui, "Received workflow bundles");
        });
}
