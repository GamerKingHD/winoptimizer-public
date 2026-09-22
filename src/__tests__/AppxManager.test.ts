import { defineComponent } from 'vue'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { flushPromises, mount } from '@vue/test-utils'

const mocks = vi.hoisted(() => ({
  listAppxCandidates: vi.fn(),
  getAppxPolicyStatus: vi.fn(),
  checkElevated: vi.fn(),
  createAppxPlan: vi.fn(),
  executeAppxPlan: vi.fn(),
}))

vi.mock('../api/tauri', () => ({
  listAppxCandidates: mocks.listAppxCandidates,
  getAppxPolicyStatus: mocks.getAppxPolicyStatus,
  checkElevated: mocks.checkElevated,
}))

vi.mock('../api/appx', () => ({
  createAppxPlan: mocks.createAppxPlan,
  executeAppxPlan: mocks.executeAppxPlan,
}))

import AppxManager from '../views/AppxManager.vue'

const familyName = 'Microsoft.BingNews_8wekyb3d8bbwe'
const plan = {
  id: 'plan-1',
  descriptor: {},
  targets: [{
    target_id: familyName,
    requested_action: 'mark',
    description: 'Mark News for removal',
  }],
  warnings: [],
  expected_effects: [],
  requires_elevation: true,
  requires_restart: false,
  reversibility: 'not_reversible',
} as never

const BaseButtonStub = defineComponent({
  props: { disabled: Boolean },
  emits: ['click'],
  template: '<button :disabled="disabled" @click="$emit(\'click\')"><slot /></button>',
})

const ConfirmDialogStub = defineComponent({
  props: {
    open: Boolean,
    items: { type: Array, default: () => [] },
    confirmLabel: { type: String, default: '' },
  },
  template: `
    <div v-if="open" data-test="appx-confirm-dialog">
      <span v-for="item in items" :key="item.label">{{ item.label }} {{ item.value }}</span>
      <button>{{ confirmLabel }}</button>
    </div>
  `,
})

const AppxHost = defineComponent({
  components: { AppxManager },
  template: '<KeepAlive><AppxManager /></KeepAlive>',
})

describe('AppxManager', () => {
  beforeEach(() => {
    mocks.listAppxCandidates.mockResolvedValue([{
      display_name: 'News',
      family_name: familyName,
      is_protected: false,
      is_marked_for_removal: false,
      installed_package_full_name: `${familyName}_1.0.0.0_neutral__test`,
      risk: 'safe',
    }])
    mocks.getAppxPolicyStatus.mockResolvedValue(false)
    mocks.createAppxPlan.mockResolvedValue(plan)
  })

  afterEach(() => {
    vi.clearAllMocks()
  })

  it('opens the review dialog with the selected package and confirmation action', async () => {
    const wrapper = mount(AppxHost, {
      global: {
        stubs: {
          AppHeader: true,
          BaseCard: { template: '<div><slot /></div>' },
          BaseButton: BaseButtonStub,
          BaseBadge: { template: '<span><slot /></span>' },
          ResultsBanner: true,
          EmptyState: true,
          ConfirmDialog: ConfirmDialogStub,
        },
      },
    })

    await flushPromises()
    await wrapper.find('input[type="checkbox"]').trigger('change')
    await wrapper.findAll('button').find(button => button.text().includes('Mark Selected'))!.trigger('click')
    await flushPromises()

    const dialog = wrapper.findComponent(ConfirmDialogStub)
    expect(mocks.createAppxPlan).toHaveBeenCalledWith({ kind: 'mark', family_names: [familyName] })
    expect(dialog.props('open')).toBe(true)
    expect(dialog.text()).toContain('Mark News for removal')
    expect(dialog.text()).toContain('Mark 1 for removal')

    wrapper.unmount()
  })
})
