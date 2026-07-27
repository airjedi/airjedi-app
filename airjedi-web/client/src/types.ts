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
  emergency: boolean | null;
  alert: boolean | null;
  category: number | null;
  airspeed: number | null;
  roll_angle: number | null;
  track_angle_rate: number | null;
  selected_altitude: number | null;
  barometric_setting: number | null;
  wind_speed: number | null;
  wind_direction: number | null;
  temperature: number | null;
  signal_level: number | null;
  distance_nm: number | null;
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
