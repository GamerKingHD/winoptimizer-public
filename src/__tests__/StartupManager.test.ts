import { describe, expect, it, vi } from 'vitest'
import { mount } from '@vue/test-utils'
import StartupManager from '../views/StartupManager.vue'
import BaseTabBar from '../components/BaseTabBar.vue'
import type { OperationPlan } from '../api/operation'
import type { StartupExecution } from '../api/startup'
import type { GuidanceInfo, ServiceInfo, StartupProgram } from '../api/tauri'
import * as api from '../api/tauri'
import * as startupApi from '../api/startup'

vi.mock('../api/tauri', () => ({
  listStartupPrograms: vi.fn().mockResolvedValue([]),
  listServices: vi.fn().mockResolvedValue([]),
  checkElevated: vi.fn().mockResolvedValue(true),
}))

vi.mock('../api/startup', () => ({
  createScheduledTaskPlan: vi.fn(),
  createScheduledTasksGuidancePlan: vi.fn(),
  createServicePlan: vi.fn(),
  createServicesGuidancePlan: vi.fn(),
  createStartupProgramEnablePlan: vi.fn(),
  createStartupProgramPlan: vi.fn(),
  executeScheduledTaskPlan: vi.fn(),
  executeScheduledTasksGuidancePlan: vi.fn(),
  executeServicePlan: vi.fn(),
  executeServicesGuidancePlan: vi.fn(),
  executeStartupProgramEnablePlan: vi.fn(),
  executeStartupProgramPlan: vi.fn(),
  scanScheduledTasks: vi.fn().mockResolvedValue({
    tasks: [],
    collected_at: '2026-07-28T20:00:00Z',
    classification: 'supporting',
    mutation_available: true,
    error: null,
  }),
}))

