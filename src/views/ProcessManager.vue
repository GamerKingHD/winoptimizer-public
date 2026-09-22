<script setup lang="ts">
import { computed, ref, shallowRef } from 'vue'
import type { ProcessInfo } from '../api/tauri'
import * as api from '../api/tauri'
import type { OperationPlan } from '../api/operation'
import { confirmAndRequestElevation } from '../api/operation'
import { createProcessPlan, executeProcessPlan } from '../api/processes'
import AppHeader from '../components/AppHeader.vue'
import BaseCard from '../components/BaseCard.vue'
import BaseButton from '../components/BaseButton.vue'
import BaseTable from '../components/BaseTable.vue'
import BaseBadge from '../components/BaseBadge.vue'
import BaseSearchInput from '../components/BaseSearchInput.vue'
import BaseProgress from '../components/BaseProgress.vue'
import EmptyState from '../components/EmptyState.vue'
import ConfirmDialog from '../components/ConfirmDialog.vue'
import ResultsBanner from '../components/ResultsBanner.vue'
import { failure, success, type ActionFeedback } from '../utils/feedback'
import { formatBytes } from '../utils/format'
import { usePolling } from '../composables/usePolling'

const processes = shallowRef<ProcessInfo[]>([])
const loading = ref(false)
const sortKey = ref<'cpu' | 'memory'>('cpu')
const filterName = ref('')
const feedback = ref<ActionFeedback | null>(null)

// Kill confirmation state
const showKillConfirm = ref(false)
const pendingKillPid = ref(0)
const pendingKillName = ref('')
const killLoading = ref(false)
const pendingKillPlan = ref<OperationPlan | null>(null)

const killDescription = computed(() =>
  `The exact process target "${pendingKillName.value}" (PID: ${pendingKillPid.value}) will be terminated immediately. Unsaved data may be lost.`
)

async function refresh() {
  loading.value = true
  try {
    processes.value = await api.processSnapshot()
    feedback.value = null
  } catch (e) {
    feedback.value = failure('Could not load processes', e)
  } finally {
    loading.value = false
  }
}

async function killProcess(pid: number, name: string) {
  const process = processes.value.find(candidate => candidate.pid === pid && candidate.name === name)
  if (!process) {
    feedback.value = failure('Could not prepare termination', 'Process is no longer present.')
    return
  }
  try {
    pendingKillPlan.value = await createProcessPlan(process)
  } catch (e) {
    feedback.value = failure(`Cannot terminate ${name}`, e)
    return
  }
  pendingKillPid.value = pid
  pendingKillName.value = name
  showKillConfirm.value = true
}

async function onConfirmKill() {
  const pid = pendingKillPid.value
  const name = pendingKillName.value
  showKillConfirm.value = false
  killLoading.value = true
  try {
    const plan = pendingKillPlan.value
    if (!plan) throw new Error('The termination plan is no longer available.')
    if (plan.requires_elevation && !(await api.checkElevated())) {
      await confirmAndRequestElevation(plan)
      feedback.value = success(
        'Administrator approval requested',
        'The exact confirmed process identity will be terminated after elevation.',
      )
      return
    }
    const execution = await executeProcessPlan(plan)
    if (execution.result.failed > 0) throw new Error(execution.message)
    processes.value = processes.value.filter(p => p.pid !== pid)
    feedback.value = success('Process terminated', `${name} (PID ${pid}) was terminated.`)
  } catch (e) {
    feedback.value = failure(`Could not terminate ${name}`, e)
  } finally {
    killLoading.value = false
    pendingKillPid.value = 0
    pendingKillName.value = ''
    pendingKillPlan.value = null
  }
}

// CPU sparkline as simple CSS bar
function cpuBar(cpu: number): string {
  return Math.min(100, cpu * 2) + '%'
}

const filteredProcesses = computed(() => {
  let list = [...processes.value]
  if (filterName.value) {
    const f = filterName.value.toLowerCase()
    list = list.filter(p => p.name.toLowerCase().includes(f))
  }
  if (sortKey.value === 'cpu') list.sort((a, b) => b.cpu_usage - a.cpu_usage)
  else list.sort((a, b) => b.memory_bytes - a.memory_bytes)
  return list.slice(0, 200)
})

const totalCpu = computed(() => processes.value.reduce((s, p) => s + p.cpu_usage, 0))
const totalMem = computed(() => processes.value.reduce((s, p) => s + p.memory_bytes, 0))

