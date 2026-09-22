import { describe, expect, it } from 'vitest'
import {
  beginAfterPhase,
  compareExperiment,
  createExperiment,
  markExperimentApplied,
  recordDecision,
  recordMeasurement,
  type PerformanceExperiment,
  type PerformanceMeasurement,
  resumeAfterReboot,
} from '../api/performanceExperiments'

const machine = {
  cpuName: 'Test CPU',
  architecture: 'x64',
  logicalProcessors: 16,
  memoryTotalBytes: 16_000,
}

function measurement(value: number, capturedAt: string): PerformanceMeasurement {
  return {
    capturedAt,
    source: 'hardware_process_snapshot',
    machine,
    metrics: { workload: value },
  }
}

function recordSamples(
  experiment: PerformanceExperiment,
  phase: 'baseline' | 'after',
  values: number[],
) {
  return values.reduce(
    (current, value, index) => recordMeasurement(current, phase, measurement(value, `${phase}-${index}`)),
    experiment,
  )
}

const workloadMetric = {
  id: 'workload',
  label: 'Workload throughput',
  unit: ' ops/s',
  lowerIsBetter: false,
  minimumMeaningfulEffect: 1,
}

describe('performance experiment lifecycle', () => {
  it('persists the apply and reboot-resume transition before collecting after samples', () => {
    const experiment = createExperiment({
      id: 'experiment-1',
      name: 'Fixture tweak',
      hypothesis: 'The workload should improve.',
      createdAt: '2026-08-10T10:00:00.000Z',
      metricDefinitions: [workloadMetric],
    })

    expect(experiment.operationId).toBeNull()

    const withBaseline = recordMeasurement(experiment, 'baseline', measurement(100, 'baseline-0'))
    expect(withBaseline.baseline).toHaveLength(1)
    expect(withBaseline.after).toHaveLength(0)
    expect(() => recordMeasurement(withBaseline, 'after', measurement(101, 'after-0'))).toThrow()

    const awaitingReboot = markExperimentApplied(withBaseline, true, '2026-08-10T10:00:30.000Z')
    expect(awaitingReboot.state).toBe('awaiting_reboot')
    expect(awaitingReboot.appliedAt).toBe('2026-08-10T10:00:30.000Z')
    expect(awaitingReboot.rebootRequired).toBe(true)
    expect(() => recordMeasurement(awaitingReboot, 'after', measurement(101, 'after-0'))).toThrow()

    const afterPhase = resumeAfterReboot(awaitingReboot, '2026-08-10T10:01:00.000Z')
    expect(afterPhase.state).toBe('awaiting_after')
    expect(afterPhase.resumedAt).toBe('2026-08-10T10:01:00.000Z')
    const completed = recordMeasurement(afterPhase, 'after', measurement(101, 'after-0'))
    expect(completed.baseline[0].metrics.workload).toBe(100)
    expect(completed.after[0].metrics.workload).toBe(101)
    expect(withBaseline.after).toHaveLength(0)
  })

  it('classifies a change below the practical threshold as no meaningful change', () => {
    let experiment = createExperiment({
      id: 'experiment-2',
      name: 'Noise fixture',
      hypothesis: 'The workload should improve.',
      metricDefinitions: [workloadMetric],
    })
    experiment = recordSamples(experiment, 'baseline', [100, 100, 100])
    experiment = beginAfterPhase(experiment)
    experiment = recordSamples(experiment, 'after', [100.1, 100.2, 100.3])

    const compared = compareExperiment(experiment, '2026-08-10T10:01:00.000Z')
    const result = compared.comparison!.metrics[0]
    expect(result.medianDelta).toBeCloseTo(0.2)
    expect(result.practicalSignificance).toBe('below_threshold')
    expect(result.conclusion).toBe('no_meaningful_change')
    expect(result.baseline.values).toEqual([100, 100, 100])
    expect(result.after.values).toEqual([100.1, 100.2, 100.3])
  })

  it('refuses directional claims for contextual system snapshot metrics', () => {
    let experiment = createExperiment({
      id: 'experiment-3',
      name: 'Context fixture',
      hypothesis: 'The workload should change.',
      metricDefinitions: [{
        id: 'workload',
        label: 'Workload observation',
        unit: ' units',
        lowerIsBetter: null,
        minimumMeaningfulEffect: 2,
      }],
    })
    experiment = recordSamples(experiment, 'baseline', [20, 20, 20])
    experiment = beginAfterPhase(experiment)
    experiment = recordSamples(experiment, 'after', [30, 30, 30])

    const compared = compareExperiment(experiment)
    const result = compared.comparison!.metrics[0]
    expect(result.conclusion).toBe('inconclusive_noisy_measurement')
    expect(result.caveat).toContain('direction is undefined')
    expect(compared.comparison!.caveats[0]).toContain('not proof of a faster system')
  })

  it('records Keep, Revert, and Unknown as decisions without executing a mutation', () => {
    let experiment = createExperiment({
      id: 'experiment-4',
      name: 'Decision fixture',
      hypothesis: 'The workload should improve.',
      metricDefinitions: [workloadMetric],
    })
    experiment = recordSamples(experiment, 'baseline', [100, 100, 100])
    experiment = beginAfterPhase(experiment)
    experiment = recordSamples(experiment, 'after', [103, 103, 103])
    const compared = compareExperiment(experiment)

    expect(recordDecision(compared, 'keep').state).toBe('keep_recorded')
    expect(recordDecision(compared, 'revert').state).toBe('revert_requested')
    expect(recordDecision(compared, 'unknown').state).toBe('unknown')
  })

  it('retains an explicitly supplied journal operation link as evidence metadata', () => {
    const experiment = createExperiment({
      name: 'Linked fixture',
      hypothesis: 'The reviewed operation may affect the workload.',
      operationId: 'operation-123',
    })

    expect(experiment.operationId).toBe('operation-123')
  })
})
