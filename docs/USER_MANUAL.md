# chatty-cog User Manual (Zero-Knowledge)

This guide assumes you are starting from scratch and have never used Rust, GGUF models, or llama.cpp before.

## What is chatty-cog?

chatty-cog is a Windows desktop app that lets you chat with local AI models stored as `.gguf` files.

- It runs offline (no call-home / no cloud API required).
- It uses a local runtime based on llama.cpp.
- It includes a CPU-only Bookkeeper that stores logs and helps you search them later.
- It supports modules ("departments") via a simple drop-in folder system.
- It can optionally connect to other nearby chatty-cog instances over local Wi-Fi or LAN.
- That optional peer networking is only for chatty-cog-to-chatty-cog connections, not chatty-edu.

## Key terms

- **GGUF**: a file format for LLM weights (the model file you download).
- **Runtime**: the local engine that can run GGUF models (`llama.dll` and `ggml-*.dll`).
- **Orchestrator**: the main Chat tab (the "pilot").
- **Bookkeeper**: the CPU-only memory/logging helper (Logs tab).
- **Hot / Luke Warm / Cold memory**:
  - Hot: small, recent/pinned sidebar items in Chat
  - Luke Warm: a rolling summary of recent activity (`lukewarm.txt`)
  - Cold: the long-term log (`cold_log.jsonl`)

## Install prerequisites

1) Install Rust
- Download and install from the official Rust site.

2) Install LLVM
- Required for generating bindings (bindgen uses libclang).
- Default install path expected: `C:\Program Files\LLVM\bin`

## Folder layout (important)

Inside `chattycog_gui/`:

- `models/`
  - Put your `.gguf` model files here.
- `runtime/windows/`
  - Put `llama.dll`, `ggml.dll`, and the backend DLLs here.
  - If you want Vulkan acceleration, include `ggml-vulkan.dll`.
- `memory/`
  - `cold_log.jsonl`: long-term logs (grows over time)
  - `lukewarm.txt`: rolling summary
  - `departments.md` / `departments.json`: latest per-module debriefs ("what happened in this module")
- `modules/`
  - Drop-in modules (each needs a `manifest.json`).
- `module_templates/`
  - Copy-safe starter modules you can duplicate into `modules/`.
- `Chatty_Sandbox/`
  - A safe scratch folder you can drop files into and open from the app. Sandbox file creation is scoped to .txt and .md only. Executables and code files cannot be created through the sandbox.

## Build and run

Open a terminal in the repo and run:

```bash
cargo build --manifest-path chattycog_gui/Cargo.toml
cargo run  --manifest-path chattycog_gui/Cargo.toml --bin chattycog_gui
```

## Using the app

### Chat tab

1) Select a model
- Use the Chat header `Model` dropdown, `Open GGUF...`, or `Refresh models`.
- The active model path also appears in the app header as `GGUF: ...`.

2) Confirm runtime is loaded
- The top of the chat shows a "Runtime:" status line.
- ChattyCog may show a GPU/Vulkan path even when a small number of tensors still stay on CPU. That is still a real GPU-assisted run, not a full CPU fallback.
- The Chat header also shows the live `Chat max tokens` value for the next orchestrator reply.

3) Type a message
- Use the input box at the bottom and press Enter or click Send.
- The composer is multiline: `Enter` sends, `Shift+Enter` adds a new line.
- If a module tab is active, the orchestrator is paused and the Chat input will be disabled.
- If ChattyCog is already generating a reply, the composer switches into a `Please wait` state until the current inference finishes or you press `Interrupt`.
- Use `Interrupt` in the chat composer if you want to stop the current reply.
- If the assistant includes a reasoning trace, use the `Show thinking` / `Hide thinking` disclosure row inside the assistant bubble to expand or collapse it without losing the visible answer.

4) Adjust generation
- Open the `Preferences` tab and use `Orchestrator (Chat)`:
  - Presets: Precise / Balanced / Creative
  - Sliders: temperature, top_p, top_k, max_tokens
- These orchestrator sliders now apply to live chat immediately, so the next reply uses the new values without a separate hidden apply step.
- `Save` writes the current defaults into `config/preferences.json`.

