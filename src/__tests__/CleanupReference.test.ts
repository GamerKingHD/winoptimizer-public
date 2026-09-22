import { flushPromises, mount } from '@vue/test-utils'
import { nextTick } from 'vue'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import CleanupReference from '../views/CleanupReference.vue'
import type { CleanupScan } from '../api/cleanup'
import type { OperationPlan } from '../api/operation'

const mocks = vi.hoisted(() => ({
  scan: vi.fn(),
  createPlan: vi.fn(),
  execute: vi.fn(),
  cancel: vi.fn(),
  checkElevated: vi.fn(),
  elevate: vi.fn(),
}))

vi.mock('../api/cleanup', () => ({
  scanCleanupTargets: mocks.scan,
  createCleanupPlan: mocks.createPlan,
  executeCleanupPlan: mocks.execute,
  cancelCleanup: mocks.cancel,
}))

vi.mock('../api/tauri', () => ({
  checkElevated: mocks.checkElevated,
}))

vi.mock('../api/operation', () => ({
  confirmAndRequestElevation: mocks.elevate,
}))

const target = {
  target_id: 'c:\\temp\\safe.tmp',
  category: 'temp',
  resource_id: 'C:\\Temp\\safe.tmp',
  description: 'Safe temporary file',
  size_bytes: 128,
  last_modified: '1',
  confidence: 'high',
  risk: 'low' as const,
  recommended: true,
  recommendation_reason: 'Temporary files are the only clearly safe default.',
  accessibility: 'accessible',
  reversibility: 'not_reversible' as const,
  affected_application: null,
  group_name: 'User temporary files',
  cleanup_type: 'Root',
  requires_elevation: false,
}

const browserTarget = {
  ...target,
  target_id: 'c:\\browser\\cache.bin',
  category: 'browser_cache',
  resource_id: 'C:\\Browser\\cache.bin',
  description: 'Browser cache file',
  size_bytes: 256,
  risk: 'medium' as const,
  recommended: false,
  recommendation_reason: 'May affect signed-in applications; select explicitly.',
  affected_application: 'Example Browser',
  group_name: 'Example Browser',
  cleanup_type: 'Cache',
}

const scan: CleanupScan = {
  scan_id: 'scan-1',
  scanned_at: '2026-07-27T12:00:00Z',
  targets: [target],
  estimated_bytes: 128,
  free_space_before: 10_000,
  failures: [],
}

const plan: OperationPlan = {
  id: 'cleanup-1',
  descriptor: {
    id: 'cleanup.reference_slice',
    title: 'Clean Up selected targets',
    category: 'cleanup',
    risk: 'low',
    privilege: 'standard_user',
    reversibility: 'not_reversible',
    reboot_policy: 'not_required',
    experimental: false,
    proof_status: 'partially_proven',
    exposure: 'core',
    recovery: 'none',
    capability_id: 'cleanup.temp',
    mutates_system: true,
    requires_confirmation: true,
    requires_revalidation: true,
    requires_verification: true,
    requires_journal: true,
  },
  targets: [{ target_id: target.target_id, requested_action: '{}', description: target.description }],
  warnings: [],
  expected_effects: [],
  requires_elevation: false,
  requires_restart: false,
  reversibility: 'not_reversible',
}

function mountView() {
  return mount(CleanupReference, {
    global: {
      stubs: { AppHeader: true, teleport: true },
    },
  })
}

async function startScan(wrapper: ReturnType<typeof mountView>) {
  const scanButton = wrapper.findAll('button').find(button => /Scan/.test(button.text()))
  expect(scanButton).toBeTruthy()
  await scanButton!.trigger('click')
}

beforeEach(() => {
  vi.clearAllMocks()
  mocks.scan.mockResolvedValue(scan)
  mocks.createPlan.mockResolvedValue(plan)
  mocks.checkElevated.mockResolvedValue(false)
})

