import { Aircraft } from "../types";
import { AircraftStore, AppState } from "../store";
import { escapeHtml } from "../util";
import {
  COCKPIT,
  getAltBandColor,
  formatAltWithIndicator,
  formatAlt,
  formatSpeed,
  formatHeading,
  formatVRate,
  formatDistance,
  isEmergencySquawk,
  getEmergencyLabel,
} from "../theme";

function row(label: string, value: string): string {
  return `<div class="detail-row"><span class="detail-label">${label}</span><span class="detail-value">${value}</span></div>`;
}

export class AircraftListPanel {
  private container: HTMLElement;
  private store: AircraftStore;
  private appState: AppState;
  private searchFilter = "";
  private sortColumn = "distance";
  private sortAsc = true;
  private renderPending = false;
  private animateDetail = false;
  private collapsing = false;

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
        Aircraft <span id="ac-count" style="color: var(--accent)">0</span>
      </div>
      <div class="panel-toolbar">
        <input type="text" class="search-input" placeholder="Search callsign or ICAO..." />
        <select class="sort-select" id="sort-select">
          <option value="distance">Dist</option>
          <option value="altitude">Alt</option>
          <option value="ground_speed">Spd</option>
          <option value="callsign">Call</option>
        </select>
        <button class="sort-btn" id="sort-dir-btn">▲</button>
      </div>
      <div class="panel-body" id="ac-cards"></div>
    `;

    const searchInput = this.container.querySelector(
      ".search-input"
    ) as HTMLInputElement;
    searchInput.addEventListener("input", () => {
      this.searchFilter = searchInput.value.toLowerCase();
      this.render();
    });

    this.container
      .querySelector("#sort-select")!
      .addEventListener("change", (e) => {
        this.sortColumn = (e.target as HTMLSelectElement).value;
        this.render();
      });

    this.container
      .querySelector("#sort-dir-btn")!
      .addEventListener("click", () => {
        this.sortAsc = !this.sortAsc;
        this.container.querySelector("#sort-dir-btn")!.textContent = this.sortAsc
          ? "▲"
          : "▼";
        this.render();
      });

    const cardsContainer = this.container.querySelector("#ac-cards")!;
    cardsContainer.addEventListener("pointerdown", (e) => {
      const card = (e.target as HTMLElement).closest(".aircraft-card[data-icao]") as HTMLElement | null;
      if (!card || this.collapsing) return;
      const icao = card.dataset.icao!;

      if (this.appState.selectedIcao === icao) {
        this.collapseDetail(() => this.appState.select(null));
      } else {
        if (this.appState.selectedIcao) {
          this.collapseDetail(() => {
            this.animateDetail = true;
            this.appState.select(icao);
          });
        } else {
          this.animateDetail = true;
          this.appState.select(icao);
        }
      }
    });

    store.onChange(() => this.scheduleRender());
    appState.onSelectionChange(() => this.render());
  }

  private collapseDetail(then: () => void): void {
    const cards = this.container.querySelector("#ac-cards")!;
    const detail = cards.querySelector(".card-detail.open");
    if (!detail) {
      then();
      return;
    }
    this.collapsing = true;
    detail.classList.remove("open");
    const onEnd = () => {
      detail.removeEventListener("transitionend", onEnd);
      this.collapsing = false;
      then();
    };
    detail.addEventListener("transitionend", onEnd);
    setTimeout(() => {
      if (this.collapsing) {
        this.collapsing = false;
        then();
      }
    }, 350);
  }

  private scheduleRender(): void {
    if (this.renderPending || this.collapsing) return;
    this.renderPending = true;
    requestAnimationFrame(() => {
      this.renderPending = false;
      if (!this.collapsing) this.render();
    });
  }

  private render(): void {
    const cards = this.container.querySelector("#ac-cards")!;
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

    const html = filtered
      .map((ac) => {
        const isSelected = this.appState.selectedIcao === ac.icao;
        const selected = isSelected ? " selected" : "";
        const label = escapeHtml(ac.callsign || ac.icao);
        const altColor = getAltBandColor(ac.altitude);
        const hasCallsign = ac.callsign !== null && ac.callsign.trim() !== "";

        // Row 1 pieces
        const icaoDim = hasCallsign
          ? `<span style="color:${COCKPIT.metrics};font-size:10px;font-family:monospace">${escapeHtml(ac.icao)}</span>`
          : "";
        const gndBadge = ac.on_ground
          ? `<span style="color:${COCKPIT.gndBadge};font-size:10px;font-weight:700">GND</span>`
          : "";
        const altDisplay = ac.altitude !== null
          ? `<span style="color:${altColor};font-size:12px;font-family:monospace;font-weight:600;margin-left:auto">${formatAltWithIndicator(ac.altitude)}</span>`
          : `<span style="color:#646464;font-size:12px;font-family:monospace;margin-left:auto">---</span>`;

        // Row 2 pieces
        const spd = formatSpeed(ac.ground_speed);
        const hdg = formatHeading(ac.heading);
        const sqk = ac.squawk ? escapeHtml(ac.squawk) : "";
        const dist = formatDistance(ac.distance_nm);

        const row2Parts = [spd, hdg, sqk].filter(s => s !== "").map(
          s => `<span>${s}</span>`
        ).join("");
        const distSpan = dist ? `<span style="color:${COCKPIT.range};margin-left:auto">${dist}</span>` : "";

        // Row 3: vertical rate
        const vr = formatVRate(ac.vertical_rate);
        const vrSpan = vr.text
          ? `<span style="color:${vr.color};font-size:10px;font-family:monospace">${vr.text}</span>`
          : "";

        const detail = isSelected ? this.renderInlineDetail(ac, altColor) : "";
        const glowStyle = isSelected
          ? `box-shadow: 0 0 10px ${altColor}55, 0 2px 6px rgba(0,0,0,0.3);`
          : "";

        return `<div class="aircraft-card${selected}" data-icao="${escapeHtml(ac.icao)}" style="${glowStyle}">
  <div class="card-header" style="background:${altColor}55">
    <span>${label}</span>
  </div>
  <div class="card-body">
    <div class="card-row">
      <span style="color:${COCKPIT.statusActive};font-size:13px">●</span>
      ${gndBadge}
      ${icaoDim}
      ${altDisplay}
    </div>
    <div class="card-row" style="font-family:monospace;font-size:11px;color:${COCKPIT.metrics}">
      ${row2Parts}
      ${distSpan}
    </div>
    <div class="card-row">
      ${vrSpan}
    </div>
  </div>
  ${detail}
</div>`;
      })
      .join("");

    cards.innerHTML = html;

    const detail = cards.querySelector(".aircraft-card.selected .card-detail");
    if (detail) {
      if (this.animateDetail) {
        this.animateDetail = false;
        requestAnimationFrame(() => {
          requestAnimationFrame(() => {
            detail.classList.add("open");
          });
        });
      } else {
        detail.classList.add("open");
      }
    }
  }

  private renderInlineDetail(ac: Aircraft, altColor: string): string {
    const vr = formatVRate(ac.vertical_rate);
    const lat = ac.latitude !== null ? ac.latitude.toFixed(4) : "---";
    const lon = ac.longitude !== null ? ac.longitude.toFixed(4) : "---";

    let duration = "---";
    if (ac.trail.length > 0) {
      const oldest = new Date(ac.trail[0].ts).getTime();
      const now = Date.now();
      const secs = Math.floor((now - oldest) / 1000);
      const mins = Math.floor(secs / 60);
      const s = secs % 60;
      duration = `${mins}:${s.toString().padStart(2, "0")}`;
    }

    const emerBanner = isEmergencySquawk(ac.squawk)
      ? `<div class="emergency-banner">${getEmergencyLabel(ac.squawk!)} - SQUAWK ${escapeHtml(ac.squawk!)}</div>`
      : "";

    let extraRows = "";
    if (ac.airspeed !== null)
      extraRows += row("Airspeed", `${Math.round(ac.airspeed)} kts`);
    if (ac.selected_altitude !== null)
      extraRows += row("Sel Alt", formatAlt(ac.selected_altitude));
    if (ac.barometric_setting !== null)
      extraRows += row("Baro", `${ac.barometric_setting.toFixed(1)} hPa`);
    if (ac.wind_speed !== null && ac.wind_direction !== null)
      extraRows += row("Wind", `${ac.wind_speed}kt / ${Math.round(ac.wind_direction)}°`);
    if (ac.temperature !== null)
      extraRows += row("Temp", `${ac.temperature.toFixed(1)}°C`);
    if (ac.roll_angle !== null)
      extraRows += row("Roll", `${ac.roll_angle.toFixed(1)}°`);
    if (ac.track_angle_rate !== null) {
      const dir = ac.track_angle_rate > 0 ? "↻" : "↺";
      extraRows += row("Turn", `${Math.abs(ac.track_angle_rate).toFixed(1)}°/s ${dir}`);
    }
    if (ac.signal_level !== null)
      extraRows += row("Signal", `${Math.round(ac.signal_level * 100)}%`);

    const extraSection = extraRows
      ? `<div class="detail-section">
           <div class="detail-section-title">Extended Data</div>
           ${extraRows}
         </div>`
      : "";

    const vrSpan = vr.text
      ? `<span style="color:${vr.color}">${vr.text}</span>`
      : "---";

    return `<div class="card-detail">
      ${emerBanner}
      <div class="detail-section">
        ${row("Position", `${lat}, ${lon}`)}
        ${row("Distance", `<span style="color:${COCKPIT.range}">${formatDistance(ac.distance_nm) || "---"}</span>`)}
      </div>
      <div class="detail-section">
        <div class="detail-section-title">Flight Data</div>
        ${row("Altitude", `<span style="color:${altColor}">${formatAlt(ac.altitude)}</span>`)}
        ${row("Speed", formatSpeed(ac.ground_speed) || "---")}
        ${row("Heading", formatHeading(ac.heading) || "---")}
        ${row("V/Rate", vrSpan)}
        ${row("Squawk", escapeHtml(ac.squawk || "---"))}
        ${row("On Ground", ac.on_ground === true ? `<span style="color:${COCKPIT.gndBadge};font-weight:700">GND</span>` : (ac.on_ground === false ? "No" : "---"))}
      </div>
      <div class="detail-section">
        <div class="detail-section-title">Track</div>
        ${row("Trail Pts", `${ac.trail.length}`)}
        ${row("Duration", duration)}
      </div>
      ${extraSection}
    </div>`;
  }

  private getSortValue(ac: Aircraft): number | string {
    switch (this.sortColumn) {
      case "distance":
        return ac.distance_nm ?? 9999;
      case "altitude":
        return ac.altitude ?? -1;
      case "ground_speed":
        return ac.ground_speed ?? -1;
      case "callsign":
        return (ac.callsign || ac.icao).toLowerCase();
      default:
        return 0;
    }
  }
}
