import type { HardwareInfo, ProcessInfo } from './tauri'
import { invoke } from '@tauri-apps/api/core'
import type { OperationPlan, OperationState, StateSnapshot, TargetOutcome } from './operation'

export type ExperimentPhase = 'baseline' | 'after'
export type ExperimentDecision = 'keep' | 'revert' | 'unknown'
export type ExperimentLifecycle =
  | 'draft'
  | 'collecting_baseline'
  | 'awaiting_reboot'
  | 'awaiting_after'
  | 'collecting_after'
  | 'compared'
  | 'keep_recorded'
  | 'revert_requested'
  | 'unknown'

export type ExperimentConclusion =
  | 'measurable_improvement'
  | 'no_meaningful_change'
  | 'regression_detected'
  | 'inconclusive_noisy_measurement'

export interface PerformanceMetricDefinition {
  id: string
  label: string
  unit: string
  lowerIsBetter: boolean | null
  minimumMeaningfulEffect: number
}

export interface ExperimentMachineProfile {
  cpuName: string
  architecture: string
  logicalProcessors: number
  memoryTotalBytes: number
}

export interface PerformanceMeasurement {
  capturedAt: string
  source: 'hardware_process_snapshot'
  machine: ExperimentMachineProfile
  metrics: Record<string, number>
}

export interface SampleStatistics {
  values: number[]
  median: number | null
  variance: number | null
  standardDeviation: number | null
}

export type PracticalSignificance = 'meaningful' | 'below_threshold' | 'insufficient_data'

export interface MetricComparison {
  metricId: string
  label: string
  unit: string
  baseline: SampleStatistics
  after: SampleStatistics
  medianDelta: number | null
  relativeDeltaPercent: number | null
  noiseFloor: number | null
  minimumMeaningfulEffect: number
  practicalSignificance: PracticalSignificance
  conclusion: ExperimentConclusion
  caveat: string
}

export interface PerformanceComparison {
  comparedAt: string
  machineMatches: boolean
  metrics: MetricComparison[]
  caveats: string[]
}

export interface PerformanceTargetEvidence {
  targetId: string
  execution: TargetOutcome['execution']
  verification: TargetOutcome['verification']
  before: StateSnapshot | null
  after: StateSnapshot | null
  verificationMessage: string
}

export interface PerformanceOperationEvidence {
  operationId: string
  journalId: string
  descriptorId: string
  terminalState: OperationState
  capturedAt: string
  readbackVerified: boolean
  targets: PerformanceTargetEvidence[]
}

export interface PerformanceExperiment {
  id: string
  operationId: string | null
  operationEvidence: PerformanceOperationEvidence | null
  decisionOperationId: string | null
  decisionEvidence: PerformanceOperationEvidence | null
  name: string
  hypothesis: string
  createdAt: string
  appliedAt: string | null
  rebootRequired: boolean
  resumedAt: string | null
  metricDefinitions: PerformanceMetricDefinition[]
  baseline: PerformanceMeasurement[]
  after: PerformanceMeasurement[]
  state: ExperimentLifecycle
  comparison: PerformanceComparison | null
  decision: ExperimentDecision | null
}

export interface CreateExperimentInput {
  name: string
  hypothesis: string
  operationId?: string | null
  id?: string
  createdAt?: string
  metricDefinitions?: PerformanceMetricDefinition[]
}

export const SYSTEM_SNAPSHOT_METRICS: PerformanceMetricDefinition[] = [
  {
    id: 'cpu_usage_percent',
    label: 'CPU usage',
    unit: '%',
    lowerIsBetter: null,
    minimumMeaningfulEffect: 2,
  },
  {
    id: 'memory_used_percent',
    label: 'Memory in use',
    unit: '%',
    lowerIsBetter: null,
    minimumMeaningfulEffect: 2,
  },
  {
    id: 'running_processes',
    label: 'Running processes',
    unit: '',
    lowerIsBetter: null,
    minimumMeaningfulEffect: 2,
  },
]

export const MINIMUM_SAMPLES_PER_PHASE = 3

export const CONCLUSION_LABELS: Record<ExperimentConclusion, string> = {
  measurable_improvement: 'Measurable improvement',
  no_meaningful_change: 'No meaningful change',
  regression_detected: 'Regression detected',
  inconclusive_noisy_measurement: 'Inconclusive / noisy measurement',
}

function makeId() {
  return globalThis.crypto?.randomUUID?.() ?? `performance-experiment-${Date.now()}`
}

function normalizeExperiment(experiment: PerformanceExperiment): PerformanceExperiment {
  return {
    ...experiment,
    operationEvidence: experiment.operationEvidence ?? null,
    decisionOperationId: experiment.decisionOperationId ?? null,
    decisionEvidence: experiment.decisionEvidence ?? null,
    appliedAt: experiment.appliedAt ?? null,
    rebootRequired: experiment.rebootRequired ?? false,
    resumedAt: experiment.resumedAt ?? null,
  }
}

function median(values: number[]) {
  if (values.length === 0) return null
  const sorted = [...values].sort((left, right) => left - right)
  const middle = Math.floor(sorted.length / 2)
  return sorted.length % 2 === 0
    ? (sorted[middle - 1] + sorted[middle]) / 2
    : sorted[middle]
}

