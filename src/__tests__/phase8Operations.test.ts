import { afterEach, describe, expect, it } from 'vitest'
import { clearMocks, mockIPC } from '@tauri-apps/api/mocks'

import { createEnvPlan, executeEnvPlan } from '../api/environment'
import { createProcessPlan, executeProcessPlan } from '../api/processes'

describe('remaining Phase 8 operation APIs', () => {
  afterEach(() => clearMocks())

  it('passes an exact scoped environment action and confirmed plan', async () => {
    const calls: Array<[string, unknown]> = []
    mockIPC((command, args) => {
      calls.push([command, args])
      return command === 'create_env_plan' ? { id: 'env-plan' } : { result: { failed: 0 } }
    })
    const action = {
      kind: 'set' as const,
      variable: { name: 'TEST', value: '1', scope: 'user' as const, var_type: 'REG_SZ' as const },
    }
    const plan = await createEnvPlan(action)
    await executeEnvPlan(plan)
    expect(calls).toEqual([
      ['create_env_plan', { action }],
      ['execute_env_plan', { confirmedPlan: plan }],
    ])
  })

  it('preserves system REG_EXPAND_SZ and delete scopes across the API boundary', async () => {
    const calls: Array<[string, unknown]> = []
    mockIPC((command, args) => {
      calls.push([command, args])
      return { id: 'env-plan' }
    })
    const setAction = {
      kind: 'set' as const,
      variable: { name: 'WINOPTIMIZER_ENV', value: '%TEMP%\\value', scope: 'system' as const, var_type: 'REG_EXPAND_SZ' as const },
    }
    await createEnvPlan(setAction)
    await createEnvPlan({ kind: 'delete', name: setAction.variable.name, scope: 'system' })
    expect(calls).toEqual([
      ['create_env_plan', { action: setAction }],
      ['create_env_plan', { action: { kind: 'delete', name: 'WINOPTIMIZER_ENV', scope: 'system' } }],
    ])
  })

  it('passes the complete process identity and confirmed plan', async () => {
    const calls: Array<[string, unknown]> = []
    mockIPC((command, args) => {
      calls.push([command, args])
      return command === 'create_process_plan' ? { id: 'process-plan' } : { result: { failed: 0 } }
    })
    const process = {
      pid: 123,
      start_time: 456,
      name: 'fixture.exe',
      cpu_usage: 0,
      memory_bytes: 0,
      disk_read_bytes: 0,
      disk_write_bytes: 0,
      status: 'Run',
      path: 'C:\\fixture.exe',
    }
    const plan = await createProcessPlan(process)
    await executeProcessPlan(plan)
    expect(calls).toEqual([
      ['create_process_plan', { process }],
      ['execute_process_plan', { confirmedPlan: plan }],
    ])
  })
})
