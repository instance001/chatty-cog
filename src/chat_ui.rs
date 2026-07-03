use super::*;

pub(super) fn left_sidebar_chat(ui: &mut egui::Ui, app: &mut ChattyCogApp) {
    ui.heading("Chat");
    ui.separator();
    ui.label("Hot memory and luke-warm context now live inside the main chat layout.");
    ui.small("Model selection and orchestrator settings live in the Models tab.");
    ui.add_space(8.0);
    if false && app.hot_memory.is_empty() {
        ui.label("(empty)");
    } else if false {
        ui.group(|ui| {
            for item in &app.hot_memory {
                ui.label(format!("* {item}"));
            }
        });
    }
    ui.horizontal(|ui| {
        if ui.button("Clear").clicked() {
            app.hot_memory.clear();
        }
        if ui.button("Pin current").clicked() {
            let last_user = app
                .messages
                .iter()
                .rev()
                .find(|m| m.role == Role::User)
                .map(|m| m.content.clone());
            if let Some(t) = last_user {
                push_hot_memory(app, format!("User intent: {}", one_line(&t, 120)));
            }
        }
    });

    ui.add_space(8.0);
    ui.small("Use the Models tab for GGUF selection, presets, and orchestrator tuning.");

    ui.separator();
    ui.add_enabled_ui(app.is_generating, |ui| {
        if ui.button("Stop current response").clicked() {
            app.stop_generation();
        }
    });

    ui.separator();
    if ui.button("Clear chat transcript").clicked() {
        app.pulse_ecg(18.0, "Cleared the chat transcript.");
        app.messages.retain(|m| m.role == Role::System);
        app.assistant_draft.clear();
        if let Some(bk) = &app.bookkeeper {
            bk.append(MemoryEvent {
                ts_unix_ms: now_unix_ms(),
                kind: MemoryKind::Cold,
                category: EventCategory::Module,
                source: "ui".to_string(),
                module: Some("chat".to_string()),
                event_type: Some("clear".to_string()),
                text: "Cleared chat".to_string(),
                tags: Vec::new(),
                entities: Vec::new(),
                payload_json: None,
            });
        }
    }
}

pub(super) fn render_chat_hot_memory_panel(
    ui: &mut egui::Ui,
    app: &mut ChattyCogApp,
    panel_height: f32,
) {
    egui::Frame::group(ui.style()).show(ui, |ui| {
        ui.set_min_height(panel_height);
        ui.set_max_width(ui.available_width());
        ui.heading("Hot Memory");
        ui.add(
            egui::Label::new("Recent working cues that stay visible while the conversation moves.")
                .wrap(),
        );
        ui.add_space(8.0);

        egui::ScrollArea::vertical()
            .id_salt("chat_hot_memory_scroll")
            .max_height((panel_height - 92.0).max(120.0))
            .auto_shrink([true, false])
            .show(ui, |ui| {
                if app.hot_memory.is_empty() {
                    ui.label("(empty)");
                } else {
                    for item in app.hot_memory.iter().rev() {
                        egui::Frame::none()
                            .fill(ui.visuals().faint_bg_color)
                            .rounding(egui::Rounding::same(6.0))
                            .inner_margin(egui::Margin::symmetric(8.0, 6.0))
                            .show(ui, |ui| {
                                ui.add(egui::Label::new(item.as_str()).wrap());
                            });
                        ui.add_space(6.0);
                    }
                }
            });

        ui.separator();
        ui.horizontal_wrapped(|ui| {
            if ui.button("Clear").clicked() {
                app.hot_memory.clear();
            }
            if ui.button("Pin current").clicked() {
                let last_user = app
                    .messages
                    .iter()
                    .rev()
                    .find(|m| m.role == Role::User)
                    .map(|m| m.content.clone());
                if let Some(t) = last_user {
                    push_hot_memory(app, format!("User intent: {}", one_line(&t, 160)));
                }
            }
        });
    });
}

