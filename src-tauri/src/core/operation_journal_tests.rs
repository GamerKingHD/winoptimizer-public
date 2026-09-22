use super::*;
use crate::core::operation::{RecoveryLevel, StateDiffStatus, StateEvidence, StateValue};
use tempfile::tempdir;

fn journal() -> (tempfile::TempDir, OperationJournal) {
    let directory = tempdir().unwrap();
    let journal = OperationJournal::open_at(&directory.path().join("journal.db")).unwrap();
    (directory, journal)
}

fn reopen_journal(journal: &OperationJournal) -> OperationJournal {
    let path = journal.db_path().to_path_buf();
    OperationJournal::open_at(&path).unwrap()
}

fn synthetic_operation_record() -> Result<OperationJournalRecord, OperationError> {
    let contract = crate::core::operation::synthetic_operation_contract()?;
    let capture = CapturedOriginalState::Synthetic {
        target_id: "synthetic.changed".into(),
        state: contract.result.outcomes[0].before.clone().ok_or_else(|| {
            OperationError::validation("synthetic operation lacks original state")
        })?,
    };
    Ok(OperationJournalRecord {
        journal_id: uuid::Uuid::new_v4().to_string(),
        record_kind: JournalRecordKind::Operation,
        related_record_id: None,
        operation_id: contract.plan.id.clone(),
        operation_schema_version: OPERATION_SCHEMA_VERSION,
        application_version: env!("CARGO_PKG_VERSION").into(),
        timestamp: chrono::Utc::now().to_rfc3339(),
        terminal_state: OperationState::PartiallySucceeded,
        confirmed_plan: contract.plan.clone(),
        requested_targets: contract.plan.targets.clone(),
        captured_original_states: vec![capture],
        target_outcomes: contract.result.outcomes.clone(),
        verification_results: contract
            .result
            .outcomes
            .iter()
            .map(|outcome| VerificationRecord {
                target_id: outcome.target_id.clone(),
                status: outcome.verification,
                message: format!("{:?}", outcome.verification),
            })
            .collect(),
        restart_requirement: RebootPolicy::NotRequired,
        undo_eligibility: UndoEligibility {
            eligible: true,
            reason: "Captured original state has an automated restoration test.".into(),
            automated_restoration_test: Some(
                "operation_journal_tests::undo_restores_captured_original_state".into(),
            ),
        },
        undo_execution_result: None,
        diagnostic: contract.result.outcomes[1].error.clone(),
    })
}

#[test]
fn journal_is_append_only_and_round_trips_complete_records() {
    let (_directory, journal) = journal();
    let record = synthetic_operation_record().unwrap();
    journal.append(&record).unwrap();
    assert!(journal.append(&record).is_err());
    assert_eq!(journal.list().unwrap(), vec![record]);
}

#[test]
fn injected_journal_write_failure_is_one_shot_and_does_not_persist() {
    let (_directory, journal) = journal();
    let record = synthetic_operation_record().unwrap();

    journal.fail_next_journal_write();
    let error = journal.append(&record).unwrap_err();

    assert_eq!(error.code, OperationErrorCode::InternalError);
    assert_eq!(error.message, "failure injection: fail_journal_write");
    assert!(error.retryable);
    assert!(journal.list().unwrap().is_empty());

    journal.append(&record).unwrap();
    assert_eq!(journal.list().unwrap(), vec![record]);
}

#[test]
fn mutating_terminal_records_require_a_durable_execution_start() {
    let (_directory, journal) = journal();
    let mut record = synthetic_operation_record().unwrap();
    record.confirmed_plan.descriptor.mutates_system = true;
    record.confirmed_plan.descriptor.requires_confirmation = true;
    record.confirmed_plan.descriptor.requires_revalidation = true;
    record.confirmed_plan.descriptor.requires_verification = true;
    record.confirmed_plan.descriptor.requires_journal = true;
    record.confirmed_plan.descriptor.reversibility = Reversibility::NotReversible;
    record.confirmed_plan.reversibility = Reversibility::NotReversible;
    record.undo_eligibility = UndoEligibility::unavailable(
        "This synthetic mutating record does not provide exact recovery.",
    );

    let error = journal.append(&record).unwrap_err();
    assert!(error
        .message
        .contains("must persist ExecutionStarting before terminal journal append"));
    assert!(journal.list().unwrap().is_empty());

    journal.record_confirmation(&record.confirmed_plan).unwrap();
    journal.begin_execution(&record.confirmed_plan).unwrap();
    journal.append(&record).unwrap();
}

#[test]
fn mutating_execution_requires_durable_confirmation_of_the_exact_plan() {
    let (_directory, journal) = journal();
    let mut record = synthetic_operation_record().unwrap();
    record.confirmed_plan.descriptor.mutates_system = true;
    record.confirmed_plan.descriptor.requires_confirmation = true;
    record.confirmed_plan.descriptor.requires_revalidation = true;
    record.confirmed_plan.descriptor.requires_verification = true;
    record.confirmed_plan.descriptor.requires_journal = true;
    record.confirmed_plan.descriptor.reversibility = Reversibility::NotReversible;
    record.confirmed_plan.reversibility = Reversibility::NotReversible;
    let plan = record.confirmed_plan.clone();

    // Execution must fail closed without a durable confirmation.
    let error = journal.begin_execution(&plan).unwrap_err();
    assert!(error.message.contains("durable confirmation"));

    // Confirmation is bound to plan identity: a changed target set must not
    // inherit the recorded confirmation.
    journal.record_confirmation(&plan).unwrap();
    let mut forged = plan.clone();
    forged.targets[0].requested_action = "different_action".into();
    let error = journal.begin_execution(&forged).unwrap_err();
    assert!(error.message.contains("durable confirmation"));

    journal.begin_execution(&plan).unwrap();
}

