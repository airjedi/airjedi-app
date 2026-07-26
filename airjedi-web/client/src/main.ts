import { initGlobe } from "./globe";
import { WebSocketClient } from "./websocket";
import { AircraftStore, AppState } from "./store";
import { AircraftManager } from "./aircraft";
import { AircraftListPanel } from "./panels/aircraft-list";
import { AircraftDetailPanel } from "./panels/aircraft-detail";
import { StatusBar } from "./panels/status-bar";
import { SettingsPanel } from "./panels/settings";
import "./style.css";

async function main() {
  const configResp = await fetch("/api/config");
  const config = await configResp.json();

  const container = document.getElementById("cesium-container");
  if (!container) throw new Error("Missing #cesium-container");

  const viewer = await initGlobe(container, config.cesium_ion_token);
  const store = new AircraftStore();
  const appState = new AppState();

  const wsClient = new WebSocketClient();
  wsClient.onMessage = (msg) => {
    switch (msg.type) {
      case "snapshot":
        store.applySnapshot(msg.aircraft);
        break;
      case "update":
        store.applyUpdate(msg.aircraft);
        break;
      case "remove":
        store.applyRemove(msg.icao);
        break;
      case "status":
        console.log("Feed status:", msg.feeds);
        break;
    }
  };

  const wsProtocol = location.protocol === "https:" ? "wss:" : "ws:";
  wsClient.connect(`${wsProtocol}//${location.host}/ws`);

  new AircraftManager(viewer, store, appState);

  const listPanel = document.getElementById("aircraft-list-panel")!;
  const detailPanel = document.getElementById("aircraft-detail-panel")!;
  const statusBar = document.getElementById("status-bar")!;

  new AircraftListPanel(listPanel, store, appState);
  new AircraftDetailPanel(detailPanel, store, appState);
  new StatusBar(statusBar, store, wsClient, viewer);

  const settingsContainer = document.getElementById("settings-panel")!;
  const settingsPanel = new SettingsPanel(settingsContainer, wsClient, viewer);
  document.getElementById("settings-btn")!.addEventListener("click", () => {
    settingsPanel.toggle();
  });
}

main().catch(console.error);