const columns = [
  { key: 'pid', label: 'PID', sortable: true, width: '70px', align: 'right' as const },
  { key: 'name', label: 'Process Name', sortable: true },
  { key: 'cpu', label: 'CPU', sortable: true, width: '140px', align: 'right' as const },
  { key: 'memory', label: 'Memory', sortable: true, width: '110px', align: 'right' as const },
  { key: 'status', label: 'Status', sortable: true, width: '90px' },
]

const processRows = computed(() =>
  filteredProcesses.value.map(p => ({
    _id: p.pid,
    pid: p.pid,
    name: p.name,
    cpu: p.cpu_usage.toFixed(1),
    cpu_raw: p.cpu_usage,
    memory: formatBytes(p.memory_bytes),
    mem_raw: p.memory_bytes,
    status: p.status,
    path: p.path || '',
    start_time: p.start_time,
  }))
)

usePolling(refresh, 3000, { immediate: true })
</script>

<template>
  <div class="process-manager">
    <AppHeader />

    <ResultsBanner
      v-if="feedback"
      :variant="feedback.type"
      :title="feedback.title"
      :description="feedback.message"
      class="process-feedback"
      @dismiss="feedback = null"
    />

    <!-- Stats bar -->
    <div class="stats-row">
      <BaseCard variant="glass" padding="sm" class="stat-chip">
        <span class="stat-chip-icon">#</span>
        <span class="stat-chip-label">Processes</span>
        <span class="stat-chip-value">{{ processes.length }}</span>
      </BaseCard>
      <BaseCard variant="glass" padding="sm" class="stat-chip">
        <span class="stat-chip-icon">%</span>
        <span class="stat-chip-label">Total CPU</span>
        <span class="stat-chip-value">{{ totalCpu.toFixed(1) }}%</span>
      </BaseCard>
      <BaseCard variant="glass" padding="sm" class="stat-chip">
        <span class="stat-chip-icon">MB</span>
        <span class="stat-chip-label">Total Memory</span>
        <span class="stat-chip-value">{{ formatBytes(totalMem) }}</span>
      </BaseCard>
    </div>

    <!-- Toolbar -->
    <div class="toolbar">
      <div class="toolbar-left">
        <BaseSearchInput v-model="filterName" placeholder="Filter by name..." size="md" />
        <BaseButton variant="primary" icon="refresh" @click="refresh" :loading="loading" size="sm">
          Refresh
        </BaseButton>
      </div>
      <div class="toolbar-right">
        <BaseBadge variant="warning" size="sm">Experimental</BaseBadge>
        <div class="sort-group">
          <button class="sort-btn" :class="{ active: sortKey === 'cpu' }" @click="sortKey = 'cpu'">CPU</button>
          <button class="sort-btn" :class="{ active: sortKey === 'memory' }" @click="sortKey = 'memory'">Memory</button>
        </div>
      </div>
    </div>

    <EmptyState
      v-if="processes.length === 0 && !loading"
      icon="activity"
      title="No Processes Loaded"
      description="Click Refresh to load running processes."
    />

    <!-- Process table -->
    <BaseTable
      v-else
      :columns="columns"
      :rows="processRows"
      :loading="loading"
      max-height="560px"
      compact
      striped
    >
      <template #cell-pid="{ row }">
        <span class="pid-cell">{{ row.pid }}</span>
      </template>
      <template #cell-name="{ row }">
        <div class="name-cell">
          <span class="proc-name">{{ row.name }}</span>
          <span v-if="row.path" class="proc-path" :title="row.path">{{ row.path.substring(0, 50) }}{{ row.path.length > 50 ? '...' : '' }}</span>
        </div>
      </template>
      <template #cell-cpu="{ row }">
        <div class="cpu-cell">
          <div class="cpu-bar-track">
            <div class="cpu-bar-fill" :style="{ width: cpuBar(row.cpu_raw) }" />
          </div>
          <span class="cpu-val">{{ row.cpu }}%</span>
        </div>
      </template>
      <template #cell-memory="{ row }">
        <span class="mem-cell">{{ row.memory }}</span>
      </template>
      <template #cell-status="{ row }">
        <BaseBadge :variant="row.status === 'Running' ? 'success' : 'default'" size="sm" :dot="row.status === 'Running'">
          {{ row.status }}
        </BaseBadge>
      </template>
      <template #row-actions="{ row }">
        <BaseButton variant="danger" size="sm" icon="x" @click="killProcess(row.pid, row.name)">
          Kill
        </BaseButton>
      </template>
    </BaseTable>

    <!-- Kill Confirmation Dialog -->
    <ConfirmDialog
      :open="showKillConfirm"
      :operation-plan="pendingKillPlan"
      title="Review process termination"
      :description="killDescription"
      :items="[
        { label: pendingKillName, value: pendingKillPlan?.targets[0]?.description || ('PID ' + pendingKillPid), risk: 'high' },
        { label: 'Privilege', value: pendingKillPlan?.descriptor.privilege || 'unknown', risk: pendingKillPlan?.requires_elevation ? 'high' : 'medium' },
        { label: 'Undo', value: 'Unavailable; unsaved data may be lost', risk: 'high' }
      ]"
      confirm-label="Terminate Process"
      cancel-label="Cancel"
      variant="danger"
      icon="x-circle"
      :loading="killLoading"
      @confirm="onConfirmKill"
      @cancel="showKillConfirm = false; pendingKillPlan = null"
    />
  </div>
