use super::*;

impl ChattyCogApp {
    pub(crate) fn seed_default_sandbox_task_ledger_from_context(&mut self) {
        let Some(dir) = self.sandbox_dir.clone() else {
            self.sandbox_status = "Sandbox folder not found.".to_string();
            return;
        };
        let current_task = self
            .messages
            .iter()
            .rev()
            .find(|message| matches!(message.role, Role::User))
            .map(|message| truncate_for_ui(message.content.trim(), 500))
            .unwrap_or_else(|| "Capture the current task here.".to_string());
        let next_step = self
            .hot_memory
            .last()
            .map(|item| truncate_for_ui(item.trim(), 220))
            .unwrap_or_else(|| "Record the next concrete step here.".to_string());
        let files_touched = self
            .sandbox_editor_path
            .as_ref()
            .and_then(|path| path.strip_prefix(&dir).ok())
            .map(|path| path.to_string_lossy().replace('\\', "/"))
            .into_iter()
            .collect::<Vec<_>>();
        let notes = self
            .hot_memory
            .iter()
            .rev()
            .take(5)
            .cloned()
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect::<Vec<_>>();
        match sandbox_write_task_ledger(
            &dir,
            "active",
            &current_task,
            &next_step,
            &Vec::new(),
            &files_touched,
            &notes,
        ) {
            Ok(path) => {
                self.sandbox_status = format!("Seeded task ledger at {}", path.display());
                self.open_sandbox_file_in_editor(&path);
            }
            Err(err) => {
                self.sandbox_status = format!("Could not seed task ledger: {err}");
            }
        }
    }

    pub(crate) fn defer_pending_sandbox_actions(&mut self) {
        let deferred_count = self.pending_sandbox_actions.len();
        self.pending_sandbox_actions.clear();
        self.sandbox_action_status = if deferred_count == 0 {
            "No sandbox actions were waiting to be deferred.".to_string()
        } else {
            format!("Deferred {deferred_count} sandbox action(s). No file changes were run.")
        };
        push_hot_memory(
            self,
            format!("Sandbox: {}", one_line(&self.sandbox_action_status, 120)),
        );
        if let Some(bk) = &self.bookkeeper {
            bk.append(MemoryEvent {
                ts_unix_ms: now_unix_ms(),
                kind: MemoryKind::Cold,
                category: EventCategory::Chat,
                source: "sandbox".to_string(),
                module: None,
                event_type: Some("deferred_actions".to_string()),
                text: self.sandbox_action_status.clone(),
                tags: vec!["sandbox".to_string(), "deferred_actions".to_string()],
                entities: Vec::new(),
                payload_json: None,
            });
        }
    }

    pub(crate) fn preload_sandbox_and_continue(&mut self) {
        let Some(dir) = self.sandbox_dir.clone() else {
            self.sandbox_action_status = "Sandbox folder not found.".to_string();
            self.pending_sandbox_actions.clear();
            return;
        };

        self.ensure_default_sandbox_scratchpad();
        self.ensure_default_sandbox_task_ledger();

        let mut paths = Vec::new();
        for action in &self.pending_sandbox_actions {
            match action {
                SandboxAction::Write { path, .. }
                | SandboxAction::Append { path, .. }
                | SandboxAction::Read { path } => {
                    if !path.trim().is_empty() {
                        paths.push(path.trim().to_string());
                    }
                }
                SandboxAction::Preload {
                    paths: more_paths, ..
                } => {
                    for path in more_paths {
                        if !path.trim().is_empty() {
                            paths.push(path.trim().to_string());
                        }
                    }
                }
                SandboxAction::Ledger { files_touched, .. } => {
                    for path in files_touched {
                        if !path.trim().is_empty() {
                            paths.push(path.trim().to_string());
                        }
                    }
                }
                SandboxAction::List => {}
            }
        }

        if let Some(editor_path) = self.sandbox_editor_path.as_ref() {
            if let Ok(rel) = editor_path.strip_prefix(&dir) {
                let rel = rel.to_string_lossy().replace('\\', "/");
                if !rel.trim().is_empty() {
                    paths.push(rel);
                }
            }
        }

        paths.sort();
        paths.dedup();

        let note = if paths.is_empty() {
            "fast preload before continuing a multi-step task"
        } else {
            "fast preload before continuing; inspect likely relevant sandbox files first"
        };

        match sandbox_preload(&dir, &paths, true, true, true, note) {
            Ok(result) => {
                self.pending_sandbox_actions.clear();
                self.sandbox_last_tool_result = result.prompt_block;
                self.sandbox_action_status = format!(
                    "Preloaded {} item(s); pending sandbox actions were deferred.",
                    result.loaded_count
                );
                if let Ok(path) = ensure_default_sandbox_scratchpad_file(&dir) {
                    self.open_sandbox_file_in_editor(&path);
                }
                push_hot_memory(
                    self,
                    format!("Sandbox: {}", one_line(&self.sandbox_action_status, 120)),
                );
                if let Some(bk) = &self.bookkeeper {
                    bk.append(MemoryEvent {
                        ts_unix_ms: now_unix_ms(),
                        kind: MemoryKind::Cold,
                        category: EventCategory::Chat,
                        source: "sandbox".to_string(),
                        module: None,
                        event_type: Some("tool_result".to_string()),
                        text: self.sandbox_last_tool_result.clone(),
                        tags: vec![
                            "sandbox".to_string(),
                            "tool_result".to_string(),
                            "preload_fast_path".to_string(),
                        ],
                        entities: Vec::new(),
                        payload_json: None,
                    });
                }
                if !self.is_generating && !self.sandbox_last_tool_result.trim().is_empty() {
                    self.start_generation(
                        "Continue from the sandbox preload context and help with the current task. Reconsider the deferred sandbox actions, and only request new sandbox JSON if it is still needed.".to_string(),
                    );
                }
            }
            Err(err) => {
                self.sandbox_action_status = format!("Sandbox preload failed: {err}");
            }
        }
    }

