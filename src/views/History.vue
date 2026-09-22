<script setup lang="ts">
import { computed, onActivated, ref, shallowRef } from 'vue'
import type { UndoEntry } from '../api/tauri'
import * as api from '../api/tauri'
import { executeRetryPlan } from '../api/retry'
import type { OperationJournalRecord, UndoPreview, UnresolvedOperation } from '../api/operation'
import * as operationApi from '../api/operation'
import AppHeader from '../components/AppHeader.vue'
import BaseCard from '../components/BaseCard.vue'
import BaseButton from '../components/BaseButton.vue'
import BaseBadge from '../components/BaseBadge.vue'
import EmptyState from '../components/EmptyState.vue'
import ResultsBanner from '../components/ResultsBanner.vue'
import ConfirmDialog from '../components/ConfirmDialog.vue'
import TargetDiff from '../components/TargetDiff.vue'
import VerificationResult from '../components/VerificationResult.vue'
import { failure, success, type ActionFeedback } from '../utils/feedback'
import { formatBytesOrDash as formatBytes } from '../utils/format'

const entries = shallowRef<UndoEntry[]>([])
const journalEntries = shallowRef<OperationJournalRecord[]>([])
const unresolvedOperations = shallowRef<UnresolvedOperation[]>([])
const loading = ref(false)
const feedback = ref<ActionFeedback | null>(null)
const pendingJournalUndo = ref<{ record: OperationJournalRecord; preview: UndoPreview } | null>(null)
const pendingRetryPlan = ref<operationApi.OperationPlan | null>(null)
const retrySelections = ref<Record<string, string[]>>({})
const showUndoConfirm = ref(false)
const showRetryConfirm = ref(false)

const kindMeta: Record<string, { label: string; icon: string; variant: 'accent' | 'warning' | 'danger' | 'info' }> = {
  file_delete: { label: 'File Deletion', icon: 'trash', variant: 'danger' },
  registry_change: { label: 'Registry Change', icon: 'file-text', variant: 'warning' },
  service_change: { label: 'Service Change', icon: 'cpu', variant: 'info' },
  startup_change: { label: 'Startup Change', icon: 'zap', variant: 'accent' },
}

function getKindMeta(kind: string) {
  return kindMeta[kind] || { label: kind, icon: 'info', variant: 'default' as const }
}

