import { afterEach, describe, expect, it } from 'vitest'
import { clearMocks, mockIPC } from '@tauri-apps/api/mocks'

import {
  executeOperationUndo,
  getOperationJournal,
  getOperationKernelContract,
  previewOperationUndo,
  type OperationDescriptor,
  type OperationJournalRecord,
  type SyntheticOperationContract,
} from '../api/operation'

const descriptor: OperationDescriptor = {
  id: 'synthetic.operation-kernel',
  title: 'Synthetic operation kernel check',
  category: 'other',
  risk: 'low',
  privilege: 'administrator',
  reversibility: 'fully_reversible',
  reboot_policy: 'not_required',
  experimental: true,
  proof_status: 'experimental',
  exposure: 'labs',
  recovery: 'best_effort',
  capability_id: 'operation.synthetic.operation-kernel',
  mutates_system: true,
  requires_confirmation: true,
  requires_revalidation: true,
  requires_verification: true,
  requires_journal: true,
}

const contract: SyntheticOperationContract = {
  descriptor,
  plan: {
    id: 'synthetic.operation-kernel',
    descriptor,
    targets: [],
    warnings: [],
    expected_effects: [],
    requires_elevation: true,
    requires_restart: false,
    reversibility: 'fully_reversible',
  },
  traces: [
    [
      'idle',
      'previewing',
      'awaiting_confirmation',
      'awaiting_elevation',
      'running',
      'verifying',
      'succeeded',
      'undo_available',
    ],
  ],
  result: {
    operation_id: 'synthetic.operation-kernel',
    requested: 2,
    changed: 1,
    unchanged: 0,
    skipped: 0,
    failed: 1,
    verified: 1,
    unverified: 1,
    outcomes: [
      {
        target_id: 'synthetic.changed',
        requested_action: 'set',
        before: { summary: 'before', properties: { value: '0' } },
        after: { summary: 'after', properties: { value: '1' } },
        execution: 'changed',
        verification: 'verified',
        error: null,
      },
      {
        target_id: 'synthetic.failed',
        requested_action: 'set',
        before: null,
        after: null,
        execution: 'failed',
        verification: 'failed',
        error: {
          code: 'target_changed_since_preview',
          message: 'Synthetic target changed after preview.',
          target_id: 'synthetic.failed',
          retryable: true,
        },
      },
    ],
    reboot_required: false,
    undo: 'undo_available',
  },
  sample_error: {
    code: 'target_changed_since_preview',
    message: 'Synthetic target changed after preview.',
    target_id: 'synthetic.failed',
    retryable: true,
  },
}

const journalRecord: OperationJournalRecord = {
  journal_id: 'journal-1',
  record_kind: 'operation',
  related_record_id: null,
  operation_id: contract.plan.id,
  operation_schema_version: 1,
  application_version: '1.0.0',
  timestamp: '2026-07-27T12:00:00Z',
  terminal_state: 'partially_succeeded',
  confirmed_plan: contract.plan,
  requested_targets: contract.plan.targets,
  captured_original_states: [
    { kind: 'synthetic', target_id: 'synthetic.changed', state: contract.result.outcomes[0].before },
  ],
  target_outcomes: contract.result.outcomes,
  verification_results: contract.result.outcomes.map(target => ({
    target_id: target.target_id,
    status: target.verification,
    message: target.verification,
  })),
  restart_requirement: 'not_required',
  undo_eligibility: {
    eligible: true,
    reason: 'Captured original state has an automated restoration test.',
    automated_restoration_test: 'operation_journal_tests::undo_restores_captured_original_state',
  },
  undo_execution_result: null,
  diagnostic: contract.sample_error,
}

describe('operation kernel Tauri contract', () => {
  afterEach(() => clearMocks())

  it('preserves lifecycle, result, and structured error fields', async () => {
    let invokedCommand = ''
    mockIPC((command) => {
      invokedCommand = command
      return contract
    })

    const received = await getOperationKernelContract()

    expect(invokedCommand).toBe('get_operation_kernel_contract')
    expect(received.result.failed).toBe(1)
    expect(received.sample_error).toEqual({
      code: 'target_changed_since_preview',
      message: 'Synthetic target changed after preview.',
      target_id: 'synthetic.failed',
      retryable: true,
    })
  })

  it('preserves a structured rejected command error', async () => {
    mockIPC(() => Promise.reject(contract.sample_error))

    await expect(getOperationKernelContract()).rejects.toEqual(contract.sample_error)
  })

  it('uses typed journal preview and confirmed undo commands', async () => {
    const commands: string[] = []
    mockIPC((command) => {
      commands.push(command)
      if (command === 'get_operation_journal') return [journalRecord]
      if (command === 'preview_operation_undo') {
        return {
          source_record_id: journalRecord.journal_id,
          plan: journalRecord.confirmed_plan,
          lifecycle: ['idle', 'previewing', 'awaiting_confirmation'],
        }
      }
      return { ...journalRecord, record_kind: 'undo', undo_eligibility: { eligible: false, reason: 'Undo records are not recursively reversible.', automated_restoration_test: null } }
    })

    const records = await getOperationJournal()
    const preview = await previewOperationUndo(records[0].journal_id)
    const undo = await executeOperationUndo(records[0].journal_id, preview.plan)

    expect(commands).toEqual([
      'get_operation_journal',
      'preview_operation_undo',
      'execute_operation_undo',
    ])
    expect(undo.record_kind).toBe('undo')
  })
})
