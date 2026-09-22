<script setup lang="ts">
import { computed, onActivated, ref } from 'vue'
import type { RepairExecutionLog } from '../api/tauri'
import * as api from '../api/tauri'
import type { OperationPlan } from '../api/operation'
import { confirmAndRequestElevation } from '../api/operation'
import type { RepairExecution, RepairGuide, RepairTarget } from '../api/repair'
import { createRepairPlan, executeRepairPlan, listRepairGuides, listRepairTargets } from '../api/repair'
import AppHeader from '../components/AppHeader.vue'
import BaseCard from '../components/BaseCard.vue'
import BaseButton from '../components/BaseButton.vue'
import BaseBadge from '../components/BaseBadge.vue'
import ConfirmDialog from '../components/ConfirmDialog.vue'
import ResultsBanner from '../components/ResultsBanner.vue'
import EmptyState from '../components/EmptyState.vue'
import { failure, success, type ActionFeedback } from '../utils/feedback'

const actions = ref<RepairTarget[]>([])
const guides = ref<RepairGuide[]>([])
const workflow = ref<'guided' | 'manual'>('guided')
const selectedGuideId = ref('windows_integrity')
const selected = ref<RepairTarget | null>(null)
const confirmedPlan = ref<OperationPlan | null>(null)
const lastLog = ref<RepairExecutionLog | null>(null)
const lastExecution = ref<RepairExecution | null>(null)
const loading = ref(false)
const running = ref(false)
const feedback = ref<ActionFeedback | null>(null)
const showConfirm = ref(false)

const groupedActions = computed(() => {
  const groups: Record<string, RepairTarget[]> = {}
  for (const action of actions.value) {
    if (!groups[action.category]) groups[action.category] = []
    groups[action.category].push(action)
  }
  return groups
})

const selectedGuide = computed(() =>
  guides.value.find((guide) => guide.id === selectedGuideId.value) ?? guides.value[0] ?? null,
)

const guideEvidenceActions = computed(() => {
  const ids = selectedGuide.value?.evidence_action_ids ?? []
  return ids
    .map((id) => actions.value.find((action) => action.id === id))
    .filter((action): action is RepairTarget => Boolean(action))
})

const guideRelatedActions = computed(() => {
  const ids = selectedGuide.value?.related_action_ids ?? []
  return ids
    .map((id) => actions.value.find((action) => action.id === id))
    .filter((action): action is RepairTarget => Boolean(action))
})

async function loadActions() {
  loading.value = true
  feedback.value = null
  try {
    const [loadedActions, loadedGuides] = await Promise.all([
      listRepairTargets(),
      listRepairGuides(),
    ])
    actions.value = loadedActions
    guides.value = loadedGuides
    if (!loadedGuides.some((guide) => guide.id === selectedGuideId.value)) {
      selectedGuideId.value = loadedGuides[0]?.id ?? ''
    }
  } catch (e) {
    feedback.value = failure('Could not load repair actions', e)
  } finally {
    loading.value = false
  }
}

async function preview(action: RepairTarget) {
  if (!action.execution_available) {
    feedback.value = failure(
      `${action.name} remains Experimental`,
      action.verification_method,
    )
    return
  }
  loading.value = true
  try {
    confirmedPlan.value = await createRepairPlan(action.id)
    selected.value = action
    showConfirm.value = true
  } catch (e) {
    feedback.value = failure(`Could not review ${action.name}`, e)
  } finally {
    loading.value = false
  }
}