#[test]
fn mutation_requires_durable_revalidation_marker() {
    let (_directory, journal) = journal();
    let contract = crate::core::operation::synthetic_operation_contract().unwrap();
    let target = &contract.plan.targets[0];
    let payload = serde_json::to_string(target).unwrap();

    journal.begin_execution(&contract.plan).unwrap();
    // Pending -> MutationStarted without the `Ready` revalidation marker must
    // fail closed.
    let error = journal
        .record_target_state(
            &contract.plan.id,
            &target.target_id,
            TargetExecutionState::MutationStarted,
            &payload,
        )
        .unwrap_err();
    assert!(error.message.contains("forbidden target transition"));

    journal
        .record_target_state(
            &contract.plan.id,
            &target.target_id,
            TargetExecutionState::Ready,
            &payload,
        )
        .unwrap();
    journal
        .record_target_state(
            &contract.plan.id,
            &target.target_id,
            TargetExecutionState::MutationStarted,
            &payload,
        )
        .unwrap();
}

#[test]
fn success_cannot_be_recorded_when_required_verification_is_missing() {
    let (_directory, journal) = journal();
    let mut record = synthetic_operation_record().unwrap();
    record.confirmed_plan.descriptor.requires_verification = true;
    record.terminal_state = OperationState::Succeeded;
    for outcome in &mut record.target_outcomes {
        outcome.verification = VerificationStatus::Unverified;
    }
    for verification in &mut record.verification_results {
        verification.status = VerificationStatus::Unverified;
    }

    let error = record.validate().unwrap_err();
    assert!(error
        .message
        .contains("cannot record success without verifying every target"));
    assert!(journal.append(&record).is_err());
}

#[test]
fn journal_rejects_illegal_operation_state_transitions() {
    let (_directory, journal) = journal();
    let contract = crate::core::operation::synthetic_operation_contract().unwrap();

    journal.begin_execution(&contract.plan).unwrap();
    // Executing must not jump straight to a pre-execution or verifying state.
    for illegal in [
        OperationExecutionState::Confirmed,
        OperationExecutionState::Verifying,
        OperationExecutionState::Succeeded,
        OperationExecutionState::Planned,
    ] {
        let error = journal
            .record_execution_state(&contract.plan, illegal)
            .unwrap_err();
        assert!(
            error.message.contains("forbidden operation transition"),
            "expected {illegal:?} to be rejected, got {error:?}"
        );
    }

    // The legal path still works.
    journal
        .record_execution_state(&contract.plan, OperationExecutionState::MutationCompleted)
        .unwrap();
    journal
        .record_execution_state(&contract.plan, OperationExecutionState::Verifying)
        .unwrap();
    journal
        .record_execution_state(&contract.plan, OperationExecutionState::Succeeded)
        .unwrap();
}

#[test]
fn fail_after_plan_persist_is_one_shot_and_recoverable() {
    let (_directory, journal) = journal();
    let contract = crate::core::operation::synthetic_operation_contract().unwrap();

    journal.fail_after_plan_persist();
    let error = journal.begin_execution(&contract.plan).unwrap_err();

    assert_eq!(error.message, "failure injection: fail_after_plan_persist");
    let persisted = reopen_journal(&journal);
    assert!(persisted.list().unwrap().is_empty());
    assert_eq!(
        persisted.latest_execution_state(&contract.plan.id).unwrap(),
        Some(OperationExecutionState::ExecutionStarting)
    );
    let unresolved = persisted.list_unresolved_operations().unwrap();
    assert_eq!(unresolved.len(), 1);
    assert_eq!(
        unresolved[0].state,
        OperationExecutionState::ExecutionStarting
    );
    assert!(unresolved[0]
        .targets
        .iter()
        .all(|target| target.state == TargetExecutionState::Pending));

    assert_eq!(persisted.reconcile_unresolved_operations().unwrap(), 1);
    assert_eq!(
        persisted.latest_execution_state(&contract.plan.id).unwrap(),
        Some(OperationExecutionState::OutcomeUnknown)
    );

    drop(persisted);
    let error = journal.begin_execution(&contract.plan).unwrap_err();
    assert!(error
        .message
        .contains("already started; retry must use a new operation id"));
}

