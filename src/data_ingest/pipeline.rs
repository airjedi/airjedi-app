use std::collections::HashMap;
use std::fmt;

use super::canonical::CanonicalRecord;

/// Pipeline execution phase. Stages run in phase order; within the same phase
/// they run in the order they were registered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum PipelinePhase {
    /// Raw bytes/text → parsed domain structs
    Parse = 0,
    /// Field-level validation and normalization
    Validate = 1,
    /// Coordinate transforms, unit conversion, dedup
    Transform = 2,
    /// Final enrichment before delivery (e.g. cross-referencing)
    Enrich = 3,
}

impl fmt::Display for PipelinePhase {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Parse => write!(f, "Parse"),
            Self::Validate => write!(f, "Validate"),
            Self::Transform => write!(f, "Transform"),
            Self::Enrich => write!(f, "Enrich"),
        }
    }
}

/// Mutable data bag passed through the pipeline.
#[derive(Debug, Default)]
pub struct PipelineData {
    /// Raw bytes from the fetch (consumed by the Parse stage).
    pub raw_bytes: Option<Vec<u8>>,
    /// Parsed canonical records accumulate here.
    pub records: Vec<CanonicalRecord>,
    /// Arbitrary key-value metadata stages can read/write.
    pub metadata: HashMap<String, String>,
}

/// Errors that can occur during pipeline execution.
#[derive(Debug)]
pub enum PipelineError {
    /// A stage failed with a human-readable message.
    StageError { stage: String, message: String },
}

impl fmt::Display for PipelineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::StageError { stage, message } => {
                write!(f, "pipeline stage '{}' failed: {}", stage, message)
            }
        }
    }
}

impl std::error::Error for PipelineError {}

/// A single transformation stage in the data pipeline.
pub trait PipelineStage: Send + Sync {
    /// Human-readable name for logging and error reporting.
    fn name(&self) -> &str;

    /// Which phase this stage belongs to.
    fn phase(&self) -> PipelinePhase;

    /// Execute the stage, mutating `data` in place.
    fn execute(&self, data: &mut PipelineData) -> Result<(), PipelineError>;
}

/// Run all `stages` in phase order, then insertion order within a phase.
/// Returns the final `PipelineData` on success, or the first error encountered.
pub fn run_pipeline(
    stages: &[Box<dyn PipelineStage>],
    mut data: PipelineData,
) -> Result<PipelineData, PipelineError> {
    let mut sorted_indices: Vec<usize> = (0..stages.len()).collect();
    sorted_indices.sort_by_key(|&i| stages[i].phase());

    for &i in &sorted_indices {
        stages[i].execute(&mut data)?;
    }

    Ok(data)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper stage that records its name in metadata when executed.
    struct RecordingStage {
        name: String,
        phase: PipelinePhase,
    }

    impl PipelineStage for RecordingStage {
        fn name(&self) -> &str {
            &self.name
        }
        fn phase(&self) -> PipelinePhase {
            self.phase
        }
        fn execute(&self, data: &mut PipelineData) -> Result<(), PipelineError> {
            let order = data.metadata.get("order").cloned().unwrap_or_default();
            let updated = if order.is_empty() {
                self.name.clone()
            } else {
                format!("{},{}", order, self.name)
            };
            data.metadata.insert("order".to_string(), updated);
            Ok(())
        }
    }

    /// Stage that always fails.
    struct FailingStage {
        name: String,
        phase: PipelinePhase,
    }

    impl PipelineStage for FailingStage {
        fn name(&self) -> &str {
            &self.name
        }
        fn phase(&self) -> PipelinePhase {
            self.phase
        }
        fn execute(&self, _data: &mut PipelineData) -> Result<(), PipelineError> {
            Err(PipelineError::StageError {
                stage: self.name.clone(),
                message: "intentional failure".to_string(),
            })
        }
    }

    #[test]
    fn empty_pipeline_succeeds() {
        let stages: Vec<Box<dyn PipelineStage>> = vec![];
        let data = PipelineData::default();
        let result = run_pipeline(&stages, data);
        assert!(result.is_ok());
    }

    #[test]
    fn phases_execute_in_order() {
        let stages: Vec<Box<dyn PipelineStage>> = vec![
            Box::new(RecordingStage {
                name: "enrich1".into(),
                phase: PipelinePhase::Enrich,
            }),
            Box::new(RecordingStage {
                name: "parse1".into(),
                phase: PipelinePhase::Parse,
            }),
            Box::new(RecordingStage {
                name: "validate1".into(),
                phase: PipelinePhase::Validate,
            }),
            Box::new(RecordingStage {
                name: "transform1".into(),
                phase: PipelinePhase::Transform,
            }),
        ];
        let data = PipelineData::default();
        let result = run_pipeline(&stages, data).unwrap();
        assert_eq!(
            result.metadata.get("order").unwrap(),
            "parse1,validate1,transform1,enrich1"
        );
    }

    #[test]
    fn insertion_order_within_same_phase() {
        let stages: Vec<Box<dyn PipelineStage>> = vec![
            Box::new(RecordingStage {
                name: "a".into(),
                phase: PipelinePhase::Parse,
            }),
            Box::new(RecordingStage {
                name: "b".into(),
                phase: PipelinePhase::Parse,
            }),
            Box::new(RecordingStage {
                name: "c".into(),
                phase: PipelinePhase::Parse,
            }),
        ];
        let data = PipelineData::default();
        let result = run_pipeline(&stages, data).unwrap();
        assert_eq!(result.metadata.get("order").unwrap(), "a,b,c");
    }

    #[test]
    fn stage_error_stops_pipeline() {
        let stages: Vec<Box<dyn PipelineStage>> = vec![
            Box::new(RecordingStage {
                name: "parse1".into(),
                phase: PipelinePhase::Parse,
            }),
            Box::new(FailingStage {
                name: "bad_validate".into(),
                phase: PipelinePhase::Validate,
            }),
            Box::new(RecordingStage {
                name: "transform1".into(),
                phase: PipelinePhase::Transform,
            }),
        ];
        let data = PipelineData::default();
        let result = run_pipeline(&stages, data);
        assert!(result.is_err());
        let err = result.unwrap_err();
        match err {
            PipelineError::StageError { stage, .. } => {
                assert_eq!(stage, "bad_validate");
            }
        }
    }

    #[test]
    fn pipeline_passes_raw_bytes_to_records() {
        struct ByteParser;
        impl PipelineStage for ByteParser {
            fn name(&self) -> &str {
                "byte_parser"
            }
            fn phase(&self) -> PipelinePhase {
                PipelinePhase::Parse
            }
            fn execute(&self, data: &mut PipelineData) -> Result<(), PipelineError> {
                if data.raw_bytes.is_some() {
                    data.metadata.insert("had_bytes".into(), "true".into());
                }
                Ok(())
            }
        }

        let stages: Vec<Box<dyn PipelineStage>> = vec![Box::new(ByteParser)];
        let data = PipelineData {
            raw_bytes: Some(b"test data".to_vec()),
            ..Default::default()
        };
        let result = run_pipeline(&stages, data).unwrap();
        assert_eq!(result.metadata.get("had_bytes").unwrap(), "true");
    }
}
