import { afterEach, describe, expect, it } from 'vitest'
import { clearMocks, mockIPC } from '@tauri-apps/api/mocks'

import { createHostsPlan, executeHostsPlan } from '../api/hosts'

describe('hosts operation API', () => {
  afterEach(() => clearMocks())

  it('previews the exact requested entries on the server', async () => {
    const entries = [{
      id: 1,
      ip: '127.0.0.1',
      hostnames: ['example.local'],
      comment: null,
      enabled: true,
      raw_line: null,
    }]
    let received: unknown
    mockIPC((command, args) => {
      expect(command).toBe('create_hosts_plan')
      received = args
      return { id: 'plan-1' }
    })

    await createHostsPlan(entries)

    expect(received).toEqual({ entries })
  })

  it('executes the exact confirmed plan', async () => {
    const plan = { id: 'plan-1', targets: [] }
    let received: unknown
    mockIPC((command, args) => {
      expect(command).toBe('execute_hosts_plan')
      received = args
      return { result: { failed: 0 } }
    })

    await executeHostsPlan(plan as never)

    expect(received).toEqual({ confirmedPlan: plan })
  })
})
