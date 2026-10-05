# AirJedi Web Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a web-based aircraft tracker using CesiumJS + Axum that reuses the existing `adsb-client` crate for real-time ADS-B data.

**Architecture:** An Axum server wraps `adsb-client::Client` instances and pushes aircraft state to browser clients over WebSocket. The frontend is vanilla TypeScript with CesiumJS for 3D globe rendering. Both live in a new `airjedi-web` workspace member. Vite handles frontend builds; in dev mode it proxies API/WS calls to Axum.

**Tech Stack:** Rust (Axum, tokio, serde), TypeScript, CesiumJS, Vite

## Global Constraints

- Rust edition 2021, workspace resolver 2
- `adsb-client` crate is a path dependency at `crates/adsb-client` - do not modify it; build DTOs in `airjedi-web`
- CesiumJS requires a Cesium ion token for terrain/imagery - read from `CESIUM_ION_TOKEN` env var or config file
- Server binds `0.0.0.0:3000` for LAN access
- No UI framework (React/Vue) - vanilla TypeScript + DOM
- Commit style: plain imperative, no conventional prefixes, no emojis

## File Map

### Rust Backend (`airjedi-web/`)

| File | Responsibility |
|------|---------------|
| `Cargo.toml` | Binary crate, depends on axum, tokio, serde, tower-http, adsb-client |
| `src/main.rs` | Server startup, router assembly, static file serving, config loading |
| `src/config.rs` | Config struct (TOML), feed definitions, Cesium token, server listen address |
| `src/feeds.rs` | `FeedManager` - spawns/stops `adsb-client::Client` instances, polls aircraft, merges by ICAO |
| `src/dto.rs` | `AircraftDto`, `TrailPointDto`, `FeedStatusDto` - serializable structs mapped from adsb-client types |
| `src/ws.rs` | WebSocket upgrade handler, per-client broadcast loop, client command dispatch |
| `src/api.rs` | REST handlers: GET/POST/DELETE feeds, GET status, GET config |

### TypeScript Frontend (`airjedi-web/client/`)

| File | Responsibility |
|------|---------------|
| `index.html` | Shell HTML with Cesium container div and panel mount points |
| `src/main.ts` | Entry point - fetches config, inits globe, connects WebSocket, mounts panels |
| `src/types.ts` | TypeScript interfaces matching backend DTOs and WS message protocol |
| `src/globe.ts` | CesiumJS Viewer setup, imagery/terrain providers, scene mode toggle |
| `src/aircraft.ts` | `AircraftManager` - creates/updates/removes Cesium entities from store events |
| `src/websocket.ts` | `WebSocketClient` - connect, reconnect, parse messages, feed into AircraftStore |
| `src/store.ts` | `AircraftStore` - Map<string, Aircraft>, emits change/remove events; `AppState` - selection, settings |
| `src/panels/aircraft-list.ts` | Sortable aircraft table, search filter, row click to select |
| `src/panels/aircraft-detail.ts` | Selected aircraft info panel |
| `src/panels/status-bar.ts` | Connection status, aircraft count, camera position |
| `src/panels/settings.ts` | Feed management, basemap selection, trail config |
| `src/style.css` | All panel/overlay styling |
| `package.json` | Dependencies: cesium, vite, typescript, vite-plugin-cesium |
| `tsconfig.json` | Strict TypeScript config |
| `vite.config.ts` | Vite config with Cesium plugin and API proxy |

---

### Task 1: Project Scaffolding and Build Verification

**Files:**
- Create: `airjedi-web/Cargo.toml`
- Create: `airjedi-web/src/main.rs`
- Create: `airjedi-web/client/package.json`
- Create: `airjedi-web/client/tsconfig.json`
- Create: `airjedi-web/client/vite.config.ts`
- Create: `airjedi-web/client/index.html`
- Create: `airjedi-web/client/src/main.ts`
- Modify: `Cargo.toml` (workspace members list)

**Interfaces:**
- Consumes: nothing (first task)
- Produces: A running Axum server on port 3000 that serves a "Hello from AirJedi Web" page. Vite dev server on port 5173 proxies to it.

- [ ] **Step 1: Add airjedi-web to workspace**

In the root `Cargo.toml`, add `"airjedi-web"` to the workspace members:

```toml
[workspace]
members = ["airjedi-fusion", "crates/adsb-client", "airjedi-web"]
```

- [ ] **Step 2: Create the Rust binary crate**

Create `airjedi-web/Cargo.toml`:

```toml
[package]
name = "airjedi-web"
version = "0.1.0"
edition = "2021"

[dependencies]
axum = { version = "0.8", features = ["ws"] }
tokio = { version = "1", features = ["rt-multi-thread", "macros", "signal"] }
tower-http = { version = "0.6", features = ["fs", "cors"] }
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"
toml = "0.8"
uuid = { version = "1", features = ["v4", "serde"] }
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
chrono = { version = "0.4", features = ["serde"] }
adsb-client = { path = "../crates/adsb-client" }
```

Create `airjedi-web/src/main.rs`:

```rust
use axum::{Router, routing::get, response::Html};
use tower_http::services::ServeDir;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();

    let app = Router::new()
        .route("/api/health", get(|| async { "ok" }))
        .fallback_service(ServeDir::new("client/dist"));

    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await.unwrap();
    tracing::info!("listening on {}", listener.local_addr().unwrap());
    axum::serve(listener, app).await.unwrap();
}
```

- [ ] **Step 3: Verify Rust builds**

Run: `cargo build -p airjedi-web`
Expected: compiles successfully with no errors.

- [ ] **Step 4: Create the frontend project**

Create `airjedi-web/client/package.json`:

```json
{
  "name": "airjedi-web-client",
  "private": true,
  "type": "module",
  "scripts": {
    "dev": "vite",
    "build": "tsc && vite build",
    "preview": "vite preview"
  },
  "dependencies": {
    "cesium": "^1.124"
  },
  "devDependencies": {
    "typescript": "^5.7",
    "vite": "^6.0",
    "vite-plugin-cesium": "^1.2"
  }
}
```

Create `airjedi-web/client/tsconfig.json`:

```json
{
  "compilerOptions": {
    "target": "ES2022",
    "module": "ES2022",
    "moduleResolution": "bundler",
    "strict": true,
    "noUnusedLocals": true,
    "noUnusedParameters": true,
    "outDir": "dist",
    "rootDir": "src",
    "sourceMap": true
  },
  "include": ["src"]
}
```

Create `airjedi-web/client/vite.config.ts`:

```typescript
import { defineConfig } from "vite";
import cesium from "vite-plugin-cesium";

export default defineConfig({
  plugins: [cesium()],
  server: {
    port: 5173,
    proxy: {
      "/api": "http://localhost:3000",
      "/ws": {
        target: "ws://localhost:3000",
        ws: true,
      },
    },
  },
});
```

Create `airjedi-web/client/index.html`:

```html
<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8" />
  <meta name="viewport" content="width=device-width, initial-scale=1.0" />
  <title>AirJedi</title>
</head>
<body>
  <div id="cesium-container"></div>
  <script type="module" src="/src/main.ts"></script>
</body>
</html>
```