async function runSelected() {
  if (!selected.value || !confirmedPlan.value) return
  showConfirm.value = false
  running.value = true
  feedback.value = null
  lastLog.value = null
  try {
    if (confirmedPlan.value.requires_elevation && !(await api.checkElevated())) {
      await confirmAndRequestElevation(confirmedPlan.value)
      feedback.value = success(
        'Administrator approval requested',
        selected.value.mutates_system
          ? 'The exact confirmed repair will continue after elevation and reopen in History.'
          : 'The exact confirmed diagnostic will continue after elevation and reopen in History.',
      )
      return
    }
    const execution = await executeRepairPlan(confirmedPlan.value)
    lastExecution.value = execution
    lastLog.value = execution.log
    feedback.value = execution.result.unverified === 0
      ? success(
        verificationOnlyIntegrityCheck(selected.value.id)
          ? 'System File Checker completed; verification step succeeded'
          : selected.value.mutates_system ? 'Repair completed and postcondition verified' : 'Diagnostic completed and verified',
        execution.verification_message,
      )
      : failure(
        verificationOnlyIntegrityCheck(selected.value.id)
          ? 'System File Checker completed; verification did not confirm a result'
          : selected.value.mutates_system ? 'Repair completed but postcondition could not be verified' : 'Diagnostic result could not be verified',
        execution.verification_message,
      )
  } catch (e) {
    feedback.value = failure(`Could not run ${selected.value.name}`, e)
  } finally {
    running.value = false
    confirmedPlan.value = null
  }
}

async function cancelRunning() {
  try {
    await api.cancelRepair()
  } catch (e) {
    feedback.value = failure('Could not cancel the running repair', e)
  }
}

function commandLine(action: RepairTarget): string {
  return [action.command, ...action.args].join(' ')
}

// `sfc /verifyonly` and its follow-up only prove the verification command ran;
// they do not prove files were repaired, so the UI must not claim a repaired
// system integrity postcondition for these actions.
function verificationOnlyIntegrityCheck(actionId: string | null | undefined): boolean {
  return typeof actionId === 'string' && actionId.toLowerCase().startsWith('sfc')
}

function riskVariant(risk: string): 'success' | 'warning' | 'danger' | 'info' {
  if (risk === 'high') return 'danger'
  if (risk === 'medium') return 'warning'
  if (risk === 'low') return 'success'
  return 'info'
}

const hasLoaded = ref(false)
onActivated(() => {
  if (hasLoaded.value) return
  hasLoaded.value = true
  void loadActions()
})
</script>

