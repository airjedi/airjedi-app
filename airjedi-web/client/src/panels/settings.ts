import { Viewer } from "cesium";
import { WebSocketClient } from "../websocket";
import { escapeHtml } from "../util";

interface FeedConfig {
  id: string;
  address: string;
  protocol: string;
}

export class SettingsPanel {
  private container: HTMLElement;
  private wsClient: WebSocketClient;
  private viewer: Viewer;
  private visible = false;

  constructor(
    container: HTMLElement,
    wsClient: WebSocketClient,
    viewer: Viewer
  ) {
    this.container = container;
    this.wsClient = wsClient;
    this.viewer = viewer;
    this.container.style.display = "none";
    this.render();
  }

  toggle(): void {
    this.visible = !this.visible;
    this.container.style.display = this.visible ? "block" : "none";
    if (this.visible) this.render();
  }

  private async render(): Promise<void> {
    let feeds: FeedConfig[] = [];
    try {
      const resp = await fetch("/api/feeds");
      feeds = await resp.json();
    } catch (e) {
      console.error("Failed to fetch feeds:", e);
    }

    this.container.innerHTML = `
      <div class="panel-header">
        Settings
        <button class="close-btn" id="settings-close">x</button>
      </div>
      <div class="panel-body" style="padding: 8px 12px;">
        <h4 style="margin: 0 0 8px; font-size: 13px; color: #a0a0c0;">Data Sources</h4>
        <div id="feeds-list">
          ${feeds
            .map(
              (f) => `
            <div class="detail-row" style="align-items: center;">
              <span class="detail-value" style="font-size: 12px;">${escapeHtml(f.address)} (${escapeHtml(f.protocol)})</span>
              <button class="close-btn remove-feed" data-id="${escapeHtml(f.id)}">x</button>
            </div>
          `
            )
            .join("")}
        </div>
        <div style="margin-top: 8px; display: flex; gap: 4px;">
          <input type="text" id="new-feed-addr" class="search-input" style="margin: 0; flex: 1;" placeholder="host:port" />
          <select id="new-feed-proto" style="background: rgba(40,40,60,0.8); color: #e0e0e0; border: 1px solid rgba(80,80,120,0.3); border-radius: 4px; padding: 4px; font-size: 12px;">
            <option value="beast">BEAST</option>
            <option value="basestation">SBS-1</option>
          </select>
          <button id="add-feed-btn" style="background: rgba(60,60,100,0.6); color: #e0e0e0; border: 1px solid rgba(80,80,120,0.3); border-radius: 4px; padding: 4px 10px; cursor: pointer; font-size: 12px;">Add</button>
        </div>

        <h4 style="margin: 16px 0 8px; font-size: 13px; color: #a0a0c0;">Basemap</h4>
        <div style="display: flex; gap: 8px;">
          <label style="font-size: 12px; cursor: pointer;">
            <input type="radio" name="basemap" value="default" checked /> Bing Aerial
          </label>
          <label style="font-size: 12px; cursor: pointer;">
            <input type="radio" name="basemap" value="osm" /> OpenStreetMap
          </label>
        </div>
      </div>
    `;

    this.container
      .querySelector("#settings-close")!
      .addEventListener("click", () => this.toggle());

    this.container.querySelectorAll(".remove-feed").forEach((btn) => {
      btn.addEventListener("click", async () => {
        const id = (btn as HTMLElement).dataset.id!;
        await fetch(`/api/feeds/${id}`, { method: "DELETE" });
        this.render();
      });
    });

    this.container
      .querySelector("#add-feed-btn")!
      .addEventListener("click", () => {
        const addr = (
          this.container.querySelector("#new-feed-addr") as HTMLInputElement
        ).value.trim();
        const proto = (
          this.container.querySelector("#new-feed-proto") as HTMLSelectElement
        ).value;
        if (addr) {
          this.wsClient.send({
            type: "add_feed",
            address: addr,
            protocol: proto,
          });
          setTimeout(() => this.render(), 500);
        }
      });

    this.container.querySelectorAll('input[name="basemap"]').forEach((radio) => {
      radio.addEventListener("change", () => {
        const value = (radio as HTMLInputElement).value;
        const layers = this.viewer.imageryLayers;
        if (value === "osm") {
          layers.get(0).show = false;
          if (layers.length > 1) layers.get(1).show = true;
        } else {
          layers.get(0).show = true;
          if (layers.length > 1) layers.get(1).show = false;
        }
      });
    });
  }
}
