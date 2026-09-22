import { mount } from '@vue/test-utils'
import { describe, expect, it } from 'vitest'
import BaseTable from '../components/BaseTable.vue'

describe('BaseTable accessibility and sorting', () => {
  it('exposes sortable headers as keyboard-accessible buttons with sort state', async () => {
    const wrapper = mount(BaseTable, {
      props: {
        columns: [
          { key: 'name', label: 'Name', sortable: true },
          { key: 'status', label: 'Status' },
        ],
        rows: [
          { _id: 'b', name: 'Beta', status: 'Ready' },
          { _id: 'a', name: 'Alpha', status: 'Ready' },
        ],
      },
    })

    const nameHeader = wrapper.find('th')
    const sortButton = nameHeader.find('button')
    expect(sortButton.attributes('type')).toBe('button')
    expect(sortButton.attributes('aria-label')).toBe('Sort by Name')
    expect(nameHeader.attributes('aria-sort')).toBe('none')

    await sortButton.trigger('click')
    expect(nameHeader.attributes('aria-sort')).toBe('ascending')
    expect(wrapper.find('tbody tr td').text()).toBe('Alpha')

    await sortButton.trigger('click')
    expect(nameHeader.attributes('aria-sort')).toBe('descending')
    expect(wrapper.find('tbody tr td').text()).toBe('Beta')
  })

  it('announces loading state to assistive technology', () => {
    const wrapper = mount(BaseTable, {
      props: {
        columns: [{ key: 'name', label: 'Name' }],
        rows: [],
        loading: true,
      },
    })

    expect(wrapper.get('.table-container').attributes('aria-busy')).toBe('true')
    expect(wrapper.get('.table-loading').attributes('role')).toBe('status')
  })
})
