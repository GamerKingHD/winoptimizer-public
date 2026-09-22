use crate::commands::repair::execute_repair_confirmed;
use crate::core::resumable_elevation::ElevationTicket;
use crate::*;
use std::sync::Arc;

// ============================================================
// Tauri Commands - Elevation
// ============================================================

#[tauri::command]
pub(crate) fn check_elevated(state: tauri::State<'_, Arc<AppState>>) -> bool {
    state.adapters.elevation.is_elevated()
}

#[tauri::command]
pub(crate) fn has_resume_operation_args() -> bool {
    has_resume_operation_args_from(std::env::args())
}

pub(crate) fn has_resume_operation_args_from<I>(args: I) -> bool
where
    I: IntoIterator<Item = String>,
{
    args.into_iter()
        .any(|argument| argument == "--resume-operation")
}

// The decision is built once per launch and carries an owned plan; boxing it
// would only move the allocation without reducing total work.
#[derive(Debug)]
#[allow(clippy::large_enum_variant)]
pub(crate) enum ResumeStartupDecision {
    Noop,
    Execute(ElevationTicket, core::elevation_broker::BrokerSession),
}

pub(crate) fn resume_startup_decision(
    args: &[String],
    elevated: bool,
) -> std::result::Result<ResumeStartupDecision, core::operation::OperationError> {
    let Some((ticket, broker_session)) = resumed_arguments(args)? else {
        return Ok(ResumeStartupDecision::Noop);
    };
    if !elevated {
        return Err(core::operation::OperationError {
            code: core::operation::OperationErrorCode::ElevationRequired,
            message: "resumed operations require an elevated process".into(),
            target_id: None,
            retryable: true,
        });
    }
    Ok(ResumeStartupDecision::Execute(ticket, broker_session))
}

#[tauri::command]
pub(crate) async fn confirm_and_request_elevation(
    state: tauri::State<'_, Arc<AppState>>,
    confirmed_plan: core::operation::OperationPlan,
) -> std::result::Result<ElevationTicket, core::operation::ContextualOperationError> {
    let error_context = core::operation::OperationErrorContext::for_plan(&confirmed_plan);
    if !confirmed_plan.requires_elevation {
        return Err(core::operation::OperationError::validation(
            "the confirmed plan does not require elevation",
        )
        .with_context(error_context.clone()));
    }
    confirmed_plan
        .descriptor
        .validate_contract()
        .map_err(|error| error.with_context(error_context.clone()))?;
    confirmed_plan
        .validate_relationships()
        .map_err(|error| error.with_context(error_context.clone()))?;
    if state.runtime.is_fixture() {
        return Err(core::operation::OperationError::validation(
            "elevation and broker requests are unavailable in fixture runtime",
        )
        .with_context(error_context));
    }
    let journal = state
        .ensure_operation_journal()
        .map_err(|error| error.with_context(error_context.clone()))?;
    journal
        .record_confirmation(&confirmed_plan)
        .map_err(|error| error.with_context(error_context.clone()))?;
    let confirmed_plans = state.runtime.paths.confirmed_plans();
    tauri::async_runtime::spawn_blocking(move || {
        let ticket = core::resumable_elevation::store_confirmed_plan_at_path(
            &confirmed_plan,
            &confirmed_plans,
        )?;
        let broker_session = core::elevation_broker::BrokerSession::create(
            &confirmed_plan,
            &ticket.plan_operation_id,
            &ticket.nonce,
            &ticket.plan_digest,
        )
        .map_err(|error| core::operation::OperationError::validation(error.to_string()))?;
        core::elevation_broker::start_server(broker_session.clone())
            .map_err(|error| core::operation::OperationError::validation(error.to_string()))?;
        core::elevation::request_resumable_elevation(&ticket, &broker_session).map_err(
            |error| core::operation::OperationError {
                code: core::operation::OperationErrorCode::OperationCancelled,
                message: error.to_string(),
                target_id: Some(confirmed_plan.id.0),
                retryable: true,
            },
        )?;
        Ok(ticket)
    })
    .await
    .map_err(|error| {
        core::operation::OperationError {
            code: core::operation::OperationErrorCode::InternalError,
            message: error.to_string(),
            target_id: None,
            retryable: true,
        }
        .with_context(error_context.clone())
    })?
    .map_err(|error: core::operation::OperationError| error.with_context(error_context))
}

