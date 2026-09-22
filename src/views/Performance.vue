<script setup lang="ts">
import { computed, ref, shallowRef } from 'vue'
import { useRouter } from 'vue-router'
import AppHeader from '../components/AppHeader.vue'
import BaseButton from '../components/BaseButton.vue'
import BaseCard from '../components/BaseCard.vue'
import EmptyState from '../components/EmptyState.vue'
import OperationPreview from '../components/OperationPreview.vue'
import ResultsBanner from '../components/ResultsBanner.vue'
import {
  compareExperiment,
  CONCLUSION_LABELS,
  createExperiment,
  createSystemSnapshotMeasurement,
  listPerformanceExperiments,
  markExperimentApplied,
  MINIMUM_SAMPLES_PER_PHASE,
  recordDecision,
  recordMeasurement,
  preparePerformanceExperimentRevert,
  recordPerformanceExperimentDecision,
  resumeAfterReboot,
  savePerformanceExperiment,
  type ExperimentDecision,
  type PerformanceExperiment,
} from '../api/performanceExperiments'
import { confirmAndRequestElevation } from '../api/operation'
import type { OperationPlan } from '../api/operation'
import { executeSystemTweakPlan } from '../api/systemTweaks'
import { checkElevated, getHardwareInfo, processSnapshot, type HardwareInfo, type ProcessInfo } from '../api/tauri'
import { failure, type ActionFeedback } from '../utils/feedback'
import { formatBytes, formatPercent } from '../utils/format'
import { usePolling } from '../composables/usePolling'

const router = useRouter()
const hardware = shallowRef<HardwareInfo | null>(null)
const processes = shallowRef<ProcessInfo[]>([])
const loading = shallowRef(false)
const feedback = shallowRef<ActionFeedback | null>(null)
const experiment = shallowRef<PerformanceExperiment | null>(null)
const experimentName = ref('')
const experimentHypothesis = ref('')
const experimentOperationId = ref('')
const experimentError = ref<string | null>(null)
const experimentsLoaded = ref(false)
const pendingRevertPlan = shallowRef<OperationPlan | null>(null)

const memoryUsed = computed(() => hardware.value?.memory.in_use_percent ?? 0)
const diskSummary = computed(() => {
  const disks = hardware.value?.disks ?? []
  const total = disks.reduce((sum, disk) => sum + disk.total_bytes, 0)
  const available = disks.reduce((sum, disk) => sum + disk.available_bytes, 0)
  return { total, available, usedPercent: total ? Math.round((1 - available / total) * 100) : 0 }
})
const topCpuProcess = computed(() => [...processes.value].sort((a, b) => b.cpu_usage - a.cpu_usage)[0])
const topMemoryProcess = computed(() => [...processes.value].sort((a, b) => b.memory_bytes - a.memory_bytes)[0])
const experimentPhase = computed(() => {
  if (!experiment.value) return null
  return ['draft', 'collecting_baseline'].includes(experiment.value.state) ? 'baseline' : 'after'
})
const experimentCanCompare = computed(() => {
  const current = experiment.value
  return Boolean(current
    && current.baseline.length >= MINIMUM_SAMPLES_PER_PHASE
    && current.after.length >= MINIMUM_SAMPLES_PER_PHASE)
})
const experimentStateLabel = computed(() => {
  switch (experiment.value?.state) {
    case 'draft': return 'Ready for baseline measurements'
    case 'collecting_baseline': return 'Collecting baseline measurements'
    case 'awaiting_reboot': return 'Waiting to resume after reboot'
    case 'awaiting_after': return 'Waiting for the separately applied change'
    case 'collecting_after': return 'Collecting after measurements'
    case 'compared': return 'Comparison ready for review'
    case 'keep_recorded': return 'Keep decision recorded'
    case 'revert_requested': return 'Revert decision recorded'
    case 'unknown': return 'Outcome marked unknown'
    default: return 'No experiment selected'
  }
})

function usagePercent(available: number, total: number) {
  return total ? Math.round((1 - available / total) * 100) : 0
}

async function persistExperiment(next: PerformanceExperiment) {
  try {
    experiment.value = await savePerformanceExperiment(next)
  } catch (error) {
    experimentError.value = error instanceof Error ? error.message : String(error)
  }
}

