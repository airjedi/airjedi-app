//! `airjedi-core` - dependency-light domain vocabulary and serializable display
//! components shared between the fusion agent and the (current or future thin)
//! client.
//!
//! Nothing here pulls in `airjedi-fusion` (no nalgebra / kiddo / pathfinding),
//! so both sides can name these types without dragging in the fusion engine.
//!
//! The four display components ([`DisplayTrack`], [`DisplayEstimate`],
//! [`SensorContributions`], [`DisplayTrail`]) are the serializable projection
//! boundary from fused state to render-ready state. In fat mode the agent
//! systems write them into the app's own `World` and the UI reads them directly;
//! in thin mode a replication layer copies the same components agent -> client
//! and the UI cannot tell the difference. See
//! `docs/plans/2026-09-03-design-b-phase1-display-components.md`.

pub mod display;
pub mod ids;
pub mod sensor_kind;
pub mod source;
pub mod status;

pub use display::*;
pub use ids::*;
pub use sensor_kind::*;
pub use source::*;
pub use status::*;
