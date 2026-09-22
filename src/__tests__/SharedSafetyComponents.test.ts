import { mount } from '@vue/test-utils'
import { describe, expect, it } from 'vitest'
import EvidenceList from '../components/EvidenceList.vue'
import ExperimentalBanner from '../components/ExperimentalBanner.vue'
import LabsBanner from '../components/LabsBanner.vue'
import PrivilegeBadge from '../components/PrivilegeBadge.vue'
import ProofBadge from '../components/ProofBadge.vue'
import RecoveryBadge from '../components/RecoveryBadge.vue'
import RestartBadge from '../components/RestartBadge.vue'
import RiskBadge from '../components/RiskBadge.vue'
import TargetDiff from '../components/TargetDiff.vue'
import VerificationResult from '../components/VerificationResult.vue'

describe('shared operation safety components', () => {
  it('maps operation metadata to consistent risk, proof, recovery, privilege, and restart labels', () => {
    expect(mount(RiskBadge, { props: { risk: 'high' } }).text()).toContain('High risk')
    expect(mount(ProofBadge, { props: { proof: 'partially_proven' } }).text()).toContain('Partially proven')
    expect(mount(RecoveryBadge, { props: { recovery: 'exact' } }).text()).toContain('Exact recovery')
    expect(mount(PrivilegeBadge, { props: { privilege: 'administrator' } }).text()).toContain('Administrator approval')
    expect(mount(RestartBadge, { props: { policy: 'required' } }).text()).toContain('Required')
  })

  it('renders evidence and verification status without collapsing unverified results into success', () => {
    const evidence = mount(EvidenceList, {
      props: { items: [{ label: 'Post-state readback', status: 'verified' }, { label: 'VM acceptance', status: 'unverified' }] },
    })
    expect(evidence.text()).toContain('Post-state readback')
    expect(evidence.text()).toContain('unverified')
    expect(mount(VerificationResult, { props: { status: 'failed', message: 'Readback differed.' } }).text()).toContain('failed')
  })

  it('shows before/after summaries and changed properties in a target diff', () => {
    const wrapper = mount(TargetDiff, {
      props: {
        before: { summary: 'Disabled', properties: { state: 'disabled', start_type: 'manual' } },
        after: { summary: 'Enabled', properties: { state: 'enabled', start_type: 'manual' } },
      },
    })
    expect(wrapper.text()).toContain('Before: Disabled')
    expect(wrapper.text()).toContain('After: Enabled')
    expect(wrapper.text()).toContain('state')
    expect(wrapper.find('tr.changed').exists()).toBe(true)
  })
  it('provides explicit experimental and labs context banners', () => {
    expect(mount(ExperimentalBanner).text()).toContain('Experimental feature')
    expect(mount(LabsBanner).text()).toContain('Labs')
  })
})
