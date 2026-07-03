use super::*;

struct OutgoingChatTurn {
    content: String,
    visible_user_message: String,
    generation_prompt: String,
}

pub(super) fn sync_pending_sandbox_actions(app: &mut ChattyCogApp) {
    if !app.pending_sandbox_actions.is_empty() && !app.prefs.allow_sandbox_tool_requests {
        app.pending_sandbox_actions.clear();
    }
}

pub(super) fn reject_pending_sandbox_actions(app: &mut ChattyCogApp) {
    app.pending_sandbox_actions.clear();
    app.sandbox_action_status = "Rejected sandbox actions.".to_string();
}

pub(super) fn handle_chat_send(ctx: &egui::Context, app: &mut ChattyCogApp) {
    if app.is_generating {
        app.runtime_status =
            "Runtime: please wait for the current reply to finish, or press Interrupt.".to_string();
        ctx.request_repaint();
        return;
    }

    let Some(turn) = prepare_outgoing_chat_turn(ctx, app) else {
        return;
    };

    if let Err(reason) = app.shared_chat_can_send_mirrored_main_chat_message() {
        app.networking_status = format!("Shared room: {reason}");
        ctx.request_repaint();
        return;
    }

    app.composer.clear();
    app.pulse_ecg(20.0, "Queued a chat message.");
    app.messages.push(Message {
        role: Role::User,
        content: turn.visible_user_message,
        thinking: None,
    });
    trim_live_chat_messages(&mut app.messages);
    push_hot_memory(app, format!("User: {}", one_line(&turn.content, 120)));
    if let Some(bk) = &app.bookkeeper {
        bk.append(MemoryEvent {
            ts_unix_ms: now_unix_ms(),
            kind: MemoryKind::Cold,
            category: EventCategory::Chat,
            source: "user".to_string(),
            module: None,
            event_type: Some("message".to_string()),
            text: turn.content.clone(),
            tags: Vec::new(),
            entities: Vec::new(),
            payload_json: None,
        });
    }
    app.scroll_to_bottom = true;
    if app.networking_shared_chat_mirror_main_chat {
        app.broadcast_shared_chat_message("user", "You", &turn.content);
    }
    if app.shared_chat_local_ai_allowed() {
        app.start_generation(turn.generation_prompt);
    } else {
        app.runtime_status =
            "Runtime: shared room policy left AI off for this local turn.".to_string();
    }
    ctx.request_repaint();
}

fn prepare_outgoing_chat_turn(
    ctx: &egui::Context,
    app: &mut ChattyCogApp,
) -> Option<OutgoingChatTurn> {
    let content = app.composer.trim().to_string();
    if content.is_empty() {
        return None;
    }

    let sandbox_path = normalize_sandbox_task_path_input(&app.sandbox_task_path);
    let sandbox_mode_active = app.sandbox_task_enabled;
    let mut generation_prompt = content.clone();
    let visible_user_message = if sandbox_mode_active {
        format!(
            "[Sandbox {} -> {}] {}",
            app.sandbox_task_intent.label(),
            sandbox_path,
            content
        )
    } else {
        content.clone()
    };
    app.sandbox_task_nudge =
        build_task_ledger_user_hint(&content, app.sandbox_dir.as_deref()).unwrap_or_default();

    if sandbox_mode_active {
        if !app.prefs.allow_sandbox_tool_requests {
            app.sandbox_action_status =
                "Sandbox task mode needs `Allow sandbox tool requests` turned on.".to_string();
            ctx.request_repaint();
            return None;
        }
        if app.sandbox_dir.is_none() {
            app.sandbox_action_status =
                "Sandbox task mode needs a live `Chatty_Sandbox/` folder.".to_string();
            ctx.request_repaint();
            return None;
        }
        if sandbox_path.is_empty() {
            app.sandbox_action_status =
                "Sandbox task mode needs a target `.md` or `.txt` path.".to_string();
            ctx.request_repaint();
            return None;
        }
        if let Err(err) = sandbox_ai_text_guard(&sandbox_path) {
            app.sandbox_action_status = format!("Sandbox task path blocked: {err}");
            ctx.request_repaint();
            return None;
        }
        generation_prompt =
            build_explicit_sandbox_task_prompt(&content, &sandbox_path, app.sandbox_task_intent);
    }

    Some(OutgoingChatTurn {
        content,
        visible_user_message,
        generation_prompt,
    })
}
