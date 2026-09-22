import { mount } from '@vue/test-utils'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import AppHeader from '../components/AppHeader.vue'
import AppSidebar from '../components/AppSidebar.vue'
import Tools from '../views/Tools.vue'

const mocks = vi.hoisted(() => ({
  checkElevated: vi.fn(),
  push: vi.fn(),
  route: { path: '/', meta: { title: 'Preinstalled Apps Policy', subtitle: 'Current route metadata' } },
}))

vi.mock('../api/tauri', () => ({
  checkElevated: mocks.checkElevated,
}))

vi.mock('vue-router', () => ({
  useRoute: () => mocks.route,
  useRouter: () => ({ push: mocks.push, resolve: () => ({ matched: [] }) }),
}))

describe('Phase 52 naming accuracy', () => {
  beforeEach(() => {
    mocks.checkElevated.mockResolvedValue(true)
  })

  it('describes the AppX workflow as a preinstalled-app policy and keeps its route', () => {
    const wrapper = mount(Tools, {
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

    expect(wrapper.text()).toContain('Preinstalled Apps Policy')
    expect(wrapper.text()).not.toContain('AppX Manager')
    expect(wrapper.find('a[href="/appx-manager"]').exists()).toBe(true)
  })

  it('describes history and undo without a general safety claim', () => {
    const wrapper = mount(AppSidebar)

    expect(wrapper.text()).toContain('History & Undo')
    expect(wrapper.text()).not.toContain('Safety & History')
  })
})

describe('Phase 53 factual header states', () => {
  it('allows a view to replace broader route metadata with an exact label', () => {
    const wrapper = mount(AppHeader, {
      props: {
        title: 'Preinstalled Apps Policy',
        subtitle: 'Review the policy for pre-installed Windows apps before the next sign-in',
      },
    })

    expect(wrapper.get('h1').text()).toBe('Preinstalled Apps Policy')
    expect(wrapper.get('.page-subtitle').text()).toContain('before the next sign-in')
  })
})