function formatDate(iso: string): string {
  const d = new Date(iso)
  const now = new Date()
  const diff = now.getTime() - d.getTime()
  if (diff < 60000) return 'Just now'
  if (diff < 3600000) return `${Math.floor(diff / 60000)}m ago`
  if (diff < 86400000) return `${Math.floor(diff / 3600000)}h ago`
  return d.toLocaleDateString(undefined, { month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit' })
}

const groupedEntries = computed(() => {
  const groups: Record<string, UndoEntry[]> = {}
  for (const entry of entries.value) {
    const date = new Date(entry.created_at).toLocaleDateString()
    if (!groups[date]) groups[date] = []
    groups[date].push(entry)
  }
  return groups
})

const totalRecoverable = computed(() =>
  journalEntries.value.filter(record => record.undo_eligibility.eligible).length
)

const totalFreed = computed(() =>
  entries.value.reduce((s, e) => s + (e.original_size_bytes || 0), 0)
)

async function refresh() {
  loading.value = true
  feedback.value = null
  try {
    const [legacyEntries, operationEntries, unresolved] = await Promise.all([
      api.getUndoLog(),
      operationApi.getOperationJournal(),
      operationApi.getUnresolvedOperations(),
    ])
    entries.value = legacyEntries
    journalEntries.value = operationEntries
    unresolvedOperations.value = unresolved
    retrySelections.value = Object.fromEntries(
      unresolved.map(operation => [
        operation.operation_id,
        retrySelections.value[operation.operation_id] ?? operation.targets.map(target => target.target_id),
      ]),
    )
  } catch (e) {
    feedback.value = failure('Could not load history', e)
  } finally {
    loading.value = false
  }
}

const hasLoaded = ref(false)
onActivated(() => {
  if (hasLoaded.value) return
  hasLoaded.value = true
  void refresh()
})

async function requestJournalUndo(record: OperationJournalRecord) {
  loading.value = true
  feedback.value = null
  try {
    const preview = await operationApi.previewOperationUndo(record.journal_id)
    pendingJournalUndo.value = { record, preview }
    showUndoConfirm.value = true
  } catch (e) {
    feedback.value = failure('Could not preview undo', e)
  } finally {
    loading.value = false
  }
}

async function undoEntry() {
  if (!pendingJournalUndo.value) return
  loading.value = true
  showUndoConfirm.value = false
  try {
    const { record, preview } = pendingJournalUndo.value
    if (preview.plan.requires_elevation && !(await api.checkElevated())) {
      await operationApi.confirmAndRequestElevation(preview.plan)
      feedback.value = success(
        'Administrator approval requested',
        'The exact confirmed restoration will continue after elevation and reopen in History.',
      )
      pendingJournalUndo.value = null
      return
    }
    const undoRecord = await operationApi.executeOperationUndo(record.journal_id, preview.plan)
    journalEntries.value = [undoRecord, ...journalEntries.value]
    feedback.value = success('Undo completed', `${record.confirmed_plan.descriptor.title} was restored and verified.`)
    pendingJournalUndo.value = null
  } catch (e) {
    feedback.value = failure('Undo failed', e)
  } finally {
    loading.value = false
  }
}

async function prepareRetry(operation: UnresolvedOperation) {
  loading.value = true
  feedback.value = null
  try {
    pendingRetryPlan.value = await operationApi.createRetryPlan(
      operation.operation_id,
      retrySelections.value[operation.operation_id] ?? [],
    )
    showRetryConfirm.value = true
  } catch (e) {
    feedback.value = failure('Could not create a retry plan', e)
  } finally {
    loading.value = false
  }
}

async function executeRetry() {
  const plan = pendingRetryPlan.value
  if (!plan) return
  loading.value = true
  showRetryConfirm.value = false
  try {
    if (plan.requires_elevation && !(await api.checkElevated())) {
      await operationApi.confirmAndRequestElevation(plan)
      feedback.value = success(
        'Administrator approval requested',
        'The new retry plan will continue after elevation and reopen in History.',
      )
      pendingRetryPlan.value = null
      return
    }
    await executeRetryPlan(plan)
    feedback.value = success('Retry completed', 'The rediscovered targets were executed as a new operation.')
    pendingRetryPlan.value = null
    await refresh()
  } catch (e) {
    feedback.value = failure('Retry failed', e)
  } finally {
    loading.value = false
  }
}

function isRetryTargetSelected(operationId: string, targetId: string) {
  return (retrySelections.value[operationId] ?? []).includes(targetId)
}

function selectedRetryTargetCount(operation: UnresolvedOperation) {
  return (retrySelections.value[operation.operation_id] ?? []).length
}

function toggleRetryTarget(operationId: string, targetId: string) {
  const selected = new Set(retrySelections.value[operationId] ?? [])
  if (selected.has(targetId)) selected.delete(targetId)
  else selected.add(targetId)
  retrySelections.value = {
    ...retrySelections.value,
    [operationId]: [...selected],
  }
}
</script>

<template>
  <div class="history-page">
    <AppHeader>
      <template #actions>
        <BaseButton variant="primary" icon="refresh" @click="refresh" :loading="loading" size="sm">
          Refresh
        </BaseButton>
      </template>
    </AppHeader>

    <ResultsBanner
      v-if="feedback"
      :variant="feedback.type"
      :title="feedback.title"
      :description="feedback.message"
      class="history-feedback"
      @dismiss="feedback = null"
    />

    <section v-if="unresolvedOperations.length > 0" class="journal-section" aria-labelledby="unresolved-operations-heading">
      <div class="section-heading">
        <div>
          <h3 id="unresolved-operations-heading">Operations requiring recovery inspection</h3>
          <p>A previous operation reached the durable execution boundary without a terminal result. No automatic replay is performed.</p>
        </div>
      </div>
      <BaseCard v-for="operation in unresolvedOperations" :key="operation.operation_id" class="journal-card">
        <div class="entry-header">
          <div>
            <div class="entry-type">
              <BaseBadge variant="warning" size="sm" dot>Outcome requires inspection</BaseBadge>
              <span class="entry-time">{{ operation.state.replaceAll('_', ' ') }}</span>
            </div>
            <h4>{{ operation.confirmed_plan.descriptor.title }}</h4>
          </div>
        </div>
        <p class="undo-reason">
          {{ operation.confirmed_plan.targets.length }} confirmed target{{ operation.confirmed_plan.targets.length === 1 ? '' : 's' }} require current-state inspection before a new plan can be created.
        </p>
        <ul class="unresolved-targets">
          <li v-for="target in operation.targets" :key="target.target_id">
            <label class="unresolved-target-select">
              <input
                type="checkbox"
                :checked="isRetryTargetSelected(operation.operation_id, target.target_id)"
                :aria-label="`Select ${target.target_id} for retry`"
                @change="toggleRetryTarget(operation.operation_id, target.target_id)"
              >
              <code>{{ target.target_id }}</code>
            </label>
            <BaseBadge variant="warning" size="sm">{{ target.state.replaceAll('_', ' ') }}</BaseBadge>
          </li>
        </ul>
        <div class="unresolved-actions">
          <BaseButton
            v-if="operation.confirmed_plan.descriptor.id === 'cleanup.reference_slice' && operation.targets.length > 0"
            variant="warning"
            size="sm"
            :loading="loading"
            :disabled="selectedRetryTargetCount(operation) === 0"
            @click="prepareRetry(operation)"
          >
            Create new cleanup plan
          </BaseButton>
          <span v-else class="undo-reason">This operation requires domain-specific rediscovery before it can be retried.</span>
        </div>
      </BaseCard>
    </section>

    <!-- Summary stats -->
    <div class="stats-row" v-if="entries.length > 0 || journalEntries.length > 0">
      <div class="stat-card-sm">
        <span class="stat-num">{{ entries.length + journalEntries.length }}</span>
        <span class="stat-lbl">Total Actions</span>
      </div>
      <div class="stat-card-sm">
        <span class="stat-num accent">{{ totalRecoverable }}</span>
        <span class="stat-lbl">Undo Available</span>
      </div>
      <div class="stat-card-sm">
        <span class="stat-num warning">{{ formatBytes(totalFreed) }}</span>
        <span class="stat-lbl">Total Freed</span>
      </div>
    </div>

    <section v-if="journalEntries.length > 0" class="journal-section" aria-labelledby="operation-journal-heading">
      <div class="section-heading">
        <div>
          <h3 id="operation-journal-heading">Operation journal</h3>
          <p>Confirmed plans, per-target results, verification, and captured-state undo.</p>
        </div>
      </div>
      <BaseCard
        v-for="record in journalEntries"
        :key="record.journal_id"
        class="journal-card"
        :variant="record.terminal_state === 'interrupted' || record.terminal_state === 'failed' ? 'default' : 'elevated'"
      >
        <div class="entry-header">
          <div>
            <div class="entry-type">
              <BaseBadge :variant="record.terminal_state === 'failed' || record.terminal_state === 'interrupted' ? 'danger' : 'info'" size="sm" dot>
                {{ record.terminal_state.replaceAll('_', ' ') }}
              </BaseBadge>
              <span class="entry-time">{{ formatDate(record.timestamp) }}</span>
            </div>
            <h4>{{ record.confirmed_plan.descriptor.title }}</h4>
          </div>
          <BaseButton
            v-if="record.undo_eligibility.eligible"
            variant="ghost"
            size="sm"
            icon="undo"
            :loading="loading"
            @click="requestJournalUndo(record)"
          >
            Preview Undo
          </BaseButton>
          <BaseBadge v-else variant="default" size="sm">Undo unavailable</BaseBadge>
        </div>
        <div class="result-counts" aria-label="Operation target results">
          <span>Risk <strong>{{ record.confirmed_plan.descriptor.risk }}</strong></span>
          <span>Requested <strong>{{ record.target_outcomes.length }}</strong></span>
          <span>Changed <strong>{{ record.target_outcomes.filter(target => target.execution === 'changed').length }}</strong></span>
          <span>Skipped <strong>{{ record.target_outcomes.filter(target => target.execution === 'skipped').length }}</strong></span>
          <span>Failed <strong>{{ record.target_outcomes.filter(target => target.execution === 'failed').length }}</strong></span>
          <span>Verified <strong>{{ record.target_outcomes.filter(target => target.verification === 'verified').length }}</strong></span>
          <span>Reboot <strong>{{ record.restart_requirement.replaceAll('_', ' ') }}</strong></span>
        </div>
        <details class="operation-details">
          <summary>Operation details</summary>
          <dl>
            <div>
              <dt>Confirmed plan</dt>
              <dd>{{ record.confirmed_plan.id }}</dd>
            </div>
            <div>
              <dt>Verification method</dt>
              <dd>
                {{
                  record.verification_results
                    .map(result => result.message)
                    .filter(Boolean)
                    .join(' · ') || 'Not recorded'
                }}
              </dd>
            </div>
            <div>
              <dt>Undo history</dt>
              <dd>{{ record.record_kind === 'undo' ? `Undo of ${record.related_record_id}` : 'No undo recorded' }}</dd>
            </div>
            <div>
              <dt>Captured original states</dt>
              <dd>
                {{ record.captured_original_states.length > 0
                  ? record.captured_original_states.map(state => state.target_id).join(', ')
                  : 'None' }}
              </dd>
            </div>
          </dl>
          <ul class="target-results">
            <li v-for="target in record.target_outcomes" :key="target.target_id">
              <code>{{ target.target_id }}</code>
              <span>Requested: {{ target.requested_action }}</span>
              <span>Original state: {{ target.before?.summary || 'Not captured' }}</span>
              <span>Resulting state: {{ target.after?.summary || 'Not captured' }}</span>
              <span>Verification: {{ target.verification }}</span>
              <VerificationResult :status="target.verification" :message="target.error?.message" />
              <TargetDiff :before="target.before" :after="target.after" />
              <span v-if="target.error">Error: {{ target.error.message }}</span>
            </li>
          </ul>
          <div class="operation-log">
            <strong>Logs</strong>
            <p v-if="record.confirmed_plan.warnings.length === 0">No operation warnings recorded.</p>
            <p v-for="warning in record.confirmed_plan.warnings" v-else :key="warning.code">
              {{ warning.code }}: {{ warning.message }}
            </p>
          </div>
        </details>
        <p v-if="record.diagnostic" class="journal-diagnostic">
          {{ record.diagnostic.code.replaceAll('_', ' ') }}: {{ record.diagnostic.message }}
        </p>
        <p v-if="!record.undo_eligibility.eligible" class="undo-reason">{{ record.undo_eligibility.reason }}</p>
      </BaseCard>
    </section>

    <!-- Timeline -->
    <div v-if="Object.keys(groupedEntries).length > 0" class="timeline">
      <div class="section-heading">
        <div>
          <h3>Legacy undo entries</h3>
          <p>Existing feature-specific records are retained for audit; automated undo eligibility is not established.</p>
        </div>
      </div>
      <div v-for="(dayEntries, date) in groupedEntries" :key="date" class="timeline-day">
        <div class="timeline-date-header">
          <div class="timeline-date-line" />
          <span class="timeline-date-label">{{ date === new Date().toLocaleDateString() ? 'Today' : date }}</span>
          <div class="timeline-date-line" />
        </div>
        <div class="timeline-entries">
          <div
            v-for="entry in dayEntries"
            :key="entry.id"
            class="timeline-entry"
            :class="{ 'is-rolled': entry.rolled_back_at }"
          >
            <div class="timeline-dot" :class="getKindMeta(entry.kind).variant" />
            <BaseCard
              :variant="entry.rolled_back_at ? 'default' : 'elevated'"
              class="entry-card"
              :class="{ rolled: entry.rolled_back_at }"
            >
              <div class="entry-header">
                <div class="entry-type">
                  <BaseBadge :variant="getKindMeta(entry.kind).variant" size="sm" dot>
                    {{ getKindMeta(entry.kind).label }}
                  </BaseBadge>
                  <span class="entry-time">{{ formatDate(entry.created_at) }}</span>
                </div>
                <div class="entry-meta" v-if="entry.original_size_bytes">
                  <span class="entry-size">{{ formatBytes(entry.original_size_bytes) }}</span>
                </div>
              </div>
              <div class="entry-target">{{ entry.target }}</div>
              <div class="entry-footer">
                <span v-if="entry.rolled_back_at" class="entry-rolled-badge">
                  <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
                    <path d="M1 4v6h6M3.51 15a9 9 0 102.13-9.36L1 10" />
                  </svg>
                  Undone
                </span>
                <BaseBadge v-else variant="default" size="sm">
                  Legacy record — undo eligibility unverified
                </BaseBadge>
              </div>
            </BaseCard>
          </div>
        </div>
      </div>
    </div>

    <!-- Empty -->
    <EmptyState
      v-else-if="!loading && journalEntries.length === 0"
      icon="inbox"
      title="No History Loaded"
      description="Click Refresh to load recorded actions and undo history."
    />

    <ConfirmDialog
      :open="showUndoConfirm"
      :operation-plan="pendingJournalUndo?.preview.plan"
      title="Undo this action?"
      description="Undo will restore the captured original state and verify the result."
      :items="pendingJournalUndo ? [
        { label: 'Operation', value: pendingJournalUndo.record.confirmed_plan.descriptor.title, risk: 'medium' },
        { label: 'Targets', value: String(pendingJournalUndo.preview.plan.targets.length), risk: 'info' },
        { label: 'Restore method', value: 'Captured original state', risk: 'info' }
      ] : []"
      confirm-label="Undo Action"
      cancel-label="Cancel"
      variant="warning"
      :loading="loading"
      @confirm="undoEntry"
      @cancel="showUndoConfirm = false; pendingJournalUndo = null"
    />

    <ConfirmDialog
      :open="showRetryConfirm"
      :operation-plan="pendingRetryPlan"
      title="Execute this new cleanup plan?"
      description="The unresolved targets were rediscovered and placed in a new immutable plan. Review the count before confirming."
      :items="pendingRetryPlan ? [
        { label: 'New operation', value: pendingRetryPlan.id, risk: 'info' },
        { label: 'Targets', value: String(pendingRetryPlan.targets.length), risk: 'medium' },
        { label: 'Recovery link', value: pendingRetryPlan.retry_of_operation_id || 'Previous operation', risk: 'info' }
      ] : []"
      confirm-label="Execute New Plan"
      cancel-label="Cancel"
      variant="warning"
      :loading="loading"
      @confirm="executeRetry"
      @cancel="showRetryConfirm = false; pendingRetryPlan = null"
    />
  </div>
