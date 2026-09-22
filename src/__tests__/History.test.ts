import { flushPromises, mount } from '@vue/test-utils'
import { describe, expect, it, vi } from 'vitest'
import History from '../views/History.vue'
import * as operationApi from '../api/operation'

const operationMocks = vi.hoisted(() => ({
  getOperationJournal: vi.fn(),
  getUnresolvedOperations: vi.fn(),
  createRetryPlan: vi.fn(),
  previewOperationUndo: vi.fn(),
  executeOperationUndo: vi.fn(),
}))

vi.mock('../api/tauri', () => ({
  getUndoLog: vi.fn().mockResolvedValue([
    {
      id: 'legacy-1',
      session_id: 'legacy-session',
      kind: 'registry_change',
      target: 'HKCU\\Software\\Legacy',
      reversal_command: '{}',
      original_size_bytes: null,
      created_at: '2026-07-27T11:00:00Z',
      rolled_back_at: null,
    },
  ]),
}))

const plan = {
  id: 'synthetic.operation-kernel',
  descriptor: {
    id: 'synthetic.operation-kernel',
    title: 'Synthetic journal operation',
    category: 'other' as const,
    risk: 'low' as const,
    privilege: 'standard_user' as const,
    reversibility: 'fully_reversible' as const,
    reboot_policy: 'not_required' as const,
    experimental: true,
    proof_status: 'proven' as const,
    exposure: 'internal' as const,
    recovery: 'none' as const,
    capability_id: 'operation.kernel',
    mutates_system: false,
    requires_confirmation: false,
    requires_revalidation: false,
    requires_verification: false,
    requires_journal: false,
  },
  targets: [
    { target_id: 'changed', requested_action: 'set', description: 'Changed target' },
    { target_id: 'skipped', requested_action: 'set', description: 'Skipped target' },
    { target_id: 'failed', requested_action: 'set', description: 'Failed target' },
  ],
  warnings: [],
  expected_effects: [],
  requires_elevation: false,
  requires_restart: false,
  reversibility: 'fully_reversible' as const,
}

const record: operationApi.OperationJournalRecord = {
  journal_id: 'journal-1',
  record_kind: 'operation',
  related_record_id: null,
  operation_id: plan.id,
  operation_schema_version: 1,
  application_version: '1.0.0',
  timestamp: '2026-07-27T12:00:00Z',
  terminal_state: 'interrupted',
  confirmed_plan: plan,
  requested_targets: plan.targets,
  captured_original_states: [{ kind: 'synthetic', target_id: 'changed' }],
  target_outcomes: [
    {
      target_id: 'changed',
      requested_action: 'set',
      before: { summary: 'before', properties: {} },
      after: { summary: 'after', properties: {} },
      execution: 'changed',
      verification: 'verified',
      error: null,
    },
    {
      target_id: 'skipped',
      requested_action: 'set',
      before: null,
      after: null,
      execution: 'skipped',
      verification: 'not_applicable',
      error: null,
    },
    {
      target_id: 'failed',
      requested_action: 'set',
      before: null,
      after: null,
      execution: 'failed',
      verification: 'failed',
      error: {
        code: 'operation_interrupted',
        message: 'Interrupted.',
        target_id: 'failed',
        retryable: true,
      },
    },
  ],
  verification_results: [{ target_id: 'changed', status: 'verified', message: 'Verified' }],
  restart_requirement: 'not_required',
  undo_eligibility: {
    eligible: true,
    reason: 'Captured state is tested.',
    automated_restoration_test: 'operation_journal_tests::undo_restores_captured_original_state',
  },
  undo_execution_result: null,
  diagnostic: {
    code: 'operation_interrupted',
    message: 'Interrupted after one target.',
    target_id: 'failed',
    retryable: true,
  },
}

vi.mock('../api/operation', async importOriginal => {
  const original = await importOriginal<typeof import('../api/operation')>()
  return {
    ...original,
    ...operationMocks,
  }
})

operationMocks.getOperationJournal.mockResolvedValue([record])
operationMocks.getUnresolvedOperations.mockResolvedValue([])
operationMocks.createRetryPlan.mockResolvedValue(plan)
operationMocks.previewOperationUndo.mockResolvedValue({
  source_record_id: record.journal_id,
  plan,
  lifecycle: ['idle', 'previewing', 'awaiting_confirmation'],
})

describe('History operation journal', () => {
  it('shows reconciled target statuses, interruption diagnostics, and hides legacy undo', async () => {
    const wrapper = mount(History, {
      shallow: true,
      global: {
        stubs: {
          AppHeader: { template: '<header><slot name="actions" /></header>' },
          BaseCard: { template: '<article><slot /></article>' },
          BaseBadge: { template: '<span><slot /></span>' },
          BaseButton: { template: '<button><slot /></button>' },
          EmptyState: true,
          ResultsBanner: true,
          ConfirmDialog: true,
        },
      },
    })
    await wrapper.find('button').trigger('click')
    await flushPromises()

    const text = wrapper.text()
    expect(text).toContain('Requested 3')
    expect(text).toContain('Changed 1')
    expect(text).toContain('Skipped 1')
    expect(text).toContain('Failed 1')
    expect(text).toContain('Verified 1')
    expect(text).toContain('Risk low')
    expect(text).toContain('Reboot not required')
    expect(text).toContain('Confirmed plansynthetic.operation-kernel')
    expect(text).toContain('Original state: before')
    expect(text).toContain('Resulting state: after')
    expect(text).toContain('Verification methodVerified')
    expect(text).not.toContain('Post-operation target state comparison')
    expect(text).toContain('Error: Interrupted.')
    expect(text).toContain('Undo historyNo undo recorded')
    expect(text).toContain('Captured original stateschanged')
    expect(text).toContain('LogsNo operation warnings recorded.')
    expect(text).toContain('operation interrupted: Interrupted after one target.')
    expect(text).toContain('Legacy record — undo eligibility unverified')
  })
})