#[test]
fn fail_after_execution_start_is_one_shot_and_recoverable() {
    let (_directory, journal) = journal();
    let contract = crate::core::operation::synthetic_operation_contract().unwrap();

    journal.fail_after_execution_start();
    let error = journal.begin_execution(&contract.plan).unwrap_err();

    assert_eq!(
        error.message,
        "failure injection: fail_after_execution_start"
    );
    let persisted = reopen_journal(&journal);
    assert!(persisted.list().unwrap().is_empty());
    assert_eq!(
        persisted.latest_execution_state(&contract.plan.id).unwrap(),
        Some(OperationExecutionState::Executing)
    );
    let unresolved = persisted.list_unresolved_operations().unwrap();
    assert_eq!(unresolved.len(), 1);
    assert_eq!(unresolved[0].state, OperationExecutionState::Executing);
    assert!(unresolved[0]
        .targets
        .iter()
        .all(|target| target.state == TargetExecutionState::Pending));

    assert_eq!(persisted.reconcile_unresolved_operations().unwrap(), 1);
    assert_eq!(
        persisted.latest_execution_state(&contract.plan.id).unwrap(),
        Some(OperationExecutionState::OutcomeUnknown)
    );

    drop(persisted);
    let error = journal.begin_execution(&contract.plan).unwrap_err();
    assert!(error
        .message
        .contains("already started; retry must use a new operation id"));
}

#[test]
fn fail_before_verification_rolls_back_terminal_persistence_and_is_recoverable() {
    let (_directory, journal) = journal();
    let contract = crate::core::operation::synthetic_operation_contract().unwrap();
    let record = execute_confirmed_synthetic_operation(&contract.plan).unwrap();

    journal.begin_execution(&contract.plan).unwrap();
    journal.fail_before_verification();
    let error = journal.append(&record).unwrap_err();

    assert_eq!(error.message, "failure injection: fail_before_verification");
    let persisted = reopen_journal(&journal);
    assert!(persisted.list().unwrap().is_empty());
    assert_eq!(
        persisted.latest_execution_state(&contract.plan.id).unwrap(),
        Some(OperationExecutionState::Executing)
    );
    assert!(persisted
        .list_unresolved_targets(&contract.plan.id)
        .unwrap()
        .iter()
        .all(|target| target.state == TargetExecutionState::Pending));

    // The rolled-back append is recoverable while the operation is still
    // executing; reconciliation to OutcomeUnknown is a separate terminal path.
    drop(persisted);
    journal.append(&record).unwrap();
    assert_eq!(journal.list().unwrap(), vec![record]);
    assert!(journal.list_unresolved_operations().unwrap().is_empty());
}

#[test]
fn state_diff_is_structured_and_does_not_infer_missing_after_state() {
    let contract = crate::core::operation::synthetic_operation_contract().unwrap();
    let changed = contract.result.outcomes[0].state_diff();
    assert_eq!(changed.status, StateDiffStatus::Changed);
    assert!(matches!(changed.before, StateEvidence::Observed { .. }));
    assert!(matches!(changed.after, StateEvidence::Observed { .. }));
    assert_eq!(changed.properties.len(), 1);
    assert_eq!(changed.properties[0].field, "value");
    assert!(matches!(
        changed.properties[0].before,
        StateValue::Text { ref value } if value == "0"
    ));
    assert!(matches!(
        changed.properties[0].after,
        StateValue::Text { ref value } if value == "1"
    ));

    let missing = contract.result.outcomes[1].state_diff();
    assert_eq!(missing.status, StateDiffStatus::Unknown);
    assert!(matches!(
        missing.after,
        StateEvidence::Unavailable { ref reason }
            if reason.contains("does not distinguish")
    ));
    assert!(missing.properties.is_empty());
}

#[test]
fn journal_state_diff_field_is_serialized_and_legacy_records_still_load() {
    let record = synthetic_operation_record().unwrap();
    let mut json = serde_json::to_value(&record).unwrap();
    assert_eq!(json["state_diffs"].as_array().unwrap().len(), 2);
    assert_eq!(json["state_diffs"][0]["status"], "changed");
    assert_eq!(json["state_diffs"][1]["status"], "unknown");

    json.as_object_mut().unwrap().remove("state_diffs");
    assert_eq!(
        serde_json::from_value::<OperationJournalRecord>(json).unwrap(),
        record
    );
}

#[test]
fn journal_rejects_duplicate_target_outcomes_and_verification_entries() {
    let mut record = synthetic_operation_record().unwrap();
    record.target_outcomes[1].target_id = record.target_outcomes[0].target_id.clone();
    assert!(record.validate().is_err());

    let mut record = synthetic_operation_record().unwrap();
    record.verification_results[1].target_id = record.verification_results[0].target_id.clone();
    assert!(record.validate().is_err());
}

#[test]
fn operation_result_rejects_duplicate_target_outcomes() {
    let contract = crate::core::operation::synthetic_operation_contract().unwrap();
    let mut outcomes = contract.result.outcomes;
    outcomes[1].target_id = outcomes[0].target_id.clone();

    assert!(OperationResult::new(
        contract.plan.id,
        outcomes,
        false,
        UndoAvailability::NotReversible,
    )
    .is_err());
}

#[test]
fn typed_state_capture_round_trips_without_advertising_undo() {
    let capture = CapturedOriginalState::TypedState {
        target_id: "service.example".into(),
        state: crate::core::operation::StateSnapshot {
            summary: "Example is stopped".into(),
            properties: [
                ("status".into(), "stopped".into()),
                ("start_type".into(), "manual".into()),
            ]
            .into_iter()
            .collect(),
        },
    };
    let json = serde_json::to_value(&capture).unwrap();
    assert_eq!(json["kind"], "typed_state");
    assert_eq!(
        serde_json::from_value::<CapturedOriginalState>(json).unwrap(),
        capture
    );

    let mut record = synthetic_operation_record().unwrap();
    record.captured_original_states = vec![capture];
    record.undo_eligibility = UndoEligibility::unavailable(
        "Typed state is evidence only until a domain restoration adapter is proven.",
    );
    assert!(record.validate().is_ok());
    assert!(preview_undo(&record).is_err());
}