</template>

<style scoped>
.history-page {
  width: 100%;
  min-width: 0;
}

.history-feedback {
  margin-bottom: var(--space-5);
}

.journal-section {
  display: flex;
  flex-direction: column;
  gap: var(--space-3);
  margin-bottom: var(--space-7);
}

.section-heading h3,
.journal-card h4 {
  margin: 0;
  color: var(--text-primary);
}

.section-heading p,
.undo-reason,
.journal-diagnostic {
  margin: var(--space-1) 0 0;
  color: var(--text-muted);
  font-size: var(--text-sm);
}

.unresolved-targets {
  display: grid;
  gap: var(--space-1);
  margin: var(--space-3) 0 0;
  padding: 0;
  list-style: none;
}

.unresolved-targets li {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--space-2);
  padding: var(--space-1) var(--space-2);
  background: var(--bg-surface);
  border-radius: var(--radius-md);
}

.unresolved-target-select {
  display: flex;
  align-items: center;
  min-width: 0;
  gap: var(--space-2);
}

.unresolved-target-select input {
  flex: 0 0 auto;
}

.unresolved-targets code {
  min-width: 0;
  overflow: hidden;
  color: var(--text-secondary);
  font-size: var(--text-xs);
  text-overflow: ellipsis;
  white-space: nowrap;
}

