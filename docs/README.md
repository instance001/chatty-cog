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
- **Hot memory (sidebar)**: small, user-visible "working set" of recent/pinned items.
- **Logs tab**:
  - CPU-only **bookkeeper** that records events/messages to disk (`cold_log.jsonl`)
  - semantic search (Option B, embeddings) + keyword search fallback (Option A)
  - "Luke Warm" rolling summary persisted to `lukewarm.txt`
  - semantic search filters by `module_id` (department) and/or `tag`
  - append ad-hoc events with `module_id` + `tags` + optional `payload_json`
- **Sandbox tab**: browse/edit/save files inside `Chatty_Sandbox/`.
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
