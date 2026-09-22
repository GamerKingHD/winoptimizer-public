import { flushPromises, mount } from '@vue/test-utils'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import SystemTweaks from '../views/SystemTweaks.vue'

const mocks = vi.hoisted(() => ({
  listSystemTweakTargets: vi.fn(),
  listTweakPresets: vi.fn(),
  listContextMenuEntries: vi.fn(),
}))

vi.mock('../api/systemTweaks', () => ({
  listSystemTweakTargets: mocks.listSystemTweakTargets,
  listTweakPresets: mocks.listTweakPresets,
  listContextMenuEntries: mocks.listContextMenuEntries,
  createSystemTweakPlan: vi.fn(),
  createTweakPresetPlan: vi.fn(),
  executeSystemTweakPlan: vi.fn(),
  executeTweakPresetPlan: vi.fn(),
}))

vi.mock('../api/tauri', () => ({
  listContextMenuEntries: mocks.listContextMenuEntries,
  checkElevated: vi.fn(),
}))

vi.mock('../api/operation', () => ({
  confirmAndRequestElevation: vi.fn(),
}))

const stubs = {
  AppHeader: true,
  BaseTabBar: { template: '<nav />' },
  BaseCard: { template: '<div><slot /></div>' },
  BaseButton: { template: '<button><slot /></button>' },
  BaseBadge: { template: '<span><slot /></span>' },
  BaseToggle: { template: '<input type="checkbox" />' },
  EmptyState: true,
  ConfirmDialog: true,
  OperationPreview: true,
  ResultsBanner: {
    props: ['title', 'description'],
    template: '<div class="feedback">{{ title }} {{ description }}</div>',
  },
  BaseTable: true,
}

describe('SystemTweaks factual state labels', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    mocks.listSystemTweakTargets.mockResolvedValue([
      {
        kind: 'privacy',
        id: 'privacy_fixture',
        name: 'Privacy baseline',
        description: 'Fixture setting',
        risk: 'low',
        is_applied: true,
        experimental: false,
        requires_elevation: false,
        requires_restart: false,
      },
    ])
    mocks.listTweakPresets.mockResolvedValue([])
    mocks.listContextMenuEntries.mockResolvedValue([])
  })

  it('uses applied state terminology and clears stale cards after reload failure', async () => {
    const wrapper = mount(SystemTweaks, { global: { stubs } })
    const vm = wrapper.vm as unknown as { refresh: () => Promise<void> }

    await vm.refresh()
    await flushPromises()

    expect(wrapper.text()).toContain('Applied')
    expect(wrapper.text()).not.toContain('Implemented')

    mocks.listSystemTweakTargets.mockRejectedValueOnce(new Error('state unavailable'))
    await vm.refresh()
    await flushPromises()

    expect(wrapper.text()).not.toContain('Privacy baseline')
    expect(wrapper.text()).toContain('Could not load privacy tweaks')
  })
})
