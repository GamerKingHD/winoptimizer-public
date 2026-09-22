import { flushPromises, mount, RouterLinkStub } from '@vue/test-utils'
import { describe, expect, it, vi } from 'vitest'
import router from '../router'
import Privacy from '../views/Privacy.vue'
import Settings from '../views/Settings.vue'

const mocks = vi.hoisted(() => ({
  getSettings: vi.fn(),
  updateSettings: vi.fn(),
  exportSupportBundle: vi.fn(),
}))

vi.mock('../api/settings', () => mocks)
vi.mock('../api/operation', () => ({
  exportSupportBundle: mocks.exportSupportBundle,
}))

mocks.getSettings.mockResolvedValue({
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
})

describe('Privacy & Updates route', () => {
  it('is registered as a read-only route', () => {
    const route = router.getRoutes().find(candidate => candidate.path === '/privacy')

    expect(route?.name).toBe('Privacy')
    expect(route?.meta.title).toBe('Privacy & Updates')
    expect(route?.meta.mutatesSystem).toBe(false)
    expect(route?.meta.requiresAdmin).toBe(false)
  })

  it('discloses local storage, disabled collection, and authenticated opt-in updates', () => {
    const wrapper = mount(Privacy, {
      global: { stubs: { AppHeader: true } },
    })

    const storage = wrapper.get('[aria-labelledby="local-storage-heading"]')
    expect(storage.text()).toContain('Operation journal')
    expect(storage.text()).toContain('Application settings')
    expect(storage.text()).toContain('Scan snapshots')
    expect(wrapper.text()).toContain('Telemetry')
    expect(wrapper.text()).toContain('Crash reporting')
    expect(wrapper.text()).toContain('Telemetry is off by default')
    expect(wrapper.text()).toContain('this build has no telemetry upload transport')
    expect(wrapper.text()).toContain('Crash reporting is off by default')
    expect(wrapper.get('[aria-labelledby="updates-heading"]').text()).toContain('opt-in')
    expect(wrapper.get('[aria-labelledby="updates-heading"]').text()).toContain('cryptographically authenticated')
    expect(wrapper.get('[aria-labelledby="internet-heading"]').text()).toContain('bundled guidance active')
    expect(wrapper.get('[aria-labelledby="updates-heading"]').text()).toContain('payload-digest checks')
    // Automatic checks require stored consent; an explicit user action is itself
    // consent for that request. The disclosure must not claim otherwise.
    expect(wrapper.get('[aria-labelledby="internet-heading"]').text()).toContain('Automatic release checks')
    expect(wrapper.get('[aria-labelledby="internet-heading"]').text()).toContain('explicit action')
    expect(wrapper.get('[aria-labelledby="updates-heading"]').text()).toContain('separate, explicit action')
    expect(wrapper.get('.status').attributes('role')).toBe('status')
    expect(wrapper.findAll('button')).toHaveLength(0)
  })

  it('links the privacy disclosure from Settings', async () => {
    const wrapper = mount(Settings, {
      global: {
        stubs: {
          AppHeader: true,
          BaseButton: true,
          RouterLink: RouterLinkStub,
        },
      },
    })

    await flushPromises()

    const privacyLink = wrapper.findAllComponents(RouterLinkStub).find(link => link.props('to') === '/privacy')
    expect(privacyLink).toBeDefined()
    expect(privacyLink!.text()).toContain('Privacy & Updates')
  })
})
