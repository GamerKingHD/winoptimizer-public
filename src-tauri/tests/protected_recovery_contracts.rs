use tempfile::tempdir;
use winoptimizer_lib::core::operation::RecoveryLevel;
use winoptimizer_lib::core::operation_journal::{
    execute_confirmed_synthetic_operation, preview_undo, OperationJournal, UndoEligibility,
};

#[test]
fn journal_eligibility_is_explicit_and_survives_validated_readback() {
    let contract = winoptimizer_lib::core::operation::synthetic_operation_contract().unwrap();
    let record = execute_confirmed_synthetic_operation(&contract.plan).unwrap();
    assert!(record.ui_undo_available());

    let directory = tempdir().unwrap();
    let journal = OperationJournal::open_at(&directory.path().join("operations.db")).unwrap();
    journal.append(&record).unwrap();
    assert!(journal.list().unwrap()[0].ui_undo_available());

    let mut backup_only = record.clone();
    backup_only.undo_eligibility =
        UndoEligibility::unavailable("A captured backup exists, but exact recovery is not proven.");
    assert!(!backup_only.ui_undo_available());
    assert!(preview_undo(&backup_only).is_err());

    let mut blank_test_id = record.clone();
    blank_test_id.undo_eligibility.automated_restoration_test = Some(" ".into());
    assert!(!blank_test_id.ui_undo_available());
    assert!(blank_test_id.validate().is_err());

    let mut best_effort = record;
    best_effort.confirmed_plan.descriptor.recovery = RecoveryLevel::BestEffort;
    assert!(best_effort.validate().is_err());
}