5) Optional voice / personality capsules
- Open the `Preferences` tab and use the `Capsule Library` on the right.
- You can:
  - save a named personality / behavior capsule
  - activate it for the current chat style
  - deactivate it and return to native ChattyCog voice
- The top of the Chat tab shows the active voice state:
  - `Voice: native ChattyCog`
  - or the active capsule name plus a short preview

6) Sandbox tool requests (optional)
- The orchestrator cannot directly read/write your files.
- If it needs to read/write within `Chatty_Sandbox/`, it can request sandbox actions like `write`, `append`, `read`, `list`, `preload`, or `ledger`.
- Chatty-cog sandbox is limited to .txt and .md files.
- The Chat UI will show a "Pending sandbox actions" panel with `Seed ledger from current prompt`, `Defer actions`, `Preload + Continue`, `Approve`, `Approve + Continue`, and `Reject`.
- Only approved actions execute, and they are restricted to `Chatty_Sandbox/`.
- This feature can be enabled/disabled in the Preferences tab.
- ChattyCog now keeps a persistent scratchpad at `Chatty_Sandbox/scratchpad/current.md`.
- ChattyCog also keeps a structured task ledger at `Chatty_Sandbox/scratchpad/task_ledger.md`.
- The Chat prompt can see:
  - a compact recent chat block
  - a compact sandbox file index
  - the scratchpad contents
  - the task ledger
  - the last approved sandbox tool result

Tips for using this in plain language:
- You can ask in normal English, e.g. "Create a file `notes/todo.md` in the sandbox and write my task list in it."
- If the model doesn't trigger a sandbox request, add: "Use the sandbox tool to do it."
- For the most reliable file workflow, use the `Sandbox task` checkbox in the chat composer:
  - turn it on
  - choose `Create file` or `Edit file`
  - enter the target path, such as `notes/todo.md`
  - type the actual content request in plain language
- In sandbox task mode, ChattyCog explicitly tells the model this turn is a sandbox file job so it wastes fewer tokens trying to infer whether you meant normal conversation or file work.
- For durable working notes, say things like:
  - "Append this plan to the sandbox scratchpad."
  - "Read the scratchpad and continue from it."
  - "Use the scratchpad to keep track of intermediate notes while you work."
- For bigger tasks, you can also steer it toward a deterministic preload:
  - "Preload the scratchpad and relevant sandbox files first."
  - "Use the sandbox preload tool before you answer."
- For long multi-step tasks, you can ask for structured progress tracking:
  - "Update the task ledger before you continue."
  - "Keep the current task, next step, and files touched in the task ledger."
- ChattyCog now also tries to notice when your prompt looks multi-step. When that happens, the Chat tab can show a small task hint and the model gets a nudge to use `sandbox.preload` + `sandbox.ledger` more deterministically.
- For longer jobs, `Approve + Continue` usually gives the smoothest flow because ChattyCog can inspect the approved sandbox result and keep going without losing its place.
- The top of the Chat tab now also has sandbox quick-access buttons for `Open scratchpad`, `Open ledger`, and `Reopen last working file` so it is easier to jump back into the sandbox without digging through tabs.
- Keep requests specific:
  - exact filename (and optional subfolder), e.g. `notes/errands.md`
  - what you want written (or what file you want read)
- Approve/Reject is your safety gate:
  - `Seed ledger from current prompt` if you want the structured task record initialized before the file actions go through
  - `Defer actions` if you want to clear the current sandbox requests without treating them as wrong or running any file changes yet
  - `Preload + Continue` if you want ChattyCog to gather scratchpad / ledger / likely relevant sandbox file context first and reconsider the task before any pending write actions are run
  - Approve only actions you expect
  - Reject if the path/operation looks wrong, then rephrase
- `Approve + Continue` is the fastest workflow when you want ChattyCog to use the file result immediately after the action runs.
- `sandbox.preload` is the cleanest way to gather context for complex tasks because it can bundle:
  - the current scratchpad
  - the current task ledger
  - the sandbox file index
  - specific files you want loaded before the next reasoning step
- `sandbox.ledger` is the cleanest way to keep a deterministic task record without burying structure inside free-form scratchpad notes.
- The sandbox is locked:
  - actions are restricted to `Chatty_Sandbox/`
  - path traversal (like `..`) is blocked
  - AI-requested sandbox file actions are limited to `.txt`, `.md`, and `.markdown`
  - approved sandbox actions now reopen the touched file and focus the Sandbox tab automatically

