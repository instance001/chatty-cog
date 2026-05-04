# Changelog

All notable changes to this project will be documented here.

This project is pre-release and changes may be breaking.

## Unreleased

- Chat UI refresh:
  - three-column chat layout (`Hot Memory` / transcript / `Luke Warm`)
  - chat-header GGUF selector, quick-open, refresh, active voice preview, and live `Chat max tokens` readout
  - multiline composer with explicit `Please wait` guard while a reply is still generating
  - inline `Interrupt` control in the composer
- Chat reasoning polish:
  - collapsible assistant thinking panel
  - stronger prompt rules against repeated self-correction loops
  - heuristic cleanup that siphons obvious meta-reasoning out of visible replies
  - stream-time trimming of exact repeated draft suffixes
- Preferences / personality:
  - Preferences tab now houses reusable capsule-style personality / behavior injections
  - users can save multiple capsules, activate one, or fall back to native ChattyCog voice
  - orchestrator chat settings now apply live from Preferences instead of requiring a separate hidden runtime sync step
  - legacy untouched `256` orchestrator max-token defaults migrate forward to `1024` on load
- Runtime hardening:
  - one-time backend init now tracks live runtime handles so stale backend flags do not survive after the last runtime instance drops
- Rust GUI scaffold (eframe/egui) with old-school tabbed UI.
- Option B llama.cpp integration (dynamic `llama.dll` + ggml backends).
- Runtime hardening: serialize llama.cpp backend init/free across threads to reduce `llama_decode failed` from concurrent chat/bookkeeper/module work.
- Runtime hardening: sample logits with `idx = -1` (llama.h recommended) to avoid out-of-range sampling after chunked prompt eval.
- Runtime hardening: safer context params (clamp `n_batch`, set `n_ubatch`, disable flash-attn in CPU paths, deterministic abort callback for CPU cancellation).
- Runtime hardening: extra "CPU safe-mode" retry when GPU + normal CPU paths fail.
- Chat tab with streaming output and generation params (temp/top_p/top_k/max_tokens) + presets.
- CPU-only bookkeeper:
  - cold logs (`cold_log.jsonl`)
  - semantic search (embeddings + cosine)
  - semantic search filters (`module_id`, `tag`)
  - schemaless cold-log envelope (`module_id`, `event_type`, `summary`, `tags`, optional `payload_json`)
  - keyword search fallback in GUI
  - Luke Warm rolling summary persisted to `lukewarm.txt`
- Logs tab: log folder explorer + preview pane.
- Logs tab: append ad-hoc module events with tags + payload.
- Sandbox tab: browse/edit/save files in `Chatty_Sandbox/`.
- Module tabs: graceful orchestrator pause (lets current response finish, then pauses; chat input disabled while module active).
- Chat tab: user-approved sandbox tool requests (read/write/list restricted to `Chatty_Sandbox/` only).
- Sandbox hardening: stricter path-jail (blocks absolute paths / traversal / `:`) and allows safe nested subfolder writes.
- Modules: discovery from `chattycog_gui/modules/*/manifest.json` and dynamic tabs.
- Modules: optional per-module AI runner for demo modules (`ai_enabled`, `default_model` in manifest).
- Modules: on tab leave/close, emit a `suspend_rundown` cold-log event to keep cross-module context up to date.
- Modules: optional auto-generation of `suspend_rundown` via CPU-only Bookkeeper summarization (snapshots module form/workspace + last module AI output).
- Bookkeeper: maintains `memory/departments.md` + `memory/departments.json` (latest per-module status updates) from `suspend_rundown` events.
- Preferences: new `config/preferences.json` storing orchestrator/bookkeeper defaults and per-module settings (including sandbox tool access toggle and auto module rundown toggle).
