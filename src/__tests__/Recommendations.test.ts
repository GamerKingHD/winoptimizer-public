import { flushPromises, mount, RouterLinkStub } from '@vue/test-utils'
import { afterEach, describe, expect, it } from 'vitest'
import { clearMocks, mockIPC } from '@tauri-apps/api/mocks'
import router from '../router'
import Recommendations from '../views/Recommendations.vue'

afterEach(() => clearMocks())

describe('Recommendations review route', () => {
  it('is registered as read-only', () => {
    const route = router.getRoutes().find(candidate => candidate.path === '/recommendations')

    expect(route?.name).toBe('Recommendations')
    expect(route?.meta.mutatesSystem).toBe(false)
  })

  it('fails closed and only links into normal review workflows', async () => {
    mockIPC(() => [])
    const wrapper = mount(Recommendations, {
      global: {
        stubs: {
          AppHeader: true,
          RouterLink: RouterLinkStub,
        },
      },
    })

    expect(wrapper.get('.recommendations-page').attributes('aria-busy')).toBe('true')
    await flushPromises()

    expect(wrapper.get('.recommendations-page').attributes('aria-busy')).toBe('false')
    expect(wrapper.text()).toContain('Review manually')
    expect(wrapper.text()).toContain('No machine-specific recommendation is available')
    expect(wrapper.text()).not.toContain('Recommended optimization')
    expect(wrapper.findAllComponents(RouterLinkStub).map(link => link.props('to'))).toEqual([
      '/disk-cleaner',
      '/startup-manager',
      '/system-tweaks',
    ])
    expect(wrapper.findAll('button')).toHaveLength(0)
  })

  it('shows mechanism, calibrated confidence, and provenance without direct actions', async () => {
    mockIPC((command) => {
      if (command !== 'list_tweak_presets') return []
      return [{
        id: 'privacy_essentials',
        name: 'Privacy essentials',
        description: 'Stable targets',
        target_ids: ['privacy:disable_telemetry'],
        finding: 'One stable constituent setting is not currently applied on this machine.',
        mechanism: 'Writes the selected Windows privacy policy registry values.',
        tradeoffs: ['Some convenience behavior may be reduced.'],
        confidence: 'unknown',
        provenance: {
          source_type: 'win_optimizer_deterministic_rule',
          source_name: 'WinOptimizer local system-tweak state',
          source_version: 'system-tweak-rules-1',
          scope: 'This Windows desktop machine',
          matching_rule: 'deterministic_rule',
          confidence: 'unknown',
          rationale: 'Exact local state read.',
          tradeoffs: ['Some convenience behavior may be reduced.'],
          conflict: 'none',
          last_bundled_update: null,
        },
      }]
    })

    const wrapper = mount(Recommendations, {
      global: {
        stubs: {
          AppHeader: true,
          RouterLink: RouterLinkStub,
        },
      },
    })
    await flushPromises()

    expect(wrapper.text()).toContain('Machine-aware review candidates')
    expect(wrapper.text()).toContain('Writes the selected Windows privacy policy registry values.')
    expect(wrapper.text()).toContain('Review manually — no machine-specific benefit measured')
    expect(wrapper.text()).toContain('WinOptimizer local system-tweak state')
    expect(wrapper.text()).toContain('Exact local state read.')
    expect(wrapper.text()).not.toContain('Recommended optimization')
    expect(wrapper.findAll('button')).toHaveLength(0)
    expect(wrapper.findAllComponents(RouterLinkStub).map(link => link.props('to'))).toContain('/system-tweaks')
  })
})
