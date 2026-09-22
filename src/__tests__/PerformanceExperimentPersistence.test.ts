import { afterEach, describe, expect, it } from 'vitest'
import { clearMocks, mockIPC } from '@tauri-apps/api/mocks'
import {
  listPerformanceExperiments,
  preparePerformanceExperimentRevert,
  recordPerformanceExperimentDecision,
  savePerformanceExperiment,
  type PerformanceExperiment,
} from '../api/performanceExperiments'

const experiment = {
  id: 'experiment-1',
  operationId: 'operation-1',
  name: 'Fixture experiment',
  hypothesis: 'The workload should improve.',
  createdAt: '2026-08-11T10:00:00.000Z',
  appliedAt: '2026-08-11T10:02:00.000Z',
  rebootRequired: true,
  resumedAt: '2026-08-11T10:03:00.000Z',
  metricDefinitions: [],
  baseline: [{ metrics: { workload: 100 } }],
  after: [{ metrics: { workload: 103 } }],
  state: 'compared',
  comparison: { metrics: [{ conclusion: 'measurable_improvement' }] },
  decision: 'keep',
} as unknown as PerformanceExperiment

describe('performance experiment persistence API', () => {
  afterEach(() => clearMocks())

  it('persists and reloads the complete evidence document through Tauri', async () => {
    let stored: PerformanceExperiment[] = []
    const calls: string[] = []
    mockIPC((command, args) => {
      calls.push(command)
      if (command === 'save_performance_experiment') {
        const saved = (args as { experiment?: PerformanceExperiment } | undefined)?.experiment
        stored = [saved as PerformanceExperiment]
        return saved
      }
      if (command === 'list_performance_experiments') return stored
      return undefined
    })

    await savePerformanceExperiment(experiment)
    const loaded = await listPerformanceExperiments()

    expect(calls).toEqual(['save_performance_experiment', 'list_performance_experiments'])
    expect(loaded[0].baseline[0].metrics.workload).toBe(100)
    expect(loaded[0].after[0].metrics.workload).toBe(103)
    expect(loaded[0].comparison?.metrics[0].conclusion).toBe('measurable_improvement')
    expect(loaded[0].operationId).toBe('operation-1')
    expect(loaded[0].appliedAt).toBe('2026-08-11T10:02:00.000Z')
    expect(loaded[0].rebootRequired).toBe(true)
    expect(loaded[0].resumedAt).toBe('2026-08-11T10:03:00.000Z')
    expect(loaded[0].operationEvidence).toBeNull()
    expect(loaded[0].decisionOperationId).toBeNull()
    expect(loaded[0].decisionEvidence).toBeNull()
  })

  it('normalizes legacy records without apply metadata when resuming after restart', async () => {
    mockIPC((command) => {
      if (command === 'list_performance_experiments') {
        return [{ ...experiment, appliedAt: undefined, rebootRequired: undefined, resumedAt: undefined }]
      }
      return undefined
    })

    const loaded = await listPerformanceExperiments()

    expect(loaded[0].appliedAt).toBeNull()
    expect(loaded[0].rebootRequired).toBe(false)
    expect(loaded[0].resumedAt).toBeNull()
  })

  it('uses a normal operation-plan preview and journaled decision finalization boundary', async () => {
    const calls: Array<{ command: string; args: unknown }> = []
    mockIPC((command, args) => {
      calls.push({ command, args })
      if (command === 'prepare_performance_experiment_revert') return { id: 'revert-plan-1' }
      if (command === 'record_performance_experiment_decision') {
        const experiment = (args as { experiment?: PerformanceExperiment } | undefined)?.experiment
        return { ...experiment, state: 'keep_recorded' }
      }
      return undefined
    })

    const plan = await preparePerformanceExperimentRevert('operation-1')
    const finalized = await recordPerformanceExperimentDecision({
      ...experiment,
      state: 'compared',
      decision: null,
    }, 'keep')

    expect(plan.id).toBe('revert-plan-1')
    expect(finalized.state).toBe('keep_recorded')
    expect(calls.map(call => call.command)).toEqual([
      'prepare_performance_experiment_revert',
      'record_performance_experiment_decision',
    ])
    expect((calls[0].args as { operationId: string }).operationId).toBe('operation-1')
    expect((calls[1].args as { decision: string; decisionOperationId: string | null }).decision).toBe('keep')
    expect((calls[1].args as { decisionOperationId: string | null }).decisionOperationId).toBeNull()
  })
})
