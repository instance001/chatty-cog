# chatty-cog

Offline-first, tabbed desktop GUI for chatting with local **GGUF** models via a bundled **llama.cpp** runtime on Windows.

chatty-cog does not require internet or cloud services to function. It can also optionally connect to other nearby chatty-cog instances over local Wi-Fi or LAN when a user deliberately enables local networking.

That peer-to-peer mode is only for chatty-cog-to-chatty-cog connections. It is intentionally incompatible with chatty-edu networking.

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
  - peer networking is intentionally limited to other ChattyCog instances, not Chatty-EDU devices
- **Modules**:
  - scan `modules/*/manifest.json`, open modules from the **Modules** menu into closable tabs
  - module state surfaces (`ui.json` form -> `state.json`, or `STATE_TEMPLATE.md`/`workspace.md`)
  - auto "suspend rundown" (Bookkeeper-generated) on tab leave, used as cross-module context
  - optional module shared-state transfer stays separate from the generic workflow-bundle lane

## Compounding workflow loops

One of ChattyCog's strongest use cases is not just "open a module in a tab."
It is using the orchestrator, sandbox, hosted module tabs, and handoff lanes together to build a compounding workflow loop without bouncing back out to the desktop.

In practical terms, that means:

- the main ChattyCog AI can help you plan the next move
- the sandbox can hold working notes, templates, prompts, and task ledgers
- a hosted module can keep its own real dashboard or native UI open in a tab
- the bridge or module rundown can feed back "what happened" into the next step
- you can iterate across tools while staying inside one window

Example flywheel:

1. Use ChattyCog to help draft a dataset template or content structure for a `chatty-quest` modding pack.
2. Keep those notes in `Chatty_Sandbox/` so the orchestrator can refine them over multiple turns.
3. Open a hosted tool module such as `chatty-art` in a tab and generate candidate media without minimizing to desktop.
4. Have ChattyCog inspect or discuss those outputs through the hosted surface, sandbox notes, and module handoff context.
5. Move to a `chatty-lora` module lane and ask for training-plan advice, dataset cleanup ideas, or prompt adjustments based on what just worked or failed.
6. Feed the improved prompts, media decisions, or training notes back into the next `chatty-quest` or `chatty-art` pass.

That creates a flywheel:

- plan
- generate
- inspect
- adjust
- hand off
- repeat

Why ChattyCog is useful here:

- the orchestrator can keep the wider goal in view
- the sandbox gives you a durable local working area
- hosted module tabs keep the real tool surfaces in front of you
- suspend rundowns and bridge status give the next step usable context
- the whole loop can stay inside the app instead of becoming a desktop juggling act

This is especially useful for "compound creation" work where one tool's output becomes another tool's input:

- game modding datasets
- media generation loops
- LoRA training preparation
- prompt refinement across tools
- iterative asset pipelines

ChattyCog does not magically collapse those tools into one monolith.
What it does is give them a shared working surface, a shared handoff language, and a shared iteration loop.

## Troubleshooting

- If chat replies still feel cut off too early:
  - confirm the chat header `Chat max tokens` value is what you expect
  - the Preferences tab now updates live chat settings immediately, and the next Save will persist them to `config/preferences.json`
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
