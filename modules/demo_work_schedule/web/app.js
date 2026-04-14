const MODULE_ID = "demo_work_schedule";
const STORAGE_KEY = "chattycog.demo_work_schedule.webview.v1";
const INCOMING_ASSET_LANE = "lesson_assets";
let lastIncomingFingerprint = "";
let lastRoomFingerprint = "";
let latestRoomState = null;
let activeSessionKey = "";
let lastAppliedSharedRevision = 0;
let lastAppliedSharedFrom = "";
let lastSyncFlashUntil = 0;
let lastRoomEventsFingerprint = "";
let sharedRoomEvents = [];
let optimisticRoomEvents = [];
let roomToasts = [];
let incomingAssets = [];
let selectedIncomingAssetId = "";
let incomingAssetPreviewText = "(preview appears here)";
const MAX_ROOM_EVENTS = 8;
const ROOM_TOAST_MS = 4200;
const fieldIds = ["timezone", "work_window", "focus_top3", "non_negotiables", "projects", "draft_schedule_notes", "risks_conflicts", "artifacts"];
const fields = Object.fromEntries(fieldIds.map((id) => [id, document.getElementById(id)]));

function collectState() {
  const state = {};
  for (const [id, element] of Object.entries(fields)) state[id] = element.value ?? "";
  return state;
}

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

function saveLocalState(state) {
  localStorage.setItem(STORAGE_KEY, JSON.stringify(state));
}

function buildHandoffPreview(timelineLines) {
  return [
    "# Work Schedule Snapshot", "", `- Timezone: ${fields.timezone.value.trim() || "not set"}`, `- Work window: ${fields.work_window.value.trim() || "not set"}`, "",
    "## Focus Top 3", fields.focus_top3.value.trim() || "(empty)", "",
    "## Non-negotiables", fields.non_negotiables.value.trim() || "(empty)", "",
    "## Timeline", timelineLines.length > 0 ? timelineLines.join("\n") : "(empty)", "",
    "## Risks", fields.risks_conflicts.value.trim() || "(none listed)", "",
    "## Artifacts", fields.artifacts.value.trim() || "(none)"
  ].join("\n");
}

function buildSharedState(state, projectLines, timelineLines, snapshot) {
  const summary = [
    `${projectLines.length} project(s) and ${timelineLines.length} timeline line(s) are currently tracked.`,
    `Timezone is ${state.timezone?.trim() || "not set"} and work window is ${state.work_window?.trim() || "not set"}.`,
    `Next handoff focus: ${state.draft_schedule_notes?.trim() ? "refine the draft schedule" : "draft the schedule timeline"}.`
  ].join(" ");
  return {
    module_id: MODULE_ID,
    summary,
    payload: {
      fields: state,
      metrics: {
        projectCount: projectLines.length,
        timelineCount: timelineLines.length
      }
    },
    updated_at_unix_ms: Date.now(),
    host_authoritative: !!latestRoomState?.host_authoritative
  };
}

function describeParticipants(roomState) {
  if (!roomState || !Array.isArray(roomState.participants) || roomState.participants.length === 0) {
    return "Just this device for now.";
  }
  return roomState.participants
    .map((participant) => {
      const parts = [participant.device_name || participant.device_id || "unknown device"];
      if (participant.is_local) parts.push("you");
      if (participant.connected === false) parts.push("offline");
      return parts.join(" - ");
    })
    .join(" | ");
}

function makeSessionKey(roomState) {
  if (!roomState || !roomState.session_active) {
    return "";
  }
  return [
    roomState.session_id || roomState.session_label || "session",
    roomState.host_device_id || roomState.host_device_name || "host"
  ].join("|");
}

function setSyncBadge(elementId, label, tone) {
  const element = document.getElementById(elementId);
  if (!element) {
    return;
  }
  element.textContent = label;
  element.className = `sync-badge ${tone}`;
}

function formatRelativeAge(ageMs) {
  if (ageMs <= 0) {
    return "just now";
  }
  const seconds = Math.floor(ageMs / 1000);
  if (seconds < 5) {
    return "just now";
  }
  if (seconds < 60) {
    return `${seconds}s ago`;
  }
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) {
    return `${minutes}m ago`;
  }
  const hours = Math.floor(minutes / 60);
  if (hours < 48) {
    return `${hours}h ago`;
  }
  const days = Math.floor(hours / 24);
  return `${days}d ago`;
}

function makeRoomEventId() {
  return `room-${Date.now()}-${Math.random().toString(16).slice(2, 10)}`;
}

function eventTimestamp(event) {
  return Number(
    event?.received_at_unix_ms ||
      event?.sent_at_unix_ms ||
      event?.created_at_unix_ms ||
      0
  );
}

function trimRoomEventText(value) {
  const text = String(value || "").trim();
  if (!text) {
    return "";
  }
  return text.length > 180 ? `${text.slice(0, 177)}...` : text;
}

function appendUniqueLine(currentValue, nextLine) {
  const existing = meaningfulLines(currentValue);
  if (existing.includes(nextLine)) {
    return existing.join("\n");
  }
  existing.push(nextLine);
  return existing.join("\n");
}

function appendNamedSection(currentValue, heading, body) {
  const trimmedBody = String(body || "").trim();
  if (!trimmedBody) {
    return currentValue;
  }
  const section = `[${heading}]\n${trimmedBody}`;
  const current = String(currentValue || "").trim();
  if (current.includes(section)) {
    return current;
  }
  return current ? `${current}\n\n${section}` : section;
}

