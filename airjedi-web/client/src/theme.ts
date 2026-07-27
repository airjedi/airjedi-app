export const COCKPIT = {
  bgPrimary: "#272a2e",
  bgSecondary: "#1c1e22",
  bgTriage: "#141518",
  bgAuxiliary: "#32353a",
  bgContrast: "#3e4147",
  accent: "#cc6622",
  accent2: "#e09440",
  text: "#d0d2d6",
  textDim: "#808590",
  overlay: "#4a4d54",
  success: "#7dba6a",
  warn: "#e0b050",
  error: "#d45050",
  altLow: "#5a9ea0",
  altHigh: "#cc6622",
  altUltra: "#e09440",
  metrics: "#aaaaaa",
  range: "#64c8ff",
  statusActive: "#64ff64",
  vrateUp: "#64ff64",
  vrateDown: "#ff9664",
  vrateLevel: "#969696",
  gndBadge: "#b48c50",
  milBadge: "#dcb43c",
  mfgModel: "#b4a0dc",
} as const;

export function getAltBandColor(alt: number | null): string {
  if (alt === null) return "#646464";
  if (alt >= 30000) return "#c864ff";
  if (alt >= 20000) return "#ff9632";
  if (alt >= 10000) return "#c8c864";
  return "#64c8c8";
}

export function getAltIndicator(alt: number | null): string {
  if (alt === null) return "─";
  if (alt >= 10000) return "▲";
  return "▼";
}

export function formatAltWithIndicator(alt: number | null): string {
  if (alt === null) return "---";
  const indicator = getAltIndicator(alt);
  if (alt >= 18000) return `${indicator} FL${String(Math.round(alt / 100)).padStart(3, "0")}`;
  return `${indicator} ${alt} ft`;
}

export function formatAlt(alt: number | null): string {
  if (alt === null) return "---";
  if (alt >= 18000) return `FL${String(Math.round(alt / 100)).padStart(3, "0")}`;
  return `${alt} ft`;
}

export function formatSpeed(spd: number | null): string {
  if (spd === null) return "";
  return `${String(Math.round(spd)).padStart(3, "0")}kt`;
}

export function formatHeading(hdg: number | null): string {
  if (hdg === null) return "";
  return `${String(Math.round(hdg)).padStart(3, "0")}°`;
}

export function formatVRate(vr: number | null): { text: string; color: string } {
  if (vr === null) return { text: "", color: COCKPIT.vrateLevel };
  if (vr > 100) return { text: `↑ ${Math.abs(vr)}ft/min`, color: COCKPIT.vrateUp };
  if (vr < -100) return { text: `↓ ${Math.abs(vr)}ft/min`, color: COCKPIT.vrateDown };
  return { text: `─ level`, color: COCKPIT.vrateLevel };
}

export function formatDistance(nm: number | null): string {
  if (nm === null) return "";
  return `${nm.toFixed(1)}nm`;
}

export function isEmergencySquawk(squawk: string | null): boolean {
  return squawk === "7500" || squawk === "7600" || squawk === "7700";
}

export function getEmergencyLabel(squawk: string): string {
  if (squawk === "7500") return "HIJACK";
  if (squawk === "7600") return "RADIO";
  if (squawk === "7700") return "EMERG";
  return "";
}