pub(super) fn render_chat_lukewarm_panel(
    ui: &mut egui::Ui,
    app: &mut ChattyCogApp,
    panel_height: f32,
) {
    egui::Frame::group(ui.style()).show(ui, |ui| {
        ui.set_min_height(panel_height);
        ui.set_max_width(ui.available_width());
        ui.heading("Luke Warm");
        ui.add(
            egui::Label::new(
                "Rolling summary from the bookkeeper so longer sessions stay grounded.",
            )
            .wrap(),
        );
        ui.add_space(8.0);

        let text = if app.lukewarm_summary.trim().is_empty() {
            "(no summary yet)".to_string()
        } else {
            app.lukewarm_summary.clone()
        };

        egui::ScrollArea::vertical()
            .id_salt("chat_lukewarm_scroll")
            .max_height((panel_height - 64.0).max(120.0))
            .auto_shrink([true, false])
            .show(ui, |ui| {
                egui::Frame::none()
                    .fill(ui.visuals().extreme_bg_color)
                    .inner_margin(egui::Margin::symmetric(8.0, 6.0))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.add(egui::Label::new(text.as_str()).wrap());
                    });
            });
    });
}

pub(super) fn render_chat_ecg_window(ui: &mut egui::Ui, app: &ChattyCogApp) {
    let payload = app.ecg_window.payload();
    let desired_size = egui::vec2(208.0, 60.0);
    let (rect, response) = ui.allocate_exact_size(desired_size, egui::Sense::hover());

    let surface = egui::Color32::from_rgb(238, 244, 248);
    let chart_surface = egui::Color32::from_rgb(247, 250, 252);
    let border = egui::Color32::from_rgb(152, 168, 182);
    let muted = ui.visuals().weak_text_color();
    let accent = if payload.current_percent >= 55.0 {
        egui::Color32::from_rgb(42, 146, 92)
    } else if payload.current_percent >= 20.0 {
        egui::Color32::from_rgb(88, 123, 168)
    } else {
        muted.gamma_multiply(0.9)
    };
    let state = if !payload.supported {
        "unsupported"
    } else if payload.available {
        "live"
    } else {
        "waiting"
    };

    ui.painter().rect(
        rect,
        egui::Rounding::same(8.0),
        surface,
        egui::Stroke::new(1.0, border),
    );

    let inner = rect.shrink2(egui::vec2(10.0, 8.0));
    let small_font = egui::TextStyle::Small.resolve(ui.style());
    let body_font = egui::TextStyle::Body.resolve(ui.style());
    let heading_font = egui::TextStyle::Button.resolve(ui.style());

    ui.painter().text(
        inner.left_top(),
        egui::Align2::LEFT_TOP,
        "ECG",
        heading_font,
        egui::Color32::from_rgb(74, 92, 112),
    );
    ui.painter().text(
        egui::pos2(inner.min.x + 34.0, inner.min.y + 2.0),
        egui::Align2::LEFT_TOP,
        state,
        small_font,
        muted.gamma_multiply(0.9),
    );
    ui.painter().text(
        inner.right_top(),
        egui::Align2::RIGHT_TOP,
        format!("{:.0}%", payload.current_percent),
        body_font,
        accent,
    );

    let chart_rect = egui::Rect::from_min_max(
        egui::pos2(inner.min.x, inner.min.y + 22.0),
        egui::pos2(inner.max.x, inner.max.y - 10.0),
    );
    ui.painter().rect_filled(
        chart_rect.expand2(egui::vec2(1.0, 2.0)),
        egui::Rounding::same(5.0),
        chart_surface,
    );
    ui.painter().line_segment(
        [
            egui::pos2(chart_rect.left(), chart_rect.bottom()),
            egui::pos2(chart_rect.right(), chart_rect.bottom()),
        ],
        egui::Stroke::new(1.0, border.gamma_multiply(0.6)),
    );

    let points = app
        .ecg_window
        .points(chart_rect.width(), chart_rect.height())
        .into_iter()
        .map(|point| egui::pos2(chart_rect.left() + point.x, chart_rect.top() + point.y))
        .collect::<Vec<_>>();

    if points.len() >= 2 {
        ui.painter()
            .add(egui::Shape::line(points, egui::Stroke::new(1.8, accent)));
    } else if let Some(point) = points.first() {
        ui.painter().circle_filled(*point, 2.0, accent);
    }

    response.on_hover_text(format!(
        "{}\nState: {}\n{}\nCurrent: {:.0}%",
        payload.label, state, payload.note, payload.current_percent
    ));
}