#[test]
fn execution_start_is_durable_and_recoverable_after_interruption() {
    let (_directory, journal) = journal();
    let contract = crate::core::operation::synthetic_operation_contract().unwrap();

    journal.begin_execution(&contract.plan).unwrap();

    let unresolved = journal.list_unresolved_operations().unwrap();
    assert_eq!(unresolved.len(), 1);
    assert_eq!(unresolved[0].operation_id, contract.plan.id);
    assert_eq!(unresolved[0].state, OperationExecutionState::Executing);
    assert_eq!(unresolved[0].confirmed_plan, contract.plan);
    assert_eq!(
        journal.list_unresolved_targets(&contract.plan.id).unwrap(),
        contract
            .plan
            .targets
            .iter()
            .map(|target| UnresolvedTarget {
                operation_id: contract.plan.id.clone(),
                target_id: target.target_id.clone(),
                state: TargetExecutionState::Pending,
                payload_json: serde_json::to_string(target).unwrap(),
            })
            .collect::<Vec<_>>()
    );
}

#[test]
fn started_operation_cannot_be_replayed_under_the_same_id() {
    let (_directory, journal) = journal();
    let contract = crate::core::operation::synthetic_operation_contract().unwrap();

    journal.begin_execution(&contract.plan).unwrap();

    let error = journal.begin_execution(&contract.plan).unwrap_err();
    assert!(error
        .message
        .contains("already started; retry must use a new operation id"));
}

#[test]
fn retry_selection_returns_only_currently_unresolved_targets() {
    let (_directory, journal) = journal();
    let contract = crate::core::operation::synthetic_operation_contract().unwrap();
    let resolved = &contract.plan.targets[0];
    let unresolved = &contract.plan.targets[1];

    journal.begin_execution(&contract.plan).unwrap();
    journal
        .record_target_state(
            &contract.plan.id,
            &resolved.target_id,
            TargetExecutionState::Ready,
            "{}",
        )
        .unwrap();
    journal
        .record_target_state(
            &contract.plan.id,
            &resolved.target_id,
            TargetExecutionState::MutationStarted,
            "{}",
        )
        .unwrap();
    journal
        .record_target_state(
            &contract.plan.id,
            &resolved.target_id,
            TargetExecutionState::Changed,
            "{}",
        )
        .unwrap();
    journal
        .record_target_state(
            &contract.plan.id,
            &resolved.target_id,
            TargetExecutionState::VerificationSucceeded,
            "{\"verified\":true}",
        )
        .unwrap();

    let selected = journal
        .select_unresolved_targets_for_retry(
            &contract.plan.id,
            std::slice::from_ref(&unresolved.target_id),
        )
        .unwrap();
    assert_eq!(selected.len(), 1);
    assert_eq!(selected[0].target_id, unresolved.target_id);

    let error = journal
        .select_unresolved_targets_for_retry(
            &contract.plan.id,
            std::slice::from_ref(&resolved.target_id),
        )
        .unwrap_err();
    assert!(error
        .message
        .contains("currently unresolved in the operation journal"));
}

#[test]
fn recovery_inspection_can_prove_one_target_without_closing_the_operation() {
    let (_directory, journal) = journal();
    let contract = crate::core::operation::synthetic_operation_contract().unwrap();
    let verified = &contract.plan.targets[0];
    let still_unknown = &contract.plan.targets[1];

    journal.begin_execution(&contract.plan).unwrap();
    assert_eq!(journal.reconcile_unresolved_operations().unwrap(), 1);

    assert_eq!(
        journal
            .reconcile_unresolved_target(
                &contract.plan.id,
                &verified.target_id,
                ReconciliationDecision::PostconditionVerified {
                    payload_json: "{\"postcondition\":\"observed\"}".into(),
                },
            )
            .unwrap(),
        TargetExecutionState::VerificationSucceeded
    );
    let unresolved = journal.list_unresolved_targets(&contract.plan.id).unwrap();
    assert_eq!(unresolved.len(), 1);
    assert_eq!(unresolved[0].target_id, still_unknown.target_id);
    assert_eq!(
        journal.latest_execution_state(&contract.plan.id).unwrap(),
        Some(OperationExecutionState::OutcomeUnknown)
    );
}

