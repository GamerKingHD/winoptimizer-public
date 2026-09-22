import { afterEach, describe, expect, it } from 'vitest'
import { clearMocks, mockIPC } from '@tauri-apps/api/mocks'

import type { OperationPlan } from '../api/operation'
import {
  createServicePlan,
  createScheduledTaskPlan,
  createScheduledTasksGuidancePlan,
  createServicesManualPlan,
  createServicesGuidancePlan,
  createStartupProgramEnablePlan,
  createStartupProgramPlan,
  executeScheduledTaskPlan,
  executeScheduledTasksGuidancePlan,
  executeServicePlan,
  executeServicesManualPlan,
  executeServicesGuidancePlan,
  executeStartupProgramEnablePlan,
  executeStartupProgramPlan,
  scanScheduledTasks,
} from '../api/startup'

const plan: OperationPlan = {
  id: 'startup-plan-1',
  descriptor: {
    id: 'startup.disable_program',
    title: 'Disable startup program',
    category: 'startup',
    risk: 'low',
    privilege: 'standard_user',
    reversibility: 'fully_reversible',
    reboot_policy: 'not_required',
    experimental: false,
    proof_status: 'partially_proven',
    exposure: 'core',
    recovery: 'best_effort',
    capability_id: 'startup.program',
    mutates_system: true,
    requires_confirmation: true,
    requires_revalidation: true,
    requires_verification: true,
    requires_journal: true,
  },
  targets: [
    {
      target_id: 'fixture-id',
      requested_action: '{"id":"fixture-id"}',
      description: 'Disable Fixture from HKCU_Run',
    },
  ],
  warnings: [],
  expected_effects: [],
  requires_elevation: false,
  requires_restart: false,
  reversibility: 'fully_reversible',
}