pub(super) fn chat_tab(ui: &mut egui::Ui, ctx: &egui::Context, app: &mut ChattyCogApp) {
    egui::TopBottomPanel::top("chat_status").show_inside(ui, |ui| {
        render_chat_status_panel(ui, app);
    });

    let mut send_now = false;
    egui::TopBottomPanel::bottom("chat_input").show_inside(ui, |ui| {
        render_chat_input_panel(ui, app, &mut send_now);
    });

    egui::CentralPanel::default().show_inside(ui, |ui| render_chat_columns(ui, app));

    app.scroll_to_bottom = false;

    if send_now {
        chat_actions::handle_chat_send(ctx, app);
    }
}

fn render_chat_status_panel(ui: &mut egui::Ui, app: &mut ChattyCogApp) {
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            egui::Frame::none()
                .fill(ui.visuals().faint_bg_color)
                .inner_margin(egui::Margin::symmetric(8.0, 6.0))
                .show(ui, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.small(&app.runtime_status);
                        let (badge, color) = runtime_backend_summary(&app.runtime_status);
                        ui.colored_label(color, format!("[{badge}]"));
                        ui.small("Vulkan may still leave a few tensors on CPU.");
                    });
                });
            ui.add_space(4.0);
            render_chat_model_controls(ui, app);
            ui.add_space(4.0);
            render_chat_voice_summary(ui, app);
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::TOP), |ui| {
            render_chat_ecg_window(ui, app);
        });
    });
    if app.sandbox_dir.is_some() {
        render_sandbox_quick_access(ui, app);
    }
    if !app.sandbox_task_nudge.trim().is_empty() {
        ui.horizontal_wrapped(|ui| {
            ui.small(format!("Task hint: {}", app.sandbox_task_nudge));
            if ui.button("Open task ledger").clicked() {
                app.open_default_sandbox_task_ledger();
            }
        });
    }
    if !app.sandbox_action_status.trim().is_empty() {
        ui.horizontal_wrapped(|ui| {
            ui.small(format!("Sandbox: {}", app.sandbox_action_status));
        });
    }
}

