use super::*;

impl ChattyCogApp {
    pub(crate) fn ensure_default_sandbox_scratchpad(&mut self) {
        let Some(dir) = self.sandbox_dir.clone() else {
            return;
        };
        match ensure_default_sandbox_scratchpad_file(&dir) {
            Ok(_) => {}
            Err(err) => {
                if self.sandbox_status.trim().is_empty() {
                    self.sandbox_status = format!("Scratchpad setup failed: {err}");
                }
            }
        }
    }

    pub(crate) fn ensure_default_sandbox_task_ledger(&mut self) {
        let Some(dir) = self.sandbox_dir.clone() else {
            return;
        };
        match ensure_default_sandbox_task_ledger_file(&dir) {
            Ok(_) => {}
            Err(err) => {
                if self.sandbox_status.trim().is_empty() {
                    self.sandbox_status = format!("Task ledger setup failed: {err}");
                }
            }
        }
    }

    pub(crate) fn open_default_sandbox_scratchpad(&mut self) {
        let Some(dir) = self.sandbox_dir.clone() else {
            self.sandbox_status = "Sandbox folder not found.".to_string();
            return;
        };
        match ensure_default_sandbox_scratchpad_file(&dir) {
            Ok(path) => self.open_sandbox_file_in_editor(&path),
            Err(err) => self.sandbox_status = format!("Scratchpad setup failed: {err}"),
        }
    }

    pub(crate) fn open_default_sandbox_task_ledger(&mut self) {
        let Some(dir) = self.sandbox_dir.clone() else {
            self.sandbox_status = "Sandbox folder not found.".to_string();
            return;
        };
        match ensure_default_sandbox_task_ledger_file(&dir) {
            Ok(path) => self.open_sandbox_file_in_editor(&path),
            Err(err) => self.sandbox_status = format!("Task ledger setup failed: {err}"),
        }
    }

    pub(crate) fn reopen_last_sandbox_working_file(&mut self) {
        let Some(path) = self.sandbox_last_working_path.clone() else {
            self.sandbox_status = "No sandbox working file has been opened yet.".to_string();
            return;
        };
        self.open_sandbox_file_and_focus_tab(&path);
    }

    pub(crate) fn current_sandbox_editor_rel_path(&self, dir: &Path) -> Option<String> {
        self.sandbox_editor_path
            .as_ref()
            .and_then(|path| path.strip_prefix(dir).ok())
            .map(|path| path.to_string_lossy().replace('\\', "/"))
    }

    pub(crate) fn promote_editor_text_to_scratchpad(&mut self) {
        let Some(dir) = self.sandbox_dir.clone() else {
            self.sandbox_status = "Sandbox folder not found.".to_string();
            return;
        };
        let text = self.sandbox_editor_text.trim().to_string();
        if text.is_empty() {
            self.sandbox_status = "Editor is empty. Nothing to promote.".to_string();
            return;
        }

        let source = self
            .current_sandbox_editor_rel_path(&dir)
            .unwrap_or_else(|| "(unsaved scratch buffer)".to_string());
        let block = format!(
            "\n## Promoted note ({})\nSource: `{}`\n\n{}\n",
            now_unix_ms().max(0),
            source,
            text
        );

        match sandbox_append(&dir, DEFAULT_SANDBOX_SCRATCHPAD_REL_PATH, &block) {
            Ok(path) => {
                self.sandbox_status = format!("Promoted editor text to {}", path.display());
                self.open_sandbox_file_in_editor(&path);
                push_hot_memory(
                    self,
                    format!("Sandbox: {}", one_line(&self.sandbox_status, 120)),
                );
            }
            Err(err) => {
                self.sandbox_status = format!("Could not promote editor text: {err}");
            }
        }
    }

    pub(crate) fn promote_editor_text_to_ledger_notes(&mut self) {
        let Some(dir) = self.sandbox_dir.clone() else {
            self.sandbox_status = "Sandbox folder not found.".to_string();
            return;
        };
        let text = self.sandbox_editor_text.trim().to_string();
        if text.is_empty() {
            self.sandbox_status = "Editor is empty. Nothing to promote.".to_string();
            return;
        }

        self.ensure_default_sandbox_task_ledger();

        let mut summary = read_task_ledger_summary(&dir).unwrap_or_default();
        if summary.status.trim().is_empty() {
            summary.status = "active".to_string();
        }

        let source = self
            .current_sandbox_editor_rel_path(&dir)
            .unwrap_or_else(|| "(unsaved scratch buffer)".to_string());
        if source != "(unsaved scratch buffer)" && !summary.files_touched.contains(&source) {
            summary.files_touched.push(source.clone());
        }

        let mut promoted_notes = vec![format!(
            "Promoted from {} at {}",
            source,
            now_unix_ms().max(0)
        )];
        promoted_notes.extend(
            text.lines()
                .map(str::trim)
                .filter(|line| !line.is_empty())
                .take(8)
                .map(|line| truncate_for_ui(line, 220)),
        );
        summary.notes.extend(promoted_notes);
        if summary.notes.len() > 24 {
            let keep_from = summary.notes.len() - 24;
            summary.notes = summary.notes.split_off(keep_from);
        }

        match sandbox_write_task_ledger(
            &dir,
            if summary.status.trim().is_empty() {
                "active"
            } else {
                summary.status.trim()
            },
            summary.current_task.trim(),
            summary.next_step.trim(),
            &summary.open_questions,
            &summary.files_touched,
            &summary.notes,
        ) {
            Ok(path) => {
                self.sandbox_status = format!("Promoted editor notes into {}", path.display());
                self.open_sandbox_file_in_editor(&path);
                push_hot_memory(
                    self,
                    format!("Sandbox: {}", one_line(&self.sandbox_status, 120)),
                );
            }
            Err(err) => {
                self.sandbox_status = format!("Could not promote editor notes: {err}");
            }
        }
    }

