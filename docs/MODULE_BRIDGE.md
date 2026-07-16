# Module Bridge (Portable Status Plug)

Use the bridge when you want a module to:

- keep its own standalone UI and state
- run normally outside ChattyCog
- optionally report a short status + snapshot back to ChattyCog when hosted

This is the simplest compatibility rule:

- `manifest.json` makes the module discoverable
- `visual_load.json` lets ChattyCog host the real UI
- `bridge/status.json` lets the module tell ChattyCog what happened
- `bridge/shared_room_state.json` lets room-aware modules see the current shared-room policy when hosted
- `bridge/shared_room_events.json` lets a hosted module read recent low-latency room/session events
- `bridge/outgoing_room_events.json` lets a hosted module emit lightweight room/session events back into the LAN room
- `bridge/incoming_assets/<lane_id>/` lets ChattyCog drop approved files or binary/text payloads into declared module inbox lanes
- optional `bridge/log_sources.json` lets ChattyCog tail declared module-local logs for auto-rundown context

If you remove the bridge logic, the module still runs standalone. It just stops reporting back to ChattyCog.

## Why the bridge matters in practice

The bridge is what turns a hosted module from "a tool in a tab" into "a tool that can participate in a larger loop."

Typical pattern:

- ChattyCog's main AI helps frame the task
- a specialist module does focused work in its own real UI
- the module reports status, logs, or waiting assets through the bridge
- the orchestrator or the next module picks that work up for another pass

That is the intended shape for compound workflows such as:

- drafting a `chatty-quest` dataset template
- reviewing or generating companion media in `chatty-art`
- feeding the results into `chatty-lora` for prompt or training guidance
- looping again with the user still inside the same ChattyCog session

The bridge keeps those handoffs standardized without forcing every module to give up its own runtime or internal state model.

## Host boundary

This bridge contract is for ChattyCog-hosted modules.

- ChattyCog and Chatty-EDU intentionally use different host ecosystems
- their bridge helpers, policies, and surrounding runtime assumptions are similar in shape but not meant to be mixed casually
- if you want versions of a tool in both ecosystems, treat that as two deliberate host targets, not one shared safety boundary

That separation is by design, especially because the EDU side carries stricter classroom and child-safety expectations.

Quick copy-paste helpers:

- `docs/MODULE_BRIDGE_SNIPPETS.md`
- `docs/MODULE_BRIDGE_HELPER_RUST.rs`
- `docs/MODULE_BRIDGE_HELPER_WEBVIEW.js`

Builder preflight:

- `docs/MODULE_BUILDER_CHECKLIST.md`
- `docs/MODULE_PACKAGING_GUIDE.md`
- `docs/MODULE_REVIEW_RUBRIC.md`

## File contract

Runtime files:

```text
your_module/
  bridge/
    status.json
    shared_room_state.json
    shared_room_events.json
    outgoing_room_events.json
    incoming_assets/
      lesson_assets/
        <asset metadata>.json
        <payload file>
    log_sources.json
```

ChattyCog reads `bridge/status.json` when you leave or close a module tab.

If present, ChattyCog prefers this file over the old generic fallback snapshot flow.

If `bridge/log_sources.json` exists, ChattyCog can also tail declared module-local logs and feed their recent tail into the Bookkeeper summary flow.

## `status.json` schema

```json
{
  "module_id": "demo_meal_planner",
  "event_type": "suspend_rundown",
  "summary": "Meal Planner is 75% filled out. Budget is medium. Next handoff focus: finalize the grocery list.",
  "snapshot": "# Meal Planner Snapshot\n\n- Budget: medium\n- Busy days: Tuesday, Thursday",
  "tags": ["meal_planner", "planning", "webview"],
  "payload": {
    "budget": "medium",
    "busy_days": 2
  },
  "updated_at_unix_ms": 1774588800000
}
```

Fields:

- `module_id` - should match `manifest.json`
- `event_type` - usually `suspend_rundown`
- `summary` - short human rundown; this is the main handoff text
- `snapshot` - optional longer state dump / preview / notes
- `tags` - optional search/filter tags
- `payload` - optional free-form JSON object
- `updated_at_unix_ms` - optional timestamp in Unix milliseconds

