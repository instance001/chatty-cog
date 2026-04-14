# ChattyCog Local Networking

This page explains ChattyCog's optional local networking feature in plain language.

## What it is

ChattyCog can optionally connect to other **nearby ChattyCog instances on the same local Wi-Fi or LAN**.

This is for:
- discovering other local ChattyCog devices
- connecting nearby instances together
- sharing lightweight presence information
- sending short handoff notes between connected peers
- sending generic workflow bundles so one ChattyCog instance can share its current setup with another
- giving nearby devices clearer local names and group labels so you can tell them apart quickly

This is **not** a cloud feature.

## What it is not

ChattyCog local networking is **not**:
- internet sync
- cloud accounts
- remote telemetry
- hidden background data export
- a requirement for normal inference

If you never turn it on, ChattyCog still works normally as a single-machine offline app.

## Why it exists

The idea is simple:
- one ChattyCog machine can host
- another nearby ChattyCog machine can connect
- they can pass short local handoffs or share lightweight status

That makes it easier to coordinate nearby devices without needing outside services.

Examples:
- one workstation hands a short brief to another
- one local instance says what it is currently working on
- one device acts as the visible host for a nearby local workflow
- one local instance shares a reusable Chat tab setup with another nearby machine

## How to use it

1. Open the `Network` menu or the `Networking` tab.
2. On one machine, turn on `Make available for connectivity`.
3. On another machine, click `Refresh discovery`.
4. When a device appears, click `Connect`.
5. Use the handoff panel if you want to send a short note.

For setup-sharing workflows, the common pattern is:
1. connect the nearby ChattyCog devices
2. prepare the source machine the way you want it
3. send a workflow bundle from the Networking tab
4. let the receiving machine preview it in the bundle inbox
5. apply it only when you are ready

What gets shared:
- device name
- active tab label
- runtime status
- selected model label
- optional short handoff text

What can also be sent deliberately:
- **workflow bundles** -> received into a setup inbox first
- **module shared state** -> received into a workflow inbox tied to a specific module
- **session events** -> lightweight room/module signals for multiplayer or room-aware modules
- **chunked file-style transfers** -> the transport lane can now carry larger text or binary payloads for future modules and tools

That split matters:
- workflow bundles are for whole-app setup
- shared module state is for one specific module
- short handoffs are just notes

Nothing here is meant to silently overwrite another machine.

## Managing multiple local devices

When you only have one or two nearby machines, the default device names are usually fine.

When you have several local ChattyCog instances around at once, the networking tab is easier to use if you:
- rename devices to something human-readable
- add short group labels for your own workflow
- use the search box to narrow the list quickly

Examples:
- `Office PC`
- `Bench Laptop 2`
- `Writer Station`
- group labels like `Research`, `Ops`, or `Testing`

This is why ChattyCog now lets you:
- click a device name to set a custom local alias
- click the group chip to set or clear a local group label
- click `Trust` to remember a nearby machine by its stable device ID
- search by device name, device ID, address, or group label

Important note:
- aliases and group labels are **local preferences on your machine**
- they exist to make your device list easier to manage
- they do **not** turn ChattyCog into an account system or cloud directory
- ChattyCog now keeps a **stable local device ID** across restarts, so aliases, blocked peers, and shared-room roles stay attached to the same nearby machine

## Allow vs Trust vs Block

When `Allow unknown devices` is turned off, new peers ask first.

You now have three distinct choices:
- **Allow** -> approve this peer for the current running session
- **Trust** -> remember this peer's stable device ID so future connections are approved automatically
- **Block** -> remember a deny rule for that peer until you unblock it

That split is intentional:
- `Allow` is the lightweight "yes, for now" option
- `Trust` is the nearby-machine pairing option
- `Block` is the stronger "do not let this device back in" option

Trusted peers appear in their own section in the Networking tab, so you can review and remove remembered pairings later without blocking the device.

You can also now:
- click `Export trusted list` to save your remembered pairings to a portable JSON file
- click `Import trusted list` to load that list on another ChattyCog machine
- click `Export blocked list` to carry your deny rules to another ChattyCog machine
- click `Import blocked list` when you want that machine to inherit the same blocked-peer policy

That is handy when you are setting up a few repeat devices and do not want to rebuild the trust list by hand on every install.

## Host handoff and session recovery

Shared-room and module-room sessions now keep a lightweight **recoverable host snapshot** on the current host machine.

- If the host restarts, open the Networking tab and use `Resume saved session`.
- If the current host intentionally wants another peer to take over, select that connected peer and click `Hand off host to selected peer`.
- If the host disappears unexpectedly, other peers will see `Current room host appears offline` and can choose `Take over as host`.

This is deliberately manual and low-magic:
- sessions do not silently jump hosts behind your back
- recovery only arms on the current host
- handoff keeps the room/session identity stable while moving control cleanly

Recovery now also keeps the **latest module session bridge state** alongside room ownership:
- `Restore state to bridge` rewrites the last cached `shared_state.json` back into the hosted module bridge after a restart
- `Re-share latest state` pushes that last known good module session state back out to selected peers, or to the room if nothing is selected
- `Replay cached assets` is the companion lane for future module-tagged files/assets that belong to the same host-owned module session