Create `airjedi-web/client/src/main.ts`:

```typescript
console.log("AirJedi Web starting");
const container = document.getElementById("cesium-container");
if (container) {
  container.textContent = "AirJedi Web - Cesium will load here";
}
```

- [ ] **Step 5: Install npm dependencies and verify frontend builds**

Run:
```bash
cd airjedi-web/client && npm install
npm run build
```
Expected: builds to `dist/` with no errors.

- [ ] **Step 6: Verify end-to-end serving**

Run: `cargo run -p airjedi-web` (in one terminal)
Then open `http://localhost:3000` in a browser.
Expected: page shows "AirJedi Web - Cesium will load here".

Also verify: `curl http://localhost:3000/api/health` returns `ok`.

- [ ] **Step 7: Commit**

```bash
git add airjedi-web/ Cargo.toml
git commit -m "Add airjedi-web workspace member with Axum server and Vite frontend scaffold"
```

---

### Task 2: Configuration and DTOs

**Files:**
- Create: `airjedi-web/src/config.rs`
- Create: `airjedi-web/src/dto.rs`
- Modify: `airjedi-web/src/main.rs` (add mod declarations)

**Interfaces:**
- Consumes: `adsb-client::Aircraft` (fields: icao, callsign, latitude, longitude, altitude, track, velocity, vertical_rate, squawk, is_on_ground, heading, last_seen, position_history), `adsb-client::PositionPoint` (fields: lat, lon, altitude, timestamp), `adsb-client::Icao` (Display impl formats as 6-char hex)
- Produces:
  - `Config` struct with `load(path) -> Result<Config>` and fields: `server.listen: String`, `cesium.ion_token: Option<String>`, `feeds: Vec<FeedConfig>`
  - `FeedConfig` struct with fields: `id: Uuid`, `address: String`, `protocol: String`
  - `AircraftDto` struct implementing `Serialize` with fields matching the spec JSON
  - `TrailPointDto` struct implementing `Serialize` with fields: `lat, lon, alt, ts`
  - `AircraftDto::from_aircraft(aircraft: &adsb_client::Aircraft, max_trail: usize) -> Self`
  - `FeedStatusDto` struct with fields: `id: Uuid`, `address: String`, `state: String`
  - WebSocket message enums: `ServerMessage` (Snapshot, Update, Remove, Status) and `ClientMessage` (AddFeed, RemoveFeed)

- [ ] **Step 1: Create config module**

Create `airjedi-web/src/config.rs`:

```rust
use serde::{Deserialize, Serialize};
use std::path::Path;
use uuid::Uuid;

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub server: ServerConfig,
    #[serde(default)]
    pub cesium: CesiumConfig,
    #[serde(default)]
    pub feeds: Vec<FeedConfig>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ServerConfig {
    #[serde(default = "default_listen")]
    pub listen: String,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            listen: default_listen(),
        }
    }
}

fn default_listen() -> String {
    "0.0.0.0:3000".to_string()
}

#[derive(Debug, Clone, Deserialize)]
pub struct CesiumConfig {
    pub ion_token: Option<String>,
}

impl Default for CesiumConfig {
    fn default() -> Self {
        Self { ion_token: None }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeedConfig {
    #[serde(default = "Uuid::new_v4")]
    pub id: Uuid,
    pub address: String,
    #[serde(default = "default_protocol")]
    pub protocol: String,
}

fn default_protocol() -> String {
    "basestation".to_string()
}

impl Config {
    pub fn load(path: &Path) -> Result<Self, Box<dyn std::error::Error>> {
        let contents = std::fs::read_to_string(path)?;
        let config: Config = toml::from_str(&contents)?;
        Ok(config)
    }

    pub fn cesium_ion_token(&self) -> Option<String> {
        std::env::var("CESIUM_ION_TOKEN")
            .ok()
            .or_else(|| self.cesium.ion_token.clone())
    }
}
```

- [ ] **Step 2: Create DTO module**

Create `airjedi-web/src/dto.rs`:

```rust
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize)]
pub struct TrailPointDto {
    pub lat: f64,
    pub lon: f64,
    pub alt: Option<i32>,
    pub ts: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AircraftDto {
    pub icao: String,
    pub callsign: Option<String>,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub altitude: Option<i32>,
    pub ground_speed: Option<f64>,
    pub heading: Option<f64>,
    pub vertical_rate: Option<i32>,
    pub squawk: Option<String>,
    pub on_ground: Option<bool>,
    pub last_seen: DateTime<Utc>,
    pub trail: Vec<TrailPointDto>,
}

impl AircraftDto {
    pub fn from_aircraft(ac: &adsb_client::Aircraft, max_trail: usize) -> Self {
        let trail_start = ac.position_history.len().saturating_sub(max_trail);
        let trail = ac.position_history[trail_start..]
            .iter()
            .map(|p| TrailPointDto {
                lat: p.lat,
                lon: p.lon,
                alt: p.altitude,
                ts: p.timestamp,
            })
            .collect();

        Self {
            icao: format!("{}", ac.icao),
            callsign: ac.callsign.clone(),
            latitude: ac.latitude,
            longitude: ac.longitude,
            altitude: ac.altitude,
            ground_speed: ac.velocity,
            heading: ac.track.or(ac.heading),
            vertical_rate: ac.vertical_rate,
            squawk: ac.squawk.clone(),
            on_ground: ac.is_on_ground,
            last_seen: ac.last_seen,
            trail,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct FeedStatusDto {
    pub id: Uuid,
    pub address: String,
    pub state: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMessage {
    Snapshot { aircraft: Vec<AircraftDto> },
    Update { aircraft: Vec<AircraftDto> },
    Remove { icao: Vec<String> },
    Status { feeds: Vec<FeedStatusDto> },
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientMessage {
    AddFeed { address: String, protocol: String },
    RemoveFeed { id: Uuid },
}
```

- [ ] **Step 3: Wire modules into main.rs**

Add to the top of `airjedi-web/src/main.rs`:

```rust
mod config;
mod dto;
```

- [ ] **Step 4: Verify it compiles**

Run: `cargo build -p airjedi-web`
Expected: compiles with no errors.

- [ ] **Step 5: Commit**

```bash
git add airjedi-web/src/config.rs airjedi-web/src/dto.rs airjedi-web/src/main.rs
git commit -m "Add config loading and DTO types for WebSocket protocol"
```

---

### Task 3: Feed Manager

**Files:**
- Create: `airjedi-web/src/feeds.rs`
- Modify: `airjedi-web/src/main.rs` (add mod declaration)

**Interfaces:**
- Consumes: `adsb_client::{Client, ClientConfig, ConnectionConfig, TrackerConfig, ProtocolType}`, `config::FeedConfig` (fields: id, address, protocol), `dto::AircraftDto::from_aircraft(ac, max_trail)`
- Produces:
  - `FeedManager::new() -> Self`
  - `FeedManager::add_feed(&mut self, config: FeedConfig)` - spawns a new `adsb_client::Client`
  - `FeedManager::remove_feed(&mut self, id: Uuid)` - shuts down and removes a client
  - `FeedManager::get_all_aircraft(&self, max_trail: usize) -> Vec<AircraftDto>` - merges aircraft from all feeds
  - `FeedManager::feed_statuses(&self) -> Vec<FeedStatusDto>`
  - `FeedManager::process(&mut self)` - calls `process_next` on all clients (non-blocking poll)

