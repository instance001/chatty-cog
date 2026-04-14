# Meal Planner (Demo) - Handshake

## Module identity (required)

- **module_id**: `demo_meal_planner`
- **display_name**: `Meal Planner (Demo)`

## What this module is for (required)

Create a simple meal plan and grocery list that fits your schedule (busy days, prep windows) and your food constraints.

## Inputs this module expects (required)

- Dietary constraints (allergies, preferences)
- Budget level (low/medium/high)
- Cooking time limits (weekday vs weekend)
- Equipment constraints (stove/oven/air fryer)
- Number of people and leftovers preference
- Any "busy days" pulled from the Work Schedule module

## Outputs this module produces (required)

- A 3-7 day meal plan
- A consolidated grocery list
- A prep plan (what to batch-cook and when)

## Operating rules / preferences (optional)

- Tone: concise
- Risk level: low
- Default tags to use in logs: meals, groceries, prep

## Suspend rundown template (required)

> **Status:** Meal plan draft and grocery list are updated for the current schedule.
> **What changed:** Busy days were handled with low-prep meals; prep windows were assigned.
> **Open questions:** Confirm any missing dietary constraints or budget limits.
> **Next action:** Do a quick pantry check before shopping.
> **Artifacts:** (optional) `Chatty_Sandbox/meals.md`

