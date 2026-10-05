# AirJedi Web - Design Spec

## Overview

A web-based version of AirJedi that replaces the desktop Bevy app with a CesiumJS 3D globe frontend and an Axum backend server. The backend reuses the existing `adsb-client` crate for ADS-B protocol handling. Aircraft state is pushed to browser clients over WebSocket.

**Deployment target:** LAN-accessible (0.0.0.0 binding, any device on the home network).

## Project Structure

New workspace member `airjedi-web` alongside existing crates:

```
airjedi-app/
  Cargo.toml              (workspace: add airjedi-web)
  src/                    (existing Bevy app - untouched)
  crates/adsb-client/     (shared by both apps)
  airjedi-fusion/         (existing - future use)
  airjedi-web/
    Cargo.toml            (Axum server binary)
    src/
      main.rs             (server startup, static file serving)
      ws.rs               (WebSocket handler - broadcasts aircraft state)
      api.rs              (REST endpoints - feeds config, settings)
      state.rs            (shared app state - feed manager, config)
      feeds.rs            (manages adsb-client instances per feed)
    client/
      index.html
      src/
        main.ts           (entry point)
        globe.ts          (CesiumJS viewer setup, imagery providers)
        aircraft.ts       (entity management - icons, labels, trails)
        websocket.ts      (WebSocket client, reconnection)
        panels/
          aircraft-list.ts
          aircraft-detail.ts
          status-bar.ts
        types.ts          (shared TypeScript types for WS messages)
      package.json
      tsconfig.json
      vite.config.ts
```

`adsb-client` has zero Bevy dependencies, so `airjedi-web` depends on it directly with no code duplication for protocol handling.

## Backend Architecture (Rust / Axum)

### Feed Manager

Wraps one or more `adsb-client::Client` instances. Each configured feed gets a separate `Client::spawn()` with its own TCP connection. The feed manager polls all clients on a ~100ms tick, merges aircraft by ICAO address into a single unified aircraft map, and broadcasts a snapshot to all WebSocket subscribers.

### WebSocket Handler

Each browser client connects at `/ws`. On connect, the server sends a full snapshot of all aircraft. After that, it sends incremental updates every ~250ms containing only aircraft that changed since the last push. Messages are JSON-encoded. The server also accepts commands from clients (add/remove feed).

### REST API

Lightweight endpoints for non-real-time operations:

- `GET /api/feeds` - list configured feeds
- `POST /api/feeds` - add a feed
- `DELETE /api/feeds/:id` - remove a feed
- `GET /api/status` - connection states, aircraft count, uptime
- `GET /api/config` - client configuration (Cesium ion token, defaults)

### Static File Server

Serves Vite-built frontend assets. In dev mode, Vite's dev server runs separately and proxies `/ws` and `/api` to Axum. In production, Axum serves from the built `client/dist/` directory.

### WebSocket Message Protocol

Server to client:

```json
{ "type": "snapshot", "aircraft": [...] }
{ "type": "update",  "aircraft": [...] }
{ "type": "remove",  "icao": ["A1B2C3", "D4E5F6"] }
{ "type": "status",  "feeds": [{ "id": "...", "address": "...", "state": "connected" }] }
```

Client to server:

```json
{ "type": "add_feed",    "address": "localhost:30005", "protocol": "beast" }
{ "type": "remove_feed", "id": "feed-uuid" }
```

### Aircraft DTO

A web-specific DTO struct in `airjedi-web` maps from `adsb-client::Aircraft` to avoid adding `serde::Serialize` to the library crate. This keeps the library dependency-light and lets the web layer control exactly what gets serialized (e.g., trail truncation, field naming). Fields sent to the client:

```json
{
  "icao": "A1B2C3",
  "callsign": "UAL1234",
  "latitude": 37.6872,
  "longitude": -97.3301,
  "altitude": 35000,
  "ground_speed": 450.0,
  "heading": 270.0,
  "vertical_rate": 0,
  "squawk": "1200",
  "on_ground": false,
  "last_seen": "2026-07-26T15:30:00Z",
  "trail": [
    { "lat": 37.69, "lon": -97.32, "alt": 35000, "ts": "..." },
    { "lat": 37.68, "lon": -97.33, "alt": 35000, "ts": "..." }
  ]
}
```

Trail points are included in snapshots and appended in updates. A configurable max trail length (default 100 points) limits payload size.

### Rust Dependencies

- `axum` - HTTP/WebSocket server
- `tokio` - async runtime (already used by adsb-client)
- `tower-http` - static file serving, CORS
- `serde` / `serde_json` - serialization
- `adsb-client` - ADS-B protocol (path dependency)
- `toml` - config file parsing
- `uuid` - feed identifiers
- `tracing` / `tracing-subscriber` - logging

