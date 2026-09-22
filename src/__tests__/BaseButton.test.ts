import { describe, it, expect } from 'vitest'
import { mount } from '@vue/test-utils'
import BaseButton from '../components/BaseButton.vue'

describe('BaseButton', () => {
  it('renders default slot content', () => {
    const wrapper = mount(BaseButton, {
      slots: { default: 'Click me' },
    })
    expect(wrapper.text()).toContain('Click me')
  })

  it('applies variant class', () => {
    const wrapper = mount(BaseButton, {
      props: { variant: 'danger' },
    })
    expect(wrapper.classes()).toContain('variant-danger')
  })

  it('applies size class', () => {
    const wrapper = mount(BaseButton, {
      props: { size: 'lg' },
    })
    expect(wrapper.classes()).toContain('size-lg')
  })

  it('disables button when loading', () => {
    const wrapper = mount(BaseButton, {
      props: { loading: true },
    })
    const btn = wrapper.find('button')
    expect(btn.attributes('disabled')).toBeDefined()
    expect(wrapper.classes()).toContain('is-loading')
  })

  it('disables button when disabled prop is true', () => {
    const wrapper = mount(BaseButton, {
      props: { disabled: true },
    })
    expect(wrapper.find('button').attributes('disabled')).toBeDefined()
  })

  it('emits click on button press', async () => {
    const wrapper = mount(BaseButton, {
      slots: { default: 'Go' },
    })
    await wrapper.find('button').trigger('click')
    expect(wrapper.emitted('click')).toBeTruthy()
  })

  it('shows spinner icon when loading', () => {
    const wrapper = mount(BaseButton, {
      props: { loading: true },
    })
    expect(wrapper.find('.btn-spinner').exists()).toBe(true)
  })

  it('shows SVG icon when icon prop is set', () => {
    const wrapper = mount(BaseButton, {
      props: { icon: 'trash' },
    })
    expect(wrapper.find('svg').exists()).toBe(true)
  })

  it('adds full-width class', () => {
    const wrapper = mount(BaseButton, {
      props: { fullWidth: true },
    })
    expect(wrapper.classes()).toContain('is-full-width')
  })
})
