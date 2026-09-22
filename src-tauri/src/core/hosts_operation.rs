use super::operation::{
    ExecutionStatus, ExpectedEffect, ExposureLevel, OperationCategory, OperationDescriptor,
    OperationError, OperationErrorCode, OperationId, OperationPlan, OperationResult,
    OperationWarning, PlannedTarget, PrivilegeRequirement, ProofStatus, RebootPolicy,
    RecoveryLevel, Reversibility, RiskLevel, StateSnapshot, TargetExecutionState, TargetOutcome,
    UndoAvailability, VerificationStatus,
};
use super::operation_journal::{
    CapturedOriginalState, JournalRecordKind, OperationJournal, OperationJournalRecord,
    UndoEligibility, VerificationRecord, OPERATION_SCHEMA_VERSION,
};
use crate::tweaks::hosts::{render_snapshot_bytes, HostEntry, HostsSnapshot};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

const TARGET_ID: &str = r"C:\Windows\System32\drivers\etc\hosts";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct HostsPlanSnapshot {
    before: HostsSnapshot,
    entries: Vec<HostEntry>,
}

#[derive(Debug, Clone, Serialize)]
pub struct HostsExecution {
    pub result: OperationResult,
    pub message: String,
}

/// Run one already-revalidated hosts target through its durable target
/// lifecycle. The mutation closure is not called unless both pre-mutation
/// journal states have been persisted.
pub(crate) fn execute_target_with_journal(
    journal: &OperationJournal,
    plan: &OperationPlan,
    target: &PlannedTarget,
    execute: impl FnOnce() -> Result<HostsExecution, OperationError>,
) -> Result<HostsExecution, OperationError> {
    let payload = serde_json::to_string(target).map_err(|error| OperationError {
        code: OperationErrorCode::SerializationFailed,
        message: error.to_string(),
        target_id: Some(target.target_id.clone()),
        retryable: false,
    })?;
    journal.record_target_state(
        &plan.id,
        &target.target_id,
        TargetExecutionState::Ready,
        &payload,
    )?;
    journal.record_target_state(
        &plan.id,
        &target.target_id,
        TargetExecutionState::MutationStarted,
        &payload,
    )?;

    let execution = match execute() {
        Ok(execution) => execution,
        Err(error) => {
            journal.record_target_state(
                &plan.id,
                &target.target_id,
                TargetExecutionState::OutcomeUnknown,
                &error.to_string(),
            )?;
            return Err(error);
        }
    };
    if execution.result.outcomes.len() != 1
        || execution.result.outcomes[0].target_id != target.target_id
    {
        let error = OperationError::validation(
            "hosts target execution must yield exactly its confirmed target outcome",
        );
        journal.record_target_state(
            &plan.id,
            &target.target_id,
            TargetExecutionState::OutcomeUnknown,
            &error.to_string(),
        )?;
        return Err(error);
    }
    journal.record_target_outcome(&plan.id, &execution.result.outcomes[0])?;
    Ok(execution)
}

pub fn create_plan(
    before: &HostsSnapshot,
    entries: Vec<HostEntry>,
) -> Result<OperationPlan, OperationError> {
    if before
        .acl
        .as_deref()
        .is_none_or(|acl| acl.trim().is_empty())
    {
        return Err(validation(
            "Exact hosts DACL evidence is unavailable; refusing to create a mutation plan",
        ));
    }
    validate_entries(&entries)?;
    let action = serde_json::to_string(&HostsPlanSnapshot {
        before: before.clone(),
        entries,
    })
    .map_err(|error| validation(format!("failed to serialize hosts plan: {error}")))?;
    Ok(OperationPlan {
        id: OperationId(uuid::Uuid::new_v4().to_string()),
        descriptor: OperationDescriptor {
            id: OperationId("hosts.save".into()),
            capability_id: "hosts.editor".into(),
            title: "Save Hosts File".into(),
            category: OperationCategory::SystemConfiguration,
            risk: RiskLevel::High,
            privilege: PrivilegeRequirement::Administrator,
            reversibility: Reversibility::BestEffort,
            reboot_policy: RebootPolicy::NotRequired,
            experimental: true,
            proof_status: ProofStatus::Experimental,
            exposure: ExposureLevel::Advanced,
            recovery: RecoveryLevel::BestEffort,
            restore_point_policy: crate::core::operation::RestorePointPolicy::NotNeeded,
            mutates_system: true,
            requires_confirmation: true,
            requires_revalidation: true,
            requires_verification: true,
            requires_journal: true,
        },
        targets: vec![PlannedTarget {
            target_id: TARGET_ID.into(),
            requested_action: action,
            description: "Replace only the canonical Windows hosts-file entries.".into(),
        }],
        warnings: vec![
            OperationWarning {
                code: "acl_restore_unproven".into(),
                message: "The original DACL is captured and restored for the replacement, with exact DACL readback verification. Exact automatic undo remains disabled because backup restoration does not restore the DACL; last-access time may change when Windows reads the file."
                    .into(),
            },
            OperationWarning {
                code: "dns_cache".into(),
                message:
                    "Existing DNS cache entries may remain until the cache expires or is flushed."
                        .into(),
            },
        ],
        expected_effects: vec![ExpectedEffect {
            target_id: TARGET_ID.into(),
            description: "The requested entries are present after an exact file re-read.".into(),
        }],
        requires_elevation: true,
        requires_restart: false,
        reversibility: Reversibility::BestEffort,
        parent_operation_id: None,
        retry_of_operation_id: None,
    })
}