function selectedIncomingAsset() {
  return incomingAssets.find((asset) => asset.asset_id === selectedIncomingAssetId) || null;
}

function incomingAssetDisplayName(asset) {
  return (
    asset?.label?.trim() ||
    asset?.file_name?.trim() ||
    asset?.payload_file_name?.trim() ||
    asset?.kind?.trim() ||
    "Incoming asset"
  );
}

function incomingAssetSource(asset) {
  return asset?.from_device_name?.trim() || asset?.from_device_id?.trim() || "unknown device";
}

function incomingAssetLooksText(asset) {
  const contentType = String(asset?.content_type || "").toLowerCase();
  const fileName = String(asset?.file_name || asset?.payload_file_name || "").toLowerCase();
  return (
    contentType.startsWith("text/") ||
    contentType.includes("json") ||
    contentType.includes("markdown") ||
    contentType.includes("csv") ||
    contentType.includes("xml") ||
    contentType.includes("yaml") ||
    /\.((txt)|(md)|(markdown)|(json)|(csv)|(xml)|(yaml)|(yml))$/.test(fileName)
  );
}

function incomingAssetMeta(asset) {
  const meta = [
    incomingAssetSource(asset),
    asset?.content_type?.trim() || asset?.kind?.trim() || "unknown type",
    `${Number(asset?.byte_len || 0)} bytes`
  ];
  if (Number(asset?.chunk_count || 0) > 1) {
    meta.push(`${asset.chunk_count} chunks`);
  }
  return meta.join(" | ");
}

async function refreshIncomingAssetPreview() {
  const asset = selectedIncomingAsset();
  if (!asset) {
    incomingAssetPreviewText = "(preview appears here)";
    return;
  }

  if (!incomingAssetLooksText(asset)) {
    incomingAssetPreviewText = "Binary asset detected. Use Open payload to inspect it externally, or Apply to planner to record it in the artifacts list.";
    return;
  }

  const payloadUrl = chattyCogIncomingAssetUrl(INCOMING_ASSET_LANE, asset.payload_file_name);
  if (!payloadUrl) {
    incomingAssetPreviewText = "Payload URL is not available right now.";
    return;
  }

  try {
    const response = await fetch(payloadUrl);
    if (!response.ok) {
      incomingAssetPreviewText = `Could not read payload preview (${response.status}).`;
      return;
    }
    let text = await response.text();
    const contentType = String(asset.content_type || "").toLowerCase();
    if (contentType.includes("json")) {
      try {
        text = JSON.stringify(JSON.parse(text), null, 2);
      } catch {
        // keep raw text if it is not valid JSON
      }
    }
    if (text.length > 5000) {
      text = `${text.slice(0, 5000)}\n\n...[preview truncated]`;
    }
    incomingAssetPreviewText = text || "(empty asset payload)";
  } catch (err) {
    incomingAssetPreviewText = `Could not preview this asset: ${err}`;
  }
}

function renderIncomingAssets() {
  const status = document.getElementById("incoming-asset-status");
  const list = document.getElementById("incoming-asset-list");
  const title = document.getElementById("incoming-asset-title");
  const meta = document.getElementById("incoming-asset-meta");
  const preview = document.getElementById("incoming-asset-preview");
  const openButton = document.getElementById("incoming-asset-open");
  const applyButton = document.getElementById("incoming-asset-apply");
  const consumeButton = document.getElementById("incoming-asset-consume");
  if (!status || !list || !title || !meta || !preview || !openButton || !applyButton || !consumeButton) {
    return;
  }

  if (!window.chattyCogBridge?.available) {
    status.textContent = "Local only";
    status.className = "sync-badge subtle";
  } else if (incomingAssets.length > 0) {
    status.textContent = `${incomingAssets.length} waiting`;
    status.className = "sync-badge waiting";
  } else {
    status.textContent = "Lane empty";
    status.className = "sync-badge ok";
  }

  list.innerHTML = "";
  if (incomingAssets.length === 0) {
    const empty = document.createElement("li");
    empty.className = "incoming-asset-empty";
    empty.textContent = window.chattyCogBridge?.available
      ? "No incoming assets waiting in lesson_assets."
      : "Open this module inside ChattyCog to test incoming asset delivery.";
    list.appendChild(empty);
  } else {
    for (const asset of incomingAssets) {
      const item = document.createElement("li");
      item.className = `incoming-asset-item${asset.asset_id === selectedIncomingAssetId ? " selected" : ""}`;

      const titleRow = document.createElement("div");
      titleRow.className = "incoming-asset-title-row";

      const strong = document.createElement("strong");
      strong.textContent = incomingAssetDisplayName(asset);
      titleRow.appendChild(strong);

      const kind = document.createElement("span");
      kind.className = "incoming-asset-kind";
      kind.textContent = asset.kind || "asset";
      titleRow.appendChild(kind);

      const summary = document.createElement("div");
      summary.className = "incoming-asset-summary";
      summary.textContent = asset.summary?.trim() || incomingAssetMeta(asset);

      const metaLine = document.createElement("div");
      metaLine.className = "asset-meta";
      metaLine.textContent = incomingAssetMeta(asset);

      item.appendChild(titleRow);
      item.appendChild(summary);
      item.appendChild(metaLine);
      item.addEventListener("click", async () => {
        selectedIncomingAssetId = asset.asset_id;
        await refreshIncomingAssetPreview();
        renderIncomingAssets();
      });
      list.appendChild(item);
    }
  }

  const selected = selectedIncomingAsset();
  title.textContent = selected ? incomingAssetDisplayName(selected) : "No asset selected";
  meta.textContent = selected
    ? `${incomingAssetMeta(selected)}${selected.summary?.trim() ? ` | ${selected.summary.trim()}` : ""}`
    : "Pick an incoming asset to preview or import it into this planning board.";
  preview.value = incomingAssetPreviewText;
  openButton.disabled = !selected;
  applyButton.disabled = !selected;
  consumeButton.disabled = !selected;
}