describe('startup operation Tauri contract', () => {
  afterEach(() => clearMocks())

  it('previews by opaque identifier and executes the unchanged confirmed plan', async () => {
    const calls: Array<{ command: string; args?: unknown }> = []
    mockIPC((command, args) => {
      calls.push({ command, args })
      if (command === 'create_startup_program_plan') return plan
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
        recovery_action_id: 'undo-1',
        message: 'Fixture will no longer start with Windows',
      }
    })

    const confirmed = await createStartupProgramPlan('fixture-id')
    const execution = await executeStartupProgramPlan(confirmed)

    expect(calls).toEqual([
      {
        command: 'create_startup_program_plan',
        args: { programId: 'fixture-id' },
      },
      {
        command: 'execute_startup_program_plan',
        args: { confirmedPlan: plan },
      },
    ])
    expect(execution.result.verified).toBe(1)
    expect(execution.recovery_action_id).toBe('undo-1')
  })

  it('sends one explicit Experimental service action through preview and execution', async () => {
    const calls: Array<{ command: string; args?: unknown }> = []
    const servicePlan: OperationPlan = {
      ...plan,
      descriptor: {
        ...plan.descriptor,
        id: 'startup.service_change',
        experimental: true,
        reversibility: 'best_effort',
        privilege: 'administrator',
      },
      targets: [{ ...plan.targets[0], target_id: 'Steam Client Service' }],
      requires_elevation: true,
      reversibility: 'best_effort',
    }
    mockIPC((command, args) => {
      calls.push({ command, args })
      if (command === 'create_service_plan') return servicePlan
      return {
        result: {
          operation_id: servicePlan.id,
          requested: 1,
          changed: 1,
          unchanged: 0,
          skipped: 0,
          failed: 0,
          verified: 1,
          unverified: 0,
          outcomes: [],
          reboot_required: false,
          undo: 'not_reversible',
        },
        message: 'Steam Client Service was updated and verified.',
      }
    })

    const confirmed = await createServicePlan('Steam Client Service', { kind: 'stop' })
    const execution = await executeServicePlan(confirmed)

    expect(calls).toEqual([
      {
        command: 'create_service_plan',
        args: { serviceName: 'Steam Client Service', action: { kind: 'stop' } },
      },
      {
        command: 'execute_service_plan',
        args: { confirmedPlan: servicePlan },
      },
    ])
    expect(execution.result.undo).toBe('not_reversible')
  })

  it('keeps startup enable preview and execution on explicit IPC contracts', async () => {
    const calls: Array<{ command: string; args?: unknown }> = []
    const enablePlan: OperationPlan = {
      ...plan,
      descriptor: {
        ...plan.descriptor,
        id: 'startup.enable_program',
        title: 'Enable startup program',
        reversibility: 'not_reversible',
      },
      targets: [{ ...plan.targets[0], target_id: 'fixture-id' }],
      reversibility: 'not_reversible',
    }
    mockIPC((command, args) => {
      calls.push({ command, args })
      if (command === 'create_startup_program_enable_plan') return enablePlan
      return {
        result: {
          operation_id: enablePlan.id,
          requested: 1,
          changed: 1,
          unchanged: 0,
          skipped: 0,
          failed: 0,
          verified: 1,
          unverified: 0,
          outcomes: [],
          reboot_required: false,
          undo: 'not_reversible',
        },
        message: 'Fixture was enabled and verified.',
      }
    })

    const confirmed = await createStartupProgramEnablePlan('fixture-id')
    const execution = await executeStartupProgramEnablePlan(confirmed)

    expect(calls).toEqual([
      {
        command: 'create_startup_program_enable_plan',
        args: { programId: 'fixture-id' },
      },
      {
        command: 'execute_startup_program_enable_plan',
        args: { confirmedPlan: enablePlan },
      },
    ])
    expect(execution.result.verified).toBe(1)
  })

  it('preserves scheduled-task collection failure instead of returning false empty success', async () => {
    mockIPC((command) => {
      expect(command).toBe('scan_scheduled_tasks')
      return {
        tasks: [],
        collected_at: '2026-07-28T20:00:00Z',
        classification: 'supporting',
        mutation_available: false,
        error: 'Task Scheduler query failed: access denied',
      }
    })

    const scan = await scanScheduledTasks()

    expect(scan.tasks).toEqual([])
    expect(scan.error).toContain('access denied')
    expect(scan.mutation_available).toBe(false)
  })

  it('keeps bulk service and scheduled-task operations on explicit IPC contracts', async () => {
    const calls: Array<{ command: string; args?: unknown }> = []
    const bulkPlan = { ...plan, descriptor: { ...plan.descriptor, id: 'startup.services_set_manual' } }
    const taskPlan = { ...plan, descriptor: { ...plan.descriptor, id: 'startup.scheduled_task_change' } }
    mockIPC((command, args) => {
      calls.push({ command, args })
      if (command === 'create_services_manual_plan') return bulkPlan
      if (command === 'create_scheduled_task_plan') return taskPlan
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
          undo: 'not_reversible',
        },
        message: 'updated',
      }
    })

    await createServicesManualPlan()
    await executeServicesManualPlan(bulkPlan)
    await createScheduledTaskPlan('\\Vendor\\Task', false)
    await executeScheduledTaskPlan(taskPlan)

    expect(calls).toEqual([
      { command: 'create_services_manual_plan', args: {} },
      { command: 'execute_services_manual_plan', args: { confirmedPlan: bulkPlan } },
      { command: 'create_scheduled_task_plan', args: { taskName: '\\Vendor\\Task', enabled: false } },
      { command: 'execute_scheduled_task_plan', args: { confirmedPlan: taskPlan } },
    ])
  })

  it('sends selected service and task guidance targets as exact IPC payloads', async () => {
    const calls: Array<{ command: string; args?: unknown }> = []
    const serviceGuidancePlan = { ...plan, descriptor: { ...plan.descriptor, id: 'startup.services_guidance' } }
    const taskGuidancePlan = { ...plan, descriptor: { ...plan.descriptor, id: 'startup.scheduled_tasks_guidance' } }
    mockIPC((command, args) => {
      calls.push({ command, args })
      if (command === 'create_services_guidance_plan') return serviceGuidancePlan
      if (command === 'create_scheduled_tasks_guidance_plan') return taskGuidancePlan
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
          undo: 'not_reversible',
        },
        message: 'updated',
      }
    })

    await createServicesGuidancePlan([
      { serviceName: 'BITS', desiredStartType: 'demand' },
      { serviceName: 'DiagTrack', desiredStartType: 'disabled' },
    ])
    await executeServicesGuidancePlan(serviceGuidancePlan)
    await createScheduledTasksGuidancePlan([
      { taskName: '\\Microsoft\\Windows\\Maps\\MapsUpdateTask', enabled: false },
    ])
    await executeScheduledTasksGuidancePlan(taskGuidancePlan)

    expect(calls).toEqual([
      {
        command: 'create_services_guidance_plan',
        args: {
          targets: [
            { serviceName: 'BITS', desiredStartType: 'demand' },
            { serviceName: 'DiagTrack', desiredStartType: 'disabled' },
          ],
        },
      },
      {
        command: 'execute_services_guidance_plan',
        args: { confirmedPlan: serviceGuidancePlan },
      },
      {
        command: 'create_scheduled_tasks_guidance_plan',
        args: {
          targets: [{ taskName: '\\Microsoft\\Windows\\Maps\\MapsUpdateTask', enabled: false }],
        },
      },
      {
        command: 'execute_scheduled_tasks_guidance_plan',
        args: { confirmedPlan: taskGuidancePlan },
      },
    ])
  })
})
