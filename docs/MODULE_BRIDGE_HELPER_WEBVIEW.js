function updateChattyCogBridgeStatus(buildPayload) {
  if (!window.chattyCogBridge?.available || typeof window.chattyCogBridge.updateStatus !== "function") {
    return false;
  }

  try {
    const payload = typeof buildPayload === "function" ? buildPayload() : buildPayload;
    if (!payload || typeof payload !== "object") {
      return false;
    }

    window.chattyCogBridge.updateStatus({
      event_type: "suspend_rundown",
      ...payload,
    });
    return true;
  } catch (err) {
    console.warn("ChattyCog bridge update failed", err);
    return false;
  }
}

function clearChattyCogBridgeStatus() {
  if (!window.chattyCogBridge?.available || typeof window.chattyCogBridge.clearStatus !== "function") {
    return false;
  }

  try {
    window.chattyCogBridge.clearStatus();
    return true;
  } catch (err) {
    console.warn("ChattyCog bridge clear failed", err);
    return false;
  }
}

async function readChattyCogSharedRoomState() {
  if (!window.chattyCogBridge?.available || typeof window.chattyCogBridge.readSharedRoomState !== "function") {
    return null;
  }

  try {
    return await window.chattyCogBridge.readSharedRoomState();
  } catch (err) {
    console.warn("ChattyCog shared-room read failed", err);
    return null;
  }
}

async function readChattyCogSharedRoomEvents() {
  if (!window.chattyCogBridge?.available || typeof window.chattyCogBridge.readSharedRoomEvents !== "function") {
    return null;
  }

  try {
    return await window.chattyCogBridge.readSharedRoomEvents();
  } catch (err) {
    console.warn("ChattyCog shared-room events read failed", err);
    return null;
  }
}

async function readChattyCogIncomingAssets(laneId) {
  if (!window.chattyCogBridge?.available || typeof window.chattyCogBridge.readIncomingAssets !== "function") {
    return [];
  }

  try {
    return await window.chattyCogBridge.readIncomingAssets(laneId);
  } catch (err) {
    console.warn("ChattyCog incoming-assets read failed", err);
    return [];
  }
}

function chattyCogIncomingAssetUrl(laneId, payloadFileName) {
  if (!window.chattyCogBridge?.available || typeof window.chattyCogBridge.incomingAssetUrl !== "function") {
    return null;
  }

  try {
    return window.chattyCogBridge.incomingAssetUrl(laneId, payloadFileName);
  } catch (err) {
    console.warn("ChattyCog incoming-asset URL failed", err);
    return null;
  }
}

async function consumeChattyCogIncomingAsset(laneId, assetId) {
  if (!window.chattyCogBridge?.available || typeof window.chattyCogBridge.consumeIncomingAsset !== "function") {
    return false;
  }

  try {
    return await window.chattyCogBridge.consumeIncomingAsset(laneId, assetId);
  } catch (err) {
    console.warn("ChattyCog incoming-asset consume failed", err);
    return false;
  }
}

function emitChattyCogRoomEvent(buildPayload) {
  if (!window.chattyCogBridge?.available || typeof window.chattyCogBridge.emitRoomEvent !== "function") {
    return false;
  }

  try {
    const payload = typeof buildPayload === "function" ? buildPayload() : buildPayload;
    if (!payload || typeof payload !== "object") {
      return false;
    }
    window.chattyCogBridge.emitRoomEvent(payload);
    return true;
  } catch (err) {
    console.warn("ChattyCog room event emit failed", err);
    return false;
  }
}