describe('Clean Up reference slice', () => {
  it('stays idle until Scan, then selects only recommended targets', async () => {
    let resolveScan!: (value: CleanupScan) => void
    mocks.scan.mockReturnValue(new Promise(resolve => { resolveScan = resolve }))
    const wrapper = mountView()
    await nextTick()

    expect(mocks.scan).not.toHaveBeenCalled()
    expect(wrapper.text()).toContain('Scan to review cleanable files')
    expect(wrapper.text()).not.toContain('Scanning cleanable targets')

    await startScan(wrapper)
    await nextTick()
    expect(wrapper.text()).toContain('Scanning cleanable targets')
    resolveScan(scan)
    await flushPromises()

    expect(wrapper.text()).toContain('Choose what to clean')
    expect(wrapper.text()).toContain('Temporary files')
    expect(wrapper.text()).toContain('low risk')
    expect(wrapper.text()).toContain('1 selected')
    expect(wrapper.find('input[type="checkbox"]').exists()).toBe(false)

    await wrapper.get('.category-details-button').trigger('click')
    expect(wrapper.get('input[type="checkbox"]').element).toHaveProperty('checked', true)
  })

  it('groups popular-software findings by program and cleanup type', async () => {
    mocks.scan.mockResolvedValueOnce({
      ...scan,
      targets: [target, browserTarget],
      estimated_bytes: 384,
    })
    const wrapper = mountView()
    await startScan(wrapper)
    await flushPromises()

    const categories = wrapper.findAll('.category-card')
    expect(categories).toHaveLength(2)
    expect(categories[1].text()).toContain('Temporary files')
    expect(categories[1].text()).toContain('Recommended')
    expect(wrapper.text()).toContain('Popular software')
    expect(wrapper.text()).toContain('Cookies and saved sign-in data are not scanned')
    expect(categories[0].text()).toContain('Example Browser')
    expect(categories[0].text()).toContain('Browser cache')
    expect(categories[0].text()).toContain('Cache')
    expect(categories[0].text()).toContain('1 file found')
    expect(categories[0].text()).toContain('Not selected')
    expect(wrapper.text()).not.toContain(browserTarget.resource_id)

    await categories[0].get('.category-select').trigger('click')
    expect(categories[0].text()).toContain('1 selected')
    expect(wrapper.get('button.variant-accent').text()).toContain('Review 2 selected')

    await categories[0].get('.category-details-button').trigger('click')
    expect(wrapper.text()).toContain(browserTarget.resource_id)
  })

  it('renders scan failure and empty scan states', async () => {
    mocks.scan.mockRejectedValueOnce(new Error('scan unavailable'))
    const failure = mountView()
    await startScan(failure)
    await flushPromises()
    expect(failure.text()).toContain('scan unavailable')

    mocks.scan.mockResolvedValueOnce({ ...scan, targets: [], estimated_bytes: 0 })
    const empty = mountView()
    await startScan(empty)
    await flushPromises()
    expect(empty.text()).toContain('No cleanable targets found')
  })

  it('creates the immutable selection, confirms it, and renders reconciled partial results', async () => {
    mocks.execute.mockResolvedValue({
      result: {
        operation_id: plan.id,
        requested: 1,
        changed: 0,
        unchanged: 0,
        skipped: 0,
        failed: 1,
        verified: 0,
        unverified: 1,
        outcomes: [{
          target_id: target.target_id,
          requested_action: 'remove',
          before: null,
          after: null,
          execution: 'failed',
          verification: 'failed',
          error: {
            code: 'execution_failed',
            message: 'file is locked',
            target_id: target.target_id,
            retryable: true,
          },
        }],
        reboot_required: false,
        undo: 'not_reversible',
      },
      estimated_bytes: 128,
      measured_disk_space_delta: 0,
      journal_id: 'journal-1',
    })
    const wrapper = mountView()
    await startScan(wrapper)
    await flushPromises()

    await wrapper.get('button.variant-accent').trigger('click')
    await flushPromises()
    expect(mocks.createPlan).toHaveBeenCalledWith(scan, [target.target_id], 'recycle_bin')
    expect(wrapper.text()).toContain('Confirm exact cleanup plan')
    expect(wrapper.text()).toContain('Cleanup operation preview')
    expect(wrapper.text()).toContain('What changes')
    expect(wrapper.text()).toContain('Target count')
    expect(wrapper.text()).toContain('Recovery')

    const confirm = wrapper.findAll('button').find(button => button.text().includes('Move 1 targets to Recycle Bin'))
    expect(confirm).toBeTruthy()
    await confirm!.trigger('click')
    await flushPromises()

    expect(mocks.execute).toHaveBeenCalledWith(plan)
    expect(wrapper.text()).toContain('Measured cleanup result')
    expect(wrapper.text()).toContain('file is locked')
    expect(wrapper.text()).toContain('journal-1')
    expect(wrapper.text()).toContain('Unavailable')
  })

  it('continues an administrator plan through resumable elevation', async () => {
    mocks.createPlan.mockResolvedValueOnce({ ...plan, requires_elevation: true })
    mocks.elevate.mockResolvedValueOnce({
      operation_id: 'ticket-operation',
      plan_operation_id: plan.id,
      nonce: 'nonce',
      plan_digest: 'digest',
      authenticator: 'authenticator',
    })
    const wrapper = mountView()
    await startScan(wrapper)
    await flushPromises()

    await wrapper.get('button.variant-accent').trigger('click')
    await flushPromises()
    const confirm = wrapper.findAll('button').find(button => button.text().includes('Move 1 targets to Recycle Bin'))
    await confirm!.trigger('click')
    await flushPromises()

    expect(mocks.elevate).toHaveBeenCalled()
    expect(mocks.execute).not.toHaveBeenCalled()
    expect(wrapper.text()).toContain('Administrator approval requested')
  })
})
