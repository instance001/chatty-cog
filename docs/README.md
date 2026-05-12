# ChattyCog

Offline-first, tabbed desktop GUI for chatting with local **GGUF** models via a bundled **llama.cpp** runtime on Windows.

ChattyCog does not require internet or cloud services to function. It can also optionally connect to other nearby ChattyCog instances over local Wi-Fi or LAN when a user deliberately enables local networking.

This repo currently contains one Rust GUI app: `chattycog_gui/`.

## Quickstart (Windows)

1) Prereqs
- Install Rust (stable).
- Install LLVM (for bindgen / libclang). Default path assumed: `C:\Program Files\LLVM\bin`.

2) Folder layout
- `chattycog_gui/models/` - put your `.gguf` files here
- `chattycog_gui/runtime/windows/` - put `llama.dll` + `ggml-*.dll` backends here (Vulkan + CPU)
- `chattycog_gui/modules/` - drop-in modules (each module needs a `manifest.json`)
- `chattycog_gui/Chatty_Sandbox/` - scratch folder the app can browse/edit (and the orchestrator can access via user-approved tool requests)
- `chattycog_gui/memory/` - local logs and summaries:
  - `cold_log.jsonl` (long-term, append-only log)
  - `lukewarm.txt` (rolling summary)
  - `departments.md` / `departments.json` (latest per-module "what happened" rundowns)

3) Build + run
```bash
cargo build --manifest-path chattycog_gui/Cargo.toml
cargo run  --manifest-path chattycog_gui/Cargo.toml --bin chattycog_gui
```

Optional smoke test (loads runtime + runs a tiny generation):
```bash
cargo run --manifest-path chattycog_gui/Cargo.toml --bin smoke_llama
```

## What you get

- **Chat tab**: talk to a selected GGUF model (streaming tokens).
- **Chat runtime visibility**:
  - runtime status shows whether ChattyCog is on a GPU/Vulkan path, CPU path, or error path
  - the chat header now includes a GGUF selector, `Open GGUF...`, `Refresh models`, and a live `Chat max tokens` readout
  - the chat composer enters a `Please wait` state while a reply is still generating, so rapid-fire extra turns do not get injected into the same inference run
  - assistant reasoning can be expanded/collapsed in-place from a dedicated disclosure row
  - inline `Interrupt` button lets you stop the current reply without leaving the composer
- **Chat layout**:
  - three-column chat workspace with `Hot Memory` on the left, the main transcript in the center, and `Luke Warm` memory on the right
  - multiline composer (`Enter` sends, `Shift+Enter` adds a new line)
- **Hot memory**: small, user-visible "working set" of recent/pinned items.
- **Logs tab**:
  - CPU-only **bookkeeper** that records events/messages to disk (`cold_log.jsonl`)
  - semantic search (Option B, embeddings) + keyword search fallback (Option A)
  - "Luke Warm" rolling summary persisted to `lukewarm.txt`
  - semantic search filters by `module_id` (department) and/or `tag`
  - append ad-hoc events with `module_id` + `tags` + optional `payload_json`
- **Sandbox tab**: browse/edit/save files inside `Chatty_Sandbox/`.
- **Sandbox task mode**:
  - the chat composer can explicitly mark a turn as a sandbox file task
  - choose `Create file` or `Edit file`, set a target `.md` / `.txt` path, and ChattyCog will steer the model toward deterministic sandbox JSON instead of making it infer intent from freeform wording
  - approved sandbox actions automatically open the touched file and focus the Sandbox tab
- **Sandbox image inspection**:
  - approved read-only image inspection is also available for `.png`, `.jpg`, `.jpeg`, and `.webp` files inside `Chatty_Sandbox/`
  - the chat composer now has a direct `Sandbox reference` selector so users can point ChattyCog at a specific sandbox image or text file without making the model guess the path
  - ChattyCog supports vision routing modes: `Auto`, `Prefer active`, and `Force fallback`
  - the app prefers the active chat model when it has a matching multimodal projector unless the user explicitly forces the fallback helper
  - image inspection results are fed back into the chat flow as sandbox tool results so the orchestrator can reason over diagrams, screenshots, whiteboards, and other reference images
- **Capsule library**:
  - the Preferences tab includes a reusable capsule library for saved personality / behavior injections
  - users can save multiple named capsules, activate one for the current task, or fall back to native ChattyCog voice
- **Networking tab**:
  - discover nearby ChattyCog instances on the same trusted local network
  - connect/disconnect peers
  - make this device available for connectivity
  - send and receive short local handoff notes
  - send and receive generic workflow bundles for sharing a whole ChattyCog setup between nearby instances
  - rename/group nearby devices locally so larger peer lists stay manageable
- **Modules**:
  - scan `modules/*/manifest.json`, open modules from the **Modules** menu into closable tabs
  - module state surfaces (`ui.json` form -> `state.json`, or `STATE_TEMPLATE.md`/`workspace.md`)
  - auto "suspend rundown" (Bookkeeper-generated) on tab leave, used as cross-module context
  - optional module shared-state transfer stays separate from the generic workflow-bundle lane

## Troubleshooting

- If chat replies still feel cut off too early:
  - confirm the chat header `Chat max tokens` value is what you expect
  - the Preferences tab now updates live chat settings immediately, and the next Save will persist them to `config/preferences.json`
- If sandbox image inspection says no multimodal model is available:
  - confirm the active chat model has a matching `mmproj` file, or place a supported fallback vision pair in `models/`
- If sandbox image inspection keeps misreading the picture:
  - try `Preferences -> Access / Tools -> Vision routing -> Force fallback`
  - smaller or older multimodal models may still "see" the image while doing a poor job interpreting it
- If the app can't load the runtime, check:
  - `chattycog_gui/runtime/windows/llama.dll` exists
  - `chattycog_gui/runtime/windows/ggml.dll` exists
  - Vulkan driver is installed (Vulkan loader typically comes with the GPU driver)
- If build fails in `build.rs` with libclang errors:
  - ensure LLVM is installed and `LIBCLANG_PATH` points to the LLVM `bin` folder

## Docs

- User manual: `docs/USER_MANUAL.md`
- Networking guide: `docs/NETWORKING.md`
- Architecture notes: `docs/ARCHITECTURE.md`
- Modules / plug-in system: `docs/MODULES.md`
- Demo modules onboarding: `docs/DEMO_MODULES.md`
- Changelog: `CHANGELOG.md`