### Modules (departments)

Open modules from the top menu:
- **Modules -> Open: <display_name>**

To start from a shipped builder template:
- if you are unsure which starter fits, read `docs/MODULE_TEMPLATE_CHOOSER.md`
- then use `docs/MODULE_BUILDER_CHECKLIST.md`
- when you are ready to share it, use `docs/MODULE_PACKAGING_GUIDE.md`
- do a final self-check with `docs/MODULE_REVIEW_RUBRIC.md`
- if you are publishing an update, use `docs/MODULE_RELEASE_NOTES_TEMPLATE.md`
- if the module will keep evolving, start a `CHANGELOG.md` from `docs/CHANGELOG_TEMPLATE.md`
- if you are handing it to another person or team, use `docs/MODULE_SUBMISSION_TEMPLATE.md`
- copy `chattycog_gui/module_templates/template_module/`
- or copy `chattycog_gui/module_templates/template_native_rust_module/`
- or copy `chattycog_gui/module_templates/template_python_module/`
- paste it into `chattycog_gui/modules/`
- rename the copied folder
- update the copied module files
- use **Modules -> Rescan modules**

Close a module tab:
- Click the `x` next to the module tab name.

What you do inside a module tab depends on the module, but the general pattern is:
- Fill out the module's **Workspace** (either a form from `ui.json` or a template-backed editor).
- Optionally use any module-specific tools/AI runner the module provides.
- Leave the module (switch tabs): ChattyCog debriefs the Bookkeeper so the Chat tab stays aware of what happened.

Some modules can now open their **real standalone window inside the tab**.

When a module includes `visual_load.json`:
- ChattyCog can dock that module's own UI in the tab.
- This can be either a real native app window or a hosted browser-style webview.
- You may see buttons like **Build UI**, **Launch in tab**, or **Restart UI**.
- The usual ChattyCog-side state/rundown helpers move under a collapsed **ChattyCog bridge** panel.
- The module still remains its own tool. ChattyCog is just hosting it, not replacing its internal UI/state model.

Plain-language version:
- some modules are real desktop apps, so ChattyCog docks the real app window into the tab
- some modules are real browser dashboards, so ChattyCog hosts that real web UI in the tab
- some modules do not have their own GUI, so ChattyCog gives them a built-in workspace surface instead

So if one module looks like a "native app" and another looks more like a dashboard or form, that does **not** automatically mean one is real and the other is fake.

Usually it means:
- the first module is a desktop-window module
- the second module is a browser-style module

The true fallback / middle-ground UI is only used when the module does not ship its own hosted surface.

Why ChattyCog works this way:
- it lets desktop apps stay desktop apps
- it lets browser tools stay browser tools
- it still gives CLI/headless tools a usable in-tab home
- it keeps modules portable, so they can still run outside ChattyCog

Tip:
- If a hosted module says the launch target is missing, use **Build UI** first or build the module once from its own folder.
- If a hosted webview module fails on Windows, make sure Microsoft Edge WebView2 is available.
- The shipped modules now use this too, so what you see in a module tab should feel much closer to the module's own intended surface than the old generic placeholder view.

#### "What happened in this module" automation (recommended)

If **Preferences -> Auto-generate module suspend rundown on tab leave (Bookkeeper)** is enabled:
- When you leave a module tab, ChattyCog first checks whether the module reported its own bridge status.
- If the module wrote `bridge/status.json`, ChattyCog uses that handoff first.
- If only a richer bridge snapshot exists, the CPU-only Bookkeeper summarizes it.
- If the module declared `bridge/log_sources.json`, ChattyCog also reads the recent tail of those module-local logs and uses that as extra context.
- If no bridge status exists, ChattyCog falls back to its own form/workspace snapshot (plus last module AI output if any).
- The CPU-only Bookkeeper generates a short "suspend rundown" automatically.
- The latest per-module rundowns are written to:
  - `chattycog_gui/memory/departments.md`
  - `chattycog_gui/memory/departments.json`
- The Chat tab injects `departments.md` into the orchestrator system prompt as "Department Status Updates".

Note: this is asynchronous, so it can take a few seconds. If you switch back to Chat and ask a cross-module question immediately, wait a moment and try again.