## Frontend Architecture (TypeScript / CesiumJS)

### CesiumJS Viewer

Full-window Cesium viewer initialized with:

- **Primary imagery:** Cesium ion Bing Maps aerial with labels
- **Alternative imagery:** OpenStreetMap tiles (user-selectable)
- **Terrain:** Cesium World Terrain (ion) for 3D elevation
- **Scene mode:** Columbus View (2.5D) default, switchable to 3D globe via a toolbar button (top-right, next to a home/reset-view button)

The Cesium ion API token is fetched from `/api/config` on startup so it stays server-side.

### Aircraft Entity Management

Each aircraft is a `Cesium.Entity` tracked in a `Map<string, Cesium.Entity>` keyed by ICAO.

- **Position:** `Cartesian3.fromDegrees(lon, lat, altMeters)` with optional altitude exaggeration
- **Billboard:** aircraft icon rotated by heading, tinted by altitude band (matching the desktop app's altitude coloring - blue at low, green mid, yellow high, red very high)
- **Label:** callsign + altitude (FL) + ground speed, visibility scaled by camera distance
- **Trail:** `Cesium.PolylineCollection` per aircraft, colored by altitude, configurable max length
- **Interpolation:** `SampledPositionProperty` for smooth movement between WebSocket updates

On each WebSocket message:
- `snapshot`: clear all entities, rebuild from full state
- `update`: create new entities or update existing positions/labels/trails
- `remove`: remove entities by ICAO

### UI Panels

HTML/CSS overlays positioned over the Cesium canvas. No framework - vanilla TypeScript classes that own their DOM subtree.

**Aircraft List (left panel):**
- Sortable table: callsign, squawk, altitude, speed, distance from center
- Text filter / search
- Click row to select aircraft (camera tracks it, detail panel opens)
- Highlight selected row

**Aircraft Detail (right panel, shown on selection):**
- ICAO address, callsign, aircraft type (if available)
- Altitude, speed, heading, vertical rate
- Position (lat/lon), distance from center
- Squawk code, on-ground status
- Track history point count
- Close button to deselect

**Status Bar (bottom):**
- Feed connection states (green/red dots per feed)
- Total aircraft count
- Camera center coordinates
- Current zoom/altitude level

**Settings (modal or slide-out, triggered by gear icon):**
- Feed management: list feeds, add new (address + protocol), remove
- Basemap selection: Cesium ion / OSM
- Trail length slider
- Altitude coloring toggle

### State Management

Simple pub/sub pattern:

- `AircraftStore` - holds `Map<string, Aircraft>`, emits `change` and `remove` events
- `WebSocketClient` - manages connection with auto-reconnect, feeds data into `AircraftStore`
- `AppState` - selected aircraft, panel visibility, settings
- UI panels subscribe to store/state events and re-render affected DOM

### Frontend Dependencies

- `cesium` (CesiumJS) - 3D globe
- `vite` - build tool and dev server
- `typescript` - type safety
- `vite-plugin-cesium` - handles Cesium's static asset copying

No UI framework (React/Vue/etc). No CSS framework. Minimal dependencies.

## Development Workflow

### Dev Mode (two terminals)

1. `cd airjedi-web && cargo run` - Axum on port 3000
2. `cd airjedi-web/client && npm run dev` - Vite on port 5173, proxies `/ws` and `/api` to 3000

### Production Build

1. `cd airjedi-web/client && npm run build` - outputs to `client/dist/`
2. `cargo build --release -p airjedi-web` - server binary

### Configuration

Reads from `airjedi-web.toml` (or shares the existing `config.toml` feed format):

```toml
[server]
listen = "0.0.0.0:3000"

[cesium]
ion_token = "your-token-here"

[[feeds]]
address = "localhost:30005"
protocol = "beast"

[[feeds]]
address = "192.168.1.50:30003"
protocol = "basestation"
```

Cesium ion token can also be set via `CESIUM_ION_TOKEN` environment variable (takes precedence over config file).

## V1 Feature Scope

### In scope:
- Cesium 3D globe with terrain + OSM basemap options
- Real-time aircraft positions from ADS-B feeds via WebSocket
- Aircraft icons with altitude-based coloring
- Aircraft labels (callsign, altitude, speed)
- Flight trails / track history
- Aircraft list panel (sortable table)
- Aircraft detail panel (click to inspect)
- Map interaction (pan, zoom, tilt/rotate)
- Data source configuration (add/remove feeds)
- Connection status indicator

### Deferred:
- Flight path prediction (IMM filter)
- Recording/playback
- Measurement tools
- Weather/METAR overlay
- Airport/navaid display
- Bookmarks
- Coverage visualization
- Airspace boundaries
- Day/night cycle / sun positioning
- Emergency alerts
- airjedi-fusion integration