async function pollIncomingAssets() {
  if (!window.chattyCogBridge?.available) {
    incomingAssets = [];
    selectedIncomingAssetId = "";
    incomingAssetPreviewText = "(preview appears here)";
    renderIncomingAssets();
    return;
  }

  const nextAssets = await readChattyCogIncomingAssets(INCOMING_ASSET_LANE);
  incomingAssets = Array.isArray(nextAssets) ? nextAssets : [];
  if (!selectedIncomingAssetId || !incomingAssets.some((asset) => asset.asset_id === selectedIncomingAssetId)) {
    selectedIncomingAssetId = incomingAssets[0]?.asset_id || "";
    await refreshIncomingAssetPreview();
  }
  renderIncomingAssets();
}

function openSelectedIncomingAsset() {
  const asset = selectedIncomingAsset();
  if (!asset) {
    return;
  }
  const payloadUrl = chattyCogIncomingAssetUrl(INCOMING_ASSET_LANE, asset.payload_file_name);
  if (!payloadUrl) {
    return;
  }
  window.open(payloadUrl, "_blank", "noopener");
}

async function applySelectedIncomingAsset() {
  const asset = selectedIncomingAsset();
  if (!asset) {
    return;
  }
  if (!incomingAssetPreviewText || incomingAssetPreviewText === "(preview appears here)") {
    await refreshIncomingAssetPreview();
  }

  const displayName = incomingAssetDisplayName(asset);
  const artifactLine = `- Imported asset: ${displayName}${asset.file_name ? ` [${asset.file_name}]` : ""}`;
  fields.artifacts.value = appendUniqueLine(fields.artifacts.value, artifactLine);
  if (incomingAssetLooksText(asset)) {
    fields.draft_schedule_notes.value = appendNamedSection(
      fields.draft_schedule_notes.value,
      `Imported asset - ${displayName}`,
      incomingAssetPreviewText
    );
  }
  saveState();
  pushRoomToast("sync", "Asset imported", `${displayName} was added to the planning board.`);
  const status = document.getElementById("incoming-asset-status");
  if (status) {
    status.textContent = `Imported ${displayName}`;
    status.className = "sync-badge ok";
  }
}

async function consumeSelectedIncomingAsset() {
  const asset = selectedIncomingAsset();
  if (!asset) {
    return;
  }
  const consumed = await consumeChattyCogIncomingAsset(INCOMING_ASSET_LANE, asset.asset_id);
  if (!consumed) {
    const status = document.getElementById("incoming-asset-status");
    if (status) {
      status.textContent = "Consume failed";
      status.className = "sync-badge waiting";
    }
    return;
  }
  pushRoomToast("info", "Asset consumed", `${incomingAssetDisplayName(asset)} was cleared from the lane.`);
  incomingAssets = incomingAssets.filter((item) => item.asset_id !== asset.asset_id);
  selectedIncomingAssetId = incomingAssets[0]?.asset_id || "";
  await refreshIncomingAssetPreview();
  renderIncomingAssets();
}

function normalizeRoomEvent(event) {
  if (!event || typeof event !== "object") {
    return null;
  }
  const payload = trimRoomEventText(event.payload_text);
  return {
    event_id: String(event.event_id || ""),
    event_type: String(event.event_type || "note"),
    label: String(event.label || event.event_type || "Room event"),
    payload_text: payload,
    from_device_name: String(event.from_device_name || ""),
    local_echo: !!event.local_echo,
    received_at_unix_ms: eventTimestamp(event)
  };
}

function currentRoomEvents() {
  const shared = sharedRoomEvents
    .map(normalizeRoomEvent)
    .filter(Boolean);
  const sharedIds = new Set(shared.map((event) => event.event_id).filter(Boolean));
  optimisticRoomEvents = optimisticRoomEvents.filter((event) => {
    if (!event) {
      return false;
    }
    if (sharedIds.has(event.event_id)) {
      return false;
    }
    return Date.now() - event.received_at_unix_ms < 30_000;
  });

  return [...shared, ...optimisticRoomEvents]
    .sort((left, right) => eventTimestamp(right) - eventTimestamp(left))
    .slice(0, MAX_ROOM_EVENTS);
}

function updateRoomEventStatus() {
  const status = document.getElementById("room-event-status");
  if (!status) {
    return;
  }
  if (!window.chattyCogBridge?.available) {
    status.textContent =
      "Local activity feed only. Open this inside ChattyCog to test networked room events.";
    return;
  }
  if (!latestRoomState?.active_for_module) {
    status.textContent =
      "Hosted locally. Start a module session from Networking when you want peers to see these quick signals.";
    return;
  }
  if (latestRoomState?.session_active) {
    status.textContent =
      "Room events are live. Quick signals appear here and on connected peers without needing a full state push.";
    return;
  }
  status.textContent =
    "Room is connected. Start a shared module session to turn these local signals into shared room events.";
}