What this means in plain language:
- the module can keep behaving like its own standalone tool
- ChattyCog just reads the module's handoff note if the module chooses to provide one
- if a builder removes that bridge plug, the module still runs, but ChattyCog loses that extra context loop

#### Compound workflow loops

One of the most powerful ChattyCog patterns is using the orchestrator, sandbox, and hosted modules as a loop instead of treating each tab as a disconnected tool.

What that means in practice:

- the Chat tab helps plan the next move
- the sandbox holds durable notes, prompts, templates, and task state
- a hosted module keeps the real tool UI visible inside the app
- the bridge or suspend rundown carries "what happened" back into the next step
- the next tool starts with better context than the previous one had

Example flywheel:

1. Ask ChattyCog to help draft a dataset template for a `chatty-quest` modding pack.
2. Save or refine that structure in `Chatty_Sandbox/` so the plan survives more than one reply.
3. Open a hosted `chatty-art` module and generate candidate media while keeping the overall goal in view.
4. Return to the Chat tab and ask the orchestrator to review what worked, what failed, and what should change.
5. Move into a `chatty-lora` lane for training-plan advice, prompt improvements, or dataset-shape cleanup.
6. Feed those adjustments back into the next `chatty-quest` or `chatty-art` pass.

That creates a practical loop:

- plan
- generate
- inspect
- refine
- hand off
- repeat

Why this is useful:

- you do not need to keep minimizing out to separate desktop tools
- the sandbox can keep intermediate notes and working structure grounded
- the module bridge gives the next step something better than "start from nothing"
- ChattyCog becomes the connective tissue between specialized tools, not just another chat window

### Preferences tab (formerly Models)

The Preferences tab stores and applies defaults across ChattyCog and modules.

- Preferences file: `config/preferences.json`
- Orchestrator defaults: temperature/top_p/top_k/max_tokens
- Bookkeeper defaults: temperature/top_p/top_k/max_tokens
- Access/tools:
  - Toggle "Allow sandbox tool requests (user-approved)"
  - Toggle "Auto-generate module suspend rundown on tab leave (Bookkeeper)"
- Per-module defaults:
  - Preferred model filename (GGUF)
  - Default generation params
  - Allow receiving Luke Warm context (reserved for future module runners)
- Capsule library:
  - saved named personality / behavior injections
  - active capsule selection
  - editor for creating, updating, or deleting capsules

### Networking tab

ChattyCog can optionally connect to other nearby ChattyCog instances on the same local Wi-Fi or LAN.

Important boundary:
- this peer-to-peer mode is only for other ChattyCog instances
- Chatty-EDU devices are intentionally incompatible peer targets, even on the same network

Use it when you want:
- one ChattyCog machine to be visible to another nearby one
- lightweight shared status between local peers
- short handoff notes between connected devices
- portable workflow-bundle sharing between nearby instances
- module-specific shared workflow state through a module bridge

How to use it:
1. Open the `Network` menu or the `Networking` tab.
2. On one machine, turn on `Make available for connectivity`.
3. On another machine, click `Refresh discovery`.
4. When the other machine appears, click `Connect`.
5. Use the handoff panel if you want to send a short local note.

Helpful everyday controls:
- Click a device name to rename it locally so repeated/default names are easier to tell apart.
- Click the group chip (or `+ Group`) to tag a device for your own workflow.
- Click `Trust` when a nearby machine is one you expect to use regularly.
- Use `Export trusted list` if you want to carry your remembered pairings to another ChattyCog install.
- Use `Import trusted list` on the other machine so you do not have to rebuild that pairing list by hand.
- Use `Export blocked list` if another ChattyCog install should inherit the same deny rules.
- Use `Import blocked list` on that machine when you want block policy to travel cleanly too.
- Use `Select Connected` for the fastest "act on the live set" workflow.
- Use the `Find` box to search by name, ID, address, or group label.
- Use `Copy ID` or `Copy info` when you need to confirm exactly which device is which.
- If you are hosting a shared room and need to restart, look for `Resume saved session` in Networking when you come back.
- If you want another live peer to take over the room cleanly, select it and use `Hand off host to selected peer`.
- If the current host drops out, the remaining peers can use `Take over as host` instead of rebuilding the room from scratch.
- If the room was hosting a module session, use `Restore state to bridge` after recovery to put the last cached `shared_state.json` back where the hosted module expects it.
- Use `Re-share latest state` when rejoining peers need the host's last known good module session state again.
- `Replay cached assets` is the companion lane for module-linked files/assets as that recovery path fills out.
- Device IDs now stay stable across restarts, so renamed peers, group labels, and block decisions keep pointing at the same nearby machine.