function statistics(values: number[]): SampleStatistics {
  const cleanValues = values.filter(Number.isFinite)
  const mean = cleanValues.length
    ? cleanValues.reduce((sum, value) => sum + value, 0) / cleanValues.length
    : null
  const variance = mean === null
    ? null
    : cleanValues.reduce((sum, value) => sum + ((value - mean) ** 2), 0) / cleanValues.length

  return {
    values: cleanValues,
    median: median(cleanValues),
    variance,
    standardDeviation: variance === null ? null : Math.sqrt(variance),
  }
}

function machineProfilesMatch(baseline: PerformanceMeasurement[], after: PerformanceMeasurement[]) {
  const firstBaseline = baseline[0]?.machine
  const firstAfter = after[0]?.machine
  if (!firstBaseline || !firstAfter) return false
  return firstBaseline.cpuName === firstAfter.cpuName
    && firstBaseline.architecture === firstAfter.architecture
    && firstBaseline.logicalProcessors === firstAfter.logicalProcessors
    && firstBaseline.memoryTotalBytes === firstAfter.memoryTotalBytes
}

function compareMetric(
  definition: PerformanceMetricDefinition,
  baseline: PerformanceMeasurement[],
  after: PerformanceMeasurement[],
  machineMatches: boolean,
): MetricComparison {
  const baselineStats = statistics(baseline.map(sample => sample.metrics[definition.id]))
  const afterStats = statistics(after.map(sample => sample.metrics[definition.id]))
  const medianDelta = baselineStats.median === null || afterStats.median === null
    ? null
    : afterStats.median - baselineStats.median
  const relativeDeltaPercent = baselineStats.median === null || afterStats.median === null || baselineStats.median === 0
    ? null
    : (medianDelta! / Math.abs(baselineStats.median)) * 100
  const noiseFloor = baselineStats.standardDeviation === null || afterStats.standardDeviation === null
    ? null
    : Math.max(baselineStats.standardDeviation, afterStats.standardDeviation)
  const enoughSamples = baselineStats.values.length >= MINIMUM_SAMPLES_PER_PHASE
    && afterStats.values.length >= MINIMUM_SAMPLES_PER_PHASE
  const practicalThreshold = Math.max(definition.minimumMeaningfulEffect, noiseFloor ?? 0)

  let practicalSignificance: PracticalSignificance = 'insufficient_data'
  let conclusion: ExperimentConclusion = 'inconclusive_noisy_measurement'
  let caveat = `Collect at least ${MINIMUM_SAMPLES_PER_PHASE} samples in both phases before interpreting this metric.`

  if (enoughSamples && medianDelta !== null && machineMatches) {
    if (Math.abs(medianDelta) <= practicalThreshold) {
      practicalSignificance = 'below_threshold'
      conclusion = 'no_meaningful_change'
      caveat = `The ${Math.abs(medianDelta).toFixed(2)}${definition.unit} median delta does not exceed the ${practicalThreshold.toFixed(2)}${definition.unit} practical-effect threshold or observed noise.`
    } else if (definition.lowerIsBetter === null) {
      practicalSignificance = 'meaningful'
      caveat = 'Metric direction is undefined for this workload snapshot; no improvement or regression claim is allowed.'
    } else {
      practicalSignificance = 'meaningful'
      const improved = definition.lowerIsBetter ? medianDelta < 0 : medianDelta > 0
      conclusion = improved ? 'measurable_improvement' : 'regression_detected'
      caveat = `The result exceeds the ${practicalThreshold.toFixed(2)}${definition.unit} practical-effect threshold; repeat on the same workload before acting.`
    }
  } else if (!machineMatches) {
    caveat = 'The machine profile changed between phases; the comparison is not attributable to the experiment.'
  }

  return {
    metricId: definition.id,
    label: definition.label,
    unit: definition.unit,
    baseline: baselineStats,
    after: afterStats,
    medianDelta,
    relativeDeltaPercent,
    noiseFloor,
    minimumMeaningfulEffect: definition.minimumMeaningfulEffect,
    practicalSignificance,
    conclusion,
    caveat,
  }
}

export function createExperiment(input: CreateExperimentInput): PerformanceExperiment {
  const name = input.name.trim()
  const hypothesis = input.hypothesis.trim()
  if (!name) throw new Error('Experiment name is required.')
  if (!hypothesis) throw new Error('Experiment hypothesis is required.')

  return {
    id: input.id ?? makeId(),
    operationId: input.operationId?.trim() || null,
    operationEvidence: null,
    decisionOperationId: null,
    decisionEvidence: null,
    name,
    hypothesis,
    createdAt: input.createdAt ?? new Date().toISOString(),
    appliedAt: null,
    rebootRequired: false,
    resumedAt: null,
    metricDefinitions: input.metricDefinitions?.map(definition => ({ ...definition })) ?? SYSTEM_SNAPSHOT_METRICS.map(definition => ({ ...definition })),
    baseline: [],
    after: [],
    state: 'draft',
    comparison: null,
    decision: null,
  }
}