function renderRoomEvents() {
  const list = document.getElementById("room-event-list");
  if (!list) {
    return;
  }
  const events = currentRoomEvents();
  list.innerHTML = "";
  if (events.length === 0) {
    const empty = document.createElement("li");
    empty.className = "room-event-empty";
    empty.textContent = window.chattyCogBridge?.available
      ? "No room activity yet."
      : "No local room activity yet.";
    list.appendChild(empty);
    return;
  }

  for (const event of events) {
    const item = document.createElement("li");
    item.className = `room-event-item${event.local_echo ? " local-echo" : ""}`;

    const title = document.createElement("div");
    title.className = "room-event-title";

    const strong = document.createElement("strong");
    strong.textContent = event.label || "Room event";
    title.appendChild(strong);

    const type = document.createElement("span");
    type.className = "room-event-type";
    type.textContent = event.event_type || "note";
    title.appendChild(type);

    const meta = document.createElement("div");
    meta.className = "room-event-meta";
    const actor = event.from_device_name || (event.local_echo ? "You" : "Unknown device");
    meta.textContent = `${actor} - ${formatRelativeAge(Math.max(0, Date.now() - event.received_at_unix_ms))}`;

    item.appendChild(title);
    item.appendChild(meta);

    if (event.payload_text) {
      const payload = document.createElement("div");
      payload.className = "room-event-payload";
      payload.textContent = event.payload_text;
      item.appendChild(payload);
    }

    list.appendChild(item);
  }
}

function renderRoomToasts() {
  const stack = document.getElementById("room-toast-stack");
  if (!stack) {
    return;
  }
  stack.innerHTML = "";
  for (const toast of roomToasts) {
    const item = document.createElement("div");
    item.className = `room-toast ${toast.kind || "info"}`;

    const title = document.createElement("div");
    title.className = "room-toast-title";

    const strong = document.createElement("strong");
    strong.textContent = toast.title;
    title.appendChild(strong);

    const age = document.createElement("span");
    age.className = "room-toast-age";
    age.textContent = "now";
    title.appendChild(age);

    const detail = document.createElement("div");
    detail.className = "room-toast-detail";
    detail.textContent = toast.detail;

    item.appendChild(title);
    item.appendChild(detail);
    stack.appendChild(item);
  }
}

function pruneRoomToasts() {
  const cutoff = Date.now() - ROOM_TOAST_MS;
  roomToasts = roomToasts.filter((toast) => toast.created_at_unix_ms >= cutoff);
}

function pushRoomToast(kind, title, detail) {
  roomToasts = [
    {
      id: makeRoomEventId(),
      kind,
      title,
      detail,
      created_at_unix_ms: Date.now()
    },
    ...roomToasts
  ].slice(0, 4);
  renderRoomToasts();
}

function connectedParticipantMap(roomState) {
  const participants = Array.isArray(roomState?.participants) ? roomState.participants : [];
  const map = new Map();
  for (const participant of participants) {
    if (!participant || participant.connected === false) {
      continue;
    }
    const deviceId = String(participant.device_id || "").trim();
    if (!deviceId) {
      continue;
    }
    map.set(deviceId, participant);
  }
  return map;
}

function syncParticipantToasts(previousRoomState, nextRoomState, previousSessionKey, nextSessionKey) {
  if (!nextRoomState?.active_for_module || !nextRoomState?.session_active) {
    return;
  }
  if (!previousRoomState?.active_for_module || !previousRoomState?.session_active || previousSessionKey !== nextSessionKey) {
    return;
  }

  const previousParticipants = connectedParticipantMap(previousRoomState);
  const nextParticipants = connectedParticipantMap(nextRoomState);

  for (const [deviceId, participant] of nextParticipants) {
    if (previousParticipants.has(deviceId) || participant.is_local) {
      continue;
    }
    pushRoomToast(
      "join",
      "Participant joined",
      `${participant.device_name || deviceId} joined the shared planning session.`
    );
  }

  for (const [deviceId, participant] of previousParticipants) {
    if (nextParticipants.has(deviceId) || participant.is_local) {
      continue;
    }
    pushRoomToast(
      "leave",
      "Participant left",
      `${participant.device_name || deviceId} left the shared planning session.`
    );
  }
  pruneRoomToasts();
  renderRoomToasts();
}

function syncSessionLifecycleToasts(previousRoomState, nextRoomState, previousSessionKey, nextSessionKey) {
  const previousActive = !!(previousRoomState?.active_for_module && previousRoomState?.session_active);
  const nextActive = !!(nextRoomState?.active_for_module && nextRoomState?.session_active);

  if (!previousActive && !nextActive) {
    return;
  }

  if (!previousActive && nextActive) {
    roomToasts = [];
    pushRoomToast(
      "info",
      "Session started",
      `${nextRoomState?.session_label || nextRoomState?.session_id || "Shared planning session"} is now active.`
    );
    return;
  }

  if (previousActive && !nextActive) {
    roomToasts = [];
    pushRoomToast(
      "info",
      "Session ended",
      `${previousRoomState?.session_label || previousRoomState?.session_id || "Shared planning session"} has ended.`
    );
    return;
  }

  if (previousSessionKey !== nextSessionKey) {
    roomToasts = [];
    pushRoomToast(
      "info",
      "New session started",
      `${nextRoomState?.session_label || nextRoomState?.session_id || "Shared planning session"} replaced the previous room session.`
    );
  }
}

