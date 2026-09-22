import { mount } from '@vue/test-utils'
import { describe, expect, it } from 'vitest'
import BaseTabBar from '../components/BaseTabBar.vue'

describe('BaseTabBar accessibility', () => {
  it('exposes tabs and their selected state to assistive technology', async () => {
    const wrapper = mount(BaseTabBar, {
      props: {
        tabs: [
          { id: 'first', label: 'First' },
          { id: 'second', label: 'Second' },
        ],
        modelValue: 'first',
      },
    })

    expect(wrapper.get('[role="tablist"]').attributes('aria-label')).toBe('Sections')
    const tabs = wrapper.findAll('[role="tab"]')
    expect(tabs[0].attributes('type')).toBe('button')
    expect(tabs[0].attributes('aria-selected')).toBe('true')
    expect(tabs[1].attributes('aria-selected')).toBe('false')

    await tabs[1].trigger('click')
    expect(wrapper.emitted('update:modelValue')?.[0]).toEqual(['second'])

    await tabs[0].trigger('keydown', { key: 'ArrowRight' })
    expect(wrapper.emitted('update:modelValue')?.[1]).toEqual(['second'])
  })
})