- [ ] **Step 1: Create the feed manager**

Create `airjedi-web/src/feeds.rs`:

```rust
use std::collections::HashMap;
use uuid::Uuid;

use adsb_client::{Client, ClientConfig, ConnectionConfig, ProtocolType, TrackerConfig};
use crate::config::FeedConfig;
use crate::dto::{AircraftDto, FeedStatusDto};

struct ManagedFeed {
    id: Uuid,
    address: String,
    protocol: String,
    client: Client,
}

pub struct FeedManager {
    feeds: HashMap<Uuid, ManagedFeed>,
    max_trail_points: usize,
}

impl FeedManager {
    pub fn new() -> Self {
        Self {
            feeds: HashMap::new(),
            max_trail_points: 100,
        }
    }

    pub fn add_feed(&mut self, config: FeedConfig) {
        let protocol = match config.protocol.to_lowercase().as_str() {
            "beast" => ProtocolType::Beast,
            _ => ProtocolType::BaseStation,
        };

        let client = Client::spawn(ClientConfig {
            connection: ConnectionConfig {
                address: config.address.clone(),
                ..Default::default()
            },
            tracker: TrackerConfig::default(),
            protocol,
            ..Default::default()
        });

        self.feeds.insert(config.id, ManagedFeed {
            id: config.id,
            address: config.address,
            protocol: config.protocol,
            client,
        });

        tracing::info!("added feed {}: {}", config.id, self.feeds[&config.id].address);
    }

    pub fn remove_feed(&mut self, id: Uuid) {
        if let Some(feed) = self.feeds.remove(&id) {
            feed.client.shutdown();
            tracing::info!("removed feed {}: {}", id, feed.address);
        }
    }

    pub fn get_all_aircraft(&self) -> Vec<AircraftDto> {
        let mut merged: HashMap<String, AircraftDto> = HashMap::new();

        for feed in self.feeds.values() {
            for ac in feed.client.get_aircraft() {
                let dto = AircraftDto::from_aircraft(&ac, self.max_trail_points);
                merged
                    .entry(dto.icao.clone())
                    .and_modify(|existing| {
                        if dto.last_seen > existing.last_seen {
                            *existing = dto.clone();
                        }
                    })
                    .or_insert(dto);
            }
        }

        merged.into_values().collect()
    }

    pub fn feed_statuses(&self) -> Vec<FeedStatusDto> {
        self.feeds
            .values()
            .map(|f| FeedStatusDto {
                id: f.id,
                address: f.address.clone(),
                state: format!("{:?}", f.client.connection_state()),
            })
            .collect()
    }

    pub fn feed_configs(&self) -> Vec<FeedConfig> {
        self.feeds
            .values()
            .map(|f| FeedConfig {
                id: f.id,
                address: f.address.clone(),
                protocol: f.protocol.clone(),
            })
            .collect()
    }
}
```

- [ ] **Step 2: Wire into main.rs**

Add `mod feeds;` to `airjedi-web/src/main.rs`.

- [ ] **Step 3: Verify it compiles**

Run: `cargo build -p airjedi-web`
Expected: compiles (unused warnings are fine at this stage).

- [ ] **Step 4: Commit**

```bash
git add airjedi-web/src/feeds.rs airjedi-web/src/main.rs
git commit -m "Add FeedManager wrapping adsb-client for multi-feed aircraft tracking"
```

---

### Task 4: WebSocket Handler and REST API

**Files:**
- Create: `airjedi-web/src/ws.rs`
- Create: `airjedi-web/src/api.rs`
- Create: `airjedi-web/src/state.rs`
- Modify: `airjedi-web/src/main.rs` (full rewrite with router, shared state, broadcast loop)

**Interfaces:**
- Consumes: `FeedManager::get_all_aircraft() -> Vec<AircraftDto>`, `FeedManager::feed_statuses() -> Vec<FeedStatusDto>`, `FeedManager::add_feed(FeedConfig)`, `FeedManager::remove_feed(Uuid)`, `FeedManager::feed_configs() -> Vec<FeedConfig>`, `Config::load(path) -> Result<Config>`, `Config::cesium_ion_token() -> Option<String>`, `ServerMessage` variants (Snapshot, Update, Remove, Status), `ClientMessage` variants (AddFeed, RemoveFeed)
- Produces:
  - `AppState` (shared state with `Arc<Mutex<FeedManager>>` and broadcast channel)
  - WebSocket endpoint at `/ws` that sends `ServerMessage` JSON and receives `ClientMessage` JSON
  - REST endpoints: `GET /api/feeds`, `POST /api/feeds`, `DELETE /api/feeds/:id`, `GET /api/status`, `GET /api/config`
  - Background task that polls feeds every 250ms and broadcasts updates

- [ ] **Step 1: Create shared state**

Create `airjedi-web/src/state.rs`:

```rust
use std::sync::Arc;
use tokio::sync::{broadcast, Mutex};

use crate::config::Config;
use crate::dto::ServerMessage;
use crate::feeds::FeedManager;

pub type SharedState = Arc<AppState>;

pub struct AppState {
    pub feeds: Mutex<FeedManager>,
    pub broadcast_tx: broadcast::Sender<ServerMessage>,
    pub config: Config,
}

impl AppState {
    pub fn new(config: Config) -> Self {
        let (broadcast_tx, _) = broadcast::channel(64);
        Self {
            feeds: Mutex::new(FeedManager::new()),
            broadcast_tx,
            config,
        }
    }
}
```

- [ ] **Step 2: Create REST API handlers**

Create `airjedi-web/src/api.rs`:

```rust
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde::Serialize;
use uuid::Uuid;

use crate::config::FeedConfig;
use crate::state::SharedState;

#[derive(Serialize)]
pub struct StatusResponse {
    pub aircraft_count: usize,
    pub feeds: Vec<crate::dto::FeedStatusDto>,
}

#[derive(Serialize)]
pub struct ClientConfig {
    pub cesium_ion_token: Option<String>,
}

pub async fn get_feeds(State(state): State<SharedState>) -> Json<Vec<FeedConfig>> {
    let feeds = state.feeds.lock().await;
    Json(feeds.feed_configs())
}

pub async fn add_feed(
    State(state): State<SharedState>,
    Json(mut config): Json<FeedConfig>,
) -> (StatusCode, Json<FeedConfig>) {
    config.id = Uuid::new_v4();
    let mut feeds = state.feeds.lock().await;
    feeds.add_feed(config.clone());
    (StatusCode::CREATED, Json(config))
}

pub async fn delete_feed(
    State(state): State<SharedState>,
    Path(id): Path<Uuid>,
) -> StatusCode {
    let mut feeds = state.feeds.lock().await;
    feeds.remove_feed(id);
    StatusCode::NO_CONTENT
}

pub async fn get_status(State(state): State<SharedState>) -> Json<StatusResponse> {
    let feeds = state.feeds.lock().await;
    Json(StatusResponse {
        aircraft_count: feeds.get_all_aircraft().len(),
        feeds: feeds.feed_statuses(),
    })
}

pub async fn get_config(State(state): State<SharedState>) -> Json<ClientConfig> {
    Json(ClientConfig {
        cesium_ion_token: state.config.cesium_ion_token(),
    })
}
```