async function startExperiment() {
  experimentError.value = null
  try {
    await persistExperiment(createExperiment({
      name: experimentName.value,
      hypothesis: experimentHypothesis.value,
      operationId: experimentOperationId.value,
    }))
  } catch (error) {
    experimentError.value = error instanceof Error ? error.message : String(error)
  }
}

async function captureExperimentSample() {
  if (!experiment.value || !hardware.value || !experimentPhase.value) return
  experimentError.value = null
  try {
    await persistExperiment(recordMeasurement(
      experiment.value,
      experimentPhase.value,
      createSystemSnapshotMeasurement(hardware.value, processes.value),
    ))
  } catch (error) {
    experimentError.value = error instanceof Error ? error.message : String(error)
  }
}

async function markChangeApplied(rebootRequired: boolean) {
  if (!experiment.value) return
  experimentError.value = null
  try {
    await persistExperiment(markExperimentApplied(experiment.value, rebootRequired))
  } catch (error) {
    experimentError.value = error instanceof Error ? error.message : String(error)
  }
}

async function resumeExperimentAfterReboot() {
  if (!experiment.value) return
  experimentError.value = null
  try {
    await persistExperiment(resumeAfterReboot(experiment.value))
  } catch (error) {
    experimentError.value = error instanceof Error ? error.message : String(error)
  }
}

async function compareCurrentExperiment() {
  if (!experiment.value) return
  await persistExperiment(compareExperiment(experiment.value))
}

async function recordCurrentDecision(decision: ExperimentDecision) {
  if (!experiment.value) return
  experimentError.value = null
  try {
    if (decision === 'unknown') {
      await persistExperiment(recordDecision(experiment.value, decision))
      return
    }
    if (decision === 'keep') {
      experiment.value = await recordPerformanceExperimentDecision(experiment.value, decision)
      return
    }
    pendingRevertPlan.value = await preparePerformanceExperimentRevert(experiment.value.operationId ?? '')
  } catch (error) {
    experimentError.value = error instanceof Error ? error.message : String(error)
  }
}

async function executePendingRevert() {
  const current = experiment.value
  const plan = pendingRevertPlan.value
  if (!current || !plan) return
  experimentError.value = null
  loading.value = true
  let executedOperationId: string | null = null
  try {
    if (plan.requires_elevation && !(await checkElevated())) {
      await confirmAndRequestElevation(plan)
      experimentError.value = 'Administrator approval requested. The experiment will remain unfinalized until the journaled revert reports verified read-back.'
      return
    }
    executedOperationId = plan.id
    const execution = await executeSystemTweakPlan(plan)
    if (execution.result.failed > 0 || execution.result.verified < execution.result.requested) {
      throw new Error('The revert operation did not finish with verified read-back.')
    }
    experiment.value = await recordPerformanceExperimentDecision(current, 'revert', plan.id)
    pendingRevertPlan.value = null
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error)
    experimentError.value = executedOperationId
      ? `Revert operation ${executedOperationId} was attempted; inspect its journal entry before retrying. ${message}`
      : message
  } finally {
    loading.value = false
  }
}

function resetExperiment() {
  experiment.value = null
  pendingRevertPlan.value = null
  experimentName.value = ''
  experimentHypothesis.value = ''
  experimentOperationId.value = ''
  experimentError.value = null
}

function formatExperimentMetric(value: number | null, unit: string) {
  return value === null ? 'Not available' : `${value.toFixed(2)}${unit}`
}

async function refresh() {
  loading.value = true
  try {
    const [hardwareData, processData] = await Promise.all([getHardwareInfo(), processSnapshot()])
    hardware.value = hardwareData
    processes.value = processData
    if (!experimentsLoaded.value) {
      const persisted = await listPerformanceExperiments()
      experiment.value = persisted[0] ?? null
      experimentsLoaded.value = true
    }
    feedback.value = null
  } catch (error) {
    feedback.value = failure('Could not load performance data', error)
  } finally {
    loading.value = false
  }
}

usePolling(refresh, 5000, { immediate: true })
</script>