fn render_chat_model_controls(ui: &mut egui::Ui, app: &mut ChattyCogApp) {
    if app.models_cache.is_empty() {
        app.models_cache = scan_ggufs(app.models_dir.as_deref());
    }
    let model_opts = build_model_options(app.models_dir.as_deref(), app.modules_dir.as_deref());
    let selected_hint = app.portable_model_hint(app.gguf_path.as_deref());
    let selected_label = selected_hint
        .as_ref()
        .and_then(|hint| {
            model_opts
                .iter()
                .find(|option| option.value == *hint)
                .map(|option| option.label.clone())
        })
        .or_else(|| {
            app.gguf_path.as_ref().map(|path| {
                path.file_name()
                    .map(|name| name.to_string_lossy().to_string())
                    .unwrap_or_else(|| path.display().to_string())
            })
        })
        .unwrap_or_else(|| "(none)".to_string());

    ui.horizontal_wrapped(|ui| {
        ui.small("Model");
        egui::ComboBox::from_id_salt("chat_model_combo")
            .selected_text(selected_label)
            .width(260.0)
            .show_ui(ui, |ui| {
                if ui
                    .selectable_label(app.gguf_path.is_none(), "(none)")
                    .clicked()
                {
                    app.set_active_chat_model_path(None);
                }
                for option in &model_opts {
                    let selected = selected_hint.as_deref() == Some(option.value.as_str());
                    if ui.selectable_label(selected, &option.label).clicked() {
                        let path = app.resolve_portable_model_hint(Some(&option.value));
                        app.set_active_chat_model_path(path);
                    }
                }
            });
        if ui.button("Open GGUF...").clicked() {
            let mut dialog = rfd::FileDialog::new().add_filter("GGUF", &["gguf"]);
            if let Some(dir) = &app.models_dir {
                dialog = dialog.set_directory(dir);
            }
            if let Some(path) = dialog.pick_file() {
                app.set_active_chat_model_path(Some(path));
            }
        }
        if ui.button("Refresh").clicked() {
            app.models_cache = scan_ggufs(app.models_dir.as_deref());
        }
        ui.small(format!("Max tokens {}", app.orch_max_tokens));
    });
}

fn render_chat_voice_summary(ui: &mut egui::Ui, app: &mut ChattyCogApp) {
    ui.horizontal_wrapped(|ui| {
        if let Some(capsule) = app.active_orchestrator_capsule() {
            let preview = truncate_for_ui(&one_line(&capsule.text, 120), 72);
            ui.small(format!("Voice: {}", capsule.name));
            ui.small(format!("Preview: {preview}"));
            if ui.button("Use native voice").clicked() {
                app.prefs.active_orchestrator_capsule = None;
                app.prefs_status =
                    "Capsule deselected. ChattyCog native voice restored.".to_string();
            }
        } else {
            ui.small("Voice: native ChattyCog");
            ui.small("Capsule: none");
        }
    });
}

fn render_sandbox_quick_access(ui: &mut egui::Ui, app: &mut ChattyCogApp) {
    ui.horizontal_wrapped(|ui| {
        ui.small("Sandbox quick access:");
        if ui.button("Open scratchpad").clicked() {
            if let Some(dir) = app.sandbox_dir.clone() {
                match ensure_default_sandbox_scratchpad_file(&dir) {
                    Ok(path) => app.open_sandbox_file_and_focus_tab(&path),
                    Err(err) => app.sandbox_status = format!("Scratchpad setup failed: {err}"),
                }
            }
        }
        if ui.button("Open ledger").clicked() {
            if let Some(dir) = app.sandbox_dir.clone() {
                match ensure_default_sandbox_task_ledger_file(&dir) {
                    Ok(path) => app.open_sandbox_file_and_focus_tab(&path),
                    Err(err) => app.sandbox_status = format!("Task ledger setup failed: {err}"),
                }
            }
        }
        let last_label = app
            .sandbox_last_working_path
            .as_ref()
            .and_then(|path| path.file_name())
            .map(|name| truncate_for_ui(&name.to_string_lossy(), 40))
            .unwrap_or_else(|| "none yet".to_string());
        let reopen_response = ui.add_enabled(
            app.sandbox_last_working_path.is_some(),
            egui::Button::new("Reopen last working file"),
        );
        if reopen_response.clicked() {
            app.reopen_last_sandbox_working_file();
        }
        ui.small(format!("Last: {last_label}"));
    });
}

