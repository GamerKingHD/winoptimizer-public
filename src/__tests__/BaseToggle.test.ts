import { mount } from '@vue/test-utils'
import { describe, expect, it } from 'vitest'
import BaseToggle from '../components/BaseToggle.vue'

describe('BaseToggle accessibility', () => {
  it('provides an accessible name for an unlabeled switch', () => {
    const wrapper = mount(BaseToggle, {
      props: {
        modelValue: false,
        ariaLabel: 'Apply Privacy baseline',
      },
    })

    expect(wrapper.get('input[role="switch"]').attributes('aria-label')).toBe('Apply Privacy baseline')
  })
})
