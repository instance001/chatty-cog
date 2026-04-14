# Errand Coach (AI Demo) - Handshake

## Module identity (required)

- **module_id**: `demo_errand_coach`
- **display_name**: `Errand Coach (AI Demo)`

## What this module is for (required)

Turn a messy list of errands and constraints into a simple, ordered plan (with time estimates and "next step" guidance).

## Inputs this module expects (required)

- Your errands (one per line)
- Constraints (time window, budget, travel limits)
- Starting location (optional)
- Your energy level (low/medium/high)

## Outputs this module produces (required)

- A sorted checklist (best order)
- Estimated time per item
- A "do this first" action

## Suspend rundown template (required)

> **Status:** Errand plan draft created/updated.
> **What changed:** Added/removed errands, constraints clarified, order adjusted.
> **Open questions:** Confirm any unknown store hours or distances.
> **Next action:** Start with the first item and re-plan if time/budget changes.
> **Artifacts:** (optional) `Chatty_Sandbox/errands.md`

