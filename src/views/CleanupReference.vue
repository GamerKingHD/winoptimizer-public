<script setup lang="ts">
import { computed, nextTick, ref } from 'vue'
import AppHeader from '../components/AppHeader.vue'
import BaseButton from '../components/BaseButton.vue'
import BaseCard from '../components/BaseCard.vue'
import ConfirmDialog from '../components/ConfirmDialog.vue'
import OperationPreview from '../components/OperationPreview.vue'
import EmptyState from '../components/EmptyState.vue'
import ResultsBanner from '../components/ResultsBanner.vue'
import { checkElevated } from '../api/tauri'
import {
  cancelCleanup,
  createCleanupPlan,
  executeCleanupPlan,
  scanCleanupTargets,
  type CleanupExecution,
  type CleanupDeletionMode,
  type CleanupScan,
  type CleanupTarget,
} from '../api/cleanup'
import { confirmAndRequestElevation, type OperationPlan } from '../api/operation'
import { formatBytes } from '../utils/format'

const scan = ref<CleanupScan | null>(null)
const selected = ref(new Set<string>())
const loading = ref(false)
const executing = ref(false)
const error = ref('')
const showConfirmation = ref(false)
const confirmedPlan = ref<OperationPlan | null>(null)
const execution = ref<CleanupExecution | null>(null)
const statusMessage = ref('')
const scanButton = ref<InstanceType<typeof BaseButton> | null>(null)
const expandedCategories = ref(new Set<string>())
const deletionMode = ref<CleanupDeletionMode>('recycle_bin')

const categoryDetails: Record<string, { label: string; description: string }> = {
  temp: {
    label: 'Temporary files',
    description: 'Leftover working files that applications no longer need. Recommended for most users.',
  },
  prefetch: {
    label: 'Prefetch space',
    description: 'Optional space cleanup for Windows Prefetch data. No performance improvement is claimed.',
  },
  browser_cache: {
    label: 'Browser cache',
    description: 'Saved website images and files. Sites may load slower the next time you visit.',
  },
  update_cache: {
    label: 'Windows Update cache',
    description: 'Downloaded update files. Windows may download some of them again when needed.',
  },
  logs: {
    label: 'System logs',
    description: 'Diagnostic records used to investigate Windows and application problems.',
  },
}
const categoryOrder = ['temp', 'prefetch', 'browser_cache', 'update_cache', 'logs']
const riskOrder: CleanupTarget['risk'][] = ['low', 'medium', 'high', 'critical']

interface CleanupCategory {
  key: string
  categoryKey: string
  categoryLabel: string
  label: string
  description: string
  isSoftware: boolean
  targets: CleanupTarget[]
  selectableTargets: CleanupTarget[]
  cleanupTypes: CleanupTypeGroup[]
  sizeBytes: number
  selectedCount: number
  selectedBytes: number
  unavailableCount: number
  risk: CleanupTarget['risk']
  recommended: boolean
  requiresElevation: boolean
}

interface CleanupTypeGroup {
  key: string
  label: string
  targets: CleanupTarget[]
  sizeBytes: number
}

interface CleanupSection {
  key: string
  label: string
  description: string
  groups: CleanupCategory[]
}