<template>
  <div class="performance">
    <AppHeader>
      <template #actions>
        <BaseButton variant="primary" size="sm" icon="refresh" :loading="loading" @click="refresh">Refresh</BaseButton>
      </template>
    </AppHeader>

    <ResultsBanner
      v-if="feedback"
      :variant="feedback.type"
      :title="feedback.title"
      :description="feedback.message"
      class="performance-feedback"
      @dismiss="feedback = null"
    />

    <template v-if="hardware">
      <section class="metrics" aria-label="Current system metrics">
        <BaseCard class="metric-card">
          <span class="metric-label">Memory in use</span>
          <strong class="metric-value">{{ formatPercent(memoryUsed) }}</strong>
          <div class="meter" aria-hidden="true"><span class="meter-fill" :style="{ width: memoryUsed + '%' }" /></div>
          <span class="metric-detail">{{ formatBytes(hardware.memory.total_bytes - hardware.memory.available_bytes) }} of {{ formatBytes(hardware.memory.total_bytes) }}</span>
        </BaseCard>
        <BaseCard class="metric-card">
          <span class="metric-label">Disk space in use</span>
          <strong class="metric-value">{{ diskSummary.usedPercent }}%</strong>
          <div class="meter" aria-hidden="true"><span class="meter-fill" :style="{ width: diskSummary.usedPercent + '%' }" /></div>
          <span class="metric-detail">{{ formatBytes(diskSummary.available) }} free across {{ hardware.disks.length }} drive{{ hardware.disks.length === 1 ? '' : 's' }}</span>
        </BaseCard>
        <BaseCard class="metric-card">
          <span class="metric-label">Running processes</span>
          <strong class="metric-value">{{ processes.length }}</strong>
          <span class="metric-detail">{{ topCpuProcess ? `${topCpuProcess.name} currently reports ${topCpuProcess.cpu_usage.toFixed(1)}% CPU` : 'No process snapshot available' }}</span>
        </BaseCard>
      </section>

      <section class="details-grid">
        <BaseCard class="detail-card">
          <div class="card-heading">
            <div>
              <h2>Current workload</h2>
              <p>Snapshot from running Windows processes.</p>
            </div>
            <BaseButton variant="secondary" size="sm" @click="router.push('/process-manager')">View processes</BaseButton>
          </div>
          <div v-if="topCpuProcess || topMemoryProcess" class="workload-list">
            <div v-if="topCpuProcess" class="workload-row">
              <span>Highest reported CPU</span>
              <strong>{{ topCpuProcess.name }} · {{ topCpuProcess.cpu_usage.toFixed(1) }}%</strong>
            </div>
            <div v-if="topMemoryProcess" class="workload-row">
              <span>Largest memory user</span>
              <strong>{{ topMemoryProcess.name }} · {{ formatBytes(topMemoryProcess.memory_bytes) }}</strong>
            </div>
          </div>
          <p v-else class="empty-copy">No process data was returned. Refresh to try again.</p>
        </BaseCard>

        <BaseCard class="detail-card">
          <div class="card-heading">
            <div>
              <h2>Hardware</h2>
              <p>Installed hardware reported by Windows.</p>
            </div>
            <BaseButton variant="secondary" size="sm" @click="router.push('/system-info')">View system info</BaseButton>
          </div>
          <dl class="hardware-list">
            <div><dt>Processor</dt><dd>{{ hardware.cpu.name }}</dd></div>
            <div><dt>Logical processors</dt><dd>{{ hardware.cpu.logical_processors }}</dd></div>
            <div v-if="hardware.gpu"><dt>Graphics</dt><dd>{{ hardware.gpu.name }}</dd></div>
          </dl>
        </BaseCard>
      </section>

      <section class="experiment-section" aria-label="Performance experiment">
        <BaseCard class="experiment-card">
          <div class="section-heading">
            <div>
              <h2>Performance experiment</h2>
              <p>Record machine-specific observations around a separately reviewed change.</p>
            </div>
            <span class="experiment-state">{{ experimentStateLabel }}</span>
          </div>

          <p class="experiment-note">
            Measurements stay attached to a reviewed operation. Keep records verified evidence; Revert creates a separate normal operation plan that must be reviewed and confirmed.
          </p>

          <div v-if="!experiment" class="experiment-setup">
            <label class="experiment-field">
              <span>Experiment or tweak name</span>
              <input v-model="experimentName" class="experiment-input" placeholder="Example: processor scheduling" />
            </label>
            <label class="experiment-field">
              <span>Hypothesis</span>
              <textarea v-model="experimentHypothesis" class="experiment-input experiment-textarea" rows="2" placeholder="What change do you expect to affect this workload?" />
            </label>
            <label class="experiment-field">
              <span>Related reviewed operation ID (required for Keep/Revert)</span>
              <input v-model="experimentOperationId" class="experiment-input" placeholder="Links evidence to the journaled apply operation" />
            </label>
            <BaseButton
              variant="primary"
              :disabled="!experimentName.trim() || !experimentHypothesis.trim()"
              @click="startExperiment"
            >
              Start experiment
            </BaseButton>
          </div>

          <div v-else class="experiment-workflow">
            <div class="experiment-summary">
              <strong>{{ experiment.name }}</strong>
              <span>{{ experiment.hypothesis }}</span>
            </div>

            <div class="experiment-samples" aria-label="Experiment sample counts">
              <span>Baseline: <strong>{{ experiment.baseline.length }}</strong></span>
              <span>After: <strong>{{ experiment.after.length }}</strong></span>
              <span>Comparison minimum: <strong>{{ MINIMUM_SAMPLES_PER_PHASE }} per phase</strong></span>
            </div>

            <div class="experiment-controls">
              <template v-if="experiment.state === 'draft' || experiment.state === 'collecting_baseline'">
                <BaseButton variant="primary" :loading="loading" :disabled="loading" @click="captureExperimentSample">
                  Record baseline sample
                </BaseButton>
                <BaseButton variant="secondary" :disabled="experiment.baseline.length === 0" @click="markChangeApplied(false)">
                  Mark change applied
                </BaseButton>
                <BaseButton variant="secondary" :disabled="experiment.baseline.length === 0" @click="markChangeApplied(true)">
                  Mark applied; reboot required
                </BaseButton>
              </template>
              <template v-else-if="experiment.state === 'awaiting_reboot'">
                <BaseButton variant="secondary" @click="resumeExperimentAfterReboot">
                  Resume after reboot
                </BaseButton>
              </template>
              <template v-else-if="experiment.state === 'awaiting_after' || experiment.state === 'collecting_after'">
                <BaseButton variant="primary" :loading="loading" :disabled="loading" @click="captureExperimentSample">
                  Record after sample
                </BaseButton>
                <BaseButton variant="secondary" :disabled="!experimentCanCompare" @click="compareCurrentExperiment">
                  Compare measurements
                </BaseButton>
              </template>
              <template v-else-if="experiment.state === 'compared'">
                <BaseButton variant="success" @click="recordCurrentDecision('keep')">Record Keep</BaseButton>
                <BaseButton variant="danger" :loading="loading" :disabled="loading" @click="recordCurrentDecision('revert')">Prepare Revert plan</BaseButton>
                <BaseButton variant="secondary" @click="recordCurrentDecision('unknown')">Mark Unknown</BaseButton>
              </template>
              <BaseButton variant="ghost" @click="resetExperiment">Start another experiment</BaseButton>
            </div>

            <div v-if="pendingRevertPlan" class="experiment-revert-plan">
              <OperationPreview
                :plan="pendingRevertPlan"
                title="Review Revert plan"
                description="This exact plan revalidates the current performance tweak, changes it through the normal operation kernel, and records post-change read-back."
                action-label="Disable the reviewed performance tweak"
              />
              <div class="experiment-controls">
                <BaseButton variant="danger" :loading="loading" :disabled="loading" @click="executePendingRevert">
                  Confirm and execute Revert
                </BaseButton>
                <BaseButton variant="ghost" :disabled="loading" @click="pendingRevertPlan = null">
                  Cancel
                </BaseButton>
              </div>
            </div>

            <div v-if="experiment.comparison" class="experiment-comparison">
              <div class="comparison-heading">
                <div>
                  <h3>Comparison</h3>
                  <p>Median deltas are shown with absolute units; percentages are not treated as proof of benefit.</p>
                </div>
                <span :class="['machine-status', { 'is-matched': experiment.comparison.machineMatches }]">
                  {{ experiment.comparison.machineMatches ? 'Same machine profile' : 'Machine profile changed' }}
                </span>
              </div>
              <div class="comparison-table-wrap">
                <table class="comparison-table">
                  <thead>
                    <tr><th>Metric</th><th>Baseline median</th><th>After median</th><th>Median delta</th><th>Conclusion</th></tr>
                  </thead>
                  <tbody>
                    <tr v-for="metric in experiment.comparison.metrics" :key="metric.metricId">
                      <th scope="row">{{ metric.label }}</th>
                      <td>{{ formatExperimentMetric(metric.baseline.median, metric.unit) }}</td>
                      <td>{{ formatExperimentMetric(metric.after.median, metric.unit) }}</td>
                      <td>{{ formatExperimentMetric(metric.medianDelta, metric.unit) }}</td>
                      <td>
                        <strong>{{ CONCLUSION_LABELS[metric.conclusion] }}</strong>
                        <span class="comparison-caveat">{{ metric.caveat }}</span>
                      </td>
                    </tr>
                  </tbody>
                </table>
              </div>
              <ul class="experiment-caveats">
                <li v-for="caveat in experiment.comparison.caveats" :key="caveat">{{ caveat }}</li>
              </ul>
            </div>

            <p v-if="experiment.state === 'awaiting_reboot'" class="experiment-note">
              The reviewed change was recorded as applied and requires a reboot. Restart normally, then resume this experiment before recording after samples.
            </p>
            <p v-if="experiment.state === 'awaiting_after'" class="experiment-note">
              The reviewed change is recorded as applied. Continue through the normal reviewed operation flow; no setting was changed by this experiment panel.
            </p>
            <p v-if="experiment.state === 'keep_recorded'" class="experiment-note">
              Keep was recorded with verified read-back evidence from the reviewed operation. No additional setting was changed.
            </p>
            <p v-if="experiment.state === 'revert_requested'" class="experiment-note">
              Revert was completed through a separate normal operation plan and its read-back was verified.
            </p>
            <p v-if="experiment.state === 'unknown'" class="experiment-note">
              The outcome is unknown. Treat the measurements as observational evidence and do not claim a performance benefit.
            </p>
          </div>

          <p v-if="experimentError" class="experiment-error" role="alert">{{ experimentError }}</p>
        </BaseCard>
      </section>

      <section class="drive-section">
        <div class="section-heading">
          <div><h2>Storage</h2><p>Available capacity by drive.</p></div>
          <BaseButton variant="secondary" size="sm" @click="router.push('/disk-analyzer')">Analyze storage</BaseButton>
        </div>
        <BaseCard padding="none" class="drive-card">
          <div v-for="disk in hardware.disks" :key="`${disk.name}-${disk.mount_point}`" class="drive-row">
            <div class="drive-name"><strong>{{ disk.name }}</strong><span>{{ disk.mount_point }} · {{ disk.file_system }} · {{ disk.disk_type }}</span></div>
            <div class="drive-usage"><div class="meter" aria-hidden="true"><span class="meter-fill" :style="{ width: usagePercent(disk.available_bytes, disk.total_bytes) + '%' }" /></div><span>{{ formatBytes(disk.available_bytes) }} free of {{ formatBytes(disk.total_bytes) }}</span></div>
          </div>
          <p v-if="!hardware.disks.length" class="empty-copy">No drive information was returned.</p>
        </BaseCard>
      </section>
    </template>

    <EmptyState v-else-if="!loading" icon="activity" title="Performance data is unavailable" description="Refresh to collect the current hardware and process snapshot." />
  </div>
