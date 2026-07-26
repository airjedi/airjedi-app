import { Aircraft } from "../types";
import { AircraftStore, AppState } from "../store";
import { escapeHtml } from "../util";

export class AircraftListPanel {
  private container: HTMLElement;
  private store: AircraftStore;
  private appState: AppState;
  private searchFilter = "";
  private sortColumn = "callsign";
  private sortAsc = true;

  constructor(
    container: HTMLElement,
    store: AircraftStore,
    appState: AppState
  ) {
    this.container = container;
    this.store = store;
    this.appState = appState;

    this.container.innerHTML = `
      <div class="panel-header">
        Aircraft <span id="ac-count">0</span>
      </div>
      <input type="text" class="search-input" placeholder="Filter..." />
      <div class="panel-body">
        <table class="aircraft-table">
          <thead>
            <tr>
              <th data-col="callsign">Callsign</th>
              <th data-col="squawk">Squawk</th>
              <th data-col="altitude">Alt</th>
              <th data-col="ground_speed">Spd</th>
            </tr>
          </thead>
          <tbody id="ac-tbody"></tbody>
        </table>
      </div>
    `;

    const searchInput = this.container.querySelector(
      ".search-input"
    ) as HTMLInputElement;
    searchInput.addEventListener("input", () => {
      this.searchFilter = searchInput.value.toLowerCase();
      this.render();
    });

    this.container.querySelectorAll("th[data-col]").forEach((th) => {
      th.addEventListener("click", () => {
        const col = (th as HTMLElement).dataset.col!;
        if (this.sortColumn === col) {
          this.sortAsc = !this.sortAsc;
        } else {
          this.sortColumn = col;
          this.sortAsc = true;
        }
        this.render();
      });
    });

    store.onChange(() => this.render());
    appState.onSelectionChange(() => this.render());
  }

  private render(): void {
    const tbody = this.container.querySelector("#ac-tbody")!;
    const countEl = this.container.querySelector("#ac-count")!;
    const aircraft = Array.from(this.store.getAll().values());

    const filtered = aircraft.filter((ac) => {
      if (!this.searchFilter) return true;
      const text =
        `${ac.callsign || ""} ${ac.icao} ${ac.squawk || ""}`.toLowerCase();
      return text.includes(this.searchFilter);
    });

    filtered.sort((a, b) => {
      const av = this.getSortValue(a);
      const bv = this.getSortValue(b);
      if (av < bv) return this.sortAsc ? -1 : 1;
      if (av > bv) return this.sortAsc ? 1 : -1;
      return 0;
    });

    countEl.textContent = `${filtered.length}`;

    const rows = filtered
      .map((ac) => {
        const selected =
          this.appState.selectedIcao === ac.icao ? "selected" : "";
        const alt =
          ac.altitude !== null ? ac.altitude.toLocaleString() : "---";
        const spd =
          ac.ground_speed !== null ? Math.round(ac.ground_speed) : "---";
        return `<tr class="${selected}" data-icao="${escapeHtml(ac.icao)}">
        <td>${escapeHtml(ac.callsign || ac.icao)}</td>
        <td>${escapeHtml(ac.squawk || "---")}</td>
        <td>${alt}</td>
        <td>${spd}</td>
      </tr>`;
      })
      .join("");

    tbody.innerHTML = rows;

    tbody.querySelectorAll("tr[data-icao]").forEach((row) => {
      row.addEventListener("click", () => {
        const icao = (row as HTMLElement).dataset.icao!;
        this.appState.select(icao);
      });
    });
  }

  private getSortValue(ac: Aircraft): string | number {
    switch (this.sortColumn) {
      case "callsign":
        return (ac.callsign || ac.icao).toLowerCase();
      case "squawk":
        return ac.squawk || "9999";
      case "altitude":
        return ac.altitude ?? -1;
      case "ground_speed":
        return ac.ground_speed ?? -1;
      default:
        return "";
    }
  }
}
