const STORAGE_KEY = "chattycog.demo_meal_planner.webview.v1";

const fieldIds = [
  "diet_constraints",
  "budget_level",
  "weeknight_cooking_limit_min",
  "busy_days",
  "meal_plan_draft",
  "grocery_list_draft",
  "prep_plan",
  "artifacts",
];

const fields = Object.fromEntries(fieldIds.map((id) => [id, document.getElementById(id)]));

function loadState() {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (!raw) return {};
    return JSON.parse(raw);
  } catch {
    return {};
  }
}

function saveState() {
  const state = {};
  for (const [id, element] of Object.entries(fields)) {
    state[id] = element.value ?? "";
  }
  localStorage.setItem(STORAGE_KEY, JSON.stringify(state));
  refreshDerivedUi();
}

function resetState() {
  localStorage.removeItem(STORAGE_KEY);
  for (const element of Object.values(fields)) {
    element.value = "";
  }
  refreshDerivedUi();
}

function refreshDerivedUi() {
  const busyDays = splitMeaningfulLines(fields.busy_days.value);
  const filledCount = fieldIds.filter((id) => fields[id].value.trim().length > 0).length;
  const completion = Math.round((filledCount / fieldIds.length) * 100);
  const limit = fields.weeknight_cooking_limit_min.value.trim();
  const snapshot = buildSnapshotPreview(busyDays);

  document.getElementById("busy-day-count").textContent = String(busyDays.length);
  document.getElementById("completion-score").textContent = `${completion}%`;
  document.getElementById("limit-display").textContent = limit ? `${limit} min` : "-";
  document.getElementById("snapshot_preview").value = snapshot;
  updateBridgeStatus({ busyDays, completion, limit, snapshot });
}

function buildSnapshotPreview(busyDays) {
  return [
    "# Meal Planner Snapshot",
    "",
    `- Budget: ${fields.budget_level.value || "not set"}`,
    `- Weeknight limit: ${fields.weeknight_cooking_limit_min.value || "not set"} min`,
    `- Busy days: ${busyDays.join(", ") || "none listed"}`,
    "",
    "## Meal Plan",
    fields.meal_plan_draft.value.trim() || "(empty)",
    "",
    "## Grocery List",
    fields.grocery_list_draft.value.trim() || "(empty)",
    "",
    "## Prep Plan",
    fields.prep_plan.value.trim() || "(empty)",
    "",
    "## Artifacts",
    fields.artifacts.value.trim() || "(none)",
  ].join("\n");
}

function splitMeaningfulLines(value) {
  return value
    .split(/\r?\n/)
    .map((line) => line.trim())
    .filter(Boolean);
}

function restoreState() {
  const state = loadState();
  for (const [id, element] of Object.entries(fields)) {
    if (typeof state[id] === "string") {
      element.value = state[id];
    }
    element.addEventListener("input", saveState);
    element.addEventListener("change", saveState);
  }
  refreshDerivedUi();
}

function updateBridgeStatus({ busyDays, completion, limit, snapshot }) {
  if (!window.chattyCogBridge?.available || typeof window.chattyCogBridge.updateStatus !== "function") {
    return;
  }
  const summary = [
    `Meal Planner is ${completion}% filled out.`,
    `Budget is ${fields.budget_level.value.trim() || "not set"} and weeknight cooking limit is ${limit || "not set"} minutes.`,
    `${busyDays.length} busy day(s) are tracked.`,
    `Next handoff focus: ${fields.meal_plan_draft.value.trim() ? "finalize the meal plan draft" : "draft the meal plan"}.`
  ].join(" ");
  window.chattyCogBridge.updateStatus({
    module_id: "demo_meal_planner",
    event_type: "suspend_rundown",
    summary,
    snapshot,
    tags: ["meal_planner", "planning", "webview"]
  });
}

function initTabs() {
  const tabs = document.querySelectorAll(".tab");
  const panels = document.querySelectorAll(".tab-panel");
  tabs.forEach((tab) => {
    tab.addEventListener("click", () => {
      const selected = tab.dataset.tab;
      tabs.forEach((item) => item.classList.toggle("active", item === tab));
      panels.forEach((panel) => {
        panel.classList.toggle("active", panel.dataset.panel === selected);
      });
    });
  });
}

document.getElementById("save-draft").addEventListener("click", saveState);
document.getElementById("reset-draft").addEventListener("click", resetState);
document.getElementById("refresh-preview").addEventListener("click", refreshDerivedUi);

restoreState();
initTabs();