const selectedTargets = computed(() =>
  (scan.value?.targets ?? []).filter(target => selected.value.has(target.target_id))
)
const selectedBytes = computed(() =>
  selectedTargets.value.reduce((total, target) => total + target.size_bytes, 0)
)
const selectedRequiresElevation = computed(() =>
  selectedTargets.value.some(target => target.requires_elevation)
)
const categoryGroups = computed<CleanupCategory[]>(() => {
  const grouped = new Map<string, CleanupTarget[]>()
  for (const target of scan.value?.targets ?? []) {
    const key = target.affected_application
      ? `software:${target.affected_application}`
      : `category:${target.category}`
    const targets = grouped.get(key) ?? []
    targets.push(target)
    grouped.set(key, targets)
  }

  return [...grouped.entries()]
    .map(([key, targets]) => {
      const categoryKey = targets[0].category
      const details = categoryDetails[categoryKey] ?? {
        label: categoryKey.replaceAll('_', ' '),
        description: 'Review these files before choosing whether to remove them.',
      }
      const affectedApplication = targets[0].affected_application
      const isSoftware = Boolean(affectedApplication)
      const selectableTargets = targets.filter(target => target.accessibility === 'accessible')
      const selectedTargets = selectableTargets.filter(target => selected.value.has(target.target_id))
      const types = new Map<string, { label: string; targets: CleanupTarget[] }>()
      for (const target of targets) {
        const cleanupType = target.cleanup_type || details.label
        const groupName = target.group_name || details.label
        const typeKey = isSoftware ? cleanupType : `${groupName}:${cleanupType}`
        const label = isSoftware
          ? cleanupType
          : cleanupType && cleanupType !== 'Root'
            ? `${groupName} / ${cleanupType}`
            : groupName
        const existing = types.get(typeKey) ?? { label, targets: [] }
        existing.targets.push(target)
        types.set(typeKey, existing)
      }
      const cleanupTypes = [...types.entries()]
        .map(([typeKey, type]) => ({
          key: typeKey,
          label: type.label,
          targets: type.targets,
          sizeBytes: type.targets.reduce((total, target) => total + target.size_bytes, 0),
        }))
        .sort((left, right) => right.sizeBytes - left.sizeBytes || left.label.localeCompare(right.label))
      return {
        key,
        categoryKey,
        categoryLabel: details.label,
        label: affectedApplication ?? details.label,
        description: isSoftware
          ? 'Cleanup findings for this program. Only detected cleanup types are shown.'
          : details.description,
        isSoftware,
        targets,
        selectableTargets,
        cleanupTypes,
        sizeBytes: targets.reduce((total, target) => total + target.size_bytes, 0),
        selectedCount: selectedTargets.length,
        selectedBytes: selectedTargets.reduce((total, target) => total + target.size_bytes, 0),
        unavailableCount: targets.length - selectableTargets.length,
        risk: targets.reduce(
          (highest, target) =>
            riskOrder.indexOf(target.risk) > riskOrder.indexOf(highest) ? target.risk : highest,
          'low' as CleanupTarget['risk']
        ),
        recommended: targets.some(target => target.recommended),
        requiresElevation: targets.some(target => target.requires_elevation),
      }
    })
    .sort((left, right) => {
      const leftIndex = categoryOrder.indexOf(left.categoryKey)
      const rightIndex = categoryOrder.indexOf(right.categoryKey)
      const leftOrder = leftIndex === -1 ? categoryOrder.length : leftIndex
      const rightOrder = rightIndex === -1 ? categoryOrder.length : rightIndex
      return leftOrder - rightOrder || left.label.localeCompare(right.label)
    })
})
const cleanupSections = computed<CleanupSection[]>(() => {
  const software = categoryGroups.value.filter(group => group.isSoftware)
  const system = categoryGroups.value.filter(group => !group.isSoftware)
  return [
    {
      key: 'software',
      label: 'Popular software',
      description: 'Program-specific findings. Browser cache is scanned. Cookies and saved sign-in data are not scanned or removed.',
      groups: software,
    },
    {
      key: 'system',
      label: 'Windows and shared cleanup',
      description: 'Temporary files, optional Prefetch space, Windows caches, and diagnostic data grouped by cleanup source.',
      groups: system,
    },
  ].filter(section => section.groups.length > 0)
})

function failureMessage(value: unknown) {
  if (typeof value === 'object' && value && 'message' in value) {
    return String((value as { message: unknown }).message)
  }
  return String(value)
}

async function runScan() {
  loading.value = true
  error.value = ''
  execution.value = null
  statusMessage.value = ''
  try {
    const result = await scanCleanupTargets()
    scan.value = result
    expandedCategories.value = new Set()
    selected.value = new Set(
      result.targets.filter(target => target.recommended).map(target => target.target_id)
    )
  } catch (value) {
    error.value = failureMessage(value)
  } finally {
    loading.value = false
  }
}