<template>
  <div class="repair-page">
    <AppHeader>
      <template #actions>
        <BaseButton variant="primary" icon="refresh" @click="loadActions" :loading="loading" size="sm">
          Refresh
        </BaseButton>
      </template>
    </AppHeader>

    <ResultsBanner
      v-if="feedback"
      :variant="feedback.type"
      :title="feedback.title"
      :description="feedback.message"
      class="repair-banner"
      @dismiss="feedback = null"
    />

    <BaseCard variant="glass" class="trust-card">
      <div class="trust-grid">
        <div>
          <span class="trust-label">Preview first</span>
          <strong>Every repair shows the exact command before running.</strong>
        </div>
        <div>
          <span class="trust-label">Admin clear</span>
          <strong>Admin-only actions continue with the same immutable plan after UAC.</strong>
        </div>
        <div>
          <span class="trust-label">Logged</span>
          <strong>Exit code, verification scope, and evidence are recorded in History.</strong>
        </div>
      </div>
    </BaseCard>

    <div class="workflow-tabs" role="tablist" aria-label="Repair Center workflow">
      <BaseButton
        :variant="workflow === 'guided' ? 'accent' : 'secondary'"
        size="sm"
        role="tab"
        :aria-selected="workflow === 'guided'"
        @click="workflow = 'guided'"
      >
        Diagnose &amp; Repair
      </BaseButton>
      <BaseButton
        :variant="workflow === 'manual' ? 'accent' : 'secondary'"
        size="sm"
        role="tab"
        :aria-selected="workflow === 'manual'"
        @click="workflow = 'manual'"
      >
        Manual Tools
      </BaseButton>
      <BaseButton
        v-if="running"
        variant="secondary"
        size="sm"
        @click="cancelRunning"
      >
        Cancel running repair
      </BaseButton>
    </div>

    <section v-if="workflow === 'guided' && selectedGuide" class="guided-workflow">
      <BaseCard variant="glass" class="guide-card">
        <div class="guide-heading">
          <div>
            <span class="trust-label">Symptom selection</span>
            <h2>{{ selectedGuide.title }}</h2>
            <p>{{ selectedGuide.symptom }}</p>
          </div>
          <label class="guide-picker">
            <span class="detail-label">Choose a guided workflow</span>
            <select v-model="selectedGuideId">
              <option v-for="guide in guides" :key="guide.id" :value="guide.id">
                {{ guide.title }}
              </option>
            </select>
          </label>
        </div>
        <p class="guide-description">{{ selectedGuide.description }}</p>
        <p class="limitations"><strong>Limitations:</strong> {{ selectedGuide.limitations }}</p>
      </BaseCard>

      <BaseCard variant="elevated" class="evidence-card">
        <div class="section-heading">
          <div>
            <span class="trust-label">Read-only evidence collection</span>
            <h2>Collect evidence</h2>
          </div>
          <BaseBadge variant="info" size="sm">No automatic repair</BaseBadge>
        </div>
        <p class="section-copy">
          Run one evidence check at a time. The result is shown below and does not select or execute a repair automatically.
        </p>
        <div class="evidence-list">
          <div v-for="action in guideEvidenceActions" :key="action.id" class="evidence-row">
            <div>
              <strong>{{ action.name }}</strong>
              <p>{{ action.description }}</p>
              <span class="evidence-rule">{{ action.verification_method }}</span>
            </div>
            <BaseButton
              variant="accent"
              size="sm"
              @click="preview(action)"
              :loading="running"
              :disabled="!action.execution_available"
            >
              {{ action.execution_available ? 'Review & Collect' : 'Evidence unavailable' }}
            </BaseButton>
          </div>
        </div>
      </BaseCard>

      <BaseCard v-if="guideRelatedActions.length" variant="glass" class="related-card">
        <div class="section-heading">
          <div>
            <span class="trust-label">After review</span>
            <h2>Relevant manual tools</h2>
          </div>
          <BaseBadge variant="warning" size="sm">Manual choice</BaseBadge>
        </div>
        <p class="section-copy">
          These actions are only related to the selected symptom. They are never started from a finding, and unavailable actions remain Experimental.
        </p>
        <div class="related-list">
          <div v-for="action in guideRelatedActions" :key="action.id" class="related-row">
            <div>
              <strong>{{ action.name }}</strong>
              <p>{{ action.limitations }}</p>
            </div>
            <BaseButton
              v-if="action.execution_available"
              variant="secondary"
              size="sm"
              @click="preview(action)"
            >
              Preview manually
            </BaseButton>
            <BaseBadge v-else variant="warning" size="sm">Experimental</BaseBadge>
          </div>
        </div>
      </BaseCard>
    </section>

    <EmptyState
      v-else-if="workflow === 'guided' && !loading"
      icon="tool"
      title="No Guided Workflows Loaded"
      description="Click Refresh to load the available diagnostic workflows."
    />

    <div v-else-if="Object.keys(groupedActions).length" class="repair-groups">
      <section v-for="(groupActions, category) in groupedActions" :key="category" class="repair-group">
        <h2>{{ category }}</h2>
        <div class="action-grid">
          <BaseCard v-for="action in groupActions" :key="action.id" variant="elevated" class="action-card">
            <div class="action-header">
              <div>
                <h3>{{ action.name }}</h3>
                <span class="detail-label">Purpose</span>
                <p>{{ action.description }}</p>
              </div>
              <BaseBadge :variant="riskVariant(action.risk)" size="sm" dot>
                {{ action.risk }}
              </BaseBadge>
              <BaseBadge v-if="action.experimental" variant="warning" size="sm">
                Experimental
              </BaseBadge>
            </div>
            <div class="action-meta">
              <span>{{ action.estimated_minutes }} min</span>
              <span>{{ action.requires_admin ? 'Admin' : 'Limited OK' }}</span>
              <span>{{ action.mutates_system ? 'Changes system' : 'Read-only' }}</span>
              <span v-if="action.restart_required">Restart required</span>
            </div>
            <span class="detail-label">Exact command / API</span>
            <code>{{ commandLine(action) }}</code>
            <span class="detail-label">Verification</span>
            <p class="verification-scope">Scope: {{ action.verification_scope.replaceAll('_', ' ') }}</p>
            <p class="verification-method">{{ action.verification_method }}</p>
            <p class="limitations"><span class="detail-label">Limitations</span>{{ action.limitations }}</p>
            <BaseButton
              variant="accent"
              size="sm"
              icon="play"
              @click="preview(action)"
              :loading="running"
              :disabled="!action.execution_available"
            >
              {{ action.execution_available ? 'Preview & Run' : 'Verification unavailable' }}
            </BaseButton>
          </BaseCard>
        </div>
      </section>
    </div>

    <EmptyState
      v-else-if="workflow === 'manual' && !loading"
      icon="tool"
      title="No Repair Actions Loaded"
      description="Click Refresh to load available repair actions."
    />

    <BaseCard v-if="lastLog" variant="elevated" class="log-card">
      <div class="log-header">
        <div>
          <h2>{{ lastLog.mutates_system ? 'Last repair output' : (workflow === 'guided' ? 'Collected diagnostic evidence' : 'Last manual tool output') }}</h2>
          <p>{{ lastLog.command }} {{ lastLog.args.join(' ') }}</p>
        </div>
        <BaseBadge :variant="lastExecution && lastExecution.result.unverified === 0 ? 'success' : 'warning'" size="sm" dot>
          {{ lastExecution && lastExecution.result.unverified === 0 ? (verificationOnlyIntegrityCheck(lastLog.action_id) ? 'Verification step completed' : (lastLog.mutates_system ? 'Postcondition verified' : 'Evidence verified')) : 'Review required' }}
        </BaseBadge>
      </div>
      <p v-if="lastExecution" class="verification-method">{{ lastExecution.verification_message }}</p>
      <pre v-if="lastLog.stdout">{{ lastLog.stdout }}</pre>
      <pre v-if="lastLog.stderr" class="stderr">{{ lastLog.stderr }}</pre>
    </BaseCard>

    <ConfirmDialog
      :open="showConfirm"
      :operation-plan="confirmedPlan"
      :title="selected ? `Run ${selected.name}?` : 'Run repair action?'"
      description="Review the exact command, mutation scope, verification scope, and recovery limits before continuing."
      :items="selected ? [
        { label: 'Command', value: commandLine(selected), risk: selected.mutates_system ? 'medium' : 'info' },
        { label: 'Admin', value: selected.requires_admin ? 'Required' : 'Not required', risk: selected.requires_admin ? 'medium' : 'low' },
        { label: 'System impact', value: selected.mutates_system ? 'May change Windows state' : 'Read-only check', risk: selected.mutates_system ? 'medium' : 'low' },
        { label: 'Restart', value: selected.restart_required ? 'Required after command' : 'Not expected', risk: selected.restart_required ? 'medium' : 'info' },
        { label: 'Verification scope', value: selected.verification_scope.replaceAll('_', ' '), risk: 'info' },
        { label: 'Verification', value: selected.verification_method, risk: 'info' },
        { label: 'Limitations', value: selected.limitations, risk: 'info' }
      ] : []"
      confirm-label="Run Command"
      cancel-label="Cancel"
      variant="warning"
      :loading="running"
      @confirm="runSelected"
      @cancel="showConfirm = false"
    />
  </div>