.unresolved-actions {
  display: flex;
  justify-content: flex-end;
  margin-top: var(--space-3);
}

.result-counts {
  display: flex;
  flex-wrap: wrap;
  gap: var(--space-2);
  margin: var(--space-3) 0;
}

.result-counts span {
  padding: var(--space-1) var(--space-2);
  border-radius: var(--radius-md);
  background: var(--bg-surface);
  color: var(--text-secondary);
  font-size: var(--text-xs);
}

.target-results {
  display: grid;
  gap: var(--space-1);
  margin: 0;
  padding: 0;
  list-style: none;
}

.target-results li {
  display: grid;
  grid-template-columns: minmax(0, 1fr) auto auto;
  gap: var(--space-3);
  color: var(--text-muted);
  font-size: var(--text-xs);
}

.target-results code {
  overflow-wrap: anywhere;
  color: var(--text-secondary);
}

.journal-diagnostic {
  color: var(--danger);
}

/* Stats */
.stats-row {
  display: flex;
  gap: var(--space-4);
  margin-bottom: var(--space-6);
}

.stat-card-sm {
  flex: 1;
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: var(--space-1);
  padding: var(--space-4);
  background: var(--bg-elevated);
  border: 1px solid var(--border-default);
  border-radius: var(--radius-xl);
}

