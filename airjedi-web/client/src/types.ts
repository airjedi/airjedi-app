export interface TrailPoint {
  lat: number;
  lon: number;
  alt: number | null;
  ts: string;
}

export interface Aircraft {
  icao: string;
  callsign: string | null;
  latitude: number | null;
  longitude: number | null;
  altitude: number | null;
  ground_speed: number | null;
  heading: number | null;
  vertical_rate: number | null;
  squawk: string | null;
  on_ground: boolean | null;
  last_seen: string;
  trail: TrailPoint[];
}

export interface FeedStatus {
  id: string;
  address: string;
  state: string;
}

export type ServerMessage =
  | { type: "snapshot"; aircraft: Aircraft[] }
  | { type: "update"; aircraft: Aircraft[] }
  | { type: "remove"; icao: string[] }
  | { type: "status"; feeds: FeedStatus[] };

export type ClientMessage =
  | { type: "add_feed"; address: string; protocol: string }
  | { type: "remove_feed"; id: string };