## Optional `log_sources.json` schema

Use this when the module already has its own logging system and you want ChattyCog to use the recent log tail as debrief context.

Example:

```json
{
  "sources": [
    {
      "path": "logs/session.log",
      "label": "Session Log",
      "format": "log",
      "tail_lines": 80,
      "tail_chars": 4000
    },
    {
      "path": "logs/events.jsonl",
      "label": "Recent Events",
      "format": "jsonl",
      "tail_lines": 60,
      "tail_chars": 5000
    }
  ]
}
```

Fields:

- `path` - required, module-relative log file path
- `label` - optional friendly name shown in the context bundle
- `format` - optional hint like `log`, `txt`, `jsonl`, or `md`
- `enabled` - optional, defaults to `true`
- `tail_lines` - optional, how many recent lines to keep
- `tail_chars` - optional, final character clamp for that excerpt

Safety rules:

- ChattyCog only reads paths declared by the module
- paths must stay inside the module folder
- absolute paths and `..` traversal are ignored
- missing log files are skipped quietly

Starter file:

- `docs/MODULE_LOG_SOURCES_TEMPLATE.json`

## Native desktop modules

When ChattyCog launches a hosted native-window module, it sets:

- `CHATTYCOG_HOSTED=1`
- `CHATTYCOG_MODULE_DIR=<absolute module folder>`
- `CHATTYCOG_BRIDGE_DIR=<absolute module bridge folder>`
- `CHATTYCOG_BRIDGE_STATUS=<absolute path to bridge/status.json>`
- `CHATTYCOG_BRIDGE_SHARED_ROOM_STATE=<absolute path to bridge/shared_room_state.json>`
- `CHATTYCOG_BRIDGE_SHARED_ROOM_EVENTS=<absolute path to bridge/shared_room_events.json>`
- `CHATTYCOG_BRIDGE_OUTGOING_ROOM_EVENTS=<absolute path to bridge/outgoing_room_events.json>`
- `CHATTYCOG_BRIDGE_INCOMING_ASSETS_DIR=<absolute path to bridge/incoming_assets>`
- `CHATTYCOG_BRIDGE_LOG_SOURCES=<absolute path to bridge/log_sources.json>`

Your standalone module can ignore these unless it wants ChattyCog compatibility.

Recommended pattern:

1. Read `CHATTYCOG_BRIDGE_STATUS`
2. If present, write the JSON status file there
3. Optionally write or ship `log_sources.json` if the module wants ChattyCog to tail recent module logs
4. If absent, do nothing special

That keeps the module portable.

## Hosted webview modules

When ChattyCog hosts a `webview` module, it injects:

```js
window.chattyCogBridge.available
window.chattyCogBridge.updateStatus(payload)
window.chattyCogBridge.clearStatus()
window.chattyCogBridge.readSharedRoomState()
window.chattyCogBridge.readSharedRoomEvents()
window.chattyCogBridge.readIncomingAssets(laneId)
window.chattyCogBridge.incomingAssetUrl(laneId, payloadFileName)
window.chattyCogBridge.consumeIncomingAsset(laneId, assetId)
window.chattyCogBridge.emitRoomEvent(payload)
```

Recommended pattern:

```js
if (window.chattyCogBridge?.available) {
  window.chattyCogBridge.updateStatus({
    module_id: "your_module",
    event_type: "suspend_rundown",
    summary: "Short handoff text here.",
    snapshot: "Longer snapshot here.",
    tags: ["tag_a", "tag_b"],
    payload: { progress: 0.75 }
  });
}
```

If the module runs in a normal browser outside ChattyCog, `window.chattyCogBridge` simply will not exist, and your module still works.

## Optional `incoming_assets/` bridge inbox

This is the portable asset lane for modules that need more than tiny room events or JSON state.

Use it when the module wants to receive things like:

- lesson or workflow companion files
- small binary assets
- exported presets
- rich markdown / JSON / CSV payloads
- module-specific packs that should land in the module folder, not the global networking inbox

ChattyCog only auto-delivers a received transfer into a module lane when:

- the transfer is already scoped to that module
- exactly one declared incoming `asset_lanes[]` entry matches it
- that lane uses `delivery_mode: "bridge_inbox"`

Otherwise the transfer stays in the normal networking inbox until the user chooses a lane manually.

Each delivered asset creates:

- a small metadata JSON record in `bridge/incoming_assets/<lane_id>/`
- the original payload beside it

Hosted webviews can use:

- `readIncomingAssets(laneId)` to list waiting assets
- `incomingAssetUrl(laneId, payloadFileName)` to read the payload
- `consumeIncomingAsset(laneId, assetId)` to remove it after the module has imported or applied it

That keeps the boundary clean:

- ChattyCog fills only declared inbox lanes
- the module decides when and how to import the payload
- removing the bridge plug removes compatibility without breaking the standalone tool

## Optional `shared_room_events.json` and `outgoing_room_events.json`

These files are for modules that want a lightweight event lane in addition to the heavier shared-state lane.

Use them for things like:

- tiny multiplayer moves
- ready / waiting states
- host nudges such as "next round starting"
- short lesson-room or co-op tool signals that should not become full inbox artifacts

Recommended shape for outgoing events:

```json
{
  "events": [
    {
      "event_type": "ready_state",
      "label": "Learner ready",
      "content_type": "application/json",
      "payload_text": "{\"ready\":true}"
    }
  ]
}
```

Behavior:

- the hosted module writes or appends lightweight items to `outgoing_room_events.json`
- ChattyCog relays them across the current room/session when that module is active in the room lane
- recent incoming events are mirrored back into `shared_room_events.json`
- this lane is intentionally for **small, low-latency text payloads**, not big files or full workflow bundles

## Optional `shared_room_state.json`

This file is for modules that explicitly declare `room_aware` or `multiplayer` in `network_capabilities.json`.

ChattyCog writes it for hosted modules so they can react to the current room policy without becoming dependent on ChattyCog internals.

Use it for things like:

- showing whether the shared room is currently focused on this module
- switching between general-room and multiplayer-room behavior
- surfacing talking-stick ownership inside the module UI
- soft-disabling local AI actions when the room policy says `Host only` or `Off`

It can also carry an optional host-authoritative session layer for multiplayer or turn-based modules:

- `session_active`
- `session_id`
- `session_revision`
- `session_label`
- `host_authoritative`
- `participants[]`

That gives a hosted module a portable way to understand "there is an active room session, this is its revision, these are the current participants, and the host is authoritative" without making the module depend on ChattyCog-specific runtime code.

## Best practices

- Keep `summary` to one short paragraph
- Put richer details into `snapshot`
- If the module already has useful logs, declare them in `log_sources.json` instead of asking the user to type a handoff
- Prefer stable tags (`planning`, `research`, `training`, `handoff`)
- Update the bridge whenever state changes materially, not on every animation frame
- Treat the bridge as optional telemetry, not your main app database

## What ChattyCog does with it

When a module tab is left or closed:

- ChattyCog reads `bridge/status.json`
- it tails any declared files in `bridge/log_sources.json`
- it appends the `summary` to cold memory
- it updates department status files used by the main Chat tab
- if only a `snapshot` exists, the Bookkeeper can summarize it through its current local or cloud lane
- if the module declared recent logs, those log tails are included in the auto-generated rundown context

## Shipped examples

- `chattycog_gui/modules/demo_work_schedule/web/app.js` - room-aware + multiplayer + host-authoritative session example
- `chattycog_gui/modules/demo_meal_planner/web/app.js`
- `chattycog_gui/modules/demo_errand_coach/web/app.js`
- `chattycog_gui/modules/new_janet_school/web/app.js`
- `chattycog_gui/modules/llm_tweaker/src/main.rs`
- `chattycog_gui/modules/chattyfactory-module/chattyfactory_gui/src/main.rs`