.stat-num {
  font-size: var(--text-3xl);
  font-weight: var(--weight-bold);
  color: var(--text-primary);
  font-variant-numeric: tabular-nums;
  line-height: 1;
}

.stat-num.accent { color: var(--accent); }
.stat-num.warning { color: var(--warning); }

.stat-lbl {
  font-size: var(--text-xs);
  color: var(--text-muted);
  text-transform: uppercase;
  letter-spacing: var(--tracking-wide);
}

/* Timeline */
.timeline {
  display: flex;
  flex-direction: column;
  gap: var(--space-6);
}

.timeline-day {
  display: flex;
  flex-direction: column;
  gap: var(--space-3);
}

.timeline-date-header {
  display: flex;
  align-items: center;
  gap: var(--space-3);
}

.timeline-date-line {
  flex: 1;
  height: 1px;
  background: var(--border-subtle);
}

.timeline-date-label {
  font-size: var(--text-xs);
  font-weight: var(--weight-semibold);
  color: var(--text-muted);
  text-transform: uppercase;
  letter-spacing: var(--tracking-wide);
  white-space: nowrap;
}

.timeline-entries {
  display: flex;
  flex-direction: column;
  gap: var(--space-3);
  position: relative;
  padding-left: var(--space-4);
}

.timeline-entries::before {
  content: '';
  position: absolute;
  left: 7px;
  top: 0;
  bottom: 0;
  width: 2px;
  background: var(--border-subtle);
}

