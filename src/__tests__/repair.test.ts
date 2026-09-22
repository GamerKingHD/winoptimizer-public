import { afterEach, describe, expect, it } from 'vitest'
import { clearMocks, mockIPC } from '@tauri-apps/api/mocks'

import type { OperationPlan } from '../api/operation'
import { createRepairPlan, executeRepairPlan, listRepairGuides, listRepairTargets } from '../api/repair'

const plan: OperationPlan = {
  id: 'repair-plan',
  descriptor: {
    id: 'repair.verified_diagnostic',
    title: 'Check Windows Update services',
    category: 'repair',
    risk: 'low',
    privilege: 'standard_user',
    reversibility: 'not_reversible',
    reboot_policy: 'not_required',
    experimental: false,
    proof_status: 'partially_proven',
    exposure: 'supporting',
    recovery: 'none',
    capability_id: 'repair.diagnostic',
    mutates_system: false,
    requires_confirmation: false,
    requires_revalidation: false,
    requires_verification: false,
    requires_journal: false,
  },
  targets: [
    {
      target_id: 'windows_update_status',
      requested_action: '{}',
      description: 'Query Windows Update status',
    },
  ],
  warnings: [],
  expected_effects: [],
  requires_elevation: false,
  requires_restart: false,
  reversibility: 'not_reversible',
}

describe('repair operation Tauri contract', () => {
  afterEach(() => clearMocks())

  it('uses canonical targets and executes only a verified diagnostic plan', async () => {
    const commands: string[] = []
    mockIPC((command) => {
      commands.push(command)
      if (command === 'list_repair_targets') {
        return [
          {
            id: 'windows_update_status',
            category: 'Windows Update',
            name: 'Check Windows Update services',
            description: 'Query status',
            command: 'sc',
            args: ['query', 'wuauserv'],
            estimated_minutes: 1,
            requires_admin: false,
            mutates_system: false,
            restart_required: false,
            risk: 'low',
            limitations: 'Read-only evidence only.',
            experimental: false,
            execution_available: true,
            verification_method: 'SCM returns identity and state.',
          },
        ]
      }
      if (command === 'list_repair_guides') {
        return [
          {
            id: 'windows_update',
            title: 'Windows Update',
            symptom: 'Windows Update is not starting or appears stuck',
            description: 'Check the Windows Update service.',
            evidence_action_ids: ['windows_update_status'],
            related_action_ids: ['rebuild_windows_update'],
            limitations: 'Does not rebuild the cache.',
          },
        ]
      }
      if (command === 'create_repair_plan') return plan
      return {
        result: {
          operation_id: plan.id,
          requested: 1,
          changed: 0,
          unchanged: 1,
          skipped: 0,
          failed: 0,
          verified: 1,
          unverified: 0,
          outcomes: [],
          reboot_required: false,
          undo: 'not_reversible',
        },
        log: {
          id: 'log',
          action_id: 'windows_update_status',
          command: 'sc',
          args: ['query', 'wuauserv'],
          started_at: 'start',
          ended_at: 'end',
          exit_code: 0,
          stdout: 'SERVICE_NAME wuauserv STATE RUNNING',
          stderr: '',
          requires_admin: false,
          mutates_system: false,
          restart_required: false,
        },
        verification_message: 'SCM returns identity and state.',
      }
    })

    const targets = await listRepairTargets()
    const guides = await listRepairGuides()
    const confirmed = await createRepairPlan(targets[0].id)
    const execution = await executeRepairPlan(confirmed)

    expect(commands).toEqual([
      'list_repair_targets',
      'list_repair_guides',
      'create_repair_plan',
      'execute_repair_plan',
    ])
    expect(commands).not.toContain('list_repair_actions')
    expect(guides[0].evidence_action_ids).toEqual(['windows_update_status'])
    expect(guides[0].limitations).toContain('rebuild')
    expect(execution.result.verified).toBe(1)
  })
})
