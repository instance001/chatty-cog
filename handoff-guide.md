# ChattyCog Handoff Guide

This document defines the intended shape of the ChattyCog handoff system.

It exists to keep the ecosystem modular:

- ChattyCog is the meeting room, whiteboard, orchestrator, and shared review space.
- Modules are the departments, work areas, or lab wings where the actual domain work happens.
- The plug / bridge system is the operator wire: it carries signals, summaries, and approved file transfers.

The goal is to let modules collaborate without collapsing into one giant shared runtime or one tangled state model.

## Core model

ChattyCog should:

- orchestrate
- observe
- advise
- route approved handoffs
- digest handoff activity into its own memory/context systems

Modules should:

- own their own UI
- own their own real state
- own their own domain workflows
- remain usable outside ChattyCog

The bridge should:

- carry summaries
- carry structured snapshots when useful
- carry approved file handoffs
- avoid becoming a module's primary database

## Design rule

Loose departments, strict handoff contracts.

This means:

- modules stay separate
- handoff formats stay explicit
- the user stays in the loop
- no module should quietly tunnel into another module's internal folder layout or state model

## User-in-the-loop rule

Handoffs are user-driven, not automatic.

The expected interaction model inside modules is:

1. The user selects one or more items with checkboxes.
2. The module shows the valid actions for the current environment.
3. The user presses an explicit action button such as:
   - `Send to Chatty-lora`
   - `Send to Chatty-art`
   - `Send to ChattyCog Sandbox`
   - `Delete selected`
4. The user confirms the action.
5. ChattyCog mediates the result.

Important:

- direct handoff actions are copy-only, not move-only
- originals stay in the source module
- destructive local actions like `Delete selected` are separate and explicit

## Mediation rule

All inter-module and sandbox handoffs should be mediated by ChattyCog.

Why:

- one consistent routing path
- one permission/confirmation path
- one place to log and digest the action
- one clean architecture for future room/network/multiplayer extensions

Modules should not directly copy files into each other's internal working folders on their own.

Instead:

- a module requests a handoff
- ChattyCog validates it
- ChattyCog performs the copy
- ChattyCog writes the metadata envelope
- ChattyCog records the handoff for context/memory

## Separation of lanes

There are two major handoff categories and they should remain distinct.

### 1. Artifact handoff

This is for real files and payloads.

Examples:

- images
- video clips
- audio clips
- LoRA files
- dataset candidates
- generated plans
- exported configs

Artifact handoffs should travel through declared asset lanes or sandbox export paths.

### 2. Interpretation handoff

This is for meaning, advice, and context.

Examples:

- what the artifact is for
- how it should be used
- whether it is approved
- compatibility hints
- suggested next steps

Interpretation should travel through:

- bridge status summaries
- payload metadata
- ChattyCog memory/log systems

These two lanes should not be muddled together.

## Handoff principles

### Copy, don't move

Handoffs create a shared copy.

They do not remove the original from the source module.

### Explicit import, not silent coupling

Receiving a handoff should not silently rewrite the receiver's working state.

The receiver should see:

- what arrived
- who sent it
- why it was sent
- what it is intended for

Then the receiving module can choose how and when to import or apply it.

### Metadata is required

A handoff is not just a file copy.

Every handoff should carry a metadata envelope that explains the artifact.

Recommended fields:

- `source_module_id`
- `destination_kind`
- `destination_module_id` when relevant
- `artifact_kind`
- `label`
- `summary`
- `tags`
- `original_relative_path`
- `copied_at_unix_ms`
- `user_note`
- optional compatibility details such as:
  - model family
  - media kind
  - suggested LoRA strength
  - intended usage

## ChattyCog's role in handoffs

ChattyCog is not the workshop.

ChattyCog is the shared coordination layer.

So during handoff it should:

- validate the destination
- perform the copy
- store a metadata envelope
- update logs/context
- optionally inspect artifacts multimodally when routed into the sandbox
- optionally advise downstream work using that artifact