Approval choices:
- `Allow` = approve this peer for the current running session
- `Trust` = remember this peer by stable device ID so future joins are approved automatically
- `Block` = deny that peer until you unblock it later

What else can travel across the local network:
- **Short handoffs** -> quick human notes for another nearby ChattyCog instance
- **Workflow bundles** -> whole-app setup sharing
- **Shared workflows** -> module-specific state shared through a module bridge

Transport support in the current build:
- text, JSON, and Markdown transfers
- chunked larger payloads up to **8 MiB**
- binary/file-style payloads for future modules and tools
- final delivery acknowledgement with retry, so transfers are not just "fire and hope"

#### Workflow bundles

Workflow bundles are the generic "share my setup" lane.

Use them when you want to send another nearby ChattyCog instance things like:
- the current system prompt
- orchestrator model hint
- bookkeeper model hint
- orchestrator and bookkeeper generation settings
- sandbox permission toggle
- auto-rundown toggle
- per-module AI preferences

How the flow works:
1. connect to the nearby ChattyCog device
2. open the `Networking` tab
3. fill in an optional bundle title and summary
4. click `Send current setup to selected peers`
5. on the receiving side, preview the bundle in **Received workflow bundles**
6. click `Apply bundle now` only if you want that setup on this machine

Why this is separate from module sharing:
- a workflow bundle is for the overall ChattyCog setup
- a shared workflow is for one specific module
- a handoff is just a note

So the three lanes stay clean instead of getting mashed together.

What this does **not** mean:
- it is not cloud sync
- it is not internet chat
- it is not required for local inference

Plain-language version:
- normal ChattyCog stays fully local on one machine
- networking just lets nearby ChattyCog instances talk to each other when you choose to allow it
- custom names and group labels are just local list-management helpers on your machine
- received workflow bundles and shared module workflows land in inboxes first, so you stay in control of when they apply

### Logs tab

The Logs tab contains the Bookkeeper and your on-disk logs.

- Luke Warm summary shows in the left sidebar.
- Bookkeeper (CPU):
  - Select a small model for embeddings/summarization.
  - It auto-restarts after changes (debounced).
- Search:
  - Semantic search: asks the Bookkeeper to retrieve similar items
    - Optional filters: `module_id` (department) and/or `tag`
  - Keyword search: searches `cold_log.jsonl` directly
- New cold-log event (schemaless):
  - You can append a "module event" directly from the Logs tab by filling:
    - Module/Dept (stored as `module_id`)
    - Type (stored as `event_type`)
    - Tags (comma-separated)
    - Summary (short human text)
    - Payload JSON (optional free-form JSON string)
- Log Folder pane:
  - Click `cold_log.jsonl` or `lukewarm.txt` to preview.

### Sandbox tab

Use this to inspect, draft, and maintain durable working notes locally.

- Scratchpad card:
  - `Open default scratchpad`
  - `Append hot memory snapshot`
- Task Ledger card:
  - `Open task ledger`
  - `Seed from current context`
- Editor pane:
  - shows a read-only `Task Ledger Snapshot` above the editor so you can keep the current task, next step, open questions, and files touched visible while you work
  - editor toolbar includes `Append summary to hot memory` for quickly surfacing the current editor buffer back into the chat-side working memory
  - editor toolbar includes `Use as current task` and `Use as next step` for quickly promoting working text into the ledger's main structured fields
  - editor toolbar includes `Promote to scratchpad` and `Promote to ledger notes` so useful draft text can be moved into durable memory without manual copy/paste
- Multi-step prompts can trigger a small task hint in the Chat tab, nudging ChattyCog toward preload + ledger use when the job looks complex enough to benefit from structured tracking.
- Left pane: recursive file list from `Chatty_Sandbox/`
- Right pane: editor
- Buttons:
  - New scratch
  - Save
  - Save as...