function toggle(target: CleanupTarget) {
  const next = new Set(selected.value)
  if (next.has(target.target_id)) next.delete(target.target_id)
  else next.add(target.target_id)
  selected.value = next
}

function toggleCategory(category: CleanupCategory) {
  const next = new Set(selected.value)
  const allSelected = category.selectableTargets.length > 0
    && category.selectableTargets.every(target => next.has(target.target_id))
  for (const target of category.selectableTargets) {
    if (allSelected) next.delete(target.target_id)
    else next.add(target.target_id)
  }
  selected.value = next
}

function toggleCategoryDetails(category: string) {
  const next = new Set(expandedCategories.value)
  if (next.has(category)) next.delete(category)
  else next.add(category)
  expandedCategories.value = next
}

async function reviewSelection() {
  if (!scan.value || selected.value.size === 0) return
  error.value = ''
  try {
    confirmedPlan.value = await createCleanupPlan(scan.value, [...selected.value], deletionMode.value)
    showConfirmation.value = true
  } catch (value) {
    error.value = failureMessage(value)
  }
}

async function confirmCleanup() {
  const plan = confirmedPlan.value
  if (!plan) return
  showConfirmation.value = false
  error.value = ''
  if (plan.requires_elevation && !(await checkElevated())) {
    try {
      await confirmAndRequestElevation(plan)
      statusMessage.value = 'Administrator approval requested. Progress and results will reopen in History.'
    } catch (value) {
      error.value = failureMessage(value)
    }
    return
  }

  executing.value = true
  try {
    execution.value = await executeCleanupPlan(plan)
    scan.value = scan.value
      ? {
          ...scan.value,
          targets: scan.value.targets.filter(target =>
            execution.value?.result.outcomes.some(outcome =>
              outcome.target_id === target.target_id
              && outcome.execution === 'changed'
              && outcome.verification === 'verified'
            ) !== true
          ),
        }
      : null
    selected.value = new Set()
    statusMessage.value = 'Cleanup finished and was recorded in History.'
  } catch (value) {
    error.value = failureMessage(value)
  } finally {
    executing.value = false
    await nextTick()
    ;(scanButton.value?.$el as HTMLElement | undefined)?.focus()
  }
}

async function cancelRunningCleanup() {
  if (!confirmedPlan.value) return
  try {
    await cancelCleanup(confirmedPlan.value.id)
    statusMessage.value = 'Cancellation requested. The current target will finish first.'
  } catch (value) {
    error.value = failureMessage(value)
  }
}

</script>