</template>

<style scoped>
.process-manager {
  width: 100%;
  min-width: 0;
}

.process-feedback {
  margin-bottom: var(--space-5);
}

/* Stats row */
.stats-row {
  display: flex;
  gap: var(--space-3);
  margin-bottom: var(--space-4);
}

.stat-chip {
  display: flex;
  align-items: center;
  gap: var(--space-2);
  padding: var(--space-2) var(--space-4) !important;
  min-height: auto !important;
}

.stat-chip-icon {
  font-size: var(--text-lg);
}

.stat-chip-label {
  font-size: var(--text-xs);
  color: var(--text-muted);
}

.stat-chip-value {
  font-size: var(--text-sm);
  font-weight: var(--weight-bold);
  color: var(--text-primary);
  font-variant-numeric: tabular-nums;
}

/* Toolbar */
.toolbar {
  display: flex;
  justify-content: space-between;
  align-items: center;
  margin-bottom: var(--space-4);
  gap: var(--space-3);
}

.toolbar-left {
  display: flex;
  align-items: center;
  gap: var(--space-2);
}

.toolbar-right {
  display: flex;
  align-items: center;
}

.sort-group {
  display: flex;
  background: var(--bg-elevated);
  border: 1px solid var(--border-default);
  border-radius: var(--radius-lg);
  overflow: hidden;
}

.sort-btn {
  padding: var(--space-1) var(--space-3);
  border: none;
  background: transparent;
  color: var(--text-muted);
  font-size: var(--text-xs);
  font-weight: var(--weight-medium);
  cursor: pointer;
  transition: all var(--duration-fast);
}

.sort-btn.active {
  background: var(--accent);
  color: var(--text-inverse);
}

.sort-btn:not(.active):hover {
  background: var(--glass-bg-hover);
}

/* PID */
.pid-cell {
  font-family: var(--font-mono);
  font-size: var(--text-xs);
  color: var(--text-muted);
}

/* Name */
.name-cell {
  display: flex;
  flex-direction: column;
  gap: 1px;
  min-width: 0;
}

.proc-name {
  font-size: var(--text-sm);
  font-weight: var(--weight-medium);
  color: var(--text-primary);
}

.proc-path {
  font-size: var(--text-xs);
  color: var(--text-muted);
  font-family: var(--font-mono);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  max-width: 300px;
}

/* CPU bar */
.cpu-cell {
  display: flex;
  align-items: center;
  gap: var(--space-2);
  justify-content: flex-end;
}

.cpu-bar-track {
  width: 60px;
  height: 6px;
  background: var(--bg-surface);
  border-radius: var(--radius-full);
  overflow: hidden;
}

.cpu-bar-fill {
  height: 100%;
  border-radius: var(--radius-full);
  background: var(--accent-gradient);
  transition: width 0.3s var(--ease-out);
  min-width: 2px;
}

.cpu-val {
  font-size: var(--text-xs);
  font-weight: var(--weight-semibold);
  color: var(--text-secondary);
  font-variant-numeric: tabular-nums;
  min-width: 40px;
  text-align: right;
}

/* Memory */
.mem-cell {
  font-size: var(--text-sm);
  font-weight: var(--weight-semibold);
  color: var(--text-primary);
  font-variant-numeric: tabular-nums;
}

@media (max-width: 700px) {
  .stats-row {
    flex-direction: column;
  }

  .toolbar {
    flex-direction: column;
    align-items: stretch;
  }

  .toolbar-left {
    flex-direction: column;
    align-items: stretch;
  }

  .toolbar-right {
    justify-content: flex-start;
  }
}
</style>