fn render_chat_input_panel(ui: &mut egui::Ui, app: &mut ChattyCogApp, send_now: &mut bool) {
    ui.add_space(2.0);

    let live_task_nudge = if app.prefs.allow_sandbox_tool_requests {
        build_task_ledger_user_hint(&app.composer, app.sandbox_dir.as_deref())
    } else {
        None
    };
    if let Some(hint) = live_task_nudge {
        ui.horizontal_wrapped(|ui| {
            ui.small(format!("Task hint: {hint}"));
        });
        ui.add_space(4.0);
    }

    render_sandbox_task_controls(ui, app);
    ui.add_space(4.0);

    chat_actions::sync_pending_sandbox_actions(app);
    if !app.pending_sandbox_actions.is_empty() {
        render_pending_sandbox_actions(ui, app);
        ui.add_space(4.0);
    }

    ui.horizontal_wrapped(|ui| {
        ui.checkbox(
            &mut app.networking_shared_chat_mirror_main_chat,
            "Mirror this chat into the shared room",
        );
        if app.networking_shared_chat_mirror_main_chat {
            ui.small(format!("Mode: {}", app.shared_chat_policy_summary()));
            if !app.shared_chat_local_ai_allowed() {
                ui.small("Local AI reply is currently disabled by room policy.");
            }
        }
    });

    render_composer_row(ui, app, send_now);
    if app.is_generating {
        ui.horizontal_wrapped(|ui| {
            ui.small(
                "Please wait: ChattyCog is still generating the current reply. Interrupt it if you want to change course before sending another message.",
            );
        });
    }
    ui.add_space(2.0);
}

fn render_sandbox_task_controls(ui: &mut egui::Ui, app: &mut ChattyCogApp) {
    ui.group(|ui| {
        let sandbox_mode_available =
            app.prefs.allow_sandbox_tool_requests && app.sandbox_dir.is_some();
        ui.horizontal_wrapped(|ui| {
            ui.checkbox(&mut app.sandbox_task_enabled, "Sandbox task");
            ui.small("Mark this turn as a sandbox file request so the model skips the guesswork.");
        });
        if app.sandbox_task_enabled {
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                ui.add_enabled_ui(sandbox_mode_available, |ui| {
                    ui.selectable_value(
                        &mut app.sandbox_task_intent,
                        SandboxTaskIntent::Create,
                        "Create file",
                    );
                    ui.selectable_value(
                        &mut app.sandbox_task_intent,
                        SandboxTaskIntent::Edit,
                        "Edit file",
                    );
                });
                ui.label("Target:");
                let response = ui.add_enabled(
                    sandbox_mode_available,
                    egui::TextEdit::singleline(&mut app.sandbox_task_path)
                        .hint_text("notes/request.md")
                        .desired_width(220.0),
                );
                if response.changed() {
                    app.sandbox_task_path =
                        normalize_sandbox_task_path_input(&app.sandbox_task_path);
                }
            });
            let normalized_path = normalize_sandbox_task_path_input(&app.sandbox_task_path);
            if app.sandbox_task_path != normalized_path {
                app.sandbox_task_path = normalized_path.clone();
            }
            if !sandbox_mode_available {
                ui.small(
                    "Sandbox task mode needs `Allow sandbox tool requests` enabled and a live `Chatty_Sandbox/` folder.",
                );
            } else if normalized_path.is_empty() {
                ui.small("Enter a sandbox `.md` or `.txt` path for this task.");
            } else if let Err(err) = sandbox_ai_text_guard(&normalized_path) {
                ui.small(format!("Sandbox path blocked: {err}"));
            } else {
                ui.small(format!(
                    "This turn will explicitly tell the AI to {} `Chatty_Sandbox/{}`.",
                    app.sandbox_task_intent.summary_verb(),
                    normalized_path
                ));
            }
        }
    });
}

fn render_pending_sandbox_actions(ui: &mut egui::Ui, app: &mut ChattyCogApp) {
    ui.group(|ui| {
        ui.label("Pending sandbox actions (requires approval):");
        for action in &app.pending_sandbox_actions {
            render_pending_sandbox_action(ui, action);
        }
        ui.horizontal(|ui| {
            if ui.button("Seed ledger from current prompt").clicked() {
                app.seed_default_sandbox_task_ledger_from_context();
            }
            if ui.button("Defer actions").clicked() {
                app.defer_pending_sandbox_actions();
            }
            if ui.button("Preload + Continue").clicked() {
                app.preload_sandbox_and_continue();
            }
            if ui.button("Approve").clicked() {
                app.apply_pending_sandbox_actions(false);
            }
            if ui.button("Approve + Continue").clicked() {
                app.apply_pending_sandbox_actions(true);
            }
            if ui.button("Reject").clicked() {
                chat_actions::reject_pending_sandbox_actions(app);
            }
        });
    });
}