pub fn create_retry_plan(
    current: &HostsSnapshot,
    previous_plan: &OperationPlan,
    unresolved_target_ids: &[String],
) -> Result<OperationPlan, OperationError> {
    if previous_plan.descriptor.id.0 != "hosts.save" {
        return Err(validation(
            "retry requires domain-specific rediscovery for this operation",
        ));
    }
    if unresolved_target_ids.len() != 1 || unresolved_target_ids[0] != TARGET_ID {
        return Err(validation("hosts retry requires exactly the hosts target"));
    }

    let previous = decode_plan(previous_plan)?;
    let expected_bytes =
        render_snapshot_bytes(&previous.before, &previous.entries).map_err(validation)?;
    let already_satisfied = current.bytes == expected_bytes
        && crate::tweaks::hosts::basic_metadata_matches(&previous.before, current);
    if already_satisfied {
        return Err(validation(
            "hosts target already matches the requested postcondition; no retry plan was created",
        ));
    }

    let mut plan = create_plan(current, previous.entries)?;
    plan.parent_operation_id = Some(previous_plan.id.clone());
    plan.retry_of_operation_id = Some(previous_plan.id.clone());
    plan.warnings.push(OperationWarning {
        code: "retry_of_operation".into(),
        message: previous_plan.id.0.clone(),
    });
    plan.validate_relationships()?;
    Ok(plan)
}

pub fn confirmed_entries(
    plan: &OperationPlan,
    current: &HostsSnapshot,
) -> Result<Vec<HostEntry>, OperationError> {
    let snapshot = decode_plan(plan)?;
    if !crate::tweaks::hosts::snapshot_matches(&snapshot.before, current) {
        return Err(OperationError {
            code: OperationErrorCode::TargetChangedSincePreview,
            message: "The hosts file changed after confirmation; refresh and review it again."
                .into(),
            target_id: Some(TARGET_ID.into()),
            retryable: true,
        });
    }
    validate_entries(&snapshot.entries)?;
    Ok(snapshot.entries)
}

pub(crate) fn mark_target_revalidation_failed(
    journal: &OperationJournal,
    plan: &OperationPlan,
    target_id: &str,
) -> Result<(), OperationError> {
    let target = plan
        .targets
        .iter()
        .find(|target| target.target_id == target_id)
        .ok_or_else(|| OperationError::validation("hosts target is not in the plan"))?;
    let payload = serde_json::to_string(target).map_err(|error| OperationError {
        code: OperationErrorCode::SerializationFailed,
        message: error.to_string(),
        target_id: Some(target_id.into()),
        retryable: false,
    })?;
    journal.record_target_state(
        &plan.id,
        target_id,
        TargetExecutionState::RevalidationFailed,
        &payload,
    )
}

pub fn finish(
    plan: &OperationPlan,
    before: &HostsSnapshot,
    after: &HostsSnapshot,
) -> Result<HostsExecution, OperationError> {
    let requested = decode_plan(plan)?.entries;
    let expected_bytes = render_snapshot_bytes(before, &requested).map_err(validation)?;
    let metadata_verified = crate::tweaks::hosts::basic_metadata_matches(before, after);
    let readback_verified = after.bytes == expected_bytes && metadata_verified;
    let verified = readback_verified && before.bytes != after.bytes;
    let unchanged = readback_verified && before.bytes == after.bytes;
    let error = (!verified && !unchanged).then(|| OperationError {
        code: OperationErrorCode::VerificationFailed,
        message: "The hosts file did not match the confirmed bytes, DACL, attributes, creation time, and last-write time after write."
            .into(),
        target_id: Some(TARGET_ID.into()),
        retryable: true,
    });
    let outcome = TargetOutcome {
        target_id: TARGET_ID.into(),
        requested_action: "save".into(),
        before: Some(state(before)),
        after: Some(state(after)),
        execution: if verified {
            ExecutionStatus::Changed
        } else if unchanged {
            ExecutionStatus::Unchanged
        } else {
            ExecutionStatus::Failed
        },
        verification: if verified || unchanged {
            VerificationStatus::Verified
        } else {
            VerificationStatus::Failed
        },
        error,
    };
    let result = OperationResult::new(
        plan.id.clone(),
        vec![outcome],
        false,
        UndoAvailability::NotReversible,
    )?;
    Ok(HostsExecution {
        message: if result.failed == 0 {
            "Hosts file saved and verified.".into()
        } else {
            "Hosts file write failed verification.".into()
        },
        result,
    })
}