#[tauri::command]
pub(crate) async fn execute_resumed_operation(
    state: tauri::State<'_, Arc<AppState>>,
) -> std::result::Result<Option<OperationJournalRecord>, core::operation::ContextualOperationError>
{
    execute_resumed_operation_with_state(state.inner().clone()).await
}

pub(crate) async fn execute_resumed_operation_with_state(
    state: Arc<AppState>,
) -> std::result::Result<Option<OperationJournalRecord>, core::operation::ContextualOperationError>
{
    if state.runtime.is_fixture() {
        return Err(core::operation::OperationError::validation(
            "elevated resume is unavailable in fixture runtime",
        )
        .with_context(core::operation::OperationErrorContext::default()));
    }
    let args = std::env::args().collect::<Vec<_>>();
    let (ticket, broker_session) =
        match resume_startup_decision(&args, state.adapters.elevation.is_elevated()).map_err(
            |error| error.with_context(core::operation::OperationErrorContext::default()),
        )? {
            ResumeStartupDecision::Noop => return Ok(None),
            ResumeStartupDecision::Execute(ticket, broker_session) => (ticket, broker_session),
        };
    let confirmed_plans = state.runtime.paths.confirmed_plans();
    let plan = core::resumable_elevation::load_confirmed_plan_at_path(&ticket, &confirmed_plans)
        .map_err(|error| error.with_context(core::operation::OperationErrorContext::default()))?;
    let error_context = core::operation::OperationErrorContext::for_plan(&plan);
    core::elevation_broker::validate_plan_binding(&broker_session, &plan)
        .map_err(broker_error)
        .map_err(|error| error.with_context(error_context.clone()))?;
    let journal = state
        .ensure_operation_journal()
        .map_err(|error| error.with_context(error_context.clone()))?;
    let safe_paths = state.safe_paths.clone();
    let needs_startup_recovery_store = matches!(
        plan.descriptor.id.0.as_str(),
        "startup.disable_program" | "startup.enable_program"
    );
    let startup_recovery = if needs_startup_recovery_store {
        Some(
            state
                .ensure_startup_recovery()
                .map_err(|error| error.with_context(error_context.clone()))?
                .clone(),
        )
    } else {
        None
    };
    let session_id = state.session_id.clone();
    let backup_mgr = state.backup_mgr.clone();
    let confirmed_plans = state.runtime.paths.confirmed_plans();
    let broker_operation_id = broker_session.operation_id.clone();
    let broker_plan_digest = broker_session.plan_digest.clone();
    let execution_journal = journal.clone();
    let settings_path = state.runtime.paths.settings.clone();
    let invalidates_startup_program_cache = matches!(
        plan.descriptor.id.0.as_str(),
        "startup.disable_program" | "startup.enable_program"
    );
    let invalidates_services_cache = matches!(
        plan.descriptor.id.0.as_str(),
        "startup.service_change" | "startup.services_set_manual" | "startup.services_guidance"
    );
    let invalidates_tasks_cache = matches!(
        plan.descriptor.id.0.as_str(),
        "startup.scheduled_task_change" | "startup.scheduled_tasks_guidance"
    );
    let adapters = state.adapters.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        let mut broker_client =
            core::elevation_broker::connect(broker_session).map_err(broker_error)?;
        core::resumable_elevation::claim_confirmed_plan_at_path(&ticket, &confirmed_plans)?;
        if plan.descriptor.category == core::operation::OperationCategory::SystemTweak
            || plan.descriptor.id.0.starts_with("preset.")
        {
            core::system_tweak_operation::ensure_labs_enabled(&plan, &settings_path)?;
        }
        journal.begin_execution(&plan)?;
        let is_cleanup = plan.descriptor.category == core::operation::OperationCategory::Cleanup;
        let retry_of_operation_id = plan.retry_of_operation_id.clone();

        let execution: std::result::Result<
            OperationJournalRecord,
            core::operation::OperationError,
        > = (|| {
            let record = if let Some(source_id) = plan.descriptor.id.0.strip_prefix("undo.") {
                let source = journal.get(source_id)?;
                execute_captured_undo_confirmed(&source, &plan, &journal, &adapters)?
            } else if plan.descriptor.category == core::operation::OperationCategory::Cleanup {
                let fresh_scan = core::cleanup_operation::scan(&safe_paths);
                let (allowed_target_ids, _) =
                    core::cleanup_operation::validate_authoritative_scan(&plan, &fresh_scan)?;
                let (_, record) = core::cleanup_operation::execute_with_recycle_bin_journaled(
                    &plan,
                    &AtomicBool::new(false),
                    &allowed_target_ids,
                    &execution_journal,
                    &safe_paths,
                )?;
                record
            } else if plan.descriptor.id.0 == "startup.disable_program" {
                let target_id = plan
                    .targets
                    .first()
                    .map(|target| target.target_id.clone())
                    .ok_or_else(|| {
                        core::operation::OperationError::validation("startup plan has no target")
                    })?;
                let current = match adapters.startup_programs.find_by_id(&target_id) {
                    Ok(current) => current,
                    Err(error) => {
                        let operation_error = core::operation::OperationError {
                            code: core::operation::OperationErrorCode::TargetNotFound,
                            message: error.to_string(),
                            target_id: Some(target_id.clone()),
                            retryable: true,
                        };
                        core::startup_operation::mark_target_revalidation_failed(
                            &journal,
                            &plan,
                            &target_id,
                        )?;
                        return Err(operation_error);
                    }
                };
                if let Err(error) = core::startup_operation::validate_current(&plan, &current) {
                    core::startup_operation::mark_target_revalidation_failed(
                        &journal,
                        &plan,
                        &target_id,
                    )?;
                    return Err(error);
                }
                core::startup_operation::mark_target_ready_and_mutation_started(
                    &journal,
                    &plan,
                    &target_id,
                )?;
                let recovery_store = startup_recovery.clone().ok_or_else(|| {
                    core::operation::OperationError::validation("startup disable requires the recovery store")
                })?;
                let mutation = adapters.startup_programs.disable(
                    &current,
                    &recovery_store,
                    &session_id,
                    &backup_mgr,
                )
                    .map_err(|error| core::operation::OperationError {
                        code: core::operation::OperationErrorCode::ExecutionFailed,
                        message: error.to_string(),
                        target_id: Some(target_id.clone()),
                        retryable: true,
                    });
                let mutation = match mutation {
                    Ok(mutation) => mutation,
                    Err(error) => {
                        let failure = core::startup_operation::failed_outcome(
                            &current,
                            "disable",
                            error.clone(),
                        );
                        core::startup_operation::record_target_outcome(
                            &journal,
                            &plan,
                            &failure,
                        )?;
                        return Err(error);
                    }
                };
                let execution = match core::startup_operation::finish(&plan, &current, mutation) {
                    Ok(execution) => execution,
                    Err(error) => {
                        let payload = serde_json::json!({
                            "error": error.message,
                        })
                        .to_string();
                        journal.record_target_state(
                            &plan.id,
                            &target_id,
                            core::operation::TargetExecutionState::OutcomeUnknown,
                            &payload,
                        )?;
                        return Err(error);
                    }
                };
                for outcome in &execution.result.outcomes {
                    core::startup_operation::record_target_outcome(&journal, &plan, outcome)?;
                }
                core::startup_operation::journal_record(&plan, &current, &execution)
            } else if plan.descriptor.id.0 == "startup.enable_program" {
                let recovery_store = startup_recovery.clone().ok_or_else(|| {
                    core::operation::OperationError::validation("startup enable requires the recovery store")
                })?;
                let (_, record) = crate::commands::startup::execute_startup_program_enable_confirmed_with_journal(
                    plan,
                    recovery_store,
                    backup_mgr.clone(),
                    journal.clone(),
                    adapters.startup_programs.clone(),
                )?;
                record
            } else if plan.descriptor.id.0 == "startup.service_change" {
                let (_, record) = crate::commands::startup::execute_service_confirmed_with_journal(
                    plan,
                    journal.clone(),
                    adapters.services.clone(),
                )?;
                record
            } else if plan.descriptor.id.0 == "startup.services_set_manual" {
                let (_, record) = crate::commands::startup::execute_services_manual_confirmed_with_journal(
                    plan,
                    journal.clone(),
                    adapters.services.clone(),
                )?;
                record
            } else if plan.descriptor.id.0 == "startup.services_guidance" {
                let (_, record) = crate::commands::startup::execute_services_guidance_confirmed_with_journal(
                    plan,
                    journal.clone(),
                    adapters.services.clone(),
                )?;
                record
            } else if plan.descriptor.id.0 == "startup.scheduled_task_change" {
                let (_, record) = crate::commands::startup::execute_scheduled_task_confirmed_with_journal(
                    plan,
                    journal.clone(),
                    adapters.tasks.clone(),
                )?;
                record
            } else if plan.descriptor.id.0 == "startup.scheduled_tasks_guidance" {
                let (_, record) = crate::commands::startup::execute_scheduled_tasks_guidance_confirmed_with_journal(
                    plan,
                    journal.clone(),
                    adapters.tasks.clone(),
                )?;
                record
            } else if plan.descriptor.id.0 == "system_tweak.change" {
                let plan_operation_id = plan.id.clone();
                let (_, record) = crate::commands::system_tweaks::execute_system_tweak_confirmed_with_journal(
                    plan,
                    journal.clone(),
                    plan_operation_id,
                )?;
                record
            } else if plan.descriptor.id.0.starts_with("preset.") {
                let (_, record) = crate::commands::system_tweaks::execute_tweak_preset_confirmed_with_journal(
                    plan,
                    journal.clone(),
                )?;
                record
            } else if matches!(
                plan.descriptor.id.0.as_str(),
                "repair.verified_diagnostic" | "repair.maintenance"
            ) {
                let (_, record) = execute_repair_confirmed(
                    plan,
                    journal.clone(),
                    core::bounded_process::RunControl::unbounded_cancel(),
                )?;
                record
            } else if plan.descriptor.id.0 == "appx.policy_change" {
                let (_, record) = crate::commands::appx::execute_appx_confirmed_with_journal(
                    plan,
                    journal.clone(),
                )?;
                record
            } else if plan.descriptor.id.0 == "hosts.save" {
                let (_, record) = crate::commands::hosts::execute_hosts_confirmed_with_journal(
                    plan,
                    &journal,
                )?;
                record
            } else if plan.descriptor.id.0 == "environment.change" {
                let (_, record) = crate::commands::environment::execute_env_confirmed_with_journal(
                    plan,
                    &journal,
                    adapters.registry.as_ref(),
                )?;
                record
            } else if plan.descriptor.id.0 == "process.terminate" {
                let (_, record) = crate::commands::processes::execute_process_confirmed_with_journal(
                    plan,
                    journal.clone(),
                    adapters.processes.clone(),
                )?;
                record
            } else {
                core::operation_journal::execute_confirmed_synthetic_operation(&plan)?
            };
            journal.append(&record)?;
            if is_cleanup {
                if let Some(retry_of_operation_id) = retry_of_operation_id.as_ref() {
                    if let Some(previous) = journal
                        .list_unresolved_operations()?
                        .into_iter()
                        .find(|operation| operation.operation_id == *retry_of_operation_id)
                    {
                        let recovery_scan = core::cleanup_operation::scan(&safe_paths);
                        let reconciliation = core::cleanup_operation::reconciliation_record(
                            &previous.confirmed_plan,
                            &recovery_scan,
                        )?;
                        journal.append(&reconciliation)?;
                    }
                }
            }
            Ok(record)
        })();

        let broker_outcome = match &execution {
            Ok(record) => core::elevation_broker::BrokerOutcome {
                operation_id: broker_operation_id,
                plan_digest: broker_plan_digest,
                status: broker_status(record.terminal_state),
                error: None,
            },
            Err(error) => core::elevation_broker::BrokerOutcome {
                operation_id: broker_operation_id,
                plan_digest: broker_plan_digest,
                status: core::elevation_broker::BrokerOutcomeStatus::Failed,
                error: Some(core::elevation_broker::BrokerError {
                    code: core::elevation_broker::BrokerErrorCode::Execution,
                    phase: "execute".into(),
                    message: error.to_string(),
                    retryable: error.retryable,
                    native_error_code: None,
                }),
            },
        };
        let broker_completion = broker_client.complete(broker_outcome);
        finish_resumed_execution(execution, broker_completion)
    })
    .await
    .map_err(|error| {
        core::operation::OperationError {
            code: core::operation::OperationErrorCode::InternalError,
            message: error.to_string(),
            target_id: None,
            retryable: true,
        }
        .with_context(error_context.clone())
    })?
    .map_err(|error: core::operation::OperationError| error.with_context(error_context))?;
    if invalidates_startup_program_cache {
        state.cache.invalidate("list_startup_programs");
    }
    if invalidates_services_cache {
        state.cache.invalidate("list_services");
    }
    if invalidates_tasks_cache {
        state.cache.invalidate("list_startup_tasks");
    }
    Ok(result)
}

