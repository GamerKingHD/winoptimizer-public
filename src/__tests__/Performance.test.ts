import { flushPromises, mount } from '@vue/test-utils'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { clearMocks, mockIPC } from '@tauri-apps/api/mocks'
import Performance from '../views/Performance.vue'

const mocks = vi.hoisted(() => ({
  getHardwareInfo: vi.fn(),
  processSnapshot: vi.fn(),
  push: vi.fn(),
}))

vi.mock('../api/tauri', () => ({
  getHardwareInfo: mocks.getHardwareInfo,
  processSnapshot: mocks.processSnapshot,
}))

vi.mock('vue-router', () => ({
  useRouter: () => ({ push: mocks.push }),
}))

const hardware = {
  cpu: { name: 'Test CPU', cores: 8, logical_processors: 16, architecture: 'x64', usage_percent: 25 },
  memory: { total_bytes: 16_000, available_bytes: 8_000, in_use_percent: 50 },
  disks: [{ name: 'C:', mount_point: 'C:\\', total_bytes: 100_000, available_bytes: 40_000, file_system: 'NTFS', is_ssd: true, disk_type: 'SSD' }],
  gpu: null,
}

const processes = [{
  pid: 1,
  start_time: 1,
  name: 'fixture.exe',
  cpu_usage: 1,
  memory_bytes: 100,
  disk_read_bytes: 0,
  disk_write_bytes: 0,
  status: 'running',
  path: null,
}]

const revertPlan = {
  id: 'revert-operation-1',
  descriptor: {
    id: 'system_tweak.change',
    title: 'Disable Processor scheduling',
    category: 'system_tweak',
    risk: 'low',
    privilege: 'standard_user',
    reversibility: 'fully_reversible',
    reboot_policy: 'not_required',
    experimental: false,
    proof_status: 'proven',
    exposure: 'advanced',
    recovery: 'exact',
    capability_id: 'system_tweak.change',
    mutates_system: true,
    requires_confirmation: true,
    requires_revalidation: true,
    requires_verification: true,
    requires_journal: true,
  },
  targets: [{ target_id: 'performance:processor_scheduling', requested_action: '{"apply":false}', description: 'Fixture tweak' }],
  warnings: [],
  expected_effects: [{ target_id: 'performance:processor_scheduling', description: 'Read back disabled state.' }],
  requires_elevation: false,
  requires_restart: false,
  reversibility: 'fully_reversible',
  parent_operation_id: 'apply-operation-1',
} as const

async function clickButton(wrapper: ReturnType<typeof mount>, label: string) {
  const button = wrapper.findAll('button').find(candidate => candidate.text().trim() === label)
  expect(button, `button ${label} should be present`).toBeDefined()
  await button!.trigger('click')
}

describe('Performance experiment view', () => {
  afterEach(() => clearMocks())

  beforeEach(() => {
    vi.clearAllMocks()
    mockIPC((command, args) => {
      if (command === 'list_performance_experiments') return []
      if (command === 'save_performance_experiment') {
        return (args as { experiment?: unknown } | undefined)?.experiment
      }
      if (command === 'prepare_performance_experiment_revert') return revertPlan
      if (command === 'execute_system_tweak_plan') {
        return { result: { requested: 1, failed: 0, verified: 1 }, message: 'Revert was applied and verified.' }
      }
      if (command === 'record_performance_experiment_decision') {
        const experiment = (args as { experiment?: Record<string, unknown> } | undefined)?.experiment
        return {
          ...experiment,
          state: 'revert_requested',
          decision: 'revert',
          operationEvidence: { readbackVerified: true },
          decisionOperationId: 'revert-operation-1',
          decisionEvidence: { readbackVerified: true },
        }
      }
      return undefined
    })
    hardware.cpu.usage_percent = 25
    hardware.memory.in_use_percent = 50
    mocks.getHardwareInfo.mockResolvedValue(hardware)
    mocks.processSnapshot.mockResolvedValue(processes)
  })

  it('records the explicit lifecycle and keeps the page mutation-free', async () => {
    const wrapper = mount(Performance, {
      global: {
        stubs: {
          AppHeader: true,
          BaseButton: {
            props: ['disabled', 'loading'],
            template: '<button :disabled="disabled" @click="$emit(\'click\')"><slot /></button>',
          },
          BaseCard: { template: '<div><slot /></div>' },
          EmptyState: { template: '<div><slot /></div>' },
          OperationPreview: { props: ['title'], template: '<div>{{ title }}<slot /></div>' },
          ResultsBanner: { template: '<div><slot /></div>' },
        },
      },
    })

    await flushPromises()
    await wrapper.find('input').setValue('Processor scheduling')
    await wrapper.find('textarea').setValue('The selected workload should show a measurable change.')
    await wrapper.findAll('input')[1].setValue('apply-operation-1')
    await clickButton(wrapper, 'Start experiment')

    for (let sample = 0; sample < 3; sample += 1) await clickButton(wrapper, 'Record baseline sample')
    await clickButton(wrapper, 'Mark applied; reboot required')
    expect(wrapper.text()).toContain('Waiting to resume after reboot')
    await clickButton(wrapper, 'Resume after reboot')
    hardware.cpu.usage_percent = 35
    hardware.memory.in_use_percent = 60
    for (let sample = 0; sample < 3; sample += 1) await clickButton(wrapper, 'Record after sample')
    await clickButton(wrapper, 'Compare measurements')

    expect(wrapper.text()).toContain('No meaningful change')
    expect(wrapper.text()).toContain('not proof of a faster system')
    expect(wrapper.text()).toContain('Metric direction is undefined')
    expect(wrapper.text()).toContain('Measurements stay attached to a reviewed operation')

    await clickButton(wrapper, 'Prepare Revert plan')
    expect(wrapper.text()).toContain('Review Revert plan')
    await clickButton(wrapper, 'Confirm and execute Revert')
    await flushPromises()
    expect(wrapper.text()).toContain('Revert was completed through a separate normal operation plan')
    expect(mocks.push).not.toHaveBeenCalled()

    wrapper.unmount()
  })
})
