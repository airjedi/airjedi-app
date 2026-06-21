use crate::prelude_imports::*;
use crate::associator::AssociatorConfig;
use crate::filter::ekf::ProcessNoiseConfig;
use crate::filter::OosmConfig;
use crate::sensor::FusionTier;
use crate::store::StoreConfig;
use crate::track::LifecycleProfiles;
use crate::transport::NatsTransportConfig;

#[derive(Resource, Debug, Clone)]
pub struct FusionConfig {
    pub store: StoreConfig,
    pub lifecycle: LifecycleProfiles,
    pub associator: AssociatorConfig,
    pub filter_defaults: ProcessNoiseConfig,
    pub oosm: OosmConfig,
    pub node_id: String,
    pub tier: FusionTier,
    pub spatial_cell_size_deg: f64,
    pub transport: Option<NatsTransportConfig>,
}

impl Default for FusionConfig {
    fn default() -> Self {
        Self {
            store: StoreConfig::default(),
            lifecycle: LifecycleProfiles::default(),
            associator: AssociatorConfig::default(),
            filter_defaults: ProcessNoiseConfig::default(),
            oosm: OosmConfig::default(),
            node_id: "local".to_string(),
            tier: FusionTier::Regional,
            spatial_cell_size_deg: 0.5,
            transport: None,
        }
    }
}