fn finish_resumed_execution<T>(
    execution: std::result::Result<T, core::operation::OperationError>,
    broker_completion: std::result::Result<(), core::elevation_broker::BrokerError>,
) -> std::result::Result<Option<T>, core::operation::OperationError> {
    if let Err(error) = broker_completion {
        return Err(core::operation::OperationError {
            code: core::operation::OperationErrorCode::OperationInterrupted,
            message: format!(
                "resumed operation outcome was not acknowledged by the broker: {error}"
            ),
            target_id: None,
            retryable: true,
        });
    }
    execution.map(Some)
}

fn argument_value(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|argument| argument == name)
        .and_then(|position| args.get(position + 1))
        .cloned()
}

fn resumed_arguments(
    args: &[String],
) -> std::result::Result<
    Option<(ElevationTicket, core::elevation_broker::BrokerSession)>,
    core::operation::OperationError,
> {
    let Some(operation_id) = argument_value(args, "--resume-operation") else {
        return Ok(None);
    };
    let has_command_line_secrets = [
        argument_value(args, "--nonce"),
        argument_value(args, "--resume-plan-digest"),
        argument_value(args, "--ticket-authenticator"),
        argument_value(args, "--broker-pipe"),
    ]
    .iter()
    .any(Option::is_some);
    if !has_command_line_secrets {
        return core::resumable_elevation::load_elevation_handoff(&operation_id)
            .map(Some)
            .map_err(|error| core::operation::OperationError::validation(error.to_string()));
    }
    let values = [
        argument_value(args, "--resume-plan-operation"),
        argument_value(args, "--nonce"),
        argument_value(args, "--resume-plan-digest"),
        argument_value(args, "--ticket-authenticator"),
        argument_value(args, "--broker-pipe"),
        argument_value(args, "--broker-session"),
        argument_value(args, "--broker-expires"),
    ];
    let [Some(plan_operation_id), Some(nonce), Some(plan_digest), Some(authenticator), Some(endpoint), Some(session_id), Some(expires_at)] =
        values
    else {
        return Err(core::operation::OperationError::validation(
            "incomplete authenticated elevation broker arguments",
        ));
    };
    let ticket = ElevationTicket {
        operation_id,
        plan_operation_id: plan_operation_id.clone(),
        nonce: nonce.clone(),
        plan_digest: plan_digest.clone(),
        authenticator,
    };
    let session = core::elevation_broker::BrokerSession::from_parts(
        endpoint,
        session_id,
        nonce,
        plan_operation_id,
        plan_digest,
        expires_at,
    )
    .map_err(broker_error)?;
    Ok(Some((ticket, session)))
}

