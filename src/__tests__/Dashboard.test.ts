import { flushPromises, mount } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { nextTick } from 'vue'
import Dashboard from '../views/Dashboard.vue'
import { SYSTEM_SNAPSHOT_TTL_MS } from '../stores/systemSnapshot'

const mocks = vi.hoisted(() => ({
  getHardwareInfo: vi.fn(),
  getOsInfo: vi.fn(),
  getOperationJournal: vi.fn(),
  push: vi.fn(),
}))

vi.mock('../api/tauri', () => ({
  getHardwareInfo: mocks.getHardwareInfo,
  getOsInfo: mocks.getOsInfo,
}))

vi.mock('../api/operation', () => ({
  getOperationJournal: mocks.getOperationJournal,
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

const os = {
  version: 'Windows 11',
  edition: 'Pro',
  build: '26100',
  arch: 'x64',
  release_id: null,
  install_date: null,
  last_boot: null,
  user_name: 'Test',
  domain: null,
}

describe('Home workflow shell', () => {
  beforeEach(() => {
    setActivePinia(createPinia())
    vi.clearAllMocks()
    mocks.getHardwareInfo.mockResolvedValue(hardware)
    mocks.getOsInfo.mockResolvedValue(os)
    mocks.getOperationJournal.mockResolvedValue([])
  })

  it('shows factual metrics, scan status, history, system identity, and a separate recommendations link', async () => {
    const wrapper = mount(Dashboard, {
      global: {
        stubs: {
          AppHeader: true,
          BaseBadge: { template: '<span><slot /></span>' },
          BaseButton: { template: '<button @click="$emit(\'click\')"><slot /></button>' },
        },
      },
    })

    await flushPromises()

    expect(wrapper.get('[aria-label="Live system metrics"]').text()).toContain('CPU25%')
    expect(wrapper.get('[aria-label="Live system metrics"]').text()).toContain('Memory50%')
    expect(wrapper.get('[aria-label="Live system metrics"]').text()).toContain('Storage60%')
    expect(wrapper.text()).toContain('Metrics available')
    expect(wrapper.text()).not.toContain('Data healthy')
    expect(wrapper.text()).not.toMatch(/health score|issues found|fix everything|boost pc/i)
    expect(wrapper.text()).not.toContain('Recommended action')
    expect(wrapper.text()).toContain('Recommendations are reviewed separately')
    expect(wrapper.text()).toContain('Latest scan')
    expect(wrapper.text()).toContain('Recent changes')
    expect(wrapper.text()).toContain('Windows 11 · Pro · x64')
    await wrapper.findAll('button').find(button => button.text() === 'Review recommendations')!.trigger('click')
    expect(mocks.push).toHaveBeenCalledWith('/recommendations')

    wrapper.unmount()
  })

  it('exposes metric collection failure instead of stale live status', async () => {
    mocks.getHardwareInfo.mockRejectedValue(new Error('collector unavailable'))

    const wrapper = mount(Dashboard, {
      global: {
        stubs: {
          AppHeader: true,
          BaseBadge: { template: '<span><slot /></span>' },
          BaseButton: { template: '<button @click="$emit(\'click\')"><slot /></button>' },
        },
      },
    })

    await flushPromises()

    expect(wrapper.get('[aria-label="Live system metrics"]').text()).toContain('Collection failed')
    expect(wrapper.get('[aria-label="Live system metrics"]').text()).toContain('Not available')

    wrapper.unmount()
  })

  it('clears previously collected metrics after a later collection failure', async () => {
    let calls = 0
    mocks.getHardwareInfo.mockImplementation(() => {
      calls += 1
      return calls === 1
        ? Promise.resolve(hardware)
        : Promise.reject(new Error('collector unavailable'))
    })

    const wrapper = mount(Dashboard, {
      global: {
        stubs: {
          AppHeader: true,
          BaseBadge: { template: '<span><slot /></span>' },
          BaseButton: { template: '<button @click="$emit(\'click\')"><slot /></button>' },
        },
      },
    })

    try {
      await flushPromises()
      expect(wrapper.get('[aria-label="Live system metrics"]').text()).toContain('CPU25%')

      await wrapper.findAll('button').find(button => button.text() === 'Refresh')!.trigger('click')
      await flushPromises()

      const metrics = wrapper.get('[aria-label="Live system metrics"]').text()
      expect(metrics).toContain('CPU—')
      expect(metrics).toContain('Collection failed')
      expect(metrics).toContain('Updated Not available')
    } finally {
      wrapper.unmount()
    }
  })

  it('marks cached metrics stale after the shared freshness window expires', async () => {
    vi.useFakeTimers()
    const wrapper = mount(Dashboard, {
      global: {
        stubs: {
          AppHeader: true,
          BaseBadge: { template: '<span><slot /></span>' },
          BaseButton: { template: '<button @click="$emit(\'click\')"><slot /></button>' },
        },
      },
    })

    try {
      await flushPromises()
      expect(wrapper.text()).toContain('Metrics available')

      vi.advanceTimersByTime(SYSTEM_SNAPSHOT_TTL_MS)
      await nextTick()
      expect(wrapper.text()).toContain('Metrics stale')
    } finally {
      wrapper.unmount()
      vi.useRealTimers()
    }
  })

  it('explains when history data is unavailable instead of implying no changes', async () => {
    mocks.getOperationJournal.mockRejectedValue(new Error('history unavailable'))

    const wrapper = mount(Dashboard, {
      global: {
        stubs: {
          AppHeader: true,
          BaseBadge: { template: '<span><slot /></span>' },
          BaseButton: { template: '<button @click="$emit(\'click\')"><slot /></button>' },
        },
      },
    })

    await flushPromises()

    expect(wrapper.text()).toContain('History unavailable')
    expect(wrapper.text()).not.toContain('No recorded changes')
    wrapper.unmount()
  })
})
