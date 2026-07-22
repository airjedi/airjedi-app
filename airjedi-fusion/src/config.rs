use crate::associator::AssociatorConfig;
use crate::filter::ekf::{Ekf6Dof, ProcessNoiseConfig};
use crate::filter::imm::ImmFilter;
use crate::filter::surface::{Surface4Dof, SurfaceConfig};
use crate::filter::transition::{ConstantVelocity3D, TransitionModel};
use crate::filter::ukf::{Ukf, UkfConfig};
use crate::filter::{FilterVariant, OosmConfig, TrackFilter, TrackerState};
use crate::prelude_imports::*;
use crate::sensor::FusionTier;
use crate::store::StoreConfig;
use crate::track::initiation::{InitiationConfig, MofNInitiator};
use crate::track::LifecycleProfiles;
use crate::transport::NatsTransportConfig;
use crate::types::{StateVectorType, TargetCategory};
use nalgebra::DMatrix;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilterStrategy {
    Ekf,
    Ukf,
    Imm,
    Surface,
}

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
    pub initiation: InitiationConfig,
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
            initiation: InitiationConfig::default(),
        }
    }
}

impl FusionConfig {
    pub fn strategy_for(&self, category: &TargetCategory) -> FilterStrategy {
        match category {
            TargetCategory::FixedWing
            | TargetCategory::RotaryWing
            | TargetCategory::Drone
            | TargetCategory::Missile => FilterStrategy::Imm,
            TargetCategory::SurfaceVessel
            | TargetCategory::GroundVehicle
            | TargetCategory::Person => FilterStrategy::Surface,
            _ => FilterStrategy::Ekf,
        }
    }

    pub fn create_tracker(&self, category: &TargetCategory) -> TrackerState {
        match self.strategy_for(category) {
            FilterStrategy::Imm => {
                let low_noise = &self.filter_defaults;
                let high_noise = ProcessNoiseConfig {
                    position_noise: low_noise.position_noise * 10.0,
                    velocity_noise: low_noise.velocity_noise * 10.0,
                };

                let filters: Vec<Box<dyn TrackFilter>> = vec![
                    Box::new(Ekf6Dof::new(low_noise.clone())),
                    Box::new(Ekf6Dof::new(high_noise)),
                ];

                #[rustfmt::skip]
                let tm = DMatrix::from_row_slice(2, 2, &[
                    0.95, 0.05,
                    0.05, 0.95,
                ]);

                TrackerState {
                    variant: FilterVariant::new(ImmFilter::new(filters, tm)),
                    state_type: StateVectorType::Cartesian6Dof,
                    last_update: None,
                }
            }
            FilterStrategy::Ukf => {
                let model: Box<dyn TransitionModel> = Box::new(ConstantVelocity3D::new(
                    self.filter_defaults.position_noise,
                    self.filter_defaults.velocity_noise,
                ));
                TrackerState {
                    variant: FilterVariant::new(Ukf::new(6, model, UkfConfig::default())),
                    state_type: StateVectorType::Cartesian6Dof,
                    last_update: None,
                }
            }
            FilterStrategy::Surface => TrackerState {
                variant: FilterVariant::new(Surface4Dof::new(SurfaceConfig::default())),
                state_type: StateVectorType::Surface4Dof,
                last_update: None,
            },
            FilterStrategy::Ekf => TrackerState::new_6dof(self.filter_defaults.clone()),
        }
    }
}