#[test]
fn per_target_transitions_are_durable_until_verification() {
    let (_directory, journal) = journal();
    let contract = crate::core::operation::synthetic_operation_contract().unwrap();
    let target = &contract.plan.targets[0];

    journal.begin_execution(&contract.plan).unwrap();
    assert_eq!(
        journal
            .list_unresolved_targets(&contract.plan.id)
            .unwrap()
            .iter()
            .find(|unresolved| unresolved.target_id == target.target_id)
            .unwrap()
            .state,
        TargetExecutionState::Pending
    );
    journal
        .record_target_state(
            &contract.plan.id,
            &target.target_id,
            TargetExecutionState::Ready,
            &serde_json::to_string(target).unwrap(),
        )
        .unwrap();
    assert_eq!(
        journal
            .list_unresolved_targets(&contract.plan.id)
            .unwrap()
            .iter()
            .find(|unresolved| unresolved.target_id == target.target_id)
            .unwrap()
            .state,
        TargetExecutionState::Ready
    );
    journal
        .record_target_state(
            &contract.plan.id,
            &target.target_id,
            TargetExecutionState::MutationStarted,
            &serde_json::to_string(target).unwrap(),
        )
        .unwrap();
    journal
        .record_target_state(
            &contract.plan.id,
            &target.target_id,
            TargetExecutionState::Changed,
            &serde_json::to_string(target).unwrap(),
        )
        .unwrap();

    let reopened = reopen_journal(&journal);
    let unresolved = reopened.list_unresolved_targets(&contract.plan.id).unwrap();
    let changed = unresolved
        .iter()
        .find(|unresolved| unresolved.target_id == target.target_id)
        .unwrap();
    assert_eq!(changed.state, TargetExecutionState::Changed);

    reopened
        .record_target_state(
            &contract.plan.id,
            &target.target_id,
            TargetExecutionState::VerificationSucceeded,
            &serde_json::to_string(target).unwrap(),
        )
        .unwrap();
    assert!(reopened
        .list_unresolved_targets(&contract.plan.id)
        .unwrap()
        .iter()
        .all(|target| target.target_id != "synthetic.changed"));
}

#[test]
fn reconciliation_keeps_partial_target_progress_conservative_after_crash() {
    let (_directory, journal) = journal();
    let contract = crate::core::operation::synthetic_operation_contract().unwrap();
    let ready_target = &contract.plan.targets[0];

    journal.begin_execution(&contract.plan).unwrap();
    journal
        .record_target_state(
            &contract.plan.id,
            &ready_target.target_id,
            TargetExecutionState::Ready,
            &serde_json::to_string(ready_target).unwrap(),
        )
        .unwrap();
    journal
        .record_target_state(
            &contract.plan.id,
            &ready_target.target_id,
            TargetExecutionState::MutationStarted,
            &serde_json::to_string(ready_target).unwrap(),
        )
        .unwrap();

    let reopened = reopen_journal(&journal);
    let unresolved = reopened.list_unresolved_targets(&contract.plan.id).unwrap();
    assert_eq!(
        unresolved
            .iter()
            .find(|target| target.target_id == ready_target.target_id)
            .unwrap()
            .state,
        TargetExecutionState::MutationStarted
    );
    assert!(unresolved
        .iter()
        .filter(|target| target.target_id != ready_target.target_id)
        .all(|target| target.state == TargetExecutionState::Pending));

    assert_eq!(reopened.reconcile_unresolved_operations().unwrap(), 1);
    let reconciled = reopened.list_unresolved_operations().unwrap();
    assert_eq!(reconciled.len(), 1);
    assert_eq!(reconciled[0].state, OperationExecutionState::OutcomeUnknown);
    assert!(reconciled[0]
        .targets
        .iter()
        .all(|target| target.state == TargetExecutionState::OutcomeUnknown));
}

#[test]
fn failed_target_remains_available_for_retry_after_interruption() {
    let (_directory, journal) = journal();
    let contract = crate::core::operation::synthetic_operation_contract().unwrap();
    let target = &contract.plan.targets[0];

    journal.begin_execution(&contract.plan).unwrap();
    journal
        .record_target_state(
            &contract.plan.id,
            &target.target_id,
            TargetExecutionState::Ready,
            &serde_json::to_string(target).unwrap(),
        )
        .unwrap();
    journal
        .record_target_state(
            &contract.plan.id,
            &target.target_id,
            TargetExecutionState::MutationStarted,
            &serde_json::to_string(target).unwrap(),
        )
        .unwrap();
    journal
        .record_target_outcome(
            &contract.plan.id,
            &TargetOutcome {
                target_id: target.target_id.clone(),
                requested_action: target.requested_action.clone(),
                before: None,
                after: None,
                execution: ExecutionStatus::Failed,
                verification: VerificationStatus::Failed,
                error: Some(OperationError {
                    code: OperationErrorCode::ExecutionFailed,
                    message: "synthetic mutation failed after it started".into(),
                    target_id: Some(target.target_id.clone()),
                    retryable: true,
                }),
            },
        )
        .unwrap();

    let unresolved = journal.list_unresolved_targets(&contract.plan.id).unwrap();
    assert_eq!(unresolved.len(), 2);
    assert_eq!(
        unresolved
            .iter()
            .find(|target| target.target_id == contract.plan.targets[0].target_id)
            .unwrap()
            .state,
        TargetExecutionState::Failed
    );

    assert_eq!(journal.reconcile_unresolved_operations().unwrap(), 1);
    let reconciled = journal.list_unresolved_targets(&contract.plan.id).unwrap();
    // A durable Failed target observes real work; reconciliation must preserve
    // it and keep it available for retry rather than erasing it as unknown.
    assert_eq!(
        reconciled
            .iter()
            .find(|target| target.target_id == contract.plan.targets[0].target_id)
            .unwrap()
            .state,
        TargetExecutionState::Failed
    );
}