fn broker_error(error: core::elevation_broker::BrokerError) -> core::operation::OperationError {
    let code = match error.code {
        core::elevation_broker::BrokerErrorCode::EndpointUnauthorized => {
            core::operation::OperationErrorCode::PermissionDenied
        }
        core::elevation_broker::BrokerErrorCode::Transport => {
            core::operation::OperationErrorCode::OperationCancelled
        }
        core::elevation_broker::BrokerErrorCode::Execution => {
            core::operation::OperationErrorCode::ExecutionFailed
        }
        core::elevation_broker::BrokerErrorCode::Internal => {
            core::operation::OperationErrorCode::InternalError
        }
        _ => core::operation::OperationErrorCode::ValidationFailed,
    };
    core::operation::OperationError {
        code,
        message: error.to_string(),
        target_id: None,
        retryable: error.retryable,
    }
}

fn broker_status(
    state: core::operation::OperationState,
) -> core::elevation_broker::BrokerOutcomeStatus {
    match state {
        core::operation::OperationState::Succeeded
        | core::operation::OperationState::UndoAvailable
        | core::operation::OperationState::NotReversible => {
            core::elevation_broker::BrokerOutcomeStatus::Succeeded
        }
        core::operation::OperationState::PartiallySucceeded => {
            core::elevation_broker::BrokerOutcomeStatus::PartiallySucceeded
        }
        core::operation::OperationState::Failed | core::operation::OperationState::Cancelled => {
            core::elevation_broker::BrokerOutcomeStatus::Failed
        }
        core::operation::OperationState::Interrupted => {
            core::elevation_broker::BrokerOutcomeStatus::OutcomeUnknown
        }
        _ => core::elevation_broker::BrokerOutcomeStatus::OutcomeUnknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unacknowledged_broker_outcome_is_not_reported_as_success() {
        let broker_error = core::elevation_broker::BrokerError {
            code: core::elevation_broker::BrokerErrorCode::Transport,
            phase: "receipt".into(),
            message: "connection closed".into(),
            retryable: true,
            native_error_code: None,
        };

        let error = finish_resumed_execution::<()>(Ok(()), Err(broker_error)).unwrap_err();

        assert_eq!(
            error.code,
            core::operation::OperationErrorCode::OperationInterrupted
        );
        assert!(error.message.contains("outcome was not acknowledged"));
    }

    #[test]
    fn acknowledged_broker_outcome_preserves_execution_result() {
        assert_eq!(finish_resumed_execution(Ok(42_u8), Ok(())), Ok(Some(42_u8)));
    }

    #[test]
    fn resumed_arguments_are_absent_without_resume_flag() {
        let parsed = resumed_arguments(&["winoptimizer.exe".into()]).unwrap();
        assert!(parsed.is_none());
    }

    #[test]
    fn no_resume_args_are_a_success_noop_without_elevation() {
        let decision = resume_startup_decision(&["winoptimizer.exe".into()], false).unwrap();
        assert!(matches!(decision, ResumeStartupDecision::Noop));
        assert!(!has_resume_operation_args_from(["winoptimizer.exe".into()]));
    }

    #[test]
    fn incomplete_resume_flags_are_rejected() {
        let error = resumed_arguments(&[
            "winoptimizer.exe".into(),
            "--resume-operation".into(),
            "11111111-1111-1111-1111-111111111111".into(),
            "--nonce".into(),
            "22222222-2222-2222-2222-222222222222".into(),
        ])
        .unwrap_err();
        assert!(error
            .message
            .contains("incomplete authenticated elevation broker arguments"));
    }
}