<template>
  <div class="cleanup-page">
    <AppHeader />

    <div class="toolbar">
      <BaseButton ref="scanButton" icon="refresh" :loading="loading" @click="runScan">
        {{ scan ? 'Scan again' : 'Scan' }}
      </BaseButton>
      <BaseButton
        variant="accent"
        icon="trash"
        :disabled="selected.size === 0 || loading || executing"
        @click="reviewSelection"
      >
        Review {{ selected.size }} selected
      </BaseButton>
      <BaseButton v-if="executing" variant="danger" icon="x" @click="cancelRunningCleanup">
        Cancel cleanup
      </BaseButton>
      <label class="deletion-mode">
        <span>Deletion mode</span>
        <select v-model="deletionMode" :disabled="executing || showConfirmation">
          <option value="recycle_bin">Move to Recycle Bin (default)</option>
          <option value="permanent">Permanently delete (explicit)</option>
        </select>
      </label>
      <span v-if="selected.size" class="selection-summary">
        {{ formatBytes(selectedBytes) }} estimated
        <span v-if="selectedRequiresElevation"> · Administrator approval required</span>
      </span>
    </div>

    <ResultsBanner
      v-if="error"
      variant="error"
      title="Clean Up could not continue"
      :description="error"
      class="message"
    />
    <ResultsBanner
      v-else-if="statusMessage"
      variant="info"
      title="Clean Up status"
      :description="statusMessage"
      class="message"
    />

    <BaseCard v-if="loading && !scan" class="state-card">
      <p role="status">Scanning cleanable targets without changing the system…</p>
    </BaseCard>

    <BaseCard v-else-if="scan && scan.targets.length" class="targets-card">
      <div class="scan-heading">
        <div>
          <h2>Choose what to clean</h2>
          <p>
            {{ scan.targets.length }} files in {{ categoryGroups.length }} program or source groups ·
            {{ formatBytes(scan.estimated_bytes) }} available
          </p>
          <p class="selection-help">
            Recommended files are selected. Choose whole categories, or review individual files when needed.
          </p>
        </div>
        <time :datetime="scan.scanned_at">Scanned {{ new Date(scan.scanned_at).toLocaleString() }}</time>
      </div>

      <div class="cleanup-sections">
        <section v-for="section in cleanupSections" :key="section.key" class="cleanup-section">
          <div class="section-heading">
            <h3>{{ section.label }}</h3>
            <p>{{ section.description }}</p>
          </div>
          <div class="category-list" role="list" :aria-label="section.label">
            <section
              v-for="category in section.groups"
              :key="category.key"
              class="category-card"
              :class="{ 'is-selected': category.selectedCount > 0 }"
              role="listitem"
            >
              <div class="category-summary">
                <div class="category-copy">
                  <div class="category-title">
                    <h4>{{ category.label }}</h4>
                    <span v-if="category.isSoftware" class="category-badge">{{ category.categoryLabel }}</span>
                    <span class="risk-badge" :class="`risk-${category.risk}`">{{ category.risk }} risk</span>
                    <span v-if="category.recommended" class="recommended-badge">Recommended</span>
                  </div>
                  <p>{{ category.description }}</p>
                  <div class="cleanup-type-summary">
                    <span
                      v-for="cleanupType in category.cleanupTypes.slice(0, category.isSoftware ? 12 : 4)"
                      :key="cleanupType.key"
                      class="cleanup-type-chip"
                    >
                      <strong>{{ cleanupType.label }}</strong>
                      <span>
                        {{ cleanupType.targets.length }} {{ cleanupType.targets.length === 1 ? 'file' : 'files' }} found ·
                        {{ formatBytes(cleanupType.sizeBytes) }}
                      </span>
                    </span>
                    <span
                      v-if="category.cleanupTypes.length > (category.isSoftware ? 12 : 4)"
                      class="more-types"
                    >
                      +{{ category.cleanupTypes.length - (category.isSoftware ? 12 : 4) }} more groups
                    </span>
                  </div>
                  <div class="category-meta">
                    <strong>{{ formatBytes(category.sizeBytes) }}</strong>
                    <span>{{ category.targets.length }} {{ category.targets.length === 1 ? 'file' : 'files' }} found</span>
                    <span>{{ category.cleanupTypes.length }} cleanup {{ category.cleanupTypes.length === 1 ? 'type' : 'types' }}</span>
                    <span v-if="category.requiresElevation">Administrator approval</span>
                    <span v-if="category.unavailableCount">{{ category.unavailableCount }} unavailable</span>
                  </div>
                </div>

                <div class="category-actions">
                  <span class="category-selection">
                    <template v-if="category.selectedCount">
                      {{ category.selectedCount }} selected · {{ formatBytes(category.selectedBytes) }}
                    </template>
                    <template v-else>Not selected</template>
                  </span>
                  <button
                    type="button"
                    class="category-select"
                    :disabled="category.selectableTargets.length === 0"
                    @click="toggleCategory(category)"
                  >
                    {{
                      category.selectedCount === category.selectableTargets.length
                        ? 'Clear group'
                        : 'Select group'
                    }}
                  </button>
                  <button
                    type="button"
                    class="category-details-button"
                    :aria-expanded="expandedCategories.has(category.key)"
                    :aria-controls="`cleanup-category-${category.key}`"
                    @click="toggleCategoryDetails(category.key)"
                  >
                    {{ expandedCategories.has(category.key) ? 'Hide files' : 'Review files' }}
                  </button>
                </div>
              </div>

              <div
                v-if="expandedCategories.has(category.key)"
                :id="`cleanup-category-${category.key}`"
                class="category-details"
              >
                <div class="details-heading">
                  <strong>What the scan found</strong>
                  <span>Only accessible files can be selected.</span>
                </div>
                <section
                  v-for="cleanupType in category.cleanupTypes"
                  :key="cleanupType.key"
                  class="cleanup-type-details"
                >
                  <div class="cleanup-type-heading">
                    <strong>{{ cleanupType.label }}</strong>
                    <span>
                      {{ cleanupType.targets.length }} {{ cleanupType.targets.length === 1 ? 'file' : 'files' }} ·
                      {{ formatBytes(cleanupType.sizeBytes) }}
                    </span>
                  </div>
                  <ul class="target-list" :aria-label="`${category.label} ${cleanupType.label} cleanup targets`">
                    <li v-for="target in cleanupType.targets" :key="target.target_id">
                      <label class="target-row">
                        <input
                          type="checkbox"
                          :checked="selected.has(target.target_id)"
                          :disabled="target.accessibility !== 'accessible'"
                          @change="toggle(target)"
                        >
                        <span class="target-copy">
                          <strong>{{ target.description }}</strong>
                          <span class="path">{{ target.resource_id }}</span>
                          <span class="metadata">
                            {{ formatBytes(target.size_bytes) }} · {{ target.accessibility }}
                          </span>
                          <span class="reason">{{ target.recommendation_reason }}</span>
                        </span>
                      </label>
                    </li>
                  </ul>
                </section>
              </div>
            </section>
          </div>
        </section>
      </div>
    </BaseCard>

    <EmptyState
      v-else-if="!scan && !loading"
      icon="search"
      title="Scan to review cleanable files"
      description="Clean Up stays idle until you start a scan."
      action-label="Scan"
      @action="runScan"
    />

    <EmptyState
      v-else-if="scan && !loading"
      icon="check"
      title="No cleanable targets found"
      description="The scan completed without finding files eligible for this cleanup workflow."
      action-label="Scan again"
      @action="runScan"
    />

    <BaseCard v-if="scan?.failures.length" class="scan-warnings">
      <h2>Scan warnings</h2>
      <ul><li v-for="failure in scan.failures" :key="failure">{{ failure }}</li></ul>
    </BaseCard>

    <BaseCard v-if="execution" class="result-card" aria-live="polite">
      <h2>Measured cleanup result</h2>
      <dl class="result-grid">
        <div><dt>Requested</dt><dd>{{ execution.result.requested }}</dd></div>
        <div><dt>Removed</dt><dd>{{ execution.result.changed }}</dd></div>
        <div><dt>Already absent</dt><dd>{{ execution.result.unchanged }}</dd></div>
        <div><dt>Skipped</dt><dd>{{ execution.result.skipped }}</dd></div>
        <div><dt>Failed</dt><dd>{{ execution.result.failed }}</dd></div>
        <div><dt>Verified</dt><dd>{{ execution.result.verified }}</dd></div>
        <div><dt>Unverified</dt><dd>{{ execution.result.unverified }}</dd></div>
        <div><dt>Estimated bytes</dt><dd>{{ formatBytes(execution.estimated_bytes) }}</dd></div>
        <div><dt>Measured disk-space delta</dt><dd>{{ execution.measured_disk_space_delta == null ? 'Unavailable' : formatBytes(execution.measured_disk_space_delta) }}</dd></div>
        <div><dt>Reboot</dt><dd>{{ execution.result.reboot_required ? 'Required' : 'Not required' }}</dd></div>
        <div><dt>Undo</dt><dd>{{ execution.result.undo === 'not_reversible' ? 'Unavailable' : 'Available' }}</dd></div>
        <div><dt>History ID</dt><dd class="identifier">{{ execution.journal_id }}</dd></div>
      </dl>
      <ul v-if="execution.result.outcomes.some(outcome => outcome.error)" class="outcome-errors">
        <li v-for="outcome in execution.result.outcomes.filter(item => item.error)" :key="outcome.target_id">
          {{ outcome.target_id }} — {{ outcome.error?.message }}
        </li>
      </ul>
    </BaseCard>

    <OperationPreview
      v-if="showConfirmation && confirmedPlan"
      :plan="confirmedPlan"
      title="Cleanup operation preview"
      action-label="Remove the selected targets"
      class="operation-preview-card"
    />

    <ConfirmDialog
      :open="showConfirmation"
      title="Confirm exact cleanup plan"
      :description="deletionMode === 'permanent'
        ? 'Files may have changed since this preview. Changed targets will be skipped. This is permanent deletion and exact undo is unavailable.'
        : 'Files may have changed since this preview. Changed targets will be skipped. Selected files will be moved to the Recycle Bin; exact WinOptimizer undo is unavailable.'"
      :items="selectedTargets.map(target => ({
        label: target.resource_id,
        value: formatBytes(target.size_bytes),
        risk: target.risk === 'critical' ? 'high' : target.risk,
      }))"
      :confirm-label="deletionMode === 'permanent' ? `Permanently delete ${selectedTargets.length} targets` : `Move ${selectedTargets.length} targets to Recycle Bin`"
      cancel-label="Keep files"
      variant="warning"
      :loading="executing"
      @confirm="confirmCleanup"
      @cancel="showConfirmation = false"
    />
  </div>