#[test]
fn durable_target_observations_survive_startup_reconciliation() {
    let (_directory, journal) = journal();
    let contract = crate::core::operation::synthetic_operation_contract().unwrap();
    let changed_target = &contract.plan.targets[0];
    let unchanged_target = &contract.plan.targets[1];

    journal.begin_execution(&contract.plan).unwrap();
    for (target, state) in [
        (changed_target, TargetExecutionState::Changed),
        (unchanged_target, TargetExecutionState::Unchanged),
    ] {
        journal
            .record_target_state(
                &contract.plan.id,
                &target.target_id,
                TargetExecutionState::Ready,
                &serde_json::to_string(target).unwrap(),
            )
            .unwrap();
        journal
            .record_target_state(
                &contract.plan.id,
                &target.target_id,
                TargetExecutionState::MutationStarted,
                &serde_json::to_string(target).unwrap(),
            )
            .unwrap();
        journal
            .record_target_state(
                &contract.plan.id,
                &target.target_id,
                state,
                &serde_json::to_string(target).unwrap(),
            )
            .unwrap();
    }

    let reopened = reopen_journal(&journal);
    reopened.reconcile_unresolved_operations().unwrap();
    let reconciled = reopened.list_unresolved_targets(&contract.plan.id).unwrap();

    for (target, expected) in [
        (changed_target, TargetExecutionState::Changed),
        (unchanged_target, TargetExecutionState::Unchanged),
    ] {
        assert_eq!(
            reconciled
                .iter()
                .find(|entry| entry.target_id == target.target_id)
                .unwrap()
                .state,
            expected
        );
    }
}

#[test]
fn per_target_transitions_reject_unknown_targets_and_backtracking() {
    let (_directory, journal) = journal();
    let contract = crate::core::operation::synthetic_operation_contract().unwrap();

    journal.begin_execution(&contract.plan).unwrap();
    assert!(journal
        .record_target_state(
            &contract.plan.id,
            "synthetic.unknown",
            TargetExecutionState::VerificationSucceeded,
            "{}",
        )
        .is_err());
    journal
        .record_target_state(
            &contract.plan.id,
            "synthetic.changed",
            TargetExecutionState::Ready,
            "{}",
        )
        .unwrap();
    assert!(journal
        .record_target_state(
            &contract.plan.id,
            "synthetic.changed",
            TargetExecutionState::Pending,
            "{}",
        )
        .is_err());
}

#[test]
fn operation_ids_can_be_verified_for_local_evidence_linkage() {
    let (_directory, journal) = journal();
    let contract = crate::core::operation::synthetic_operation_contract().unwrap();

    assert!(!journal.contains_operation_id("missing-operation").unwrap());
    journal.begin_execution(&contract.plan).unwrap();
    assert!(journal.contains_operation_id(&contract.plan.id.0).unwrap());
}

#[test]
fn startup_reconciliation_marks_interrupted_work_outcome_unknown() {
    let (_directory, journal) = journal();
    let contract = crate::core::operation::synthetic_operation_contract().unwrap();

    journal.begin_execution(&contract.plan).unwrap();

    assert_eq!(journal.reconcile_unresolved_operations().unwrap(), 1);
    let unresolved = journal.list_unresolved_operations().unwrap();
    assert_eq!(unresolved[0].state, OperationExecutionState::OutcomeUnknown);
    assert!(unresolved[0]
        .targets
        .iter()
        .all(|target| target.state == TargetExecutionState::OutcomeUnknown));
    assert_eq!(journal.reconcile_unresolved_operations().unwrap(), 0);
}

#[test]
fn one_unreadable_operation_does_not_strand_independent_reconciliations() {
    let (_directory, journal) = journal();
    let synthetic = crate::core::operation::synthetic_operation_contract()
        .unwrap()
        .plan;

    // B and C: independent, valid interrupted operations in a different domain.
    let repair_b = crate::core::repair_operation::create_plan(
        &crate::core::repair_operation::find_target("sfc_verify").unwrap(),
    )
    .unwrap();
    let repair_c = crate::core::repair_operation::create_plan(
        &crate::core::repair_operation::find_target("dism_check_health").unwrap(),
    )
    .unwrap();

    for plan in [&synthetic, &repair_b, &repair_c] {
        journal.begin_execution(plan).unwrap();
    }

    // Corrupt only the synthetic operation's stored execution plan so its
    // reconciliation cannot read the plan.
    {
        let connection = rusqlite::Connection::open(journal.db_path()).unwrap();
        connection
            .execute(
                "UPDATE operation_execution_events SET payload_json = ?1 WHERE operation_id = ?2",
                rusqlite::params!["{not-json", synthetic.id.0],
            )
            .unwrap();
    }

    let report = journal.reconcile_unresolved_operations_reporting().unwrap();
    assert_eq!(
        report.reconciled, 2,
        "a single unreadable operation must not strand independent operations"
    );
    assert_eq!(
        report.failures.len(),
        1,
        "the unreadable operation must be reported"
    );
    assert_eq!(report.failures[0].operation_id, synthetic.id);
    assert!(report.failures[0]
        .message
        .contains("unreadable execution plan"));

    for plan in [&repair_b, &repair_c] {
        assert_eq!(
            journal.latest_execution_state(&plan.id).unwrap(),
            Some(OperationExecutionState::OutcomeUnknown),
            "independent interrupted operation {} must leave executing",
            plan.id.0
        );
    }
}

