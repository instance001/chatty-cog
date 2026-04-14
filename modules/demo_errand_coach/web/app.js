const STORAGE_KEY = "chattycog.demo_errand_coach.webview.v1";
const fieldIds = ["errands_dump", "constraints", "time_budget_min", "checklist_output"];
const fields = Object.fromEntries(fieldIds.map((id) => [id, document.getElementById(id)]));

function loadState() {
  try {
    return JSON.parse(localStorage.getItem(STORAGE_KEY) || "{}");
  } catch {
    return {};
  }
}

function meaningfulLines(value) {
  return value.split(/\r?\n/).map((line) => line.trim()).filter(Boolean);
}

function refreshDerivedUi() {
  const errands = meaningfulLines(fields.errands_dump.value);
  const checklist = meaningfulLines(fields.checklist_output.value);
  const budget = fields.time_budget_min.value.trim();
  const snapshot = [
    "# Errand Coach Intake", "", `- Time budget: ${budget || "not set"} min`, `- Items listed: ${errands.length}`, "",
    "## Raw Errands", fields.errands_dump.value.trim() || "(empty)", "",
    "## Constraints", fields.constraints.value.trim() || "(none listed)", "",
    "## Checklist Output", fields.checklist_output.value.trim() || "(empty)"
  ].join("\n");
  document.getElementById("errand-count").textContent = String(errands.length);
  document.getElementById("budget-display").textContent = budget ? `${budget} min` : "-";
  document.getElementById("checklist-count").textContent = String(checklist.length);
  document.getElementById("checklist_preview").value = checklist.length > 0 ? checklist.join("\n") : "(no checklist yet)";
  document.getElementById("handoff_preview").value = snapshot;
  updateBridgeStatus({ errands, checklist, budget, snapshot });
}

function saveState() {
  const state = {};
  for (const [id, element] of Object.entries(fields)) state[id] = element.value ?? "";
  localStorage.setItem(STORAGE_KEY, JSON.stringify(state));
  refreshDerivedUi();
}

function resetState() {
  localStorage.removeItem(STORAGE_KEY);
  for (const element of Object.values(fields)) element.value = "";
  refreshDerivedUi();
}

function restoreState() {
  const state = loadState();
  for (const [id, element] of Object.entries(fields)) {
    if (typeof state[id] === "string") element.value = state[id];
    element.addEventListener("input", saveState);
    element.addEventListener("change", saveState);
  }
  refreshDerivedUi();
}

function updateBridgeStatus({ errands, checklist, budget, snapshot }) {
  if (!window.chattyCogBridge?.available || typeof window.chattyCogBridge.updateStatus !== "function") {
    return;
  }
  const summary = [
    `Errand Coach has ${errands.length} errand item(s) captured and ${checklist.length} checklist line(s) drafted.`,
    `Time budget is ${budget || "not set"} minutes.`,
    `Next handoff focus: ${fields.checklist_output.value.trim() ? "polish the checklist order" : "draft the first checklist pass"}.`
  ].join(" ");
  window.chattyCogBridge.updateStatus({
    module_id: "demo_errand_coach",
    event_type: "suspend_rundown",
    summary,
    snapshot,
    tags: ["errands", "planning", "webview"]
  });
}

document.getElementById("save-state").addEventListener("click", saveState);
document.getElementById("reset-state").addEventListener("click", resetState);
document.getElementById("refresh-preview").addEventListener("click", refreshDerivedUi);
restoreState();
