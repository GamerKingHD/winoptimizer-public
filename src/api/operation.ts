import { invoke } from '@tauri-apps/api/core'

export type OperationState =
  | 'idle'
  | 'previewing'
  | 'awaiting_confirmation'
  | 'awaiting_elevation'
  | 'running'
  | 'verifying'
  | 'succeeded'
  | 'partially_succeeded'
  | 'failed'
  | 'undo_available'
  | 'not_reversible'
  | 'cancelled'
  | 'interrupted'

export type OperationErrorCode =
  | 'permission_denied'
  | 'elevation_required'
  | 'target_not_found'
  | 'target_changed_since_preview'
  | 'target_protected'
  | 'validation_failed'
  | 'operation_cancelled'
  | 'operation_interrupted'
  | 'execution_failed'
  | 'verification_failed'
  | 'partial_failure'
  | 'unsupported_windows_version'
  | 'reboot_required'
  | 'serialization_failed'
  | 'internal_error'

export type OperationErrorCategory =
  | 'authorization'
  | 'target'
  | 'validation'
  | 'cancellation'
  | 'execution'
  | 'verification'
  | 'platform'
  | 'serialization'
  | 'internal'

export type OperationErrorPhase =
  | 'planning'
  | 'revalidation'
  | 'execution'
  | 'verification'
  | 'cancellation'
  | 'unknown'

export interface OperationError {
  code: OperationErrorCode
  message: string
  target_id?: string | null
  retryable: boolean
  /** Optional during migration; current Rust payloads include every field. */
  category?: OperationErrorCategory
  operation_id?: string | null
  phase?: OperationErrorPhase
  native_error_code?: number | null
  human_message?: string
  requires_elevation?: boolean
  requires_restart?: boolean
  diagnostic_context?: Record<string, string> | null
}

export type OperationProofStatus = 'proven' | 'partially_proven' | 'experimental'
export type OperationExposure = 'core' | 'supporting' | 'advanced' | 'labs' | 'internal' | 'legacy'
export type OperationRecovery = 'exact' | 'best_effort' | 'none'
export type RestorePointPolicy = 'not_needed' | 'optional' | 'recommended' | 'required_if_available'

export interface OperationDescriptor {
  id: string
  title: string
  category:
    | 'cleanup'
    | 'startup'
    | 'system_tweak'
    | 'repair'
    | 'app_management'
    | 'system_configuration'
    | 'process_management'
    | 'maintenance'
    | 'other'
  risk: 'none' | 'low' | 'medium' | 'high' | 'critical'
  privilege: 'standard_user' | 'administrator'
  reversibility: 'fully_reversible' | 'best_effort' | 'not_reversible'
  reboot_policy: 'not_required' | 'may_be_required' | 'required'
  experimental: boolean
  proof_status: OperationProofStatus
  exposure: OperationExposure
  recovery: OperationRecovery
  restore_point_policy?: RestorePointPolicy
  capability_id: string
  mutates_system: boolean
  requires_confirmation: boolean
  requires_revalidation: boolean
  requires_verification: boolean
  requires_journal: boolean
}

export interface OperationPlan {
  id: string
  descriptor: OperationDescriptor
  targets: Array<{
    target_id: string
    requested_action: string
    description: string
  }>
  warnings: Array<{ code: string; message: string }>
  expected_effects: Array<{ target_id: string; description: string }>
  requires_elevation: boolean
  requires_restart: boolean
  reversibility: OperationDescriptor['reversibility']
  parent_operation_id?: string | null
  retry_of_operation_id?: string | null
}

export interface CapabilityDescriptor {
  id: string
  name: string
  exposure: OperationExposure
  proof: OperationProofStatus
  risk: OperationDescriptor['risk']
  recovery: OperationRecovery
  entitlement: 'free' | { paid: string }
  online_requirement: 'not_required' | 'optional' | 'required'
  architectures: { x64: boolean; arm64: boolean }
  route: string | null
  operation_ids: string[]
}

export const getCapabilityManifest = () =>
  invoke<CapabilityDescriptor[]>('get_capability_manifest')

