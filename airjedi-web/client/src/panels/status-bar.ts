import { Viewer } from "cesium";
import { AircraftStore } from "../store";
import { WebSocketClient } from "../websocket";

export class StatusBar {
  private container: HTMLElement;
  private store: AircraftStore;
  private wsConnected = false;

  constructor(
    container: HTMLElement,
    store: AircraftStore,
    wsClient: WebSocketClient,
    _viewer: Viewer
  ) {
    this.container = container;
    this.store = store;

    wsClient.onConnectionChange = (connected) => {
      this.wsConnected = connected;
      this.render();
    };

    store.onChange(() => this.render());

    this.render();
  }

  private render(): void {
    const dot = this.wsConnected ? "connected" : "disconnected";
    const label = this.wsConnected ? "Connected" : "Disconnected";

    this.container.innerHTML = `
      <span><span class="status-dot ${dot}"></span>${label}</span>
      <span>Aircraft: ${this.store.count}</span>
    `;
  }
}