- [ ] **Step 3: Create WebSocket handler**

Create `airjedi-web/src/ws.rs`:

```rust
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::response::IntoResponse;
use futures_util::{SinkExt, StreamExt};
use uuid::Uuid;

use crate::config::FeedConfig;
use crate::dto::{ClientMessage, ServerMessage};
use crate::state::SharedState;

pub async fn ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<SharedState>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_socket(socket, state))
}

async fn handle_socket(socket: WebSocket, state: SharedState) {
    let (mut sender, mut receiver) = socket.split();

    // Send initial snapshot
    {
        let feeds = state.feeds.lock().await;
        let aircraft = feeds.get_all_aircraft();
        let msg = ServerMessage::Snapshot { aircraft };
        if let Ok(json) = serde_json::to_string(&msg) {
            if sender.send(Message::Text(json.into())).await.is_err() {
                return;
            }
        }
    }

    let mut broadcast_rx = state.broadcast_tx.subscribe();

    let send_task = tokio::spawn(async move {
        while let Ok(msg) = broadcast_rx.recv().await {
            if let Ok(json) = serde_json::to_string(&msg) {
                if sender.send(Message::Text(json.into())).await.is_err() {
                    break;
                }
            }
        }
    });

    let state_clone = state.clone();
    let recv_task = tokio::spawn(async move {
        while let Some(Ok(msg)) = receiver.next().await {
            if let Message::Text(text) = msg {
                if let Ok(cmd) = serde_json::from_str::<ClientMessage>(&text) {
                    handle_client_message(cmd, &state_clone).await;
                }
            }
        }
    });

    tokio::select! {
        _ = send_task => {},
        _ = recv_task => {},
    }
}

async fn handle_client_message(msg: ClientMessage, state: &SharedState) {
    let mut feeds = state.feeds.lock().await;
    match msg {
        ClientMessage::AddFeed { address, protocol } => {
            feeds.add_feed(FeedConfig {
                id: Uuid::new_v4(),
                address,
                protocol,
            });
        }
        ClientMessage::RemoveFeed { id } => {
            feeds.remove_feed(id);
        }
    }
}
```

- [ ] **Step 4: Rewrite main.rs with full router and broadcast loop**

Replace `airjedi-web/src/main.rs` with:

```rust
mod api;
mod config;
mod dto;
mod feeds;
mod state;
mod ws;

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use axum::routing::{delete, get, post};
use axum::Router;
use tower_http::cors::CorsLayer;
use tower_http::services::ServeDir;
use tracing_subscriber::EnvFilter;

use crate::config::Config;
use crate::dto::ServerMessage;
use crate::state::AppState;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();

    let config_path = Path::new("airjedi-web.toml");
    let config = if config_path.exists() {
        Config::load(config_path).expect("failed to load config")
    } else {
        tracing::info!("no airjedi-web.toml found, using defaults");
        Config {
            server: Default::default(),
            cesium: Default::default(),
            feeds: vec![],
        }
    };

    let listen_addr = config.server.listen.clone();
    let state = Arc::new(AppState::new(config));

    // Spawn configured feeds
    {
        let feed_configs: Vec<_> = state.config.feeds.clone();
        let mut feeds = state.feeds.lock().await;
        for feed_config in feed_configs {
            feeds.add_feed(feed_config);
        }
    }

    // Background broadcast loop
    let broadcast_state = state.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_millis(250));
        loop {
            interval.tick().await;
            let feeds = broadcast_state.feeds.lock().await;
            let aircraft = feeds.get_all_aircraft();
            if !aircraft.is_empty() {
                let msg = ServerMessage::Update { aircraft };
                let _ = broadcast_state.broadcast_tx.send(msg);
            }
        }
    });

    let app = Router::new()
        .route("/ws", get(ws::ws_handler))
        .route("/api/feeds", get(api::get_feeds))
        .route("/api/feeds", post(api::add_feed))
        .route("/api/feeds/{id}", delete(api::delete_feed))
        .route("/api/status", get(api::get_status))
        .route("/api/config", get(api::get_config))
        .route("/api/health", get(|| async { "ok" }))
        .layer(CorsLayer::permissive())
        .fallback_service(ServeDir::new("airjedi-web/client/dist"))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(&listen_addr).await.unwrap();
    tracing::info!("listening on {}", listener.local_addr().unwrap());
    axum::serve(listener, app).await.unwrap();
}
```

- [ ] **Step 5: Add futures-util dependency**

Add to `airjedi-web/Cargo.toml` under `[dependencies]`:

```toml
futures-util = "0.3"
```

- [ ] **Step 6: Verify it compiles and runs**

Run: `cargo build -p airjedi-web`
Expected: compiles with no errors.

Run: `cargo run -p airjedi-web`
Expected: prints "listening on 0.0.0.0:3000". Verify `curl http://localhost:3000/api/health` returns `ok`.

- [ ] **Step 7: Commit**

```bash
git add airjedi-web/
git commit -m "Add WebSocket handler, REST API, and broadcast loop for aircraft state"
```

---

### Task 5: CesiumJS Globe and WebSocket Client

**Files:**
- Modify: `airjedi-web/client/index.html` (add Cesium container styling)
- Create: `airjedi-web/client/src/style.css`
- Create: `airjedi-web/client/src/types.ts`
- Create: `airjedi-web/client/src/globe.ts`
- Create: `airjedi-web/client/src/websocket.ts`
- Create: `airjedi-web/client/src/store.ts`
- Modify: `airjedi-web/client/src/main.ts` (full rewrite)

**Interfaces:**
- Consumes: `GET /api/config` response `{ cesium_ion_token: string | null }`, WebSocket at `/ws` sending `ServerMessage` JSON
- Produces:
  - `Aircraft` interface, `TrailPoint` interface, `ServerMessage` / `ClientMessage` type unions (in `types.ts`)
  - `initGlobe(container: HTMLElement, token: string | null) -> Cesium.Viewer` (in `globe.ts`)
  - `WebSocketClient` class with `connect(url: string)`, `send(msg: ClientMessage)`, `onMessage` callback (in `websocket.ts`)
  - `AircraftStore` class with `applySnapshot(aircraft: Aircraft[])`, `applyUpdate(aircraft: Aircraft[])`, `applyRemove(icao: string[])`, `getAll(): Map<string, Aircraft>`, `onChange(callback)`, `onRemove(callback)` (in `store.ts`)
  - Full app initialization tying them together (in `main.ts`)

- [ ] **Step 1: Create TypeScript types**

Create `airjedi-web/client/src/types.ts`:

```typescript
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
```

- [ ] **Step 2: Create the AircraftStore**

Create `airjedi-web/client/src/store.ts`:

```typescript
import { Aircraft } from "./types";

type ChangeCallback = (aircraft: Map<string, Aircraft>) => void;
type RemoveCallback = (icao: string[]) => void;

export class AircraftStore {
  private aircraft = new Map<string, Aircraft>();
  private changeCallbacks: ChangeCallback[] = [];
  private removeCallbacks: RemoveCallback[] = [];

  applySnapshot(list: Aircraft[]): void {
    this.aircraft.clear();
    for (const ac of list) {
      this.aircraft.set(ac.icao, ac);
    }
    this.notifyChange();
  }

  applyUpdate(list: Aircraft[]): void {
    for (const ac of list) {
      const existing = this.aircraft.get(ac.icao);
      if (existing && ac.trail.length > 0) {
        existing.trail.push(...ac.trail);
        if (existing.trail.length > 200) {
          existing.trail = existing.trail.slice(-200);
        }
        Object.assign(existing, { ...ac, trail: existing.trail });
      } else {
        this.aircraft.set(ac.icao, ac);
      }
    }
    this.notifyChange();
  }

  applyRemove(icao: string[]): void {
    for (const id of icao) {
      this.aircraft.delete(id);
    }
    for (const cb of this.removeCallbacks) cb(icao);
    this.notifyChange();
  }

  getAll(): Map<string, Aircraft> {
    return this.aircraft;
  }

  get(icao: string): Aircraft | undefined {
    return this.aircraft.get(icao);
  }

  get count(): number {
    return this.aircraft.size;
  }

  onChange(cb: ChangeCallback): void {
    this.changeCallbacks.push(cb);
  }

  onRemove(cb: RemoveCallback): void {
    this.removeCallbacks.push(cb);
  }

  private notifyChange(): void {
    for (const cb of this.changeCallbacks) cb(this.aircraft);
  }
}

export class AppState {
  selectedIcao: string | null = null;
  private selectionCallbacks: ((icao: string | null) => void)[] = [];

  select(icao: string | null): void {
    this.selectedIcao = icao;
    for (const cb of this.selectionCallbacks) cb(icao);
  }

  onSelectionChange(cb: (icao: string | null) => void): void {
    this.selectionCallbacks.push(cb);
  }
}
```

- [ ] **Step 3: Create WebSocket client**

Create `airjedi-web/client/src/websocket.ts`:

```typescript
import { ServerMessage, ClientMessage } from "./types";

export class WebSocketClient {
  private ws: WebSocket | null = null;
  private url: string = "";
  private reconnectTimer: number | null = null;
  onMessage: ((msg: ServerMessage) => void) | null = null;
  onConnectionChange: ((connected: boolean) => void) | null = null;

  connect(url: string): void {
    this.url = url;
    this.doConnect();
  }

  send(msg: ClientMessage): void {
    if (this.ws?.readyState === WebSocket.OPEN) {
      this.ws.send(JSON.stringify(msg));
    }
  }

  private doConnect(): void {
    if (this.ws) {
      this.ws.close();
    }

    this.ws = new WebSocket(this.url);

    this.ws.onopen = () => {
      console.log("WebSocket connected");
      this.onConnectionChange?.(true);
      if (this.reconnectTimer !== null) {
        clearTimeout(this.reconnectTimer);
        this.reconnectTimer = null;
      }
    };

    this.ws.onmessage = (event) => {
      try {
        const msg = JSON.parse(event.data) as ServerMessage;
        this.onMessage?.(msg);
      } catch (e) {
        console.error("Failed to parse WebSocket message:", e);
      }
    };

    this.ws.onclose = () => {
      console.log("WebSocket disconnected, reconnecting in 2s");
      this.onConnectionChange?.(false);
      this.scheduleReconnect();
    };

    this.ws.onerror = () => {
      this.ws?.close();
    };
  }

  private scheduleReconnect(): void {
    if (this.reconnectTimer === null) {
      this.reconnectTimer = window.setTimeout(() => {
        this.reconnectTimer = null;
        this.doConnect();
      }, 2000);
    }
  }
}
```

- [ ] **Step 4: Create globe initialization**

Create `airjedi-web/client/src/globe.ts`:

```typescript
import {
  Viewer,
  Ion,
  createWorldTerrainAsync,
  OpenStreetMapImageryProvider,
  SceneMode,
  ImageryLayer,
} from "cesium";
import "cesium/Build/Cesium/Widgets/widgets.css";

export async function initGlobe(
  container: HTMLElement,
  ionToken: string | null
): Promise<Viewer> {
  if (ionToken) {
    Ion.defaultAccessToken = ionToken;
  }

  const viewer = new Viewer(container, {
    sceneMode: SceneMode.COLUMBUS_VIEW,
    animation: false,
    timeline: false,
    homeButton: true,
    sceneModePicker: true,
    baseLayerPicker: false,
    navigationHelpButton: false,
    fullscreenButton: false,
    geocoder: false,
    infoBox: false,
    selectionIndicator: false,
  });

  if (ionToken) {
    try {
      const terrain = await createWorldTerrainAsync();
      viewer.scene.setTerrain(terrain);
    } catch (e) {
      console.warn("Failed to load Cesium World Terrain:", e);
    }
  }

  // Add OSM as an available layer (hidden by default)
  const osmProvider = new OpenStreetMapImageryProvider({
    url: "https://tile.openstreetmap.org/",
  });
  const osmLayer = new ImageryLayer(osmProvider);
  osmLayer.show = false;
  viewer.imageryLayers.add(osmLayer);

  // Set default view to Wichita, KS
  viewer.camera.setView({
    destination: Cesium.Cartesian3.fromDegrees(-97.3301, 37.6872, 500000),
  });

  return viewer;
}
```

- [ ] **Step 5: Create styles**

Create `airjedi-web/client/src/style.css`:

```css
* {
  margin: 0;
  padding: 0;
  box-sizing: border-box;
}

html, body {
  width: 100%;
  height: 100%;
  overflow: hidden;
  font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif;
  background: #1a1a2e;
  color: #e0e0e0;
}

#cesium-container {
  width: 100%;
  height: 100%;
}

.panel {
  position: absolute;
  background: rgba(20, 20, 40, 0.92);
  border: 1px solid rgba(100, 100, 140, 0.3);
  border-radius: 6px;
  backdrop-filter: blur(8px);
  z-index: 10;
  overflow: hidden;
}

#aircraft-list-panel {
  top: 10px;
  left: 10px;
  width: 360px;
  max-height: calc(100vh - 80px);
  display: flex;
  flex-direction: column;
}

#aircraft-detail-panel {
  top: 10px;
  right: 10px;
  width: 300px;
  display: none;
}

#status-bar {
  position: absolute;
  bottom: 0;
  left: 0;
  right: 0;
  height: 32px;
  background: rgba(20, 20, 40, 0.95);
  border-top: 1px solid rgba(100, 100, 140, 0.3);
  display: flex;
  align-items: center;
  padding: 0 12px;
  font-size: 12px;
  gap: 16px;
  z-index: 10;
}

.panel-header {
  padding: 8px 12px;
  font-size: 13px;
  font-weight: 600;
  border-bottom: 1px solid rgba(100, 100, 140, 0.2);
  display: flex;
  justify-content: space-between;
  align-items: center;
}

.panel-body {
  overflow-y: auto;
  flex: 1;
}

.aircraft-table {
  width: 100%;
  border-collapse: collapse;
  font-size: 12px;
}

.aircraft-table th {
  position: sticky;
  top: 0;
  background: rgba(30, 30, 50, 0.98);
  padding: 6px 8px;
  text-align: left;
  cursor: pointer;
  user-select: none;
  font-weight: 500;
  color: #a0a0c0;
}

.aircraft-table td {
  padding: 4px 8px;
  border-bottom: 1px solid rgba(60, 60, 80, 0.3);
}

.aircraft-table tr:hover {
  background: rgba(60, 60, 100, 0.3);
  cursor: pointer;
}

.aircraft-table tr.selected {
  background: rgba(80, 80, 160, 0.4);
}

.detail-row {
  display: flex;
  justify-content: space-between;
  padding: 4px 12px;
  font-size: 12px;
}

.detail-label {
  color: #8888aa;
}

.detail-value {
  font-weight: 500;
}

.status-dot {
  display: inline-block;
  width: 8px;
  height: 8px;
  border-radius: 50%;
  margin-right: 4px;
}

.status-dot.connected { background: #4caf50; }
.status-dot.disconnected { background: #f44336; }

.search-input {
  width: 100%;
  padding: 6px 10px;
  background: rgba(40, 40, 60, 0.8);
  border: 1px solid rgba(80, 80, 120, 0.3);
  border-radius: 4px;
  color: #e0e0e0;
  font-size: 12px;
  outline: none;
  margin: 6px;
  width: calc(100% - 12px);
}

.close-btn {
  background: none;
  border: none;
  color: #a0a0c0;
  cursor: pointer;
  font-size: 16px;
  padding: 0 4px;
}

.close-btn:hover {
  color: #ffffff;
}
```

- [ ] **Step 6: Rewrite main.ts to wire everything together**

Replace `airjedi-web/client/src/main.ts` with:

```typescript
import { initGlobe } from "./globe";
import { WebSocketClient } from "./websocket";
import { AircraftStore, AppState } from "./store";
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

  // Log store updates for now (aircraft rendering comes next task)
  store.onChange((aircraft) => {
    console.log(`Aircraft count: ${aircraft.size}`);
  });
}

main().catch(console.error);
```

- [ ] **Step 7: Update index.html**

Replace `airjedi-web/client/index.html` with:

```html
<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8" />
  <meta name="viewport" content="width=device-width, initial-scale=1.0" />
  <title>AirJedi</title>
</head>
<body>
  <div id="cesium-container"></div>
  <div id="aircraft-list-panel" class="panel"></div>
  <div id="aircraft-detail-panel" class="panel"></div>
  <div id="status-bar"></div>
  <script type="module" src="/src/main.ts"></script>
</body>
</html>
```

- [ ] **Step 8: Verify frontend builds and globe renders**

Run: `cd airjedi-web/client && npm run build`
Expected: builds with no errors.

Run both servers (Axum + Vite dev), open `http://localhost:5173`.
Expected: CesiumJS globe renders centered on Wichita, KS. Browser console shows "WebSocket connected" and aircraft count updates (0 if no feeds configured).

- [ ] **Step 9: Commit**

```bash
git add airjedi-web/client/
git commit -m "Add CesiumJS globe, WebSocket client, and aircraft store"
```

---

### Task 6: Aircraft Entity Rendering on Globe

**Files:**
- Create: `airjedi-web/client/src/aircraft.ts`
- Modify: `airjedi-web/client/src/main.ts` (integrate AircraftManager)

**Interfaces:**
- Consumes: `AircraftStore` (onChange, onRemove, getAll, get), `AppState` (select, selectedIcao, onSelectionChange), `Cesium.Viewer` instance, `Aircraft` interface (icao, callsign, latitude, longitude, altitude, ground_speed, heading, trail)
- Produces:
  - `AircraftManager` class with `constructor(viewer: Cesium.Viewer, store: AircraftStore, appState: AppState)`
  - Automatically creates/updates/removes Cesium entities from store events
  - Click-to-select wired through `appState.select(icao)`

- [ ] **Step 1: Create AircraftManager**

Create `airjedi-web/client/src/aircraft.ts`:

```typescript
import {
  Viewer,
  Entity,
  Cartesian3,
  Color,
  VerticalOrigin,
  HorizontalOrigin,
  NearFarScalar,
  PolylineGlowMaterialProperty,
  CallbackProperty,
  Cartographic,
  Math as CesiumMath,
  ScreenSpaceEventHandler,
  ScreenSpaceEventType,
  defined,
} from "cesium";
import { Aircraft } from "./types";
import { AircraftStore, AppState } from "./store";

function altitudeColor(altFeet: number | null): Color {
  if (altFeet === null) return Color.GRAY;
  if (altFeet < 1000) return Color.fromCssColorString("#2196F3");
  if (altFeet < 5000) return Color.fromCssColorString("#4CAF50");
  if (altFeet < 15000) return Color.fromCssColorString("#8BC34A");
  if (altFeet < 25000) return Color.fromCssColorString("#FFEB3B");
  if (altFeet < 35000) return Color.fromCssColorString("#FF9800");
  return Color.fromCssColorString("#F44336");
}

function feetToMeters(feet: number): number {
  return feet * 0.3048;
}

function formatAltitude(alt: number | null): string {
  if (alt === null) return "---";
  if (alt >= 18000) return `FL${Math.round(alt / 100)}`;
  return `${alt.toLocaleString()}ft`;
}

export class AircraftManager {
  private entities = new Map<string, Entity>();
  private viewer: Viewer;
  private store: AircraftStore;
  private appState: AppState;

  constructor(viewer: Viewer, store: AircraftStore, appState: AppState) {
    this.viewer = viewer;
    this.store = store;
    this.appState = appState;

    store.onChange(() => this.syncEntities());
    store.onRemove((icao) => this.removeEntities(icao));

    this.setupPicking();
  }

  private syncEntities(): void {
    const aircraft = this.store.getAll();
    for (const [icao, ac] of aircraft) {
      if (ac.latitude === null || ac.longitude === null) continue;

      const existing = this.entities.get(icao);
      if (existing) {
        this.updateEntity(existing, ac);
      } else {
        this.createEntity(ac);
      }
    }
  }

  private createEntity(ac: Aircraft): void {
    if (ac.latitude === null || ac.longitude === null) return;

    const altMeters = ac.altitude ? feetToMeters(ac.altitude) : 0;
    const color = altitudeColor(ac.altitude);
    const label = ac.callsign || ac.icao;

    const entity = this.viewer.entities.add({
      id: `aircraft-${ac.icao}`,
      position: Cartesian3.fromDegrees(ac.longitude, ac.latitude, altMeters),
      point: {
        pixelSize: 8,
        color: color,
        outlineColor: Color.BLACK,
        outlineWidth: 1,
      },
      label: {
        text: `${label}\n${formatAltitude(ac.altitude)}`,
        font: "11px sans-serif",
        fillColor: Color.WHITE,
        outlineColor: Color.BLACK,
        outlineWidth: 2,
        style: Cesium.LabelStyle.FILL_AND_OUTLINE,
        verticalOrigin: VerticalOrigin.BOTTOM,
        horizontalOrigin: HorizontalOrigin.LEFT,
        pixelOffset: new Cesium.Cartesian2(10, -5),
        scaleByDistance: new NearFarScalar(5e4, 1.0, 5e6, 0.3),
        translucencyByDistance: new NearFarScalar(5e5, 1.0, 1e7, 0.0),
      },
      properties: {
        icao: ac.icao,
      },
    });

    // Add trail polyline if there are trail points
    if (ac.trail.length > 1) {
      this.updateTrail(ac);
    }

    this.entities.set(ac.icao, entity);
  }

  private updateEntity(entity: Entity, ac: Aircraft): void {
    if (ac.latitude === null || ac.longitude === null) return;

    const altMeters = ac.altitude ? feetToMeters(ac.altitude) : 0;
    entity.position = Cartesian3.fromDegrees(
      ac.longitude,
      ac.latitude,
      altMeters
    ) as any;

    if (entity.point) {
      entity.point.color = altitudeColor(ac.altitude) as any;
    }

    if (entity.label) {
      const label = ac.callsign || ac.icao;
      entity.label.text = `${label}\n${formatAltitude(ac.altitude)}` as any;
    }

    if (ac.trail.length > 1) {
      this.updateTrail(ac);
    }
  }

  private updateTrail(ac: Aircraft): void {
    const trailId = `trail-${ac.icao}`;
    let trailEntity = this.viewer.entities.getById(trailId);

    const positions = ac.trail
      .filter((p) => p.lat !== null && p.lon !== null)
      .map((p) =>
        Cartesian3.fromDegrees(
          p.lon,
          p.lat,
          p.alt ? feetToMeters(p.alt) : 0
        )
      );

    if (ac.latitude !== null && ac.longitude !== null) {
      const altMeters = ac.altitude ? feetToMeters(ac.altitude) : 0;
      positions.push(
        Cartesian3.fromDegrees(ac.longitude, ac.latitude, altMeters)
      );
    }

    if (positions.length < 2) return;

    if (trailEntity) {
      if (trailEntity.polyline) {
        trailEntity.polyline.positions = positions as any;
      }
    } else {
      this.viewer.entities.add({
        id: trailId,
        polyline: {
          positions: positions,
          width: 2,
          material: new PolylineGlowMaterialProperty({
            glowPower: 0.2,
            color: altitudeColor(ac.altitude),
          }),
          clampToGround: false,
        },
      });
    }
  }

  private removeEntities(icao: string[]): void {
    for (const id of icao) {
      const entity = this.entities.get(id);
      if (entity) {
        this.viewer.entities.remove(entity);
        this.entities.delete(id);
      }
      const trailEntity = this.viewer.entities.getById(`trail-${id}`);
      if (trailEntity) {
        this.viewer.entities.remove(trailEntity);
      }
    }
  }

  private setupPicking(): void {
    const handler = new ScreenSpaceEventHandler(this.viewer.scene.canvas);
    handler.setInputAction((event: any) => {
      const picked = this.viewer.scene.pick(event.position);
      if (defined(picked) && picked.id?.id?.startsWith("aircraft-")) {
        const icao = picked.id.id.replace("aircraft-", "");
        this.appState.select(icao);
      } else {
        this.appState.select(null);
      }
    }, ScreenSpaceEventType.LEFT_CLICK);
  }
}
```

- [ ] **Step 2: Integrate into main.ts**

Add to `main.ts` after the store/appState creation:

```typescript
import { AircraftManager } from "./aircraft";

// After viewer, store, appState are created:
const aircraftManager = new AircraftManager(viewer, store, appState);
```

Add the import at the top and the instantiation after the `const appState = new AppState();` line.

- [ ] **Step 3: Test with a live feed**

Create a test config file `airjedi-web.toml` with a real feed:

```toml
[[feeds]]
address = "192.168.1.10:30003"
protocol = "basestation"
```

Run both servers. Open the browser.
Expected: aircraft appear as colored dots on the globe with labels and trails.

- [ ] **Step 4: Commit**

```bash
git add airjedi-web/client/src/aircraft.ts airjedi-web/client/src/main.ts
git commit -m "Add aircraft entity rendering with altitude coloring, labels, and trails"
```

---

### Task 7: UI Panels (Aircraft List, Detail, Status Bar)

**Files:**
- Create: `airjedi-web/client/src/panels/aircraft-list.ts`
- Create: `airjedi-web/client/src/panels/aircraft-detail.ts`
- Create: `airjedi-web/client/src/panels/status-bar.ts`
- Modify: `airjedi-web/client/src/main.ts` (mount panels)

**Interfaces:**
- Consumes: `AircraftStore` (onChange, getAll, get, count), `AppState` (select, selectedIcao, onSelectionChange), `WebSocketClient` (onConnectionChange), `Aircraft` interface
- Produces:
  - `AircraftListPanel` class with `constructor(container: HTMLElement, store: AircraftStore, appState: AppState)`
  - `AircraftDetailPanel` class with `constructor(container: HTMLElement, store: AircraftStore, appState: AppState)`
  - `StatusBar` class with `constructor(container: HTMLElement, store: AircraftStore, wsClient: WebSocketClient, viewer: Cesium.Viewer)`

- [ ] **Step 1: Create aircraft list panel**

Create `airjedi-web/client/src/panels/aircraft-list.ts`:

```typescript
import { Aircraft } from "../types";
import { AircraftStore, AppState } from "../store";

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
        return `<tr class="${selected}" data-icao="${ac.icao}">
        <td>${ac.callsign || ac.icao}</td>
        <td>${ac.squawk || "---"}</td>
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
```

- [ ] **Step 2: Create aircraft detail panel**

Create `airjedi-web/client/src/panels/aircraft-detail.ts`:

```typescript
import { AircraftStore, AppState } from "../store";

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
      ac.heading !== null ? `${Math.round(ac.heading)}°` : "---";
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
        ${ac.callsign || ac.icao}
        <button class="close-btn" id="detail-close">×</button>
      </div>
      <div class="panel-body" style="padding: 4px 0;">
        <div class="detail-row"><span class="detail-label">ICAO</span><span class="detail-value">${ac.icao}</span></div>
        <div class="detail-row"><span class="detail-label">Callsign</span><span class="detail-value">${ac.callsign || "---"}</span></div>
        <div class="detail-row"><span class="detail-label">Altitude</span><span class="detail-value">${alt}</span></div>
        <div class="detail-row"><span class="detail-label">Speed</span><span class="detail-value">${spd}</span></div>
        <div class="detail-row"><span class="detail-label">Heading</span><span class="detail-value">${heading}</span></div>
        <div class="detail-row"><span class="detail-label">V/Rate</span><span class="detail-value">${vrate}</span></div>
        <div class="detail-row"><span class="detail-label">Position</span><span class="detail-value">${lat}, ${lon}</span></div>
        <div class="detail-row"><span class="detail-label">Squawk</span><span class="detail-value">${ac.squawk || "---"}</span></div>
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
```

- [ ] **Step 3: Create status bar**

Create `airjedi-web/client/src/panels/status-bar.ts`:

```typescript
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
    viewer: Viewer
  ) {
    this.container = container;
    this.store = store;

    wsClient.onConnectionChange = (connected) => {
      this.wsConnected = connected;
      this.render();
    };

    store.onChange(() => this.render());

    viewer.camera.changed.addEventListener(() => this.render());

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
```

- [ ] **Step 4: Mount panels in main.ts**

Add imports and instantiations to `main.ts`:

```typescript
import { AircraftListPanel } from "./panels/aircraft-list";
import { AircraftDetailPanel } from "./panels/aircraft-detail";
import { StatusBar } from "./panels/status-bar";

// After aircraftManager creation:
const listPanel = document.getElementById("aircraft-list-panel")!;
const detailPanel = document.getElementById("aircraft-detail-panel")!;
const statusBar = document.getElementById("status-bar")!;

new AircraftListPanel(listPanel, store, appState);
new AircraftDetailPanel(detailPanel, store, appState);
new StatusBar(statusBar, store, wsClient, viewer);
```

- [ ] **Step 5: Test full UI**

Run both servers with a real feed configured.
Expected:
- Aircraft list populates on the left with sortable columns
- Clicking an aircraft in the list or on the globe shows the detail panel on the right
- Status bar shows connection state and aircraft count
- Search filter narrows the aircraft list

- [ ] **Step 6: Commit**

```bash
git add airjedi-web/client/src/panels/ airjedi-web/client/src/main.ts
git commit -m "Add aircraft list, detail, and status bar panels"
```

---

### Task 8: Settings Panel and Feed Management UI

**Files:**
- Create: `airjedi-web/client/src/panels/settings.ts`
- Modify: `airjedi-web/client/src/main.ts` (add settings button and panel)
- Modify: `airjedi-web/client/src/style.css` (add settings styles)
- Modify: `airjedi-web/client/index.html` (add settings panel div and toolbar)

**Interfaces:**
- Consumes: `WebSocketClient.send(msg: ClientMessage)`, `GET /api/feeds` response `FeedConfig[]`, `DELETE /api/feeds/:id`
- Produces: Settings panel with feed CRUD UI, basemap toggle

- [ ] **Step 1: Create settings panel**

Create `airjedi-web/client/src/panels/settings.ts`:

```typescript
import { Viewer, OpenStreetMapImageryProvider, ImageryLayer } from "cesium";
import { WebSocketClient } from "../websocket";

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
        <button class="close-btn" id="settings-close">×</button>
      </div>
      <div class="panel-body" style="padding: 8px 12px;">
        <h4 style="margin: 0 0 8px; font-size: 13px; color: #a0a0c0;">Data Sources</h4>
        <div id="feeds-list">
          ${feeds
            .map(
              (f) => `
            <div class="detail-row" style="align-items: center;">
              <span class="detail-value" style="font-size: 12px;">${f.address} (${f.protocol})</span>
              <button class="close-btn remove-feed" data-id="${f.id}">×</button>
            </div>
          `
            )
            .join("")}
        </div>
        <div style="margin-top: 8px; display: flex; gap: 4px;">
          <input type="text" id="new-feed-addr" class="search-input" style="margin: 0; flex: 1;" placeholder="host:port" />
          <select id="new-feed-proto" style="background: rgba(40,40,60,0.8); color: #e0e0e0; border: 1px solid rgba(80,80,120,0.3); border-radius: 4px; padding: 4px; font-size: 12px;">
            <option value="basestation">SBS-1</option>
            <option value="beast">BEAST</option>
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
```

- [ ] **Step 2: Add settings panel to index.html**

Add after the status-bar div:

```html
<div id="settings-panel" class="panel" style="top: 10px; right: 10px; width: 340px;"></div>
<button id="settings-btn" style="position: absolute; top: 10px; right: 360px; z-index: 10; background: rgba(20,20,40,0.9); border: 1px solid rgba(100,100,140,0.3); border-radius: 4px; color: #e0e0e0; padding: 6px 10px; cursor: pointer; font-size: 13px;">Settings</button>
```

- [ ] **Step 3: Wire settings in main.ts**

Add to `main.ts`:

```typescript
import { SettingsPanel } from "./panels/settings";

const settingsContainer = document.getElementById("settings-panel")!;
const settingsPanel = new SettingsPanel(settingsContainer, wsClient, viewer);
document.getElementById("settings-btn")!.addEventListener("click", () => {
  settingsPanel.toggle();
});
```

- [ ] **Step 4: Test feed management**

Run the app. Open Settings. Add a new feed via the UI. Verify aircraft start appearing. Remove the feed. Verify aircraft stop updating.

- [ ] **Step 5: Commit**

```bash
git add airjedi-web/client/
git commit -m "Add settings panel with feed management and basemap toggle"
```

---

### Task 9: Polish and End-to-End Verification

**Files:**
- Modify: `airjedi-web/client/src/globe.ts` (fix Cesium namespace usage)
- Modify: various files for TypeScript strictness fixes
- Create: `airjedi-web/README.md`

**Interfaces:**
- Consumes: all prior task outputs
- Produces: clean build, documented setup instructions, verified end-to-end flow

- [ ] **Step 1: Fix any TypeScript build issues**

Run: `cd airjedi-web/client && npm run build`

Fix any remaining type errors. Common issues:
- `Cesium` namespace references in `globe.ts` and `aircraft.ts` - use named imports instead
- Missing type annotations
- Unused variables

- [ ] **Step 2: Test full flow end-to-end**

1. Start Axum: `cargo run -p airjedi-web`
2. Start Vite: `cd airjedi-web/client && npm run dev`
3. Open `http://localhost:5173`
4. Configure a feed via Settings panel
5. Verify:
   - Globe renders with terrain
   - Aircraft appear as colored dots
   - Labels show callsign + altitude
   - Trails draw behind aircraft
   - Aircraft list populates and sorts
   - Clicking a row or dot selects aircraft
   - Detail panel shows full info
   - Status bar shows connection state and count
   - Adding/removing feeds works

- [ ] **Step 3: Test production build serving**

```bash
cd airjedi-web/client && npm run build
cd ../.. && cargo run -p airjedi-web
```

Open `http://localhost:3000` (served by Axum directly, no Vite).
Expected: same functionality as dev mode.

- [ ] **Step 4: Add .gitignore for frontend artifacts**

Create `airjedi-web/client/.gitignore`:

```
node_modules/
dist/
```

- [ ] **Step 5: Commit**

```bash
git add airjedi-web/
git commit -m "Polish TypeScript build, add gitignore, verify end-to-end flow"
```