export interface StateSnapshot {
  summary: string
  properties: Record<string, string>
}

export interface TargetOutcome {
  target_id: string
  requested_action: string
  before: StateSnapshot | null
  after: StateSnapshot | null
  execution: 'changed' | 'unchanged' | 'skipped' | 'failed'
  verification: 'verified' | 'unverified' | 'not_applicable' | 'failed'
  error: OperationError | null
}

export interface OperationResult {
  operation_id: string
  requested: number
  changed: number
  unchanged: number
  skipped: number
  failed: number
  verified: number
  unverified: number
  outcomes: TargetOutcome[]
  reboot_required: boolean
  undo: 'undo_available' | 'not_reversible'
}

export interface SyntheticOperationContract {
  descriptor: OperationDescriptor
  plan: OperationPlan
  traces: OperationState[][]
  result: OperationResult
  sample_error: OperationError
}

export interface OperationJournalRecord {
  journal_id: string
  record_kind: 'operation' | 'undo'
  related_record_id: string | null
  operation_id: string
  operation_schema_version: number
  application_version: string
  timestamp: string
  terminal_state: OperationState
  confirmed_plan: OperationPlan
  requested_targets: OperationPlan['targets']
  captured_original_states: Array<{ kind: string; target_id: string; [key: string]: unknown }>
  target_outcomes: TargetOutcome[]
  verification_results: Array<{
    target_id: string
    status: TargetOutcome['verification']
    message: string
  }>
  restart_requirement: OperationDescriptor['reboot_policy']
  undo_eligibility: {
    eligible: boolean
    reason: string
    automated_restoration_test: string | null
  }
  undo_execution_result: {
    source_record_id: string
    lifecycle: OperationState[]
    result: OperationResult
  } | null
  diagnostic: OperationError | null
}

export interface UnresolvedOperation {
  operation_id: string
  state: 'execution_starting' | 'executing' | 'mutation_completed' | 'verifying' | 'outcome_unknown'
  confirmed_plan: OperationPlan
  targets: Array<{
    target_id: string
    state: 'pending' | 'revalidation_failed' | 'ready' | 'mutation_started' | 'changed' | 'unchanged' | 'skipped' | 'verification_succeeded' | 'verification_failed' | 'failed' | 'outcome_unknown'
  }>
}

export interface SupportBundleOptions {
  redact_user_profile_path: boolean
  redact_network_addresses: boolean
  redact_hostnames: boolean
  include_exact_file_paths: boolean
  include_command_output: boolean
}

export interface UndoPreview {
  source_record_id: string
  plan: OperationPlan
  lifecycle: OperationState[]
}

export interface ElevationTicket {
  operation_id: string
  plan_operation_id: string
  nonce: string
  plan_digest: string
  authenticator: string
}

export const getOperationKernelContract = () =>
  invoke<SyntheticOperationContract>('get_operation_kernel_contract')

export const getOperationJournal = () =>
  invoke<OperationJournalRecord[]>('get_operation_journal')

export const getUnresolvedOperations = () =>
  invoke<UnresolvedOperation[]>('get_unresolved_operations')

export const createRetryPlan = (operationId: string, targetIds: string[]) =>
  invoke<OperationPlan>('create_retry_plan', { operationId, targetIds })

export const exportSupportBundle = (options: SupportBundleOptions) =>
  invoke<string>('export_support_bundle', { options })

export const previewOperationUndo = (journalId: string) =>
  invoke<UndoPreview>('preview_operation_undo', { journalId })

export const executeOperationUndo = (journalId: string, confirmedPlan: OperationPlan) =>
  invoke<OperationJournalRecord>('execute_operation_undo', { journalId, confirmedPlan })

export const confirmAndRequestElevation = (confirmedPlan: OperationPlan) =>
  invoke<ElevationTicket>('confirm_and_request_elevation', { confirmedPlan })

export const hasResumeOperationArgs = () =>
  invoke<boolean>('has_resume_operation_args')

export const executeResumedOperation = () =>
  invoke<OperationJournalRecord | null>('execute_resumed_operation')