Note:
- The sandbox tool system supports subfolders (e.g. `notes/todo.md`). The Sandbox tab now shows recursive files, so nested notes and scratchpad folders should appear directly in the file list.

## Common problems

### "Runtime locate error" / "Runtime load error"

Check:
- `chattycog_gui/runtime/windows/llama.dll` exists
- `chattycog_gui/runtime/windows/ggml.dll` exists
- you kept the runtime DLLs together in the same folder

### Vulkan / GPU runs out of memory (OOM)

Symptoms:
- The chat shows errors like `llama_decode failed`, `Failed to create context`, or `Failed to load model`.
- The terminal may mention `ErrorOutOfDeviceMemory`, `failed to allocate Vulkan0 buffer`, or `ggml_vulkan`.

What ChattyCog does:
- It tries Vulkan/GPU first.
- If that fails, it retries with reduced GPU offload.
- If that still fails, it falls back to CPU-only so the app keeps working.

Tips:
- Try a smaller model (or a smaller quant like Q4).
- Close other GPU-heavy apps (games, browsers with lots of tabs, screen capture, etc.).
- If you opened a GPU-powered module, close it before using Chat (modules and Chat may compete for VRAM).
- If the model loads but fails on the first reply, lower `max_tokens` and retry.
- Restarting the app clears any leaked/fragmented allocations.

### Sandbox tool requests don't create files

Checklist:
- Confirm **Preferences -> Allow sandbox tool requests** is enabled.
- Ask in plain language, but explicitly require the tool format:
  - "Output only the `sandbox.write` JSON object."
- Approve the pending action in the Chat tab (nothing runs until you click Approve).
- Verify in **Sandbox** tab or via **Open Folder**.

Safety note:
- Treat any "I created the file" claim as untrusted until you see the pending approval + a successful sandbox status line.

### Chat replies still stop too early

Checklist:
- Look at the Chat header and confirm `Chat max tokens` is not still low.
- Open `Preferences -> Orchestrator (Chat)` and confirm `max_tokens` is where you expect it.
- Save preferences if you want the higher value to persist into the next launch.
- If the reply is extremely repetitive, interrupt it and retry after simplifying the active capsule or prompt.

### Module debriefs are not appearing in Chat

Checklist:
- Confirm **Preferences -> Auto-generate module suspend rundown on tab leave (Bookkeeper)** is enabled.
- Confirm the Bookkeeper is running (Logs tab) and has a model selected.
- Leave the module tab (switch back to Chat) and wait a few seconds.

### Networking cannot find another ChattyCog device

Checklist:
- Confirm both machines are running ChattyCog, not Chatty-EDU.
- Confirm both are on the same trusted local Wi-Fi or LAN.
- Confirm at least one machine has **Make available for connectivity** turned on.
- Click **Refresh discovery** on the other machine.
- If needed, allow ChattyCog through the Windows firewall on trusted local networks.
- Check the `Compatibility note` line in the Networking tab if one machine is visible but refuses to talk cleanly.
- If that note mentions protocol/version mismatch, update the older ChattyCog copy so both sides are on a reasonably matching build generation.

If you can see several devices but cannot tell which is which:
- click `Copy info` on each candidate and compare the device IDs / addresses
- rename the ones you use often so they stay recognizable next time
- add short group labels if you want your own quick visual sorting

If two nearby machines used to connect before but now show a compatibility note:
- one of them may still be on an older local build from before the chunked-transfer upgrade
- rebuild or update the older copy so both sides are on the same networking generation

If the other machine is Chatty-EDU instead:
- do not treat that as a version mismatch
- ChattyCog peer networking and Chatty-EDU peer networking are intentionally separate and will not interoperate

### The app closes immediately

If you launched from a terminal, the last lines often contain the reason.
Common causes include:
- runtime DLL missing or mismatched

You can also run with a backtrace:
```bash
set RUST_BACKTRACE=1
cargo run --manifest-path chattycog_gui/Cargo.toml --bin chattycog_gui
```

## Privacy / offline behavior

ChattyCog is designed to run locally.
Internet access is not required for inference and can be blocked at the OS firewall level if desired.
Optional local peer networking is LAN-only, off by default, and only used when you deliberately enable it.