</template>

<style scoped>
.repair-page {
  width: 100%;
  min-width: 0;
}

.repair-banner,
.trust-card {
  margin-bottom: var(--space-5);
}

.trust-grid {
  display: grid;
  grid-template-columns: repeat(3, 1fr);
  gap: var(--space-4);
}

.trust-label {
  display: block;
  margin-bottom: var(--space-1);
  color: var(--text-muted);
  font-size: var(--text-xs);
  text-transform: uppercase;
  letter-spacing: var(--tracking-wide);
}

.trust-grid strong {
  color: var(--text-primary);
  font-size: var(--text-sm);
  line-height: var(--leading-relaxed);
}

.workflow-tabs {
  display: flex;
  flex-wrap: wrap;
  gap: var(--space-2);
  margin-bottom: var(--space-5);
}

.guided-workflow {
  display: flex;
  flex-direction: column;
  gap: var(--space-5);
}

.guide-heading,
.section-heading {
  display: flex;
  justify-content: space-between;
  align-items: flex-start;
  gap: var(--space-4);
}

.guide-heading h2,
.section-heading h2 {
  margin: 0 0 var(--space-1);
  font-size: var(--text-lg);
}

.guide-heading p,
.section-copy,
.guide-description,
.related-row p,
.evidence-row p {
  margin: 0;
  color: var(--text-secondary);
  font-size: var(--text-sm);
  line-height: var(--leading-relaxed);
}