pub fn journal_record(
    plan: &OperationPlan,
    before: &HostsSnapshot,
    execution: &HostsExecution,
) -> OperationJournalRecord {
    OperationJournalRecord {
        journal_id: uuid::Uuid::new_v4().to_string(),
        record_kind: JournalRecordKind::Operation,
        related_record_id: None,
        operation_id: plan.id.clone(),
        operation_schema_version: OPERATION_SCHEMA_VERSION,
        application_version: env!("CARGO_PKG_VERSION").into(),
        timestamp: chrono::Utc::now().to_rfc3339(),
        terminal_state: execution.result.completion_state(),
        confirmed_plan: plan.clone(),
        requested_targets: plan.targets.clone(),
        captured_original_states: vec![CapturedOriginalState::HostsFile {
            target_id: TARGET_ID.into(),
            path: TARGET_ID.into(),
            bytes: before.bytes.clone(),
            encoding: before.encoding.clone(),
            acl: before.acl.clone(),
            attributes: Some(before.attributes),
            creation_time: Some(before.creation_time),
            last_access_time: Some(before.last_access_time),
            last_write_time: Some(before.last_write_time),
        }],
        target_outcomes: execution.result.outcomes.clone(),
        verification_results: execution
            .result
            .outcomes
            .iter()
            .map(|outcome| VerificationRecord {
                target_id: outcome.target_id.clone(),
                status: outcome.verification,
                message: outcome
                    .error
                    .as_ref()
                    .map(|error| error.message.clone())
                    .unwrap_or_else(|| {
                        "Hosts bytes, DACL, attributes, creation time, and last-write time verified by re-read; last-access time is best effort.".into()
                    }),
            })
            .collect(),
        restart_requirement: RebootPolicy::NotRequired,
        undo_eligibility: UndoEligibility {
            eligible: false,
            reason: "Exact undo is disabled because the byte backup does not restore the captured DACL; replacement DACL restoration is verified.".into(),
            automated_restoration_test: None,
        },
        undo_execution_result: None,
        diagnostic: execution.result.outcomes[0].error.clone(),
    }
}

fn decode_plan(plan: &OperationPlan) -> Result<HostsPlanSnapshot, OperationError> {
    if plan.descriptor.id.0 != "hosts.save"
        || plan.descriptor.category != OperationCategory::SystemConfiguration
        || !plan.descriptor.experimental
        || plan.targets.len() != 1
        || plan.targets[0].target_id != TARGET_ID
        || plan.reversibility != Reversibility::BestEffort
    {
        return Err(validation("invalid hosts operation plan"));
    }
    serde_json::from_str(&plan.targets[0].requested_action)
        .map_err(|error| validation(format!("invalid hosts plan snapshot: {error}")))
}

fn validate_entries(entries: &[HostEntry]) -> Result<(), OperationError> {
    crate::tweaks::hosts::validate_entries(entries).map_err(validation)
}

fn state(snapshot: &HostsSnapshot) -> StateSnapshot {
    StateSnapshot {
        summary: format!("{} parsed entries", snapshot.entries.len()),
        properties: BTreeMap::from([
            ("digest".into(), snapshot.digest.clone()),
            ("encoding".into(), snapshot.encoding.clone()),
            ("attributes".into(), snapshot.attributes.to_string()),
            ("creation_time".into(), snapshot.creation_time.to_string()),
            (
                "last_access_time".into(),
                snapshot.last_access_time.to_string(),
            ),
            (
                "last_write_time".into(),
                snapshot.last_write_time.to_string(),
            ),
        ]),
    }
}

