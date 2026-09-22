import { afterEach, describe, expect, it } from 'vitest'
import { clearMocks, mockIPC } from '@tauri-apps/api/mocks'

import type { OperationPlan } from '../api/operation'
import {
  createTweakPresetPlan,
  createSystemTweakPlan,
  executeTweakPresetPlan,
  executeSystemTweakPlan,
  listSystemTweakTargets,
  listTweakPresets,
} from '../api/systemTweaks'

const plan: OperationPlan = {
  id: 'tweak-plan-1',
  descriptor: {
    id: 'system_tweak.change',
    title: 'Enable Fixture',
    category: 'system_tweak',
    risk: 'low',
    privilege: 'standard_user',
    reversibility: 'fully_reversible',
    reboot_policy: 'not_required',
    experimental: false,
    proof_status: 'partially_proven',
    exposure: 'core',
    recovery: 'best_effort',
    capability_id: 'system_tweak.change',
    mutates_system: true,
    requires_confirmation: true,
    requires_revalidation: true,
    requires_verification: true,
    requires_journal: true,
  },
  targets: [
    {
      target_id: 'privacy:fixture',
      requested_action: '{"apply":true}',
      description: 'Fixture',
    },
  ],
  warnings: [],
  expected_effects: [],
  requires_elevation: false,
  requires_restart: false,
  reversibility: 'fully_reversible',
}

describe('system tweak operation Tauri contract', () => {
  afterEach(() => clearMocks())

  it('lists classified targets, previews one target, and executes only the plan', async () => {
    const commands: string[] = []
    mockIPC((command) => {
      commands.push(command)
      if (command === 'list_system_tweak_targets') {
        return [
          {
            kind: 'privacy',
            id: 'fixture',
            name: 'Fixture',
            description: 'Fixture',
            risk: 'low',
            is_applied: false,
            experimental: false,
            requires_elevation: false,
            requires_restart: false,
          },
        ]
      }
      if (command === 'create_system_tweak_plan') return plan
      return {
        result: {
          operation_id: plan.id,
          requested: 1,
          changed: 1,
          unchanged: 0,
          skipped: 0,
          failed: 0,
          verified: 1,
          unverified: 0,
          outcomes: [],
          reboot_required: false,
          undo: 'undo_available',
        },
        message: 'Fixture was enabled and verified.',
      }
    })

    const targets = await listSystemTweakTargets()
    const confirmed = await createSystemTweakPlan(targets[0].kind, targets[0].id)
    const execution = await executeSystemTweakPlan(confirmed)

    expect(commands).toEqual([
      'list_system_tweak_targets',
      'create_system_tweak_plan',
      'execute_system_tweak_plan',
    ])
    expect(execution.result.verified).toBe(1)
  })

  it('keeps preset targets visible through preview and execution', async () => {
    const commands: string[] = []
    mockIPC((command) => {
      commands.push(command)
      if (command === 'list_tweak_presets') {
        return [
          {
            id: 'fixture',
            name: 'Fixture preset',
            description: 'Fixture',
            target_ids: ['privacy:fixture'],
          },
        ]
      }
      if (command === 'create_tweak_preset_plan') {
        return {
          ...plan,
          descriptor: { ...plan.descriptor, id: 'preset.fixture' },
        }
      }
      return {
        result: {
          operation_id: plan.id,
          requested: 1,
          changed: 1,
          unchanged: 0,
          skipped: 0,
          failed: 0,
          verified: 1,
          unverified: 0,
          outcomes: [],
          reboot_required: false,
          undo: 'undo_available',
        },
        message: 'All preset targets were verified.',
      }
    })

    const presets = await listTweakPresets()
    const confirmed = await createTweakPresetPlan(presets[0].id)
    await executeTweakPresetPlan(confirmed)

    expect(commands).toEqual([
      'list_tweak_presets',
      'create_tweak_preset_plan',
      'execute_tweak_preset_plan',
    ])
    expect(confirmed.targets[0].target_id).toBe('privacy:fixture')
  })
})