Examples:

- `chatty-art` creates a website banner
- the user sends it to ChattyCog Sandbox
- ChattyCog can inspect the image
- ChattyCog can advise whether it fits a website header well
- another module can later consume that sandbox-staged artifact

## Module roles

### Chatty-art

Chatty-art is the media-generation department.

Likely handoff surfaces:

- generated outputs
- selected outputs used as references

Likely outgoing actions:

- `Send to Chatty-lora`
- `Send to ChattyCog Sandbox`
- `Delete selected`

Examples:

- send generated images to Chatty-lora as dataset candidates
- send a banner or mockup image to ChattyCog Sandbox for multimodal review or downstream website-design use

### Chatty-lora

Chatty-lora is the dataset/training/LoRA-building department.

Likely handoff surfaces:

- curated dataset candidates
- generated trainer handoff files
- produced LoRA outputs

Likely outgoing actions:

- `Send to Chatty-art`
- `Send to ChattyCog Sandbox`
- `Delete selected`

Examples:

- send a trained LoRA back to Chatty-art for generation use
- send dataset examples or plan artifacts to ChattyCog Sandbox for review

## First-class destination shapes

### To another module

Used when the destination is known and module-specific.

Examples:

- `chatty_art -> chatty_lora`
- `chatty_lora -> chatty_art`

This should route through a declared module asset lane.

### To ChattyCog Sandbox

Used when the artifact should become visible to the wider ChattyCog coordination layer.

Examples:

- multimodal inspection
- orchestrator advice
- staging for future module use
- review before routing onward

The sandbox acts as a shared evidence tray / staging bench, not as the source module's main storage.

## Recommended sandbox convention

When ChattyCog receives a sandbox handoff, it should stage it in a predictable location such as:

`Chatty_Sandbox/handoffs/<source_module>/<timestamp>-<slug>/`

That folder should contain:

- copied asset files
- a `handoff.json` metadata file

This keeps sandbox handoffs explicit and reviewable.

## Recommended bridge/path roles

These roles should stay clear:

- `HANDSHAKE.md`
  Human-facing module contract and rundown expectations

- `bridge/status.json`
  "What happened here?" summary

- `bridge/shared_state.json`
  Structured state a module wants to publish outward

- `bridge/incoming_shared_state.json`
  Structured state mirrored back into the module

- `bridge/incoming_assets/<lane_id>/`
  Approved waiting artifacts for that module

- `Chatty_Sandbox/`
  Shared staging and review space for ChattyCog-level reasoning and downstream reuse

## Availability-driven UI

Send buttons should only surface when the destination is actually available.

Examples:

- if `chatty_lora` is not present, Chatty-art should not show `Send to Chatty-lora`
- if the module is not hosted inside ChattyCog, ChattyCog-specific send buttons should stay hidden

This keeps standalone mode clean and avoids fake affordances.

## Anti-cluster rules

To avoid turning the system into a cluster:

- do not let modules directly own each other's runtime state
- do not let shared state become a junk drawer
- do not silently auto-import cross-module handoffs
- do not make handoffs depend on private file-layout knowledge
- do not mix review/advice lanes with raw file-transfer lanes carelessly

## Initial rollout shape

The first intended handoff paths are:

1. `chatty-art -> chatty-lora`
   Purpose: send generated media as dataset candidates

2. `chatty-lora -> chatty-art`
   Purpose: send trained LoRAs or compatible generation assets back for use

3. `chatty-art -> ChattyCog Sandbox`
   Purpose: review generated media, multimodal inspection, or downstream module use

4. `chatty-lora -> ChattyCog Sandbox`
   Purpose: review LoRA artifacts, dataset examples, or generated training materials

## Short summary

ChattyCog is the room.
Modules are the departments.
The bridge is the wire.
Handoffs are explicit, mediated, copy-only, metadata-rich, and user-confirmed.

That separation is intentional and should be preserved as the system grows.