function syncTurnToasts(previousRoomState, nextRoomState, previousSessionKey, nextSessionKey) {
  if (!nextRoomState?.active_for_module || !nextRoomState?.session_active) {
    return;
  }
  if (!previousRoomState?.active_for_module || !previousRoomState?.session_active || previousSessionKey !== nextSessionKey) {
    return;
  }

  const previousTalkingStick = String(previousRoomState?.turn_mode || "").toLowerCase().includes("talking");
  const nextTalkingStick = String(nextRoomState?.turn_mode || "").toLowerCase().includes("talking");
  if (!nextTalkingStick) {
    return;
  }

  const previousHasTurn = previousRoomState?.local_has_turn !== false;
  const nextHasTurn = nextRoomState?.local_has_turn !== false;

  if (!previousTalkingStick && nextTalkingStick) {
    pushRoomToast(
      "turn",
      nextHasTurn ? "Talking stick is yours" : "Talking stick mode active",
      nextHasTurn
        ? "You can edit now and prepare the next shared revision."
        : "Another participant currently has the stick. You can still send quick room signals."
    );
    return;
  }

  if (!previousHasTurn && nextHasTurn) {
    pushRoomToast(
      "turn",
      "Your turn to edit",
      "The talking stick has been passed to you. Local editing is unlocked."
    );
    return;
  }

  if (previousHasTurn && !nextHasTurn) {
    pushRoomToast(
      "turn",
      "Turn moved away",
      "Another participant now has the talking stick. Your copy is back in follow mode."
    );
  }
}

function queueLocalRoomEvent(event) {
  const normalized = normalizeRoomEvent(event);
  if (!normalized) {
    return;
  }
  optimisticRoomEvents = [normalized, ...optimisticRoomEvents.filter((item) => item.event_id !== normalized.event_id)].slice(
    0,
    MAX_ROOM_EVENTS
  );
  renderRoomEvents();
}

function emitRoomEvent(eventType, label, payloadText) {
  const event = {
    event_id: makeRoomEventId(),
    event_type: eventType,
    label,
    payload_text: trimRoomEventText(payloadText),
    content_type: "text/plain; charset=utf-8",
    from_device_name: "You",
    local_echo: true,
    created_at_unix_ms: Date.now(),
    received_at_unix_ms: Date.now()
  };
  queueLocalRoomEvent(event);
  const sent = emitChattyCogRoomEvent(event);
  updateRoomEventStatus();
  return sent;
}

function updateSyncIndicators() {
  const syncStatus = document.getElementById("sync-status");
  const roomRevision = Math.max(0, Number(latestRoomState?.session_revision || 0));
  const hostActivityAge = Math.max(0, Date.now() - Number(latestRoomState?.host_activity_updated_at_unix_ms || 0));
  const hostEditing = String(latestRoomState?.host_activity_state || "").toLowerCase() === "editing" && hostActivityAge <= 8_000;
  const hostActivityLabel = latestRoomState?.host_activity_label?.trim() || "Host is connected";

  if (!window.chattyCogBridge?.available) {
    if (syncStatus) {
      syncStatus.textContent = "local only";
    }
    setSyncBadge("sync-revision-badge", "Revision local", "local");
    setSyncBadge("sync-source-badge", "Standalone copy", "subtle");
    setSyncBadge("presence-badge", "Local only", "subtle");
    setSyncBadge("last-activity-badge", "Last activity local only", "subtle");
    return;
  }

  if (!latestRoomState?.active_for_module) {
    if (syncStatus) {
      syncStatus.textContent = "hosted local";
    }
    setSyncBadge("sync-revision-badge", "Revision local", "local");
    setSyncBadge("sync-source-badge", "Room session not active", "subtle");
    setSyncBadge("presence-badge", "No active room", "subtle");
    setSyncBadge("last-activity-badge", "Last activity room inactive", "subtle");
    return;
  }

  if (latestRoomState.session_active && latestRoomState.local_is_host) {
    if (syncStatus) {
      syncStatus.textContent = "hosting";
    }
    setSyncBadge("sync-revision-badge", `Hosting rev ${Math.max(1, roomRevision)}`, "host");
    setSyncBadge("sync-source-badge", "Peers follow this copy", "subtle");
    setSyncBadge("presence-badge", hostEditing ? "You are preparing the next revision" : "You are the host", hostEditing ? "editing pulse" : "host");
    setSyncBadge(
      "last-activity-badge",
      latestRoomState?.host_activity_updated_at_unix_ms
        ? `Your last activity ${formatRelativeAge(hostActivityAge)}`
        : "Your activity not tracked yet",
      hostEditing ? "editing" : "subtle"
    );
    return;
  }

  if (latestRoomState.session_active && latestRoomState.host_authoritative) {
    if (lastAppliedSharedRevision >= Math.max(1, roomRevision) && roomRevision > 0) {
      const justApplied = Date.now() < lastSyncFlashUntil;
      if (syncStatus) {
        syncStatus.textContent = justApplied ? "just applied" : "synced";
      }
      setSyncBadge(
        "sync-revision-badge",
        justApplied ? `Applied rev ${lastAppliedSharedRevision}` : `Synced rev ${lastAppliedSharedRevision}`,
        justApplied ? "ok celebrate" : "ok"
      );
      setSyncBadge(
        "sync-source-badge",
        justApplied
          ? `Just applied from ${lastAppliedSharedFrom || latestRoomState.host_device_name || "host"}`
          : `Last from ${lastAppliedSharedFrom || latestRoomState.host_device_name || "host"}`,
        "subtle"
      );
      return;
    }

    if (syncStatus) {
      syncStatus.textContent = lastAppliedSharedRevision > 0 ? "out of date" : "waiting";
    }
    setSyncBadge(
      "sync-revision-badge",
      roomRevision > 0
        ? lastAppliedSharedRevision > 0
          ? `Out of date - rev ${Math.max(1, roomRevision)}`
          : `Awaiting rev ${Math.max(1, roomRevision)}`
        : "Awaiting first host revision",
      "waiting pulse"
    );
    setSyncBadge(
      "sync-source-badge",
      lastAppliedSharedRevision > 0
        ? `Last applied rev ${lastAppliedSharedRevision} - host is ahead`
        : "No host revision applied yet",
      "subtle"
    );
    setSyncBadge(
      "presence-badge",
      hostEditing ? hostActivityLabel : "Host idle / between revisions",
      hostEditing ? "editing pulse" : "subtle"
    );
    setSyncBadge(
      "last-activity-badge",
      latestRoomState?.host_activity_updated_at_unix_ms
        ? `Last host activity ${formatRelativeAge(hostActivityAge)}`
        : "Host activity not seen yet",
      hostEditing ? "editing" : "subtle"
    );
    return;
  }

  if (syncStatus) {
    syncStatus.textContent = latestRoomState.session_active ? "shared session" : "room connected";
  }
  setSyncBadge(
    "sync-revision-badge",
    latestRoomState.session_active ? `Shared rev ${Math.max(1, roomRevision)}` : "Revision local",
    latestRoomState.session_active ? "ok" : "local"
  );
  setSyncBadge("sync-source-badge", "Local edits are allowed here", "subtle");
  setSyncBadge(
    "presence-badge",
    hostEditing ? hostActivityLabel : "Shared room active",
    hostEditing ? "editing pulse" : "subtle"
  );
  setSyncBadge(
    "last-activity-badge",
    latestRoomState?.host_activity_updated_at_unix_ms
      ? `Last host activity ${formatRelativeAge(hostActivityAge)}`
      : "Host activity not seen yet",
    hostEditing ? "editing" : "subtle"
  );
}