fn validation(message: impl Into<String>) -> OperationError {
    OperationError::validation(message)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn journal() -> (tempfile::TempDir, OperationJournal) {
        let directory = tempfile::tempdir().unwrap();
        let journal = OperationJournal::open_at(&directory.path().join("journal.db")).unwrap();
        (directory, journal)
    }

    fn synthetic_execution(plan: &OperationPlan, target: &PlannedTarget) -> HostsExecution {
        let state = || StateSnapshot {
            summary: "synthetic hosts state".into(),
            properties: BTreeMap::new(),
        };
        let outcome = TargetOutcome {
            target_id: target.target_id.clone(),
            requested_action: target.requested_action.clone(),
            before: Some(state()),
            after: Some(state()),
            execution: ExecutionStatus::Changed,
            verification: VerificationStatus::Verified,
            error: None,
        };
        HostsExecution {
            message: "synthetic test execution".into(),
            result: OperationResult::new(
                plan.id.clone(),
                vec![outcome],
                false,
                UndoAvailability::NotReversible,
            )
            .unwrap(),
        }
    }

    #[test]
    fn missing_durable_target_state_blocks_hosts_mutation_before_the_closure() {
        let (_directory, journal) = journal();
        let contract = crate::core::operation::synthetic_operation_contract().unwrap();
        let target = contract.plan.targets[0].clone();
        let mut mutation_called = false;

        let error = execute_target_with_journal(&journal, &contract.plan, &target, || {
            mutation_called = true;
            Ok(synthetic_execution(&contract.plan, &target))
        })
        .unwrap_err();

        assert_eq!(error.code, OperationErrorCode::ValidationFailed);
        assert!(error.message.contains("no durable state"));
        assert!(!mutation_called);
    }

    #[test]
    fn mutation_error_is_left_as_hosts_outcome_unknown_after_target_start() {
        let (_directory, journal) = journal();
        let contract = crate::core::operation::synthetic_operation_contract().unwrap();
        let target = contract.plan.targets[0].clone();
        journal.begin_execution(&contract.plan).unwrap();

        let error = execute_target_with_journal(&journal, &contract.plan, &target, || {
            Err(OperationError {
                code: OperationErrorCode::ExecutionFailed,
                message: "synthetic hosts mutation failure".into(),
                target_id: Some(target.target_id.clone()),
                retryable: true,
            })
        })
        .unwrap_err();

        assert_eq!(error.code, OperationErrorCode::ExecutionFailed);
        let unresolved = journal.list_unresolved_targets(&contract.plan.id).unwrap();
        assert_eq!(unresolved[0].target_id, target.target_id);
        assert_eq!(unresolved[0].state, TargetExecutionState::OutcomeUnknown);
    }

    fn snapshot(bytes: &[u8]) -> HostsSnapshot {
        HostsSnapshot {
            bytes: bytes.to_vec(),
            digest: blake3::hash(bytes).to_hex().to_string(),
            encoding: if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
                "utf-8-bom".into()
            } else {
                "utf-8".into()
            },
            attributes: 0,
            creation_time: 0,
            last_access_time: 0,
            last_write_time: 0,
            acl: Some("D:(A;;FA;;;WD)".into()),
            entries: vec![],
        }
    }

    fn entry(id: usize, hostname: &str, raw_line: Option<&str>) -> HostEntry {
        HostEntry {
            id,
            ip: "127.0.0.1".into(),
            hostnames: vec![hostname.into()],
            comment: None,
            enabled: true,
            raw_line: raw_line.map(str::to_string),
        }
    }

    #[test]
    fn stale_preview_is_rejected() {
        let before = snapshot(b"before");
        let plan = create_plan(&before, vec![]).unwrap();
        let error = confirmed_entries(&plan, &snapshot(b"after")).unwrap_err();
        assert_eq!(error.code, OperationErrorCode::TargetChangedSincePreview);
    }

    #[test]
    fn stale_basic_metadata_is_rejected() {
        let before = snapshot(b"before");
        let plan = create_plan(&before, vec![]).unwrap();
        let mut changed = before.clone();
        changed.attributes = 2;
        assert_eq!(
            confirmed_entries(&plan, &changed).unwrap_err().code,
            OperationErrorCode::TargetChangedSincePreview
        );
    }

    #[test]
    fn plan_rejects_missing_acl_evidence_before_mutation() {
        let mut before = snapshot(b"before");
        before.acl = None;

        let error = create_plan(&before, vec![]).unwrap_err();

        assert_eq!(error.code, OperationErrorCode::ValidationFailed);
        assert!(error.message.contains("DACL evidence"));
    }

    #[test]
    fn plan_cannot_target_an_alternate_file() {
        let before = snapshot(b"before");
        let mut plan = create_plan(&before, vec![]).unwrap();
        plan.targets[0].target_id = r"C:\temp\hosts".into();
        assert_eq!(
            confirmed_entries(&plan, &before).unwrap_err().code,
            OperationErrorCode::ValidationFailed
        );
    }

    #[test]
    fn finish_verifies_exact_bytes_including_unknown_lines() {
        let before = snapshot(b"!include custom.conf\r\n127.0.0.1 localhost\nunknown");
        let requested = vec![entry(2, "local.test", Some("127.0.0.1 localhost"))];
        let plan = create_plan(&before, requested.clone()).unwrap();
        let expected = render_snapshot_bytes(&before, &requested).unwrap();
        let after = snapshot(&expected);
        let execution = finish(&plan, &before, &after).unwrap();
        assert_eq!(execution.result.failed, 0);

        let tampered = snapshot(b"!include changed.conf\r\n127.0.0.1 local.test\nunknown");
        let failed = finish(&plan, &before, &tampered).unwrap();
        assert_eq!(failed.result.failed, 1);
        assert_eq!(
            failed.result.outcomes[0].verification,
            VerificationStatus::Failed
        );
    }

    #[test]
    fn finish_fails_when_basic_metadata_readback_changes() {
        let before = snapshot(b"127.0.0.1 localhost\r\n");
        let requested = vec![entry(1, "local.test", Some("127.0.0.1 localhost"))];
        let plan = create_plan(&before, requested.clone()).unwrap();
        let expected = render_snapshot_bytes(&before, &requested).unwrap();
        let mut after = snapshot(&expected);
        after.last_write_time = 1;
        let execution = finish(&plan, &before, &after).unwrap();
        assert_eq!(execution.result.failed, 1);
    }

    #[test]
    fn plan_is_explicit_that_acl_restoration_is_unproven() {
        let plan = create_plan(&snapshot(b""), vec![]).unwrap();
        assert!(plan
            .warnings
            .iter()
            .any(|warning| warning.code == "acl_restore_unproven"));
        assert_eq!(plan.descriptor.proof_status, ProofStatus::Experimental);
        assert_eq!(plan.reversibility, Reversibility::BestEffort);
    }

    #[test]
    fn retry_rebuilds_from_current_hosts_state_and_links_the_source() {
        let previous = create_plan(&snapshot(b"before"), vec![]).unwrap();
        let retry = create_retry_plan(
            &snapshot(b"changed"),
            &previous,
            std::slice::from_ref(&previous.targets[0].target_id),
        )
        .unwrap();

        assert_ne!(retry.id, previous.id);
        assert_eq!(retry.parent_operation_id, Some(previous.id.clone()));
        assert_eq!(retry.retry_of_operation_id, Some(previous.id));
        assert_eq!(decode_plan(&retry).unwrap().before.bytes, b"changed");
    }

    #[test]
    fn retry_rejects_an_already_satisfied_hosts_postcondition() {
        let before = snapshot(b"before");
        let plan = create_plan(&before, vec![]).unwrap();
        let expected = render_snapshot_bytes(&before, &[]).unwrap();

        let error = create_retry_plan(
            &snapshot(&expected),
            &plan,
            std::slice::from_ref(&plan.targets[0].target_id),
        )
        .unwrap_err();
        assert_eq!(error.code, OperationErrorCode::ValidationFailed);
    }

    #[test]
    fn journal_record_captures_hosts_content_encoding_and_metadata() {
        let mut before = snapshot(b"127.0.0.1 localhost\r\n");
        before.attributes = 32;
        before.creation_time = 1;
        before.last_access_time = 2;
        before.last_write_time = 3;
        let requested = vec![entry(1, "local.test", Some("127.0.0.1 localhost"))];
        let plan = create_plan(&before, requested.clone()).unwrap();
        let after = snapshot(&render_snapshot_bytes(&before, &requested).unwrap());
        let execution = finish(&plan, &before, &after).unwrap();
        let record = journal_record(&plan, &before, &execution);

        match &record.captured_original_states[0] {
            CapturedOriginalState::HostsFile {
                bytes,
                encoding,
                acl,
                attributes,
                creation_time,
                last_access_time,
                last_write_time,
                ..
            } => {
                assert_eq!(bytes, &before.bytes);
                assert_eq!(encoding, &before.encoding);
                assert_eq!(acl, &before.acl);
                assert_eq!(*attributes, Some(before.attributes));
                assert_eq!(*creation_time, Some(before.creation_time));
                assert_eq!(*last_access_time, Some(before.last_access_time));
                assert_eq!(*last_write_time, Some(before.last_write_time));
            }
            other => panic!("unexpected captured state: {other:?}"),
        }
    }
}