#[test]
fn terminal_record_closes_the_unresolved_execution() {
    let (_directory, journal) = journal();
    let contract = crate::core::operation::synthetic_operation_contract().unwrap();
    journal.begin_execution(&contract.plan).unwrap();
    journal
        .append(&execute_confirmed_synthetic_operation(&contract.plan).unwrap())
        .unwrap();

    assert!(journal.list_unresolved_operations().unwrap().is_empty());
    assert!(journal
        .list_unresolved_targets(&contract.plan.id)
        .unwrap()
        .is_empty());
}

#[test]
fn terminal_record_hides_final_unchanged_targets_from_recovery_queue() {
    let (_directory, journal) = journal();
    let mut record = synthetic_operation_record().unwrap();
    record.target_outcomes[0].after = record.target_outcomes[0].before.clone();
    record.target_outcomes[0].execution = ExecutionStatus::Unchanged;
    record.target_outcomes[0].verification = VerificationStatus::Verified;

    journal.append(&record).unwrap();

    assert!(journal
        .list_unresolved_targets(&record.operation_id)
        .unwrap()
        .is_empty());
}

#[test]
fn resumed_execution_uses_only_the_exact_confirmed_targets() {
    let contract = crate::core::operation::synthetic_operation_contract().unwrap();
    let record = execute_confirmed_synthetic_operation(&contract.plan).unwrap();
    assert_eq!(record.requested_targets, contract.plan.targets);
    assert_eq!(record.target_outcomes, contract.result.outcomes);

    let mut expanded = contract.plan;
    expanded.targets.push(PlannedTarget {
        target_id: "synthetic.expanded".into(),
        requested_action: "set".into(),
        description: "Unconfirmed target".into(),
    });
    assert!(execute_confirmed_synthetic_operation(&expanded).is_err());
}

#[test]
fn exact_undo_requires_capture_and_automated_restoration_test() {
    let mut record = synthetic_operation_record().unwrap();
    record.captured_original_states.clear();
    assert!(record.validate().is_err());

    record = synthetic_operation_record().unwrap();
    record.undo_eligibility.automated_restoration_test = None;
    assert!(record.validate().is_err());

    record = synthetic_operation_record().unwrap();
    record.captured_original_states = vec![CapturedOriginalState::RegistryValue {
        target_id: "synthetic.changed".into(),
        hive: "HKCU".into(),
        key_path: r"Software\Example".into(),
        value_name: "Enabled".into(),
        existed: true,
        value_type: Some("REG_DWORD".into()),
        raw_value_data: Some(1_u32.to_le_bytes().to_vec()),
    }];
    assert!(record.validate().is_err());
}

#[test]
fn captured_state_alone_does_not_enable_ui_undo() {
    let mut record = synthetic_operation_record().unwrap();
    record.undo_eligibility =
        UndoEligibility::unavailable("A captured backup exists, but exact recovery is not proven.");

    assert!(!record.ui_undo_available());
    assert!(preview_undo(&record).is_err());
}

#[test]
fn best_effort_recovery_cannot_advertise_ui_undo() {
    let mut record = synthetic_operation_record().unwrap();
    record.confirmed_plan.descriptor.recovery = RecoveryLevel::BestEffort;

    let error = record.validate().unwrap_err();
    assert!(error.message.contains("Exact recovery"));
}

#[test]
fn captured_state_preserves_exact_registry_environment_startup_and_hosts_data() {
    let captures = vec![
        CapturedOriginalState::RegistryValue {
            target_id: "registry".into(),
            hive: "HKLM".into(),
            key_path: r"Software\Example".into(),
            value_name: "Enabled".into(),
            existed: true,
            value_type: Some("REG_BINARY".into()),
            raw_value_data: Some(vec![0, 1, 0xff]),
        },
        CapturedOriginalState::EnvironmentVariable {
            target_id: "environment".into(),
            name: "EXAMPLE".into(),
            scope: EnvironmentScope::System,
            existed: true,
            value_type: Some("REG_EXPAND_SZ".into()),
            value: Some(r"%SystemRoot%\Example".into()),
        },
        CapturedOriginalState::StartupEntry {
            target_id: "startup".into(),
            identifier: "Example startup identifier".into(),
            name: "Example".into(),
            location: "HKCU_Run".into(),
            value_type: Some("REG_SZ".into()),
            enabled: false,
            command: r#""C:\Example\App.exe" --quiet"#.into(),
        },
        CapturedOriginalState::HostsFile {
            target_id: "hosts".into(),
            path: r"C:\Windows\System32\drivers\etc\hosts".into(),
            bytes: vec![0xff, 0xfe, b'1', 0, b'2', 0, b'7', 0],
            encoding: "utf-16le".into(),
            acl: Some("D:(A;;FA;;;SY)".into()),
            attributes: Some(32),
            creation_time: Some(1),
            last_access_time: Some(2),
            last_write_time: Some(3),
        },
    ];

    assert_eq!(
        serde_json::from_str::<Vec<CapturedOriginalState>>(
            &serde_json::to_string(&captures).unwrap()
        )
        .unwrap(),
        captures
    );
}