function setEditorsLocked(locked, detail) {
  document.body.classList.toggle("room-locked", locked);
  for (const element of Object.values(fields)) {
    element.readOnly = locked;
  }
  document.getElementById("save-state").disabled = locked;
  document.getElementById("reset-state").disabled = locked;
  document.getElementById("room-policy-heading").textContent = locked ? "Mirroring host" : "Editing policy";
  document.getElementById("room-policy-detail").textContent = detail;
}

function updateRoomActionHint({ active, sessionActive, hostAuthoritative, localIsHost, localHasTurn, talkingStick }) {
  const heading = document.getElementById("room-action-heading");
  const hint = document.getElementById("room-action-hint");
  if (!heading || !hint) {
    return;
  }

  if (!active) {
    heading.textContent = "Testing hint";
    hint.textContent = "Start a module session from ChattyCog's Networking tab, then use the quick room-event buttons below or the module bridge panel to share the latest schedule with selected peers.";
    return;
  }

  if (sessionActive && hostAuthoritative && localIsHost) {
    heading.textContent = "Host push state now";
    hint.textContent = "You are the host. Make your edits here, use the quick room-event buttons for light coordination, then share this revision from the ChattyCog module bridge panel when it is ready.";
    return;
  }

  if (sessionActive && hostAuthoritative && !localIsHost) {
    heading.textContent = "Following host revision";
    hint.textContent = "This copy is following the host-led session. It will apply the next shared revision automatically when the host pushes state, while room-event buttons stay available for lightweight signals.";
    return;
  }

  if (talkingStick && !localHasTurn) {
    heading.textContent = "Waiting for talking stick";
    hint.textContent = "Another participant currently has the stick. Once it is passed to you, editing will unlock and your local changes can become the next shared revision. You can still send quick room-event signals in the meantime.";
    return;
  }

  if (sessionActive) {
    heading.textContent = "Shared session active";
    hint.textContent = "This module is in a shared room session. Use room-event buttons for lightweight coordination, then push the current state when you want other participants to pick up the latest plan.";
    return;
  }

  heading.textContent = "Connected room";
  hint.textContent = "The room is connected but no module session is active yet. Start one from Networking when you want everyone following the same planning flow.";
}

