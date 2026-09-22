import { flushPromises, mount } from '@vue/test-utils'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import Settings from '../views/Settings.vue'
import type { AppSettings } from '../api/settings'

const mocks = vi.hoisted(() => ({
  getSettings: vi.fn(),
  updateSettings: vi.fn(),
  exportSupportBundle: vi.fn(),
}))

vi.mock('../api/settings', () => mocks)
vi.mock('../api/operation', () => ({
  exportSupportBundle: mocks.exportSupportBundle,
}))

const defaultSettings: AppSettings = {
  schema_version: 2,
  experience_level: 'standard',
  labs_enabled: false,
  disk_analyzer_preferences: { persist_snapshots: true, max_cached_snapshots: 3 },
  cleanup_preferences: { preselect_safe_candidates: false },
  history_preferences: { retention_days: 365, show_internal_identifiers: false },
  online_preferences: { allow_metered_connections: false },
  telemetry_preferences: { enabled: false },
  crash_reporting_preferences: { enabled: false },
  update_preferences: { check_automatically: false },
  recommendation_preferences: { update_guidance: false, preselect_safe_recommendations: false },
}

beforeEach(() => {
  mocks.getSettings.mockResolvedValue(structuredClone(defaultSettings))
  mocks.updateSettings.mockImplementation(async (settings: AppSettings) => settings)
  mocks.exportSupportBundle.mockReset()
})

describe('Settings cache guidance', () => {
  it('explains adaptive WOF compression and background behavior', async () => {
    const wrapper = mount(Settings, {
      global: {
        stubs: {
          AppHeader: true,
          RouterLink: {
            props: ['to'],
            template: '<a :href="to"><slot /></a>',
          },
        },
      },
    })

    await flushPromises()
    const note = wrapper.get('[aria-labelledby="cache-compression-heading"]')
    expect(note.text()).toContain('Windows compression is automatic')
    expect(note.text()).toContain('XPRESS4K')
    expect(note.text()).toContain('XPRESS8K')
    expect(note.text()).toContain('XPRESS16K')
    expect(note.text()).toContain('LZX')
    expect(note.text()).toContain('low priority')
  })

  it('persists the selected experience level without dropping other settings', async () => {
    const wrapper = mount(Settings, {
      global: {
        stubs: {
          AppHeader: true,
          RouterLink: {
            props: ['to'],
            template: '<a :href="to"><slot /></a>',
          },
        },
      },
    })

    await flushPromises()
    await wrapper.get('select').setValue('expert')
    await wrapper.get('form').trigger('submit')

    expect(mocks.updateSettings).toHaveBeenCalledWith(expect.objectContaining({
      schema_version: 2,
      experience_level: 'expert',
    }))
    expect(wrapper.text()).toContain('Settings saved.')
  })
  it('keeps optional update checks off until explicitly enabled', async () => {
    const wrapper = mount(Settings, {
      global: {
        stubs: {
          AppHeader: true,
          RouterLink: {
            props: ['to'],
            template: '<a :href="to"><slot /></a>',
          },
        },
      },
    })

    await flushPromises()
    const updateCheckbox = wrapper.get('input[aria-label="Allow optional update checks"]')
    expect((updateCheckbox.element as HTMLInputElement).checked).toBe(false)

    await updateCheckbox.setValue(true)
    await wrapper.get('form').trigger('submit')

    expect(mocks.updateSettings).toHaveBeenCalledWith(expect.objectContaining({
      update_preferences: { check_automatically: true },
    }))
  })

  it('keeps telemetry and crash-reporting as separate explicit choices', async () => {
    const wrapper = mount(Settings, {
      global: {
        stubs: {
          AppHeader: true,
          RouterLink: {
            props: ['to'],
            template: '<a :href="to"><slot /></a>',
          },
        },
      },
    })

    await flushPromises()
    const telemetry = wrapper.get('input[aria-label="Allow privacy-minimized telemetry"]')
    const crashReporting = wrapper.get('input[aria-label="Allow crash reporting"]')
    expect((telemetry.element as HTMLInputElement).checked).toBe(false)
    expect((crashReporting.element as HTMLInputElement).checked).toBe(false)

    await telemetry.setValue(true)
    await wrapper.get('form').trigger('submit')

    expect(mocks.updateSettings).toHaveBeenCalledWith(expect.objectContaining({
      telemetry_preferences: { enabled: true },
      crash_reporting_preferences: { enabled: false },
    }))
  })

  it('keeps guidance updates as a separate opt-in', async () => {
    const wrapper = mount(Settings, {
      global: {
        stubs: {
          AppHeader: true,
          RouterLink: {
            props: ['to'],
            template: '<a :href="to"><slot /></a>',
          },
        },
      },
    })

    await flushPromises()
    const guidanceCheckbox = wrapper.get('input[aria-label="Allow optional guidance updates"]')
    expect((guidanceCheckbox.element as HTMLInputElement).checked).toBe(false)

    await guidanceCheckbox.setValue(true)
    await wrapper.get('form').trigger('submit')

    expect(mocks.updateSettings).toHaveBeenCalledWith(expect.objectContaining({
      recommendation_preferences: {
        update_guidance: true,
        preselect_safe_recommendations: false,
      },
    }))
  })

  it('shows the local path returned by support-bundle export', async () => {
    const bundlePath = String.raw`C:\Users\test\AppData\Local\WinOptimizer\support\support-123`
    mocks.exportSupportBundle.mockResolvedValue(bundlePath)
    const wrapper = mount(Settings, {
      global: {
        stubs: {
          AppHeader: true,
          RouterLink: {
            props: ['to'],
            template: '<a :href="to"><slot /></a>',
          },
        },
      },
    })

    await flushPromises()
    const exportButton = wrapper.findAll('button').find(button => button.text() === 'Export Support Bundle')
    expect(exportButton).toBeDefined()

    await exportButton!.trigger('click')
    await flushPromises()

    expect(mocks.exportSupportBundle).toHaveBeenCalledWith({
      redact_user_profile_path: true,
      redact_network_addresses: true,
      redact_hostnames: true,
      include_exact_file_paths: false,
      include_command_output: false,
    })
    expect(wrapper.get('[aria-labelledby="support-heading"] .support-result').text()).toBe(`Saved locally: ${bundlePath}`)
  })
})