.guide-picker {
  display: flex;
  flex-direction: column;
  gap: var(--space-2);
  min-width: 220px;
}

.guide-picker select {
  min-height: 34px;
  padding: 0 var(--space-3);
  border: 1px solid var(--border-default);
  border-radius: var(--radius-md);
  background: var(--bg-surface);
  color: var(--text-primary);
}

.guide-description,
.section-copy {
  margin-top: var(--space-4);
}

.detail-label {
  display: block;
  margin-bottom: var(--space-1);
  color: var(--text-muted);
  font-size: var(--text-xs);
  text-transform: uppercase;
  letter-spacing: var(--tracking-wide);
}

.limitations {
  margin: 0;
  color: var(--text-secondary);
  font-size: var(--text-xs);
  line-height: var(--leading-relaxed);
}

.limitations .detail-label {
  display: inline;
  margin-right: var(--space-2);
}

.evidence-list,
.related-list {
  display: flex;
  flex-direction: column;
  gap: var(--space-3);
  margin-top: var(--space-4);
}

.evidence-row,
.related-row {
  display: flex;
  justify-content: space-between;
  align-items: center;
  gap: var(--space-4);
  padding: var(--space-3);
  border: 1px solid var(--border-subtle);
  border-radius: var(--radius-md);
  background: var(--bg-surface);
}

.evidence-row strong,
.related-row strong {
  display: block;
  margin-bottom: var(--space-1);
  color: var(--text-primary);
  font-size: var(--text-sm);
}

.evidence-rule {
  display: block;
  margin-top: var(--space-2);
  color: var(--text-muted);
  font-size: var(--text-xs);
}

.repair-groups {
  display: flex;
  flex-direction: column;
  gap: var(--space-6);
}

.repair-group h2,
.log-header h2 {
  margin: 0 0 var(--space-3);
  font-size: var(--text-lg);
}

.action-grid {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(300px, 1fr));
  gap: var(--space-4);
}

.action-card {
  display: flex;
  flex-direction: column;
  gap: var(--space-4);
}

.action-header {
  display: flex;
  justify-content: space-between;
  align-items: flex-start;
  gap: var(--space-3);
}

.action-header h3 {
  margin: 0 0 var(--space-1);
  font-size: var(--text-base);
}

.action-header p,
.log-header p {
  margin: 0;
  color: var(--text-secondary);
  font-size: var(--text-sm);
  line-height: var(--leading-relaxed);
}

.action-card > .limitations {
  margin-top: calc(var(--space-2) * -1);
}

.action-meta {
  display: flex;
  flex-wrap: wrap;
  gap: var(--space-2);
}

.action-meta span {
  padding: var(--space-1) var(--space-2);
  border-radius: var(--radius-md);
  background: var(--bg-surface);
  color: var(--text-secondary);
  font-size: var(--text-xs);
}

code {
  display: block;
  padding: var(--space-3);
  border-radius: var(--radius-md);
  background: var(--bg-surface);
  color: var(--text-primary);
  font-size: var(--text-xs);
  white-space: pre-wrap;
  word-break: break-word;
}

.log-card {
  margin-top: var(--space-6);
}

.log-header {
  display: flex;
  justify-content: space-between;
  gap: var(--space-3);
  margin-bottom: var(--space-4);
}

pre {
  max-height: 360px;
  overflow: auto;
  padding: var(--space-4);
  border-radius: var(--radius-md);
  background: var(--bg-base);
  color: var(--text-secondary);
  font-size: var(--text-xs);
  white-space: pre-wrap;
}

pre.stderr {
  border: 1px solid var(--danger);
}

@media (max-width: 800px) {
  .trust-grid {
    grid-template-columns: 1fr;
  }

  .action-grid {
    grid-template-columns: 1fr;
  }

  .action-header,
  .log-header,
  .guide-heading,
  .section-heading,
  .evidence-row,
  .related-row {
    flex-direction: column;
    align-items: stretch;
  }

  .guide-picker {
    min-width: 0;
  }
}
</style>