function applyRoomState(roomState) {
  const previousRoomState = latestRoomState;
  const previousSessionKey = activeSessionKey;
  latestRoomState = roomState && typeof roomState === "object" ? roomState : null;
  const active = !!latestRoomState?.active_for_module;
  const sessionActive = !!latestRoomState?.session_active;
  const hostAuthoritative = !!latestRoomState?.host_authoritative;
  const localIsHost = !!latestRoomState?.local_is_host;
  const localHasTurn = latestRoomState?.local_has_turn !== false;
  const talkingStick = String(latestRoomState?.turn_mode || "").toLowerCase().includes("talking");
  const participantCount = latestRoomState?.participant_count || latestRoomState?.participants?.length || 1;
  activeSessionKey = makeSessionKey(latestRoomState);

  if (activeSessionKey !== previousSessionKey) {
    lastAppliedSharedRevision = 0;
    lastAppliedSharedFrom = "";
    lastSyncFlashUntil = 0;
    sharedRoomEvents = [];
    optimisticRoomEvents = [];
    lastRoomEventsFingerprint = "";
  }

  const roomMode = !active
    ? "standalone"
    : sessionActive
      ? hostAuthoritative
        ? "host-led session"
        : "shared session"
      : "shared room";
  document.getElementById("room-mode").textContent = roomMode;

  let role = "independent";
  if (active) {
    if (localIsHost) {
      role = hostAuthoritative ? "host / leader" : "host";
    } else if (talkingStick && localHasTurn) {
      role = "participant with stick";
    } else if (sessionActive && hostAuthoritative) {
      role = "participant / follower";
    } else {
      role = "participant";
    }
  }
  document.getElementById("room-role").textContent = role;

  document.getElementById("room-session-heading").textContent = !active
    ? "No active room session"
    : sessionActive
      ? (latestRoomState.session_label || latestRoomState.session_id || "Active shared session")
      : "Room connected without active session";
  document.getElementById("room-session-detail").textContent = !active
    ? "This copy is currently running as a local planning board."
    : [
        `Participants: ${participantCount}.`,
        `AI mode: ${latestRoomState.ai_mode || "not set"}.`,
        `Turn mode: ${latestRoomState.turn_mode || "open"}.`,
        sessionActive ? `Revision: ${Math.max(1, Number(latestRoomState.session_revision || 0))}.` : "Waiting for the host to start a module session."
      ].join(" ");
  document.getElementById("room-participants").textContent = describeParticipants(latestRoomState);

  let locked = false;
  let lockDetail = "Local editing is available.";
  if (active && sessionActive && hostAuthoritative && !localIsHost) {
    locked = true;
    lockDetail = "This is a host-led planning session. Your copy is following the host's current revision.";
  } else if (active && talkingStick && !localHasTurn) {
    locked = true;
    lockDetail = "Talking stick is with another participant right now. Wait for your turn to edit.";
  } else if (active && localIsHost) {
    lockDetail = "You are leading the current planning session.";
  } else if (active) {
    lockDetail = "Shared room is active. Local editing is allowed on this copy.";
  }
  setEditorsLocked(locked, lockDetail);
  updateRoomActionHint({
    active,
    sessionActive,
    hostAuthoritative,
    localIsHost,
    localHasTurn,
    talkingStick
  });
  syncSessionLifecycleToasts(previousRoomState, latestRoomState, previousSessionKey, activeSessionKey);
  syncParticipantToasts(previousRoomState, latestRoomState, previousSessionKey, activeSessionKey);
  syncTurnToasts(previousRoomState, latestRoomState, previousSessionKey, activeSessionKey);
  updateSyncIndicators();
  updateRoomEventStatus();
  renderRoomEvents();
}

function refreshDerivedUi() {
  const state = collectState();
  const projectLines = meaningfulLines(fields.projects.value);
  const timelineLines = meaningfulLines(fields.draft_schedule_notes.value);
  const filled = fieldIds.filter((id) => fields[id].value.trim().length > 0).length;
  const snapshot = buildHandoffPreview(timelineLines);
  const sharedState = buildSharedState(state, projectLines, timelineLines, snapshot);
  document.getElementById("project-count").textContent = String(projectLines.length);
  document.getElementById("timeline-count").textContent = String(timelineLines.length);
  document.getElementById("completion-score").textContent = `${Math.round((filled / fieldIds.length) * 100)}%`;
  document.getElementById("timeline_preview").value = timelineLines.length > 0 ? timelineLines.join("\n") : "(no schedule entries yet)";
  document.getElementById("handoff_preview").value = snapshot;
  updateBridgeStatus({ projectCount: projectLines.length, timelineLines, filled, snapshot });
  updateChattyCogBridgeSharedState(sharedState);
  updateSyncIndicators();
}

function saveState() {
  const state = collectState();
  saveLocalState(state);
  refreshDerivedUi();
}

