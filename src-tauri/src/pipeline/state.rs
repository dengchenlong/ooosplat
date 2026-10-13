use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PipelineStage {
    Created,
    ProbingVideo,
    PlanningFrames,
    ExtractingFrames,
    ExtractingFeatures,
    Matching,
    Reconstructing,
    ValidatingReconstruction,
    Segment,
    TrainingSplats,
    Exporting,
    Completed,
    Failed,
    Cancelled,
}

impl PipelineStage {
    pub fn can_transition_to(self, next: Self) -> bool {
        use PipelineStage::*;
        if matches!(next, Failed | Cancelled) && !matches!(self, Completed | Failed | Cancelled) {
            return true;
        }
        matches!(
            (self, next),
            (Created, ProbingVideo)
                | (ProbingVideo, PlanningFrames)
                | (PlanningFrames, ExtractingFrames)
                | (ExtractingFrames, ExtractingFeatures)
                | (ExtractingFeatures, Matching)
                | (Matching, Reconstructing)
                | (Reconstructing, ValidatingReconstruction)
                | (ValidatingReconstruction, Segment)
                | (Segment, TrainingSplats)
                | (TrainingSplats, Exporting)
                | (Exporting, Completed)
        )
    }
}

#[cfg(test)]
mod tests {
    use super::PipelineStage::*;

    #[test]
    fn allows_only_forward_pipeline_transitions() {
        assert!(Created.can_transition_to(ProbingVideo));
        assert!(Matching.can_transition_to(Reconstructing));
        assert!(ValidatingReconstruction.can_transition_to(Segment));
        assert!(Segment.can_transition_to(TrainingSplats));
        assert!(!Segment.can_transition_to(Reconstructing));
        assert!(Segment.can_transition_to(Failed));
        assert!(TrainingSplats.can_transition_to(Failed));
        assert!(!Matching.can_transition_to(PlanningFrames));
        assert!(!Completed.can_transition_to(Failed));
    }
}
