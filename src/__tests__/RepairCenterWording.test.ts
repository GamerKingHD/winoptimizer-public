import { flushPromises, mount } from '@vue/test-utils'
import { KeepAlive, defineComponent, h } from 'vue'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import ConfirmDialog from '../components/ConfirmDialog.vue'
import RepairCenter from '../views/RepairCenter.vue'

const mocks = vi.hoisted(() => ({
  listRepairTargets: vi.fn(),
  listRepairGuides: vi.fn(),
  createRepairPlan: vi.fn(),
  executeRepairPlan: vi.fn(),
  checkElevated: vi.fn(),
  confirmAndRequestElevation: vi.fn(),
}))

vi.mock('../api/repair', () => ({
  listRepairTargets: mocks.listRepairTargets,
  listRepairGuides: mocks.listRepairGuides,
  createRepairPlan: mocks.createRepairPlan,
  executeRepairPlan: mocks.executeRepairPlan,
}))

vi.mock('../api/operation', () => ({
  confirmAndRequestElevation: mocks.confirmAndRequestElevation,
}))

vi.mock('../api/tauri', () => ({
  checkElevated: mocks.checkElevated,
}))

vi.mock('vue-router', () => ({
  useRoute: () => ({ meta: {} }),
  useRouter: () => ({ push: vi.fn(), resolve: () => ({ matched: [] }) }),
}))

// Mirrors the production `sfc_scan` metadata emitted by the Rust catalogue.
const sfcVerificationMethod =
  'The repair command exits 0 and a controlled follow-up `sfc /verifyonly` pass also exits 0. ' +
  'This proves the verification pass completed without reporting failure; Windows does not ' +
  'document SFC exit codes, so it does not prove that any protected file was repaired.'

const sfcAction = {
  id: 'sfc_scan',
  category: 'System Files',
  name: 'Check protected system files',
  description:
    'Runs System File Checker (sfc /scannow) to scan protected Windows files and attempt ' +
    'repair when possible. Completion means the command and its configured verification step ' +
    'finished without the failure signal WinOptimizer checks; it does not prove that specific ' +
    'files were repaired or that system integrity was restored.',
  command: 'sfc',
  args: ['/scannow'],
  estimated_minutes: 30,
  requires_admin: true,
  mutates_system: true,
  restart_required: false,
  risk: 'medium',
  limitations: 'Completion does not prove files were repaired or integrity restored.',
  experimental: false,
  execution_available: true,
  verification_method: sfcVerificationMethod,
  verification_scope: 'health_postcondition',
}

const plan = {
  id: 'repair-plan',
  descriptor: {
    id: 'repair.sfc_scan',
    title: 'System File Checker scan',
    category: 'repair',
    risk: 'medium',
    privilege: 'standard_user',
    reversibility: 'not_reversible',
    reboot_policy: 'not_required',
    experimental: false,
    proof_status: 'experimental',
    exposure: 'labs',
    recovery: 'none',
    capability_id: 'repair.maintenance',
    mutates_system: true,
    requires_confirmation: true,
    requires_revalidation: true,
    requires_verification: true,
    requires_journal: true,
  },
  targets: [
    { target_id: 'sfc_scan', requested_action: '{}', description: 'Run sfc /verifyonly' },
  ],
  warnings: [],
  expected_effects: [],
  requires_elevation: false,
  requires_restart: false,
  reversibility: 'not_reversible',
}

describe('Repair Center verification wording', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    mocks.listRepairTargets.mockResolvedValue([sfcAction])
    mocks.listRepairGuides.mockResolvedValue([
      {
        id: 'windows_integrity',
        title: 'Windows integrity',
        symptom: 'System files may be damaged',
        description: 'Run a verification check.',
        evidence_action_ids: ['sfc_scan'],
        related_action_ids: [],
        limitations: 'Verification only.',
      },
    ])
    mocks.createRepairPlan.mockResolvedValue(plan)
    mocks.executeRepairPlan.mockResolvedValue({
      result: {
        operation_id: plan.id,
        requested: 1,
        changed: 1,
        unchanged: 0,
        skipped: 0,
        failed: 0,
        verified: 1,
        unverified: 0,
        outcomes: [],
        reboot_required: false,
        undo: 'not_reversible',
      },
      log: {
        id: 'log',
        action_id: 'sfc_scan',
        command: 'sfc',
        args: ['/verifyonly'],
        started_at: 'start',
        ended_at: 'end',
        exit_code: 0,
        stdout: '',
        stderr: '',
        requires_admin: true,
        mutates_system: true,
        restart_required: false,
      },
      verification_message: sfcVerificationMethod,
    })
  })

  it('does not claim a repaired postcondition for an sfc verification-only action', async () => {
    const host = defineComponent({
      render: () => h(KeepAlive, null, { default: () => h(RepairCenter) }),
    })
    const wrapper = mount(host)
    await flushPromises()

    const reviewButton = wrapper.findAll('button').find((button) => button.text().includes('Review & Collect'))
    expect(reviewButton).toBeDefined()
    await reviewButton!.trigger('click')
    await flushPromises()

    wrapper.findComponent(ConfirmDialog).vm.$emit('confirm')
    await flushPromises()

    expect(wrapper.text()).toContain('System File Checker completed; verification step succeeded')
    expect(wrapper.text()).toContain('Verification step completed')
    expect(wrapper.text()).not.toContain('postcondition verified')
    for (const overclaim of ['repaired successfully', 'integrity restored']) {
      expect(wrapper.text().toLowerCase()).not.toContain(overclaim)
    }
    expect(mocks.executeRepairPlan).toHaveBeenCalledTimes(1)
  })
})
