use crate::prelude_imports::*;
use crate::types::{Affiliation, TargetCategory, TargetDomain};

#[derive(Component, Debug, Clone, Reflect)]
pub struct TargetClassification {
    pub domain: TargetDomain,
    pub category: TargetCategory,
    pub specific_type: Option<String>,
    pub affiliation: Affiliation,
    pub confidence: f32,
}

impl Default for TargetClassification {
    fn default() -> Self {
        Self {
            domain: TargetDomain::Air,
            category: TargetCategory::Unknown,
            specific_type: None,
            affiliation: Affiliation::Unknown,
            confidence: 0.0,
        }
    }
}