#[test]
fn undo_restores_captured_original_state_through_full_lifecycle() {
    let source = synthetic_operation_record().unwrap();
    let preview = preview_undo(&source).unwrap();
    assert_eq!(
        preview.lifecycle,
        vec![
            OperationState::Idle,
            OperationState::Previewing,
            OperationState::AwaitingConfirmation
        ]
    );

    let undo = execute_synthetic_undo(&source, &preview.plan).unwrap();
    let execution = undo.undo_execution_result.unwrap();
    assert_eq!(
        execution.lifecycle,
        vec![
            OperationState::Idle,
            OperationState::Previewing,
            OperationState::AwaitingConfirmation,
            OperationState::AwaitingElevation,
            OperationState::Running,
            OperationState::Verifying,
            OperationState::Succeeded,
            OperationState::NotReversible,
        ]
    );
    assert_eq!(
        execution.result.outcomes[0].after,
        source.captured_original_states.first().and_then(|capture| {
            if let CapturedOriginalState::Synthetic { state, .. } = capture {
                Some(state.clone())
            } else {
                None
            }
        })
    );
}

#[test]
fn interrupted_operations_remain_visible_with_diagnostics() {
    let (_directory, journal) = journal();
    let mut record = synthetic_operation_record().unwrap();
    record.terminal_state = OperationState::Interrupted;
    record.diagnostic = Some(OperationError {
        code: OperationErrorCode::OperationInterrupted,
        message: "Synthetic interruption after one target.".into(),
        target_id: Some("synthetic.changed".into()),
        retryable: true,
    });

    journal.append(&record).unwrap();
    let listed = journal.list().unwrap();
    assert_eq!(listed[0].terminal_state, OperationState::Interrupted);
    assert_eq!(
        listed[0].diagnostic.as_ref().unwrap().code,
        OperationErrorCode::OperationInterrupted
    );
}

/// A plan that passes the mutating-operation safety gates so confirmation and
/// execution-start behavior can be exercised.
fn confirming_mutating_plan() -> OperationPlan {
    let mut plan = synthetic_operation_record().unwrap().confirmed_plan;
    plan.descriptor.mutates_system = true;
    plan.descriptor.requires_confirmation = true;
    plan.descriptor.requires_revalidation = true;
    plan.descriptor.requires_verification = true;
    plan.descriptor.requires_journal = true;
    plan
}

#[test]
fn durable_confirmation_accepts_the_current_digest_record() {
    let (_directory, journal) = journal();
    let plan = confirming_mutating_plan();

    journal.record_confirmation(&plan).unwrap();

    assert!(journal.has_durable_confirmation(&plan).unwrap());
}

#[test]
fn durable_confirmation_accepts_a_legacy_serialized_plan_record() {
    let (_directory, journal) = journal();
    let plan = confirming_mutating_plan();

    journal
        .record_execution_state(&plan, OperationExecutionState::Confirmed)
        .unwrap();

    assert!(journal.has_durable_confirmation(&plan).unwrap());
    journal.begin_execution(&plan).unwrap();
    assert!(matches!(
        journal.latest_execution_state(&plan.id).unwrap(),
        Some(OperationExecutionState::ExecutionStarting | OperationExecutionState::Executing)
    ));
}

#[test]
fn durable_confirmation_rejects_a_legacy_plan_with_changed_targets() {
    let (_directory, journal) = journal();
    let plan = confirming_mutating_plan();
    journal
        .record_execution_state(&plan, OperationExecutionState::Confirmed)
        .unwrap();

    let mut changed = plan.clone();
    changed.targets[0].requested_action.push_str("_modified");

    assert!(!journal.has_durable_confirmation(&changed).unwrap());
}

#[test]
fn durable_confirmation_rejects_a_malformed_legacy_payload() {
    let (_directory, journal) = journal();
    let plan = confirming_mutating_plan();
    {
        let mut conn = journal.conn.lock().unwrap();
        let transaction = conn.transaction().unwrap();
        insert_execution_event(
            &transaction,
            &plan.id,
            OperationExecutionState::Confirmed,
            "{not a serialized plan",
        )
        .unwrap();
        transaction.commit().unwrap();
    }

    assert!(!journal.has_durable_confirmation(&plan).unwrap());
}

#[test]
fn durable_confirmation_is_bound_to_the_operation() {
    let (_directory, journal) = journal();
    let plan = confirming_mutating_plan();
    journal.record_confirmation(&plan).unwrap();

    let mut other = plan.clone();
    other.id = OperationId("another-operation".into());

    assert!(!journal.has_durable_confirmation(&other).unwrap());
}

#[test]
fn journal_schema_is_versioned_without_replacing_existing_records() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("operations.db");
    let journal = OperationJournal::open_at(&path).unwrap();
    let record = synthetic_operation_record().unwrap();
    journal.append(&record).unwrap();
    assert_eq!(journal.schema_version().unwrap(), 3);
    drop(journal);

    let reopened = OperationJournal::open_at(&path).unwrap();
    assert_eq!(reopened.schema_version().unwrap(), 3);
    assert_eq!(reopened.list().unwrap(), vec![record]);
}

#[test]
fn retry_relationship_is_persisted_in_its_own_table() {
    let (_directory, journal) = journal();
    let mut record = synthetic_operation_record().unwrap();
    record.confirmed_plan.parent_operation_id = Some(OperationId("retry-source".into()));
    record.confirmed_plan.retry_of_operation_id = Some(OperationId("retry-source".into()));
    journal.append(&record).unwrap();

    assert_eq!(
        journal.list_relationships().unwrap(),
        vec![OperationRelationship {
            operation_id: record.operation_id,
            parent_operation_id: Some(OperationId("retry-source".into())),
            retry_of_operation_id: Some(OperationId("retry-source".into())),
        }]
    );
}