</template>

<style scoped>
.cleanup-page { width: 100%; min-width: 0; }
.toolbar { display: flex; align-items: center; flex-wrap: wrap; gap: var(--space-2); margin-bottom: var(--space-4); }
.selection-summary { color: var(--text-secondary); font-size: var(--font-size-sm); }
.message, .targets-card, .state-card, .scan-warnings, .result-card { margin-bottom: var(--space-4); }
.operation-preview-card { margin-bottom: var(--space-4); }
.scan-heading { display: flex; justify-content: space-between; gap: var(--space-4); align-items: start; margin-bottom: var(--space-3); }
.scan-heading h2, .scan-warnings h2, .result-card h2 { margin: 0 0 var(--space-1); }
.scan-heading p, .scan-heading time { color: var(--text-secondary); font-size: var(--font-size-sm); }
.scan-heading .selection-help { margin-top: var(--space-1); color: var(--text-muted); }
.cleanup-sections { display: grid; gap: var(--space-5); }
.section-heading { margin-bottom: var(--space-2); }
.section-heading h3 { margin: 0 0 var(--space-1); font-size: var(--text-lg); }
.section-heading p { margin: 0; color: var(--text-secondary); font-size: var(--text-sm); }
.category-list { display: grid; gap: var(--space-3); }
.category-card { overflow: hidden; border: 1px solid var(--border-default); border-radius: var(--radius-lg); background: var(--bg-elevated); }
.category-card.is-selected { border-color: var(--accent-300); box-shadow: inset 3px 0 0 var(--accent); }
.category-summary { display: grid; grid-template-columns: minmax(0, 1fr) auto; gap: var(--space-4); padding: var(--space-4); }
.category-copy { min-width: 0; }
.category-title { display: flex; align-items: center; flex-wrap: wrap; gap: var(--space-2); }
.category-title h4 { margin: 0; font-size: var(--text-lg); }
.category-copy > p { max-width: 680px; margin: var(--space-1) 0 var(--space-2); color: var(--text-secondary); }
.category-meta { display: flex; flex-wrap: wrap; gap: var(--space-2) var(--space-4); color: var(--text-muted); font-size: var(--text-sm); }
.category-meta strong { color: var(--text-primary); }
.risk-badge, .recommended-badge, .category-badge { padding: 2px var(--space-2); border-radius: var(--radius-full); font-size: var(--text-xs); font-weight: var(--weight-semibold); }
.risk-low { color: var(--success); background: var(--success-50); }
.risk-medium { color: var(--warning); background: var(--warning-50); }
.risk-high, .risk-critical { color: var(--danger); background: var(--danger-50); }
.recommended-badge { color: var(--accent); background: var(--accent-100); }
.category-badge { color: var(--text-secondary); background: var(--bg-surface); }
.cleanup-type-summary { display: flex; flex-wrap: wrap; gap: var(--space-2); margin-bottom: var(--space-2); }
.cleanup-type-chip { display: grid; gap: 1px; padding: var(--space-1) var(--space-2); border: 1px solid var(--border-subtle); border-radius: var(--radius-md); background: var(--bg-surface); font-size: var(--text-xs); }
.cleanup-type-chip span, .more-types { color: var(--text-muted); }
.more-types { align-self: center; font-size: var(--text-xs); }
.category-actions { display: grid; min-width: 148px; align-content: center; justify-items: stretch; gap: var(--space-2); }
.category-selection { color: var(--text-secondary); font-size: var(--text-xs); text-align: right; }
.category-select, .category-details-button { min-height: 32px; padding: var(--space-1) var(--space-3); border: 1px solid var(--border-default); border-radius: var(--radius-md); background: var(--bg-elevated); color: var(--text-primary); font: inherit; font-size: var(--text-sm); cursor: pointer; }
.category-select { border-color: var(--accent-200); background: var(--accent-50); color: var(--accent-700); font-weight: var(--weight-semibold); }
.category-select:hover, .category-details-button:hover { background: var(--bg-hover); }
.category-select:disabled { opacity: .45; cursor: not-allowed; }
.category-details { padding: 0 var(--space-4) var(--space-3); border-top: 1px solid var(--border-subtle); background: var(--bg-surface); }
.details-heading { display: flex; justify-content: space-between; gap: var(--space-3); padding: var(--space-3) 0; color: var(--text-secondary); font-size: var(--text-sm); }
.details-heading strong { color: var(--text-primary); }
.cleanup-type-details + .cleanup-type-details { margin-top: var(--space-3); }
.cleanup-type-heading { display: flex; justify-content: space-between; gap: var(--space-3); padding: var(--space-2); background: var(--bg-elevated); border-radius: var(--radius-md) var(--radius-md) 0 0; color: var(--text-secondary); font-size: var(--text-sm); }
.cleanup-type-heading strong { color: var(--text-primary); }
.target-list { list-style: none; margin: 0; padding: 0; max-height: 42vh; overflow: auto; border-top: 1px solid var(--border-subtle); }
.target-list li { border-bottom: 1px solid var(--border-subtle); }
.target-row { display: flex; gap: var(--space-3); padding: var(--space-3) 0; cursor: pointer; }
.target-row input { margin-top: 3px; flex: 0 0 auto; }
.target-copy { min-width: 0; display: grid; gap: 3px; }
.path, .metadata, .reason { overflow-wrap: anywhere; color: var(--text-secondary); font-size: var(--font-size-sm); }
.reason { color: var(--text-tertiary); }
.result-grid { display: grid; grid-template-columns: repeat(auto-fit, minmax(150px, 1fr)); gap: var(--space-3); }
.result-grid div { padding: var(--space-3); background: var(--bg-surface); border-radius: var(--radius-md); }
.result-grid dt { color: var(--text-secondary); font-size: var(--font-size-sm); }
.result-grid dd { margin: var(--space-1) 0 0; font-weight: 600; }
.identifier { overflow-wrap: anywhere; font-size: var(--font-size-xs); }
.outcome-errors { color: var(--status-danger); overflow-wrap: anywhere; }
@media (max-width: 900px) {
  .cleanup-page { min-width: 0; }
  .scan-heading { display: block; }
  .toolbar > * { max-width: 100%; }
  .category-summary { grid-template-columns: 1fr; }
  .category-actions { grid-template-columns: 1fr 1fr; min-width: 0; }
  .category-selection { grid-column: 1 / -1; text-align: left; }
  .details-heading { display: grid; }
}
</style>
