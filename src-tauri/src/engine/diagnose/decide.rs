//! Decide／Present：單一勝者出口與輸出上限。

/// LaunchDiagnosis 契約版本。
pub const DIAGNOSIS_SCHEMA_VERSION: u32 = 1;

const MAX_EVIDENCE: usize = 5;
const MAX_STEPS: usize = 3;

pub(super) fn clip_evidence(mut evidence: Vec<String>) -> Vec<String> {
    if evidence.len() > MAX_EVIDENCE {
        evidence.truncate(MAX_EVIDENCE);
    }
    evidence
}

pub(super) fn clip_steps(mut steps: Vec<String>) -> Vec<String> {
    if steps.len() > MAX_STEPS {
        steps.truncate(MAX_STEPS);
    }
    steps
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clips_to_caps() {
        let evidence = (0..10).map(|i| format!("e{i}")).collect::<Vec<_>>();
        assert_eq!(clip_evidence(evidence).len(), 5);
        let steps = (0..6).map(|i| format!("s{i}")).collect::<Vec<_>>();
        assert_eq!(clip_steps(steps).len(), 3);
    }
}
