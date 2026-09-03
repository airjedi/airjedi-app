//! Core domain vocabulary.
//!
//! These types moved to the dependency-light `airjedi-core` crate so the client
//! can name them without the fusion engine (see the design-b Phase 1 plan). This
//! module re-exports them unchanged; every existing `crate::types::*` /
//! `airjedi_fusion::types::*` import keeps resolving.

pub use airjedi_core::{
    Affiliation, IdentifierType, StateVectorType, TargetCategory, TargetDomain, TargetId,
    Timestamp, TrackId,
};