export async function listPerformanceExperiments() {
  const experiments = await invoke<PerformanceExperiment[]>('list_performance_experiments')
  return experiments.map(normalizeExperiment)
}

export const savePerformanceExperiment = (experiment: PerformanceExperiment) =>
  invoke<PerformanceExperiment>('save_performance_experiment', { experiment: normalizeExperiment(experiment) })

export const preparePerformanceExperimentRevert = (operationId: string) =>
  invoke<OperationPlan>('prepare_performance_experiment_revert', { operationId })

export const recordPerformanceExperimentDecision = (
  experiment: PerformanceExperiment,
  decision: Exclude<ExperimentDecision, 'unknown'>,
  decisionOperationId: string | null = null,
) => invoke<PerformanceExperiment>('record_performance_experiment_decision', {
  experiment: normalizeExperiment(experiment),
  decision,
  decisionOperationId,
})

export function recordMeasurement(
  experiment: PerformanceExperiment,
  phase: ExperimentPhase,
  measurement: PerformanceMeasurement,
): PerformanceExperiment {
  if (phase === 'baseline' && !['draft', 'collecting_baseline'].includes(experiment.state)) {
    throw new Error('Baseline measurements can only be recorded before the after phase.')
  }
  if (phase === 'after' && !['awaiting_after', 'collecting_after'].includes(experiment.state)) {
    throw new Error('Start the after phase before recording after measurements.')
  }

  return phase === 'baseline'
    ? { ...experiment, baseline: [...experiment.baseline, measurement], state: 'collecting_baseline', comparison: null, decision: null }
    : { ...experiment, after: [...experiment.after, measurement], state: 'collecting_after', comparison: null, decision: null }
}

export function beginAfterPhase(experiment: PerformanceExperiment): PerformanceExperiment {
  return markExperimentApplied(experiment, false)
}

export function markExperimentApplied(
  experiment: PerformanceExperiment,
  rebootRequired: boolean,
  appliedAt = new Date().toISOString(),
): PerformanceExperiment {
  if (experiment.baseline.length === 0) throw new Error('Record at least one baseline measurement first.')
  if (!['draft', 'collecting_baseline'].includes(experiment.state)) {
    throw new Error('The change can only be applied after baseline measurements.')
  }
  return {
    ...experiment,
    appliedAt,
    rebootRequired,
    resumedAt: null,
    state: rebootRequired ? 'awaiting_reboot' : 'awaiting_after',
    comparison: null,
    decision: null,
  }
}

export function resumeAfterReboot(
  experiment: PerformanceExperiment,
  resumedAt = new Date().toISOString(),
): PerformanceExperiment {
  if (experiment.state !== 'awaiting_reboot' || !experiment.rebootRequired) {
    throw new Error('This experiment is not waiting for a reboot resume.')
  }
  return {
    ...experiment,
    resumedAt,
    state: 'awaiting_after',
    comparison: null,
    decision: null,
  }
}

export function compareExperiment(
  experiment: PerformanceExperiment,
  comparedAt = new Date().toISOString(),
): PerformanceExperiment {
  const machineMatches = machineProfilesMatch(experiment.baseline, experiment.after)
  const caveats = [
    'These are workload observations from the current hardware/process snapshot, not proof of a faster system.',
    'A practical effect threshold and observed run-to-run noise are considered for every metric.',
  ]
  if (!machineMatches) caveats.push('Machine identity or capacity changed between phases.')
  if (experiment.baseline.length < MINIMUM_SAMPLES_PER_PHASE || experiment.after.length < MINIMUM_SAMPLES_PER_PHASE) {
    caveats.push(`Collect at least ${MINIMUM_SAMPLES_PER_PHASE} samples in each phase for a useful comparison.`)
  }

  return {
    ...experiment,
    state: 'compared',
    comparison: {
      comparedAt,
      machineMatches,
      metrics: experiment.metricDefinitions.map(definition => compareMetric(definition, experiment.baseline, experiment.after, machineMatches)),
      caveats,
    },
    decision: null,
  }
}

export function recordDecision(
  experiment: PerformanceExperiment,
  decision: ExperimentDecision,
): PerformanceExperiment {
  if (!experiment.comparison) throw new Error('Compare the experiment before recording a decision.')
  const state: ExperimentLifecycle = decision === 'keep'
    ? 'keep_recorded'
    : decision === 'revert'
      ? 'revert_requested'
      : 'unknown'
  return { ...experiment, state, decision }
}

export function createSystemSnapshotMeasurement(
  hardware: HardwareInfo,
  processes: ProcessInfo[],
  capturedAt = new Date().toISOString(),
): PerformanceMeasurement {
  return {
    capturedAt,
    source: 'hardware_process_snapshot',
    machine: {
      cpuName: hardware.cpu.name,
      architecture: hardware.cpu.architecture,
      logicalProcessors: hardware.cpu.logical_processors,
      memoryTotalBytes: hardware.memory.total_bytes,
    },
    metrics: {
      cpu_usage_percent: hardware.cpu.usage_percent,
      memory_used_percent: hardware.memory.in_use_percent,
      running_processes: processes.length,
    },
  }
}