</template>

<style scoped>
.performance { width: 100%; min-width: 0; }
.performance-feedback { margin-bottom: var(--space-5); }
.metrics, .details-grid { display: grid; gap: var(--space-4); }
.metrics { grid-template-columns: repeat(3, minmax(0, 1fr)); margin-bottom: var(--space-6); }
.details-grid { grid-template-columns: repeat(2, minmax(0, 1fr)); margin-bottom: var(--space-6); }
.metric-card, .detail-card { display: flex; flex-direction: column; gap: var(--space-3); min-width: 0; }
.metric-label, .metric-detail, .card-heading p, .section-heading p, .drive-name span, .drive-usage span, .empty-copy { color: var(--text-secondary); font-size: var(--text-sm); line-height: var(--leading-normal); }
.metric-value { color: var(--text-primary); font-size: var(--text-4xl); font-variant-numeric: tabular-nums; }
.meter { height: 6px; overflow: hidden; border-radius: var(--radius-full); background: var(--bg-hover); }
.meter-fill { display: block; height: 100%; border-radius: inherit; background: var(--accent); }
.card-heading, .section-heading { display: flex; justify-content: space-between; gap: var(--space-4); align-items: flex-start; }
h2 { margin: 0 0 var(--space-1); color: var(--text-primary); font-size: var(--text-lg); font-weight: var(--weight-semibold); }
.card-heading p, .section-heading p { margin: 0; }
.workload-list, .hardware-list { margin: 0; display: grid; gap: var(--space-3); }
.workload-row, .hardware-list > div { display: flex; justify-content: space-between; gap: var(--space-4); border-top: 1px solid var(--border-subtle); padding-top: var(--space-3); }
.workload-row span, dt { color: var(--text-secondary); font-size: var(--text-sm); }
.workload-row strong, dd { margin: 0; color: var(--text-primary); font-size: var(--text-sm); font-weight: var(--weight-semibold); text-align: right; overflow-wrap: anywhere; }
.drive-section { display: grid; gap: var(--space-4); }
.drive-card { overflow: hidden; }
.drive-row { display: grid; grid-template-columns: minmax(180px, .8fr) minmax(240px, 1.2fr); align-items: center; gap: var(--space-6); padding: var(--space-4) var(--space-5); border-bottom: 1px solid var(--border-subtle); }
.drive-row:last-child { border-bottom: 0; }
.drive-name, .drive-usage { display: grid; gap: var(--space-1); min-width: 0; }
.drive-name strong { color: var(--text-primary); font-size: var(--text-sm); }
.drive-usage .meter { margin-bottom: var(--space-1); }
.experiment-section { margin-bottom: var(--space-6); }
.experiment-card { display: grid; gap: var(--space-4); }
.experiment-state, .machine-status { color: var(--text-secondary); font-size: var(--text-xs); font-weight: var(--weight-semibold); }
.machine-status.is-matched { color: var(--success); }
.experiment-note, .experiment-summary span, .comparison-caveat { color: var(--text-secondary); font-size: var(--text-sm); line-height: var(--leading-normal); }
.experiment-note { margin: 0; }
.experiment-setup, .experiment-workflow, .experiment-field, .experiment-comparison, .experiment-revert-plan { display: grid; gap: var(--space-3); }
.experiment-field span { color: var(--text-secondary); font-size: var(--text-sm); }
.experiment-input { width: 100%; min-height: 34px; padding: var(--space-2) var(--space-3); border: 1px solid var(--border-default); border-radius: var(--radius-md); background: var(--bg-elevated); color: var(--text-primary); font: inherit; }
.experiment-textarea { resize: vertical; }
.experiment-summary { display: grid; gap: var(--space-1); }
.experiment-summary strong { color: var(--text-primary); }
.experiment-samples, .experiment-controls { display: flex; flex-wrap: wrap; gap: var(--space-3); align-items: center; }
.experiment-samples { color: var(--text-secondary); font-size: var(--text-sm); }
.experiment-samples strong { color: var(--text-primary); }
.comparison-heading { display: flex; justify-content: space-between; gap: var(--space-4); align-items: flex-start; }
.comparison-heading h3 { margin: 0 0 var(--space-1); color: var(--text-primary); font-size: var(--text-md); }
.comparison-heading p { margin: 0; color: var(--text-secondary); font-size: var(--text-sm); }
.comparison-table-wrap { overflow-x: auto; }
.comparison-table { width: 100%; border-collapse: collapse; font-size: var(--text-sm); }
.comparison-table th, .comparison-table td { padding: var(--space-3); border-top: 1px solid var(--border-subtle); text-align: left; vertical-align: top; }
.comparison-table th { color: var(--text-primary); font-weight: var(--weight-semibold); }
.comparison-table td { color: var(--text-secondary); }
.comparison-table td strong, .comparison-caveat { display: block; }
.comparison-caveat { margin-top: var(--space-1); }
.experiment-caveats { margin: 0; padding-left: var(--space-5); color: var(--text-secondary); font-size: var(--text-sm); line-height: var(--leading-normal); }
.experiment-error { margin: 0; color: var(--danger); font-size: var(--text-sm); }
@media (max-width: 800px) { .metrics, .details-grid { grid-template-columns: 1fr; } .drive-row { grid-template-columns: 1fr; gap: var(--space-3); } }
@media (max-width: 520px) { .card-heading, .section-heading, .workload-row, .hardware-list > div, .comparison-heading { align-items: stretch; flex-direction: column; } .workload-row strong, dd { text-align: left; } }
</style>