fn render_pending_sandbox_action(ui: &mut egui::Ui, action: &SandboxAction) {
    match action {
        SandboxAction::Write { path, .. } => ui.label(format!("- write: {path}")),
        SandboxAction::Append { path, .. } => ui.label(format!("- append: {path}")),
        SandboxAction::Read { path } => ui.label(format!("- read: {path}")),
        SandboxAction::List => ui.label("- list".to_string()),
        SandboxAction::Preload {
            paths,
            include_list,
            include_scratchpad,
            include_ledger,
            note,
        } => {
            let mut parts = Vec::new();
            if *include_list {
                parts.push("list".to_string());
            }
            if *include_scratchpad {
                parts.push("scratchpad".to_string());
            }
            if *include_ledger {
                parts.push("task ledger".to_string());
            }
            if !paths.is_empty() {
                parts.push(format!("files: {}", paths.join(", ")));
            }
            if !note.trim().is_empty() {
                parts.push(format!("note: {}", note));
            }
            ui.label(format!("- preload: {}", parts.join(" | ")))
        }
        SandboxAction::Ledger {
            status,
            current_task,
            next_step,
            open_questions,
            files_touched,
            ..
        } => {
            let mut parts = vec![format!("status: {}", status.trim())];
            if !current_task.trim().is_empty() {
                parts.push(format!(
                    "task: {}",
                    truncate_for_ui(current_task.trim(), 80)
                ));
            }
            if !next_step.trim().is_empty() {
                parts.push(format!(
                    "next: {}",
                    truncate_for_ui(next_step.trim(), 80)
                ));
            }
            if !open_questions.is_empty() {
                parts.push(format!("questions: {}", open_questions.len()));
            }
            if !files_touched.is_empty() {
                parts.push(format!("files: {}", files_touched.join(", ")));
            }
            ui.label(format!("- ledger: {}", parts.join(" | ")))
        }
    };
}

fn render_composer_row(ui: &mut egui::Ui, app: &mut ChattyCogApp, send_now: &mut bool) {
    ui.horizontal(|ui| {
        let paused = app.orch_freeze_pending || matches!(&app.tab, Tab::Module(_));
        let waiting_for_reply = app.is_generating;
        let composer_enabled = !paused && !waiting_for_reply;
        let action_button_width = 88.0;
        let composer_width = (ui.available_width() - action_button_width - 12.0).max(240.0);
        let composer_id = egui::Id::new("chat_composer");
        let input = ui.add_enabled_ui(composer_enabled, |ui| {
            ui.add_sized(
                [composer_width, 56.0],
                egui::TextEdit::multiline(&mut app.composer)
                    .id_salt(composer_id)
                    .hint_text(if paused {
                        "Orchestrator paused (module active)..."
                    } else if waiting_for_reply {
                        "Please wait for the current reply to finish or press Interrupt..."
                    } else {
                        "Type a message...  Enter sends, Shift+Enter adds a new line."
                    })
                    .desired_rows(2)
                    .desired_width(composer_width),
            )
        });
        let composer_response = input.inner;
        let restore_focus = composer_enabled
            && app.composer_had_focus_last_frame
            && composer_response.lost_focus()
            && !composer_response.has_focus()
            && !ui.input(|i| i.pointer.any_pressed() || i.key_pressed(egui::Key::Tab));
        if restore_focus {
            ui.ctx().memory_mut(|mem| mem.request_focus(composer_id));
        }
        app.composer_had_focus_last_frame =
            composer_enabled && (composer_response.has_focus() || restore_focus);

        if composer_response.has_focus()
            && ui.input(|i| i.key_pressed(egui::Key::Enter) && !i.modifiers.shift)
        {
            *send_now = true;
        }

        ui.add_enabled_ui(!waiting_for_reply && !paused, |ui| {
            if ui.button("Send").clicked() {
                *send_now = true;
            }
        });
        ui.add_enabled_ui(waiting_for_reply, |ui| {
            if ui.button("Interrupt").clicked() {
                app.stop_generation();
            }
        });
    });
}

