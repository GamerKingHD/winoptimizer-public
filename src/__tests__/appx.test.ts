import { afterEach, describe, expect, it } from 'vitest'
import { clearMocks, mockIPC } from '@tauri-apps/api/mocks'

import { createAppxPlan, executeAppxPlan } from '../api/appx'

describe('AppX operation API', () => {
  afterEach(() => clearMocks())

  it('creates a server plan for the selected package family', async () => {
    let received: unknown
    mockIPC((command, args) => {
      expect(command).toBe('create_appx_plan')
      received = args
      return { planId: 'plan-1' }
    })

    await createAppxPlan({
      kind: 'mark',
      family_names: ['Microsoft.BingNews_8wekyb3d8bbwe'],
    })

    expect(received).toEqual({
      action: {
        kind: 'mark',
        family_names: ['Microsoft.BingNews_8wekyb3d8bbwe'],
      },
    })
  })

  it('executes the exact confirmed server plan', async () => {
    const plan = { planId: 'plan-1', targets: [] }
    let received: unknown
    mockIPC((command, args) => {
      expect(command).toBe('execute_appx_plan')
      received = args
      return { status: 'success' }
    })

    await executeAppxPlan(plan as never)

    expect(received).toEqual({ confirmedPlan: plan })
  })
})