    pub(crate) fn set_task_ledger_field_from_editor(&mut self, set_current_task: bool) {
        let Some(dir) = self.sandbox_dir.clone() else {
            self.sandbox_status = "Sandbox folder not found.".to_string();
            return;
        };
        let text = self.sandbox_editor_text.trim().to_string();
        if text.is_empty() {
            self.sandbox_status = "Editor is empty. Nothing to promote.".to_string();
            return;
        }

        self.ensure_default_sandbox_task_ledger();

        let mut summary = read_task_ledger_summary(&dir).unwrap_or_default();
        if summary.status.trim().is_empty() {
            summary.status = "active".to_string();
        }

        let source = self
            .current_sandbox_editor_rel_path(&dir)
            .unwrap_or_else(|| "(unsaved scratch buffer)".to_string());
        if source != "(unsaved scratch buffer)" && !summary.files_touched.contains(&source) {
            summary.files_touched.push(source.clone());
        }

        let normalized = text
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .collect::<Vec<_>>()
            .join(" ");
        let normalized = truncate_for_ui(&normalized, 420);

        if set_current_task {
            summary.current_task = normalized.clone();
            if summary.next_step.trim().is_empty() {
                summary.next_step =
                    "Review the updated current task and choose the next concrete step."
                        .to_string();
            }
            summary.notes.push(format!(
                "Current task updated from {} at {}",
                source,
                now_unix_ms().max(0)
            ));
        } else {
            summary.next_step = normalized.clone();
            summary.notes.push(format!(
                "Next step updated from {} at {}",
                source,
                now_unix_ms().max(0)
            ));
        }

        if summary.notes.len() > 24 {
            let keep_from = summary.notes.len() - 24;
            summary.notes = summary.notes.split_off(keep_from);
        }

        match sandbox_write_task_ledger(
            &dir,
            summary.status.trim(),
            summary.current_task.trim(),
            summary.next_step.trim(),
            &summary.open_questions,
            &summary.files_touched,
            &summary.notes,
        ) {
            Ok(path) => {
                self.sandbox_status = if set_current_task {
                    format!(
                        "Promoted editor text into current task at {}",
                        path.display()
                    )
                } else {
                    format!("Promoted editor text into next step at {}", path.display())
                };
                self.open_sandbox_file_in_editor(&path);
                push_hot_memory(
                    self,
                    format!("Sandbox: {}", one_line(&self.sandbox_status, 120)),
                );
            }
            Err(err) => {
                self.sandbox_status = format!("Could not update task ledger: {err}");
            }
        }
    }

    pub(crate) fn append_editor_summary_to_hot_memory(&mut self) {
        let Some(dir) = self.sandbox_dir.clone() else {
            self.sandbox_status = "Sandbox folder not found.".to_string();
            return;
        };
        let text = self.sandbox_editor_text.trim().to_string();
        if text.is_empty() {
            self.sandbox_status = "Editor is empty. Nothing to summarize.".to_string();
            return;
        }

        let source = self
            .current_sandbox_editor_rel_path(&dir)
            .unwrap_or_else(|| "scratch buffer".to_string());
        let summary = text
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .take(4)
            .collect::<Vec<_>>()
            .join(" ");
        let summary = truncate_for_ui(&summary, 240);
        let hot_item = format!("Sandbox note ({source}): {summary}");

        push_hot_memory(self, hot_item.clone());
        self.sandbox_status = format!("Added editor summary to hot memory from {source}.");
        if let Some(bk) = &self.bookkeeper {
            bk.append(MemoryEvent {
                ts_unix_ms: now_unix_ms(),
                kind: MemoryKind::Cold,
                category: EventCategory::Chat,
                source: "sandbox".to_string(),
                module: None,
                event_type: Some("hot_memory_summary".to_string()),
                text: hot_item,
                tags: vec!["sandbox".to_string(), "hot_memory_summary".to_string()],
                entities: Vec::new(),
                payload_json: None,
            });
        }
    }
}