fn render_chat_columns(ui: &mut egui::Ui, app: &mut ChattyCogApp) {
    let panel_height = ui.available_height().max(320.0);
    let gap = 10.0;
    let total_width = ui.available_width();
    let side_width = ((total_width - (gap * 2.0) - 320.0) * 0.5).clamp(220.0, 320.0);
    let center_width = (total_width - (side_width * 2.0) - (gap * 2.0)).max(320.0);
    ui.horizontal_top(|ui| {
        ui.allocate_ui_with_layout(
            egui::vec2(side_width, panel_height),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                ui.set_width(side_width);
                ui.set_max_width(side_width);
                render_chat_hot_memory_panel(ui, app, panel_height);
            },
        );
        ui.add_space(gap);

        ui.allocate_ui_with_layout(
            egui::vec2(center_width, panel_height),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                ui.set_width(center_width);
                ui.set_max_width(center_width);
                render_chat_transcript(ui, app, panel_height);
            },
        );
        ui.add_space(gap);

        ui.allocate_ui_with_layout(
            egui::vec2(side_width, panel_height),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                ui.set_width(side_width);
                ui.set_max_width(side_width);
                render_chat_lukewarm_panel(ui, app, panel_height);
            },
        );
    });
}

fn render_chat_transcript(ui: &mut egui::Ui, app: &mut ChattyCogApp, panel_height: f32) {
    egui::Frame::group(ui.style())
        .inner_margin(egui::Margin::same(10.0))
        .show(ui, |ui| {
            let transcript_width = ui.available_width();
            ui.set_width(transcript_width);
            ui.set_max_width(transcript_width);
            ui.heading("Chat");
            ui.add_space(6.0);
            egui::ScrollArea::vertical()
                .id_salt("chat_scroll")
                .stick_to_bottom(app.scroll_to_bottom)
                .auto_shrink([false, false])
                .max_height(panel_height - 24.0)
                .show(ui, |ui| {
                    let scroll_width = ui.available_width();
                    ui.set_width(scroll_width);
                    ui.set_max_width(scroll_width);
                    for msg in &app.messages {
                        message_bubble(ui, msg);
                    }
                    if app.is_generating && !app.assistant_draft.is_empty() {
                        let (visible, thinking) =
                            split_assistant_output(&app.assistant_draft);
                        message_bubble(
                            ui,
                            &Message {
                                role: Role::Assistant,
                                content: visible,
                                thinking,
                            },
                        );
                    }
                });
        });
}

