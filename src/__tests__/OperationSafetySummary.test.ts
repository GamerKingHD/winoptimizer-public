import { mount } from '@vue/test-utils'
import { describe, expect, it } from 'vitest'
import OperationPreview from '../components/OperationPreview.vue'
import type { OperationPlan } from '../api/operation'

const plan: OperationPlan = {
  id: 'operation-preview-1',
  descriptor: {
    id: 'cleanup.preview',
    title: 'Preview cleanup',
    category: 'cleanup',
    risk: 'medium',
    privilege: 'administrator',
    reversibility: 'not_reversible',
    reboot_policy: 'may_be_required',
    experimental: false,
    proof_status: 'partially_proven',
    exposure: 'core',
    recovery: 'none',
    capability_id: 'cleanup.preview',
    mutates_system: true,
    requires_confirmation: true,
    requires_revalidation: true,
    requires_verification: true,
    requires_journal: true,
  },
  targets: [
    { target_id: 'C:\\Temp\\one.tmp', requested_action: 'remove', description: 'Remove one temporary file' },
    { target_id: 'C:\\Temp\\two.tmp', requested_action: 'remove', description: 'Remove another temporary file' },
  ],
  warnings: [{ code: 'target_changed_since_preview', message: 'Targets are checked again before execution.' }],
  expected_effects: [{ target_id: 'C:\\Temp\\one.tmp', description: 'Temporary files will be removed.' }],
  requires_elevation: true,
  requires_restart: false,
  reversibility: 'not_reversible',
}

describe('OperationPreview', () => {
  it('exposes structured safety facts and exact targets without changing the plan', () => {
    const before = JSON.stringify(plan)
    const wrapper = mount(OperationPreview, { props: { plan } })

    expect(wrapper.get('h2').text()).toBe('Exact operation preview')
    expect(wrapper.text()).toContain('What changes')
    expect(wrapper.text()).toContain('Temporary files will be removed.')
    expect(wrapper.text()).toContain('Target count')
    expect(wrapper.text()).toContain('2')
    expect(wrapper.text()).toContain('Administrator approval')
    expect(wrapper.text()).toContain('May be required')
    expect(wrapper.text()).toContain('No automatic recovery')
    expect(wrapper.text()).toContain('Partially proven')
    expect(wrapper.text()).toContain('Targets are checked again before execution.')
    expect(wrapper.text()).toContain('C:\\Temp\\one.tmp')
    expect(wrapper.find('[aria-labelledby$="-warnings"]').exists()).toBe(true)
    expect(wrapper.findAll('li')).toHaveLength(4)
    expect(JSON.stringify(plan)).toBe(before)
  })

  it('falls back to target descriptions when expected effects are not declared', () => {
    const wrapper = mount(OperationPreview, {
      props: {
        plan: {
          ...plan,
          expected_effects: [],
          warnings: [],
        },
      },
    })

    expect(wrapper.text()).toContain('Remove one temporary file')
    expect(wrapper.text()).toContain('Warnings')
    expect(wrapper.text()).toContain('None')
  })

  it('exposes verification gates and evidence status for high-impact confirmations', () => {
    const wrapper = mount(OperationPreview, {
      props: {
        plan: {
          ...plan,
          descriptor: { ...plan.descriptor, risk: 'high' },
        },
      },
    })

    const factLabels = wrapper.findAll('.safety-facts dt').map((element) => element.text())
    expect(factLabels).toContain('Verification')
    expect(factLabels).toContain('Evidence')
    expect(wrapper.text()).toContain('Required before and after execution')
    expect(wrapper.text()).toContain('Partially proven')
  })

  it('paginates large exact target lists without changing the plan', async () => {
    const largePlan = {
      ...plan,
      targets: Array.from({ length: 55 }, (_, index) => ({
        target_id: `C:\\Temp\\target-${index}.tmp`,
        requested_action: 'remove',
        description: `Temporary file ${index}`,
      })),
    }
    const before = JSON.stringify(largePlan)
    const wrapper = mount(OperationPreview, { props: { plan: largePlan } })

    expect(wrapper.findAll('.target-list li')).toHaveLength(50)
    expect(wrapper.text()).toContain('Showing 1–50 of 55')
    await wrapper.get('button[aria-label="Next target page"]').trigger('click')
    expect(wrapper.findAll('.target-list li')).toHaveLength(5)
    expect(wrapper.text()).toContain('Showing 51–55 of 55')
    expect(JSON.stringify(largePlan)).toBe(before)
  })

  it('bounds summary evidence while retaining the full target count', () => {
    const largePlan = {
      ...plan,
      expected_effects: Array.from({ length: 60 }, (_, index) => ({
        target_id: `C:\\Temp\\target-${index}.tmp`,
        description: `Temporary file ${index} will be removed.`,
      })),
    }
    const wrapper = mount(OperationPreview, { props: { plan: largePlan } })

    expect(wrapper.text()).toContain('Target count')
    expect(wrapper.text()).toContain('... and 37 more declared changes')
    expect(wrapper.findAll('.evidence-list')[0].findAll('li')).toHaveLength(24)
  })

  it('resets the target page when a different operation plan replaces the current one', async () => {
    const largePlan = {
      ...plan,
      id: 'operation-preview-large',
      targets: Array.from({ length: 55 }, (_, index) => ({
        target_id: `C:\\Temp\\target-${index}.tmp`,
        requested_action: 'remove',
        description: `Temporary file ${index}`,
      })),
    }
    const wrapper = mount(OperationPreview, { props: { plan: largePlan } })

    await wrapper.get('button[aria-label="Next target page"]').trigger('click')
    expect(wrapper.text()).toContain('Showing 51–55 of 55')
    await wrapper.setProps({ plan: { ...largePlan, id: 'operation-preview-replacement' } })

    expect(wrapper.text()).toContain('Showing 1–50 of 55')
  })
})