.timeline-entry {
  display: flex;
  gap: var(--space-3);
  position: relative;
}

.timeline-entry.is-rolled {
  opacity: 0.6;
}

.timeline-dot {
  width: 16px;
  height: 16px;
  border-radius: var(--radius-full);
  flex-shrink: 0;
  margin-top: 18px;
  z-index: 1;
  border: 3px solid var(--bg-base);
}

.timeline-dot.accent { background: var(--accent); }
.timeline-dot.warning { background: var(--warning); }
.timeline-dot.danger { background: var(--danger); }
.timeline-dot.info { background: var(--info); }
.timeline-dot.default { background: var(--text-muted); }

/* Entry Card */
.entry-card {
  flex: 1;
  min-width: 0;
  transition: all var(--duration-normal);
}

.entry-card.rolled {
  opacity: 0.7;
}

.entry-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
  margin-bottom: var(--space-2);
  gap: var(--space-3);
}

.entry-type {
  display: flex;
  align-items: center;
  gap: var(--space-2);
}

.entry-time {
  font-size: var(--text-xs);
  color: var(--text-muted);
}

.entry-meta {
  display: flex;
  align-items: center;
}

.entry-size {
  font-size: var(--text-xs);
  font-weight: var(--weight-semibold);
  color: var(--text-muted);
}

.entry-target {
  font-family: var(--font-mono);
  font-size: var(--text-xs);
  color: var(--text-secondary);
  line-height: var(--leading-relaxed);
  word-break: break-all;
  margin-bottom: var(--space-3);
  padding: var(--space-2);
  background: var(--bg-surface);
  border-radius: var(--radius-md);
}

.entry-footer {
  display: flex;
  justify-content: flex-end;
}

.entry-rolled-badge {
  display: flex;
  align-items: center;
  gap: var(--space-1);
  font-size: var(--text-xs);
  color: var(--text-muted);
  padding: var(--space-1) var(--space-2);
  background: var(--bg-surface);
  border-radius: var(--radius-md);
}

@media (max-width: 700px) {
  .stats-row,
  .entry-header,
  .entry-type {
    flex-direction: column;
    align-items: stretch;
  }

  .timeline-entries {
    padding-left: var(--space-2);
  }

  .timeline-entry {
    gap: var(--space-2);
  }

  .timeline-dot {
    width: 12px;
    height: 12px;
    border-width: 2px;
  }
}
</style>