pub(super) fn message_bubble(ui: &mut egui::Ui, msg: &Message) {
    let (label, color, fill) = match msg.role {
        Role::System => (
            "SYSTEM",
            egui::Color32::from_gray(120),
            egui::Color32::from_rgb(246, 246, 246),
        ),
        Role::User => (
            "YOU",
            egui::Color32::from_rgb(30, 80, 180),
            egui::Color32::from_rgb(240, 246, 255),
        ),
        Role::Assistant => (
            "ASSISTANT",
            egui::Color32::from_rgb(20, 120, 60),
            egui::Color32::from_rgb(244, 250, 245),
        ),
    };

    let width = ui.available_width().max(0.0);
    ui.allocate_ui_with_layout(
        egui::vec2(width, 0.0),
        egui::Layout::top_down(egui::Align::Min),
        |ui| {
            ui.set_width(width);
            ui.set_max_width(width);
            egui::Frame::none()
                .fill(fill)
                .stroke(egui::Stroke::new(1.0, color.gamma_multiply(0.45)))
                .rounding(egui::Rounding::same(6.0))
                .inner_margin(egui::Margin::symmetric(10.0, 8.0))
                .show(ui, |ui| {
                    ui.set_max_width(width);
                    ui.horizontal(|ui| {
                        ui.colored_label(color, label);
                    });
                    ui.add(egui::Label::new(msg.content.clone()).wrap());
                    if matches!(msg.role, Role::Assistant) {
                        if let Some(thinking) = msg
                            .thinking
                            .as_deref()
                            .filter(|value| !value.trim().is_empty())
                        {
                            ui.add_space(4.0);
                            let toggle_id = ui.make_persistent_id((
                                "assistant_thinking_toggle",
                                msg.content.as_str(),
                                thinking,
                            ));
                            let is_open = ui.ctx().data_mut(|data| {
                                data.get_persisted::<bool>(toggle_id).unwrap_or(false)
                            });
                            let label = if is_open {
                                if msg.content.trim().is_empty() {
                                    "Hide thinking (live)"
                                } else {
                                    "Hide thinking"
                                }
                            } else if msg.content.trim().is_empty() {
                                "Show thinking (live)"
                            } else {
                                "Show thinking"
                            };
                            egui::Frame::none()
                                .fill(ui.visuals().faint_bg_color)
                                .stroke(egui::Stroke::new(
                                    1.0,
                                    ui.visuals().widgets.noninteractive.bg_stroke.color,
                                ))
                                .rounding(egui::Rounding::same(6.0))
                                .inner_margin(egui::Margin::symmetric(8.0, 6.0))
                                .show(ui, |ui| {
                                    ui.horizontal(|ui| {
                                        let chevron = if is_open { "v" } else { ">" };
                                        let response = ui.add(
                                            egui::Button::new(format!("{chevron} {label}"))
                                                .frame(false),
                                        );
                                        ui.with_layout(
                                            egui::Layout::right_to_left(egui::Align::Center),
                                            |ui| {
                                                ui.small(
                                                    egui::RichText::new("Reasoning trace")
                                                        .weak()
                                                        .monospace(),
                                                );
                                            },
                                        );
                                        if response.clicked() {
                                            ui.ctx().data_mut(|data| {
                                                data.insert_persisted(toggle_id, !is_open);
                                            });
                                        }
                                    });
                                });
                            if is_open {
                                ui.add_space(4.0);
                                egui::ScrollArea::vertical()
                                    .id_salt(("assistant_thinking", msg.content.as_str()))
                                    .max_height(220.0)
                                    .show(ui, |ui| {
                                        ui.set_width(ui.available_width());
                                        ui.set_max_width(ui.available_width());
                                        ui.add(
                                            egui::Label::new(
                                                egui::RichText::new(thinking).monospace(),
                                            )
                                            .wrap(),
                                        );
                                    });
                            }
                        }
                    }
                });
        },
    );
    ui.add_space(6.0);
}

pub(super) fn runtime_backend_summary(status: &str) -> (&'static str, egui::Color32) {
    let lower = status.to_ascii_lowercase();
    if lower.contains("runtime error")
        || lower.contains("fallback failed")
        || lower.contains("load error")
    {
        ("Runtime issue", egui::Color32::from_rgb(170, 40, 40))
    } else if lower.contains("vulkan") {
        ("GPU path active", egui::Color32::from_rgb(25, 110, 70))
    } else if lower.contains("cpu") {
        ("CPU path active", egui::Color32::from_rgb(140, 95, 20))
    } else {
        ("Runtime ready", egui::Color32::from_rgb(50, 90, 150))
    }
}