    pub(crate) fn apply_pending_sandbox_actions(&mut self, continue_after: bool) {
        let Some(dir) = self.sandbox_dir.clone() else {
            self.sandbox_action_status = "Sandbox folder not found.".to_string();
            self.pending_sandbox_actions.clear();
            return;
        };

        let mut status_lines = Vec::new();
        let mut result_lines = Vec::new();
        let mut last_opened: Option<PathBuf> = None;
        let mut selected_image_for_continuation: Option<PathBuf> = None;
        for action in self.pending_sandbox_actions.drain(..) {
            match action {
                SandboxAction::Write { path, contents } => {
                    match sandbox_ai_text_guard(&path)
                        .and_then(|_| sandbox_write(&dir, &path, &contents))
                    {
                        Ok(p) => {
                            status_lines.push(format!("Wrote {}", p.display()));
                            result_lines.push(format!(
                                "sandbox.write `{}` succeeded.",
                                p.strip_prefix(&dir)
                                    .unwrap_or(&p)
                                    .to_string_lossy()
                                    .replace('\\', "/")
                            ));
                            last_opened = Some(p);
                        }
                        Err(e) => status_lines.push(format!("Write blocked/failed ({path}): {e}")),
                    }
                }
                SandboxAction::Append { path, contents } => {
                    match sandbox_ai_text_guard(&path)
                        .and_then(|_| sandbox_append(&dir, &path, &contents))
                    {
                        Ok(p) => {
                            status_lines.push(format!("Appended {}", p.display()));
                            result_lines.push(format!(
                                "sandbox.append `{}` succeeded.",
                                p.strip_prefix(&dir)
                                    .unwrap_or(&p)
                                    .to_string_lossy()
                                    .replace('\\', "/")
                            ));
                            last_opened = Some(p);
                        }
                        Err(e) => status_lines.push(format!("Append blocked/failed ({path}): {e}")),
                    }
                }
                SandboxAction::Read { path } => {
                    if sandbox_rel_path_looks_like_image(&path) {
                        match sandbox_ai_read_guard(&path).and_then(|_| {
                            let rel = parse_sandbox_rel_path(&path)?;
                            ensure_path_within_dir(&dir, &dir.join(rel))
                        }) {
                            Ok(image_path) => {
                                let file_label = image_path
                                    .strip_prefix(&dir)
                                    .unwrap_or(&image_path)
                                    .to_string_lossy()
                                    .replace('\\', "/");
                                status_lines.push(format!("Selected image {file_label} for inspection"));
                                result_lines.push(format!(
                                    "sandbox.read `{path}` succeeded.\nAttached sandbox image `{file_label}` for multimodal inspection on the continuation turn."
                                ));
                                last_opened = Some(image_path.clone());
                                selected_image_for_continuation = Some(image_path.clone());
                                self.chat_selected_file = Some(image_path);
                            }
                            Err(e) => {
                                status_lines.push(format!("Read blocked/failed ({path}): {e}"))
                            }
                        }
                    } else {
                        match sandbox_ai_read_guard(&path)
                            .and_then(|_| sandbox_read(&dir, &path, 200_000))
                        {
                            Ok(s) => {
                                let preview = truncate_for_ui(&s, 400);
                                status_lines.push(format!("Read {path}: {preview}"));
                                result_lines.push(format!(
                                    "sandbox.read `{path}` succeeded.\n{}",
                                    truncate_for_ui(&s, 4_000)
                                ));
                                if let Ok(rel) = parse_sandbox_rel_path(&path) {
                                    last_opened = Some(dir.join(rel));
                                }
                            }
                            Err(e) => status_lines.push(format!("Read blocked/failed ({path}): {e}")),
                        }
                    }
                }
                SandboxAction::List => match sandbox_list(&dir) {
                    Ok(items) => {
                        let preview = if items.is_empty() {
                            "(sandbox is empty)".to_string()
                        } else {
                            items
                                .iter()
                                .take(80)
                                .cloned()
                                .collect::<Vec<_>>()
                                .join("\n")
                        };
                        status_lines.push(format!("Sandbox files: {}", items.join(", ")));
                        result_lines.push(format!("sandbox.list succeeded.\n{preview}"));
                    }
                    Err(e) => status_lines.push(format!("List failed: {e}")),
                },
                SandboxAction::Ledger {
                    status,
                    current_task,
                    next_step,
                    open_questions,
                    files_touched,
                    notes,
                } => match sandbox_write_task_ledger(
                    &dir,
                    &status,
                    &current_task,
                    &next_step,
                    &open_questions,
                    &files_touched,
                    &notes,
                ) {
                    Ok(path) => {
                        status_lines.push(format!("Updated {}", path.display()));
                        result_lines.push(format!(
                            "sandbox.ledger updated `{}`.\n{}",
                            path.strip_prefix(&dir)
                                .unwrap_or(&path)
                                .to_string_lossy()
                                .replace('\\', "/"),
                            render_task_ledger_markdown(
                                &status,
                                &current_task,
                                &next_step,
                                &open_questions,
                                &files_touched,
                                &notes,
                            )
                        ));
                        last_opened = Some(path);
                    }
                    Err(e) => status_lines.push(format!("Ledger update failed: {e}")),
                },
                SandboxAction::Preload {
                    paths,
                    include_list,
                    include_scratchpad,
                    include_ledger,
                    note,
                } => {
                    let original_count = paths.len();
                    let filtered_paths = paths
                        .into_iter()
                        .filter(|path| sandbox_rel_path_is_ai_text_allowed(path))
                        .collect::<Vec<_>>();
                    let skipped_count = original_count.saturating_sub(filtered_paths.len());
                    match sandbox_preload(
                        &dir,
                        &filtered_paths,
                        include_list,
                        include_scratchpad,
                        include_ledger,
                        &note,
                    ) {
                        Ok(result) => {
                            status_lines.push(format!("Preloaded {} item(s)", result.loaded_count));
                            if skipped_count > 0 {
                                status_lines.push(format!(
                                    "Skipped {skipped_count} non-text sandbox path(s)"
                                ));
                            }
                            result_lines.push(result.prompt_block);
                            if include_scratchpad {
                                if let Ok(rel) =
                                    parse_sandbox_rel_path(DEFAULT_SANDBOX_SCRATCHPAD_REL_PATH)
                                {
                                    last_opened = Some(dir.join(rel));
                                }
                            } else if include_ledger {
                                if let Ok(rel) =
                                    parse_sandbox_rel_path(DEFAULT_SANDBOX_TASK_LEDGER_REL_PATH)
                                {
                                    last_opened = Some(dir.join(rel));
                                }
                            } else if let Some(first_path) = filtered_paths.first() {
                                if let Ok(rel) = parse_sandbox_rel_path(first_path) {
                                    last_opened = Some(dir.join(rel));
                                }
                            }
                        }
                        Err(e) => status_lines.push(format!("Preload failed: {e}")),
                    }
                }
            }
        }

        if status_lines.is_empty() {
            self.sandbox_action_status = "No actions applied.".to_string();
        } else {
            self.sandbox_action_status = status_lines.join(" | ");
        }

        if let Some(path) = last_opened {
            self.open_sandbox_file_and_focus_tab(&path);
        }

        if result_lines.is_empty() {
            self.sandbox_last_tool_result.clear();
        } else {
            self.sandbox_last_tool_result = result_lines.join("\n\n");
            push_hot_memory(
                self,
                format!("Sandbox: {}", one_line(&self.sandbox_action_status, 120)),
            );
            if let Some(bk) = &self.bookkeeper {
                bk.append(MemoryEvent {
                    ts_unix_ms: now_unix_ms(),
                    kind: MemoryKind::Cold,
                    category: EventCategory::Chat,
                    source: "sandbox".to_string(),
                    module: None,
                    event_type: Some("tool_result".to_string()),
                    text: self.sandbox_last_tool_result.clone(),
                    tags: vec!["sandbox".to_string(), "tool_result".to_string()],
                    entities: Vec::new(),
                    payload_json: None,
                });
            }
        }

        if continue_after && !self.is_generating && !self.sandbox_last_tool_result.trim().is_empty() {
            let continuation_prompt =
                "Continue from the approved sandbox tool result and help with the current task. If another sandbox action is needed, request it as JSON.".to_string();
            if let Some(image_path) = selected_image_for_continuation
                && self
                    .gguf_path
                    .as_deref()
                    .map(selected_model_is_vision_ready)
                    .unwrap_or(false)
            {
                self.start_multimodal_generation(continuation_prompt, image_path);
            } else {
                self.start_generation(continuation_prompt);
            }
        }
    }
}