describe('StartupManager', () => {
  const guidance = (
    action: GuidanceInfo['action'],
    source_type: GuidanceInfo['source_type'],
    protectedTarget = false,
  ): GuidanceInfo => ({
    group: action,
    action,
    rationale: `${action} rationale`,
    conditions: [],
    source_type,
    source_title: source_type,
    source_url: '',
    os_scope: 'Windows 10/11 desktop',
    catalog_version: '1.0.0',
    protected: protectedTarget,
    matched_by: protectedTarget ? 'protected' : 'exact',
  })

  it('loads data when the user opens the services or scheduled tasks tabs', async () => {
    const wrapper = mount(StartupManager, { shallow: true })
    const tabs = wrapper.findComponent(BaseTabBar)

    await tabs.vm.$emit('update:modelValue', 'services')
    await vi.waitFor(() => expect(api.listServices).toHaveBeenCalled())

    await tabs.vm.$emit('update:modelValue', 'tasks')
    await vi.waitFor(() => expect(startupApi.scanScheduledTasks).toHaveBeenCalled())
  })

  it('keeps a persisted disabled program visible after refresh and remount', async () => {
    const program: StartupProgram = {
      id: 'a'.repeat(64),
      name: 'Fixture',
      command: 'C:\\fixture.exe',
      location: 'HKCU_Run',
      value_type: 'REG_SZ',
      enabled: false,
      protected: false,
      recovery_action_id: 'history-1',
    }

    vi.mocked(api.listStartupPrograms).mockReset()
    vi.mocked(api.listStartupPrograms).mockResolvedValue([program])

    const wrapper = mount(StartupManager, { shallow: true })
    const vm = wrapper.vm as unknown as {
      loadPrograms: (force?: boolean) => Promise<void>
      programs: StartupProgram[]
    }

    await vm.loadPrograms()
    await vm.loadPrograms(true)

    expect(vm.programs).toEqual([program])

    const remounted = mount(StartupManager, { shallow: true })
    const remountedVm = remounted.vm as unknown as {
      loadPrograms: () => Promise<void>
      programs: StartupProgram[]
    }
    await remountedVm.loadPrograms()

    expect(remountedVm.programs).toEqual([program])
  })

  it('uses the enable plan for a disabled startup entry', async () => {
    const program: StartupProgram = {
      id: 'b'.repeat(64),
      name: 'Fixture',
      command: 'C:\\fixture.exe',
      location: 'HKCU_Run',
      value_type: 'REG_SZ',
      enabled: false,
      protected: false,
      recovery_action_id: 'history-2',
    }
    const plan = { requires_elevation: false } as OperationPlan
    const execution = { result: {} as StartupExecution['result'], message: 'Fixture was enabled and verified.' }

    vi.mocked(api.listStartupPrograms).mockResolvedValue([program])
    vi.mocked(startupApi.createStartupProgramEnablePlan).mockResolvedValue(plan)
    vi.mocked(startupApi.executeStartupProgramEnablePlan).mockResolvedValue(execution)

    const wrapper = mount(StartupManager, { shallow: true })
    const vm = wrapper.vm as unknown as {
      loadPrograms: () => Promise<void>
      toggleProgram: (program: StartupProgram) => Promise<void>
    }

    await vm.loadPrograms()
    await vm.toggleProgram(program)

    expect(startupApi.createStartupProgramEnablePlan).toHaveBeenCalledWith(program.id)
  })

  it('honors the backend protected flag before opening a startup mutation plan', async () => {
    vi.clearAllMocks()
    const program: StartupProgram = {
      id: 'p'.repeat(64),
      name: 'Windows Security notification icon',
      command: 'C:\\Windows\\System32\\SecurityHealthSystray.exe',
      location: 'HKCU_Run',
      value_type: 'REG_SZ',
      enabled: true,
      protected: true,
      recovery_action_id: null,
    }

    vi.mocked(api.listStartupPrograms).mockResolvedValue([program])
    const wrapper = mount(StartupManager, { shallow: true })
    const vm = wrapper.vm as unknown as {
      loadPrograms: () => Promise<void>
      toggleProgram: (program: StartupProgram) => Promise<void>
    }

    await vm.loadPrograms()
    await vm.toggleProgram(program)

    expect(startupApi.createStartupProgramPlan).not.toHaveBeenCalled()
    expect(startupApi.createStartupProgramEnablePlan).not.toHaveBeenCalled()
  })

  it('fails closed when a legacy startup payload omits the protected flag', async () => {
    vi.clearAllMocks()
    const legacyProgram = {
      id: 'l'.repeat(64),
      name: 'Legacy startup entry',
      command: 'C:\\fixture.exe',
      location: 'HKCU_Run',
      value_type: 'REG_SZ' as const,
      enabled: true,
      recovery_action_id: null,
    } as unknown as StartupProgram

    vi.mocked(api.listStartupPrograms).mockResolvedValue([legacyProgram])
    const wrapper = mount(StartupManager, { shallow: true })
    const vm = wrapper.vm as unknown as {
      loadPrograms: () => Promise<void>
      toggleProgram: (program: StartupProgram) => Promise<void>
    }

    await vm.loadPrograms()
    await vm.toggleProgram(legacyProgram)

    expect(startupApi.createStartupProgramPlan).not.toHaveBeenCalled()
    expect(startupApi.createStartupProgramEnablePlan).not.toHaveBeenCalled()
  })

  it('does not show catalog guidance when the machine has no enabled matching startup entry', async () => {
    vi.mocked(api.listStartupPrograms).mockResolvedValue([])

    const wrapper = mount(StartupManager, { shallow: true })
    const vm = wrapper.vm as unknown as {
      loadPrograms: () => Promise<void>
      detectedGuidance: Array<{ software: string }>
    }

    await vm.loadPrograms()

    expect(vm.detectedGuidance).toEqual([])
    expect(wrapper.find('.startup-guide').exists()).toBe(false)
  })

  it('shows guidance only for an enabled startup entry that matches the catalog', async () => {
    vi.mocked(api.listStartupPrograms).mockResolvedValue([
      {
        id: 'c'.repeat(64),
        name: 'Slack',
        command: 'C:\\Users\\Test\\AppData\\Local\\slack\\slack.exe',
        location: 'HKCU_Run',
        value_type: 'REG_SZ',
        enabled: true,
        protected: false,
        recovery_action_id: null,
      },
      {
        id: 'd'.repeat(64),
        name: 'Old disabled entry',
        command: 'C:\\Old\\discord.exe',
        location: 'HKCU_Run',
        value_type: 'REG_SZ',
        enabled: false,
        protected: false,
        recovery_action_id: 'history-3',
      },
    ])

    const wrapper = mount(StartupManager, { shallow: true })
    const vm = wrapper.vm as unknown as {
      loadPrograms: () => Promise<void>
      detectedGuidance: Array<{ software: string }>
    }

    await vm.loadPrograms()

    expect(vm.detectedGuidance.map(entry => entry.software)).toEqual(['Slack'])
    expect(wrapper.findAll('.guide-item')).toHaveLength(1)
  })

  it('groups, filters, and quick-selects only Microsoft/vendor service recommendations', async () => {
    const services: ServiceInfo[] = [
      {
        name: 'BITS',
        display_name: 'Background Intelligent Transfer Service',
        status: 'Stopped',
        start_type: 'auto',
        description: null,
        guidance: guidance('set_manual', 'microsoft'),
      },
      {
        name: 'DiagTrack',
        display_name: 'Connected User Experiences',
        status: 'Running',
        start_type: 'auto',
        description: null,
        guidance: guidance('disable_if_unused', 'community'),
      },
      {
        name: 'AcmeService',
        display_name: 'Acme Service',
        status: 'Stopped',
        start_type: 'auto',
        description: null,
        guidance: guidance('review_first', 'unknown'),
      },
    ]
    vi.mocked(api.listServices).mockResolvedValue(services)

    const wrapper = mount(StartupManager, { shallow: true })
    const vm = wrapper.vm as unknown as {
      loadServices: () => Promise<void>
      serviceGroupCounts: Record<string, number>
      serviceSourceFilter: string
      filteredServicesRows: ServiceInfo[]
      selectServiceRecommendations: () => void
      selectedServiceNames: Set<string>
      toggleServiceSelection: (name: string, checked: boolean) => void
      selectedServiceHasCommunity: boolean
    }

    await vm.loadServices()

    expect(vm.serviceGroupCounts).toMatchObject({
      set_manual: 1,
      disable_if_unused: 1,
      review_first: 1,
    })
    vm.serviceSourceFilter = 'community'
    expect(vm.filteredServicesRows.map(row => row.name)).toEqual(['DiagTrack'])
    vm.selectServiceRecommendations()
    expect(vm.selectedServiceNames).toEqual(new Set(['BITS']))
    vm.toggleServiceSelection('DiagTrack', true)
    expect(vm.selectedServiceHasCommunity).toBe(true)
  })
})