function resetState() {
  localStorage.removeItem(STORAGE_KEY);
  for (const element of Object.values(fields)) element.value = "";
  clearChattyCogBridgeStatus();
  clearChattyCogBridgeSharedState();
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

function updateBridgeStatus({ projectCount, timelineLines, filled, snapshot }) {
  const completion = Math.round((filled / fieldIds.length) * 100);
  const summary = [
    `Work Schedule is ${completion}% filled out.`,
    `${projectCount} project(s) and ${timelineLines.length} schedule line(s) are currently tracked.`,
    `Timezone is ${fields.timezone.value.trim() || "not set"} and work window is ${fields.work_window.value.trim() || "not set"}.`,
    `Next handoff focus: ${fields.draft_schedule_notes.value.trim() ? "refine the draft schedule" : "draft the schedule timeline"}.`,
    latestRoomState?.session_active
      ? `Room session: ${latestRoomState.session_label || latestRoomState.session_id || "active"} (${Math.max(1, Number(latestRoomState.session_revision || 0))}).`
      : "Room session: inactive."
  ].join(" ");
  updateChattyCogBridgeStatus({
    module_id: MODULE_ID,
    event_type: "suspend_rundown",
    summary,
    snapshot,
    tags: ["work_schedule", "planning", "webview", "room_aware"],
    payload: {
      projectCount,
      timelineCount: timelineLines.length,
      completion,
      room: latestRoomState
        ? {
            activeForModule: !!latestRoomState.active_for_module,
            sessionActive: !!latestRoomState.session_active,
            sessionRevision: Number(latestRoomState.session_revision || 0),
            participantCount: latestRoomState.participant_count || latestRoomState.participants?.length || 0
          }
        : null
    }
  });
}

function shouldIgnoreIncomingSharedState(incoming) {
  return !!(
    incoming &&
    latestRoomState?.session_active &&
    latestRoomState?.host_authoritative &&
    latestRoomState?.local_is_host
  );
}

function applyIncomingSharedState(incoming) {
  if (!incoming || typeof incoming !== "object" || !incoming.payload || typeof incoming.payload !== "object") {
    return false;
  }
  if (shouldIgnoreIncomingSharedState(incoming)) {
    return false;
  }
  const nextFields = incoming.payload.fields;
  if (!nextFields || typeof nextFields !== "object") {
    return false;
  }
  const fingerprint = JSON.stringify(incoming);
  if (!fingerprint || fingerprint === lastIncomingFingerprint) {
    return false;
  }

  let changed = false;
  for (const [id, element] of Object.entries(fields)) {
    const nextValue = typeof nextFields[id] === "string" ? nextFields[id] : "";
    if ((element.value ?? "") !== nextValue) {
      element.value = nextValue;
      changed = true;
    }
  }

  lastIncomingFingerprint = fingerprint;
  const nextRevision = Math.max(1, Number(incoming.session_revision || latestRoomState?.session_revision || 0));
  const previousAppliedRevision = lastAppliedSharedRevision;
  lastAppliedSharedRevision = nextRevision;
  lastAppliedSharedFrom = incoming.from_device_name || incoming.authoritative_device_name || "host";
  if (changed) {
    lastSyncFlashUntil = Date.now() + 2200;
    saveLocalState(collectState());
    refreshDerivedUi();
    if (nextRevision > previousAppliedRevision) {
      pushRoomToast(
        "sync",
        "Revision applied",
        `Applied host revision ${nextRevision} from ${lastAppliedSharedFrom || "host"}.`
      );
    }
  }
  updateSyncIndicators();
  return changed;
}

async function pollIncomingSharedState() {
  const incoming = await readChattyCogIncomingSharedState();
  applyIncomingSharedState(incoming);
}

async function pollSharedRoomState() {
  const roomState = await readChattyCogSharedRoomState();
  const fingerprint = JSON.stringify(roomState || null);
  if (fingerprint === lastRoomFingerprint) {
    return;
  }
  lastRoomFingerprint = fingerprint;
  applyRoomState(roomState);
  refreshDerivedUi();
}

async function pollSharedRoomEvents() {
  const roomEvents = await readChattyCogSharedRoomEvents();
  const fingerprint = JSON.stringify(roomEvents || null);
  if (fingerprint === lastRoomEventsFingerprint) {
    return;
  }
  lastRoomEventsFingerprint = fingerprint;
  sharedRoomEvents = Array.isArray(roomEvents?.events) ? roomEvents.events : [];
  updateRoomEventStatus();
  renderRoomEvents();
}

function sendPresetRoomEvent(eventType, label, payloadText) {
  const sent = emitRoomEvent(eventType, label, payloadText);
  const status = document.getElementById("room-event-status");
  if (!status) {
    return;
  }
  if (!window.chattyCogBridge?.available) {
    status.textContent = "Saved to the local demo event feed. Host this module inside ChattyCog to share it with peers.";
    return;
  }
  status.textContent = sent
    ? "Room event sent. Peers should see it in their recent activity feed shortly."
    : "Room event saved locally, but the bridge could not send it right now.";
}

function initTabs() {
  const tabs = document.querySelectorAll(".tab");
  const panels = document.querySelectorAll(".tab-panel");
  tabs.forEach((tab) => tab.addEventListener("click", () => {
    const selected = tab.dataset.tab;
    tabs.forEach((item) => item.classList.toggle("active", item === tab));
    panels.forEach((panel) => panel.classList.toggle("active", panel.dataset.panel === selected));
  }));
}

document.getElementById("save-state").addEventListener("click", saveState);
document.getElementById("reset-state").addEventListener("click", resetState);
document.getElementById("refresh-preview").addEventListener("click", refreshDerivedUi);
document.getElementById("event-ready").addEventListener("click", () => {
  sendPresetRoomEvent(
    "schedule_ready",
    "Ready for review",
    "This schedule board is ready for host or peer review."
  );
});
document.getElementById("event-delay").addEventListener("click", () => {
  sendPresetRoomEvent(
    "need_more_time",
    "Need more time",
    "This schedule still needs another pass before the next revision is pushed."
  );
});
document.getElementById("event-claim").addEventListener("click", () => {
  const focus = meaningfulLines(fields.focus_top3.value)[0] || meaningfulLines(fields.projects.value)[0] || "the next planning block";
  sendPresetRoomEvent(
    "claim_next_block",
    "Claimed next block",
    `Picking up ${focus} as the next active planning block.`
  );
});
document.getElementById("event-note").addEventListener("click", () => {
  const input = document.getElementById("room-note-input");
  const note = input.value.trim();
  if (!note) {
    const status = document.getElementById("room-event-status");
    if (status) {
      status.textContent = "Write a short room note first, then send it.";
    }
    return;
  }
  sendPresetRoomEvent("room_note", "Room note", note);
  input.value = "";
});
document.getElementById("room-note-input").addEventListener("keydown", (event) => {
  if (event.key === "Enter" && !event.shiftKey) {
    event.preventDefault();
    document.getElementById("event-note").click();
  }
});
document.getElementById("incoming-asset-open").addEventListener("click", openSelectedIncomingAsset);
document.getElementById("incoming-asset-apply").addEventListener("click", () => {
  applySelectedIncomingAsset();
});
document.getElementById("incoming-asset-consume").addEventListener("click", () => {
  consumeSelectedIncomingAsset();
});
restoreState();
initTabs();
applyRoomState(null);
renderIncomingAssets();
pollIncomingAssets();
pollIncomingSharedState();
pollSharedRoomState();
pollSharedRoomEvents();
window.setInterval(pollIncomingAssets, 3000);
window.setInterval(pollIncomingSharedState, 2500);
window.setInterval(pollSharedRoomState, 2500);
window.setInterval(pollSharedRoomEvents, 1500);
window.setInterval(() => {
  pruneRoomToasts();
  renderRoomToasts();
}, 1000);