That means recovery is no longer just “who owns the room?” It also helps the host rehydrate the module’s last good session state before the room carries on.

## Everyday workflow

For a simple day-to-day local setup:
1. Turn on `Make available for connectivity` on the machine you want visible.
2. Click `Refresh discovery` on the other machine.
3. Rename any repeated/default-looking devices so they are easier to recognize later.
4. Add group labels if you want quick visual organization.
5. Use `Select Connected` when you want to act on the current active peer set quickly.
6. Use `Copy ID` or `Copy info` if you need to confirm which machine is which.
7. If a machine is one you use regularly, click `Trust` so it reconnects cleanly even after both apps restart.
8. Stable device IDs now survive restarts, so your local naming and trust decisions do not wobble every launch.
9. If you want another ChattyCog install to inherit the same remembered pairings, export the trusted list and import it there.
10. If you want another ChattyCog install to inherit the same deny rules too, export the blocked list and import it there.

This is especially helpful when several nearby instances are online at once and the default names start to blur together.

## The three networking lanes

ChattyCog now has three separate local networking lanes on purpose:

- **Handoff lane**  
  For short notes between nearby peers.

- **Workflow bundle lane**  
  For whole-app setup sharing. A bundle can carry things like the current system prompt, model hints, generation settings, sandbox policy, and per-module AI preferences.

- **Module shared-state lane**  
  For a specific installed module. This uses the module bridge and lets a module share its own exported state without turning that module into a ChattyCog-owned runtime.

- **Session-event lane**  
  For fast, lightweight room-aware or multiplayer module signals such as ready states, turn nudges, tiny moves, or other small session updates. These are mirrored into `bridge/shared_room_events.json` for hosted modules instead of going through the heavier inbox/apply path.

Why we keep them separate:
- it keeps quick notes, app-wide setup, and module-specific state from getting muddled together
- it makes preview and apply flows clearer
- it helps preserve module portability

Important boundary:
- workflow bundles do **not** replace module-specific shared state
- module shared state does **not** replace the generic bundle lane
- both land in inboxes first so a person still decides when to apply them

## Transfer support and limits

The local transport layer is now built for more than tiny notes.

Current practical support:
- plain text transfers
- JSON and Markdown transfers
- chunked larger text payloads
- binary/file-style payloads for future modules and tools
- lightweight room/session event messages for fast module-state nudges

Current limits in this build generation:
- maximum decoded payload size: **8 MiB**
- chunk size: **64 KiB** per packet
- retry window: **up to 3 send attempts** waiting for final delivery acknowledgement

What this means in everyday terms:
- the current homework, revision, workflow-bundle, luke-warm, and shared-state lanes still work the same way
- larger lesson packs or future module payloads do not have to fit into one tiny packet anymore
- binary payloads are supported by the transport, even if today's built-in inbox screens mostly show metadata unless a feature or module explicitly knows what to do with the file

What we are **not** trying to be here:
- a public-internet file sync service
- a real-time cloud stream
- a huge asset CDN

So the target is "practical local-room transfers a future dev would reasonably expect," not unbounded streaming.

## Important boundaries

- This is **local-only** peer networking.
- It is **off by default**.
- It is intended for **nearby trusted local networks**, not public internet use.
- ChattyCog and Chatty-EDU use different local networking identifiers, so they do **not** accidentally cross-connect.

## Offline promise, clarified

ChattyCog is still best described as:

> local-first, no-cloud, no-calls-home

That remains true because:
- inference does not require internet
- the app ships with no cloud API requirement
- local networking is optional and user-triggered
- data stays local unless a user explicitly enables local peer connectivity
- workflow bundles and shared module states are saved to local inboxes first, rather than auto-applying in the background

## Technical notes

- discovery uses local UDP broadcast
- ChattyCog uses its own local protocol identifier
- discovery currently uses local port `45831`
- peer connections use a dynamically chosen local TCP port
- received networking inboxes live under `chattycog_gui/network_inbox/`
  - `workflow_bundles/`
  - `workflow_states/`

Depending on Windows firewall settings, you may need to allow ChattyCog on trusted local networks.

If nearby peers still do not connect cleanly:
- check the `Compatibility note` line in the Networking tab
- make sure both machines are running reasonably matching ChattyCog builds
- older local builds from before the chunked-transfer upgrade will show up as incompatible until they are rebuilt or updated
- remember that ChattyCog and Chatty-EDU intentionally use different local protocols and will not interconnect

## When to leave it off

Leave networking off if:
- you only use one machine
- you want the strictest single-device setup
- the machine should stay fully self-contained
- local firewall policy should stay as locked down as possible

## Good mental model

- **normal ChattyCog** = one fully local machine
- **network-enabled ChattyCog** = still local, but able to talk to nearby ChattyCog peers when you choose to allow it
- **renamed / grouped peers** = still the same local machines, just easier for you to recognize and manage
