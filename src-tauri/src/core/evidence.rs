//! Reusable provenance metadata for guidance, measurements, and deterministic rules.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceSourceType {
    MicrosoftGuidance,
    VendorGuidance,
    CommunityGuidance,
    WinOptimizerMeasuredEvidence,
    WinOptimizerDeterministicRule,
    #[default]
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceMatchingRule {
    Exact,
    Prefix,
    Pattern,
    MachineSpecificMeasurement,
    DeterministicRule,
    None,
    #[default]
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceConfidence {
    High,
    Medium,
    Low,
    #[default]
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceConflict {
    None,
    Conflicting,
    #[default]
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct EvidenceMetadata {
    pub source_type: EvidenceSourceType,
    pub source_name: String,
    pub source_version: Option<String>,
    pub scope: String,
    pub matching_rule: EvidenceMatchingRule,
    pub confidence: EvidenceConfidence,
    pub rationale: String,
    pub tradeoffs: Vec<String>,
    pub conflict: EvidenceConflict,
    pub last_bundled_update: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metadata_uses_stable_snake_case_wire_names() {
        let metadata = EvidenceMetadata {
            source_type: EvidenceSourceType::WinOptimizerMeasuredEvidence,
            source_name: "Local startup timing".into(),
            source_version: Some("1".into()),
            scope: "This machine".into(),
            matching_rule: EvidenceMatchingRule::MachineSpecificMeasurement,
            confidence: EvidenceConfidence::High,
            rationale: "Measured during startup.".into(),
            tradeoffs: vec!["Measurements can vary by workload.".into()],
            conflict: EvidenceConflict::None,
            last_bundled_update: Some("2026-08-10".into()),
        };

        let value = serde_json::to_value(&metadata).unwrap();
        assert_eq!(value["source_type"], "win_optimizer_measured_evidence");
        assert_eq!(value["matching_rule"], "machine_specific_measurement");
        assert_eq!(value["confidence"], "high");
        assert_eq!(value["conflict"], "none");
        assert_eq!(
            serde_json::from_value::<EvidenceMetadata>(value).unwrap(),
            metadata
        );
    }

    #[test]
    fn unknown_enum_values_fail_closed() {
        let metadata: EvidenceMetadata = serde_json::from_value(serde_json::json!({
            "source_type": "future_source",
            "matching_rule": "future_rule",
            "confidence": "future_confidence",
            "conflict": "future_conflict"
        }))
        .unwrap();

        assert_eq!(metadata.source_type, EvidenceSourceType::Unknown);
        assert_eq!(metadata.matching_rule, EvidenceMatchingRule::Unknown);
        assert_eq!(metadata.confidence, EvidenceConfidence::Unknown);
        assert_eq!(metadata.conflict, EvidenceConflict::Unknown);
        assert!(metadata.source_name.is_empty());
        assert!(metadata.tradeoffs.is_empty());
    }

    #[test]
    fn missing_optional_metadata_has_safe_defaults() {
        let metadata: EvidenceMetadata = serde_json::from_str("{}").unwrap();

        assert_eq!(metadata, EvidenceMetadata::default());
        assert_eq!(metadata.source_type, EvidenceSourceType::Unknown);
        assert_eq!(metadata.conflict, EvidenceConflict::Unknown);
    }
}
