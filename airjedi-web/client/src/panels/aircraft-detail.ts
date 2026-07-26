import { AircraftStore, AppState } from "../store";
import { escapeHtml } from "../util";

export class AircraftDetailPanel {
  private container: HTMLElement;
  private store: AircraftStore;

  constructor(
    container: HTMLElement,
    store: AircraftStore,
    appState: AppState
  ) {
    this.container = container;
    this.store = store;

    appState.onSelectionChange((icao) => {
      if (icao) {
        this.container.style.display = "block";
        this.render(icao);
      } else {
        this.container.style.display = "none";
      }
    });

    store.onChange(() => {
      if (appState.selectedIcao) {
        this.render(appState.selectedIcao);
      }
    });
  }

  private render(icao: string): void {
    const ac = this.store.get(icao);
    if (!ac) {
      this.container.style.display = "none";
      return;
    }

    const heading =
      ac.heading !== null ? `${Math.round(ac.heading)}deg` : "---";
    const vrate =
      ac.vertical_rate !== null
        ? `${ac.vertical_rate > 0 ? "+" : ""}${ac.vertical_rate} fpm`
        : "---";
    const alt =
      ac.altitude !== null ? `${ac.altitude.toLocaleString()} ft` : "---";
    const spd =
      ac.ground_speed !== null
        ? `${Math.round(ac.ground_speed)} kts`
        : "---";
    const lat = ac.latitude !== null ? ac.latitude.toFixed(4) : "---";
    const lon = ac.longitude !== null ? ac.longitude.toFixed(4) : "---";

    this.container.innerHTML = `
      <div class="panel-header">
        ${escapeHtml(ac.callsign || ac.icao)}
        <button class="close-btn" id="detail-close">x</button>
      </div>
      <div class="panel-body" style="padding: 4px 0;">
        <div class="detail-row"><span class="detail-label">ICAO</span><span class="detail-value">${escapeHtml(ac.icao)}</span></div>
        <div class="detail-row"><span class="detail-label">Callsign</span><span class="detail-value">${escapeHtml(ac.callsign || "---")}</span></div>
        <div class="detail-row"><span class="detail-label">Altitude</span><span class="detail-value">${alt}</span></div>
        <div class="detail-row"><span class="detail-label">Speed</span><span class="detail-value">${spd}</span></div>
        <div class="detail-row"><span class="detail-label">Heading</span><span class="detail-value">${heading}</span></div>
        <div class="detail-row"><span class="detail-label">V/Rate</span><span class="detail-value">${vrate}</span></div>
        <div class="detail-row"><span class="detail-label">Position</span><span class="detail-value">${lat}, ${lon}</span></div>
        <div class="detail-row"><span class="detail-label">Squawk</span><span class="detail-value">${escapeHtml(ac.squawk || "---")}</span></div>
        <div class="detail-row"><span class="detail-label">On Ground</span><span class="detail-value">${ac.on_ground ?? "---"}</span></div>
        <div class="detail-row"><span class="detail-label">Trail Points</span><span class="detail-value">${ac.trail.length}</span></div>
      </div>
    `;

    this.container
      .querySelector("#detail-close")!
      .addEventListener("click", () => {
        this.container.style.display = "none";
      });
  }
}
