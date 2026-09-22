<script setup lang="ts">
import { computed, ref, shallowRef } from 'vue'
import { storeToRefs } from 'pinia'
import { useRouter } from 'vue-router'
import AppHeader from '../components/AppHeader.vue'
import BaseButton from '../components/BaseButton.vue'
import BaseBadge from '../components/BaseBadge.vue'
import { getOperationJournal, type OperationJournalRecord } from '../api/operation'
import { useSystemSnapshotStore } from '../stores/systemSnapshot'
import { formatPercent } from '../utils/format'
import { useScanStore } from '../stores/scan'

const router = useRouter()
const scanStore = useScanStore()
const snapshotStore = useSystemSnapshotStore()
const { hardware, os, hardwareError, hardwareUpdatedAt: lastUpdated, stale } = storeToRefs(snapshotStore)
const recentChanges = shallowRef<OperationJournalRecord[]>([])
const historyError = ref(false)

const storage = computed(() => {
  const disks = hardware.value?.disks ?? []
  const total = disks.reduce((sum, disk) => sum + disk.total_bytes, 0)
  const available = disks.reduce((sum, disk) => sum + disk.available_bytes, 0)
  return { total, available }
})

const storageUsedPercent = computed(() => {
  if (!storage.value.total) return null
  return ((storage.value.total - storage.value.available) / storage.value.total) * 100
})

const refreshedAt = computed(() =>
  lastUpdated.value
    ? lastUpdated.value.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit', second: '2-digit' })
    : 'Not available'
)

const systemIdentity = computed(() => {
  if (!os.value) return 'System details unavailable'
  return [os.value.version, os.value.edition, os.value.arch].filter(Boolean).join(' · ')
})

const latestChange = computed(() => recentChanges.value[0] ?? null)

function formatStatus(status: string) {
  return status.replaceAll('_', ' ')
}

async function refreshHardware() {
  await snapshotStore.refresh({ force: true })
}

async function loadSupportingData() {
  const historyResult = await Promise.allSettled([getOperationJournal()])
  const [history] = historyResult
  if (history.status === 'fulfilled') {
    recentChanges.value = history.value.slice(0, 3)
    historyError.value = false
  } else {
    recentChanges.value = []
    historyError.value = true
  }
}

void snapshotStore.refresh()
void loadSupportingData()
</script>

<template>
  <div class="dashboard">
    <AppHeader />

    <section class="system-strip" aria-label="Live system metrics">
      <div>
        <span>CPU</span>
        <strong>{{ hardware ? formatPercent(hardware.cpu.usage_percent) : '—' }}</strong>
      </div>
      <div>
        <span>Memory</span>
        <strong>{{ hardware ? formatPercent(hardware.memory.in_use_percent) : '—' }}</strong>
      </div>
      <div>
        <span>Storage</span>
        <strong>{{ storageUsedPercent === null ? '—' : formatPercent(storageUsedPercent) }}</strong>
      </div>
      <div class="metric-status">
        <span>Updated {{ refreshedAt }}</span>
        <BaseBadge :variant="hardwareError ? 'danger' : hardware && !stale ? 'success' : hardware ? 'warning' : 'default'" size="sm" dot>
          {{ hardwareError ? 'Collection failed' : hardware && stale ? 'Metrics stale' : hardware ? 'Metrics available' : 'Collecting data' }}
        </BaseBadge>
        <BaseButton variant="secondary" size="sm" @click="refreshHardware">
          Refresh
        </BaseButton>
      </div>
    </section>

    <section class="recommendations-link" aria-labelledby="recommendations-title">
      <div>
        <p class="eyebrow">Read-only review</p>
        <h2 id="recommendations-title">Recommendations are reviewed separately</h2>
        <p>Home shows factual system status. Open Recommendations to review optional guidance before entering a normal workflow.</p>
      </div>
      <BaseButton variant="secondary" @click="router.push('/recommendations')">
        Review recommendations
      </BaseButton>
    </section>

    <div class="home-grid">
      <section class="home-section" aria-labelledby="scan-title">
        <div>
          <p class="eyebrow">Latest scan</p>
          <h2 id="scan-title">{{ scanStore.totalScannableBytes ? scanStore.formatBytes(scanStore.totalScannableBytes) : 'No findings yet' }}</h2>
          <p>{{ scanStore.totalScannableBytes ? 'Potential cleanup findings are ready for review.' : 'Run a storage scan to create a reviewed list of findings.' }}</p>
        </div>
        <BaseButton variant="secondary" size="sm" @click="router.push('/disk-cleaner')">Review scan</BaseButton>
      </section>

      <section class="home-section" aria-labelledby="changes-title">
        <div>
          <p class="eyebrow">Recent changes</p>
          <template v-if="historyError">
            <h2 id="changes-title">History unavailable</h2>
            <p>Recent changes could not be loaded. Open History to try again.</p>
          </template>
          <template v-else-if="latestChange">
            <h2 id="changes-title">{{ latestChange.confirmed_plan.descriptor.title }}</h2>
            <p>Status: {{ formatStatus(latestChange.terminal_state) }}</p>
          </template>
          <template v-else>
            <h2 id="changes-title">No recorded changes</h2>
            <p>Completed and interrupted operations will appear in History &amp; Undo.</p>
          </template>
        </div>
        <BaseButton variant="secondary" size="sm" @click="router.push('/history')">
          {{ latestChange?.undo_eligibility.eligible && !historyError ? 'Review or undo' : 'Open history' }}
        </BaseButton>
      </section>
    </div>

    <button class="system-identity" type="button" @click="router.push('/system-info')">
      <span><strong>This PC</strong><small :title="systemIdentity">{{ systemIdentity }}</small></span>
      <span aria-hidden="true">View system details →</span>
    </button>
  </div>
</template>

<style scoped>
.dashboard { width: 100%; min-width: 0; }
.system-strip { display: grid; grid-template-columns: repeat(3, minmax(90px, 1fr)) minmax(210px, auto); gap: var(--space-3); align-items: center; margin-bottom: var(--space-5); padding: var(--space-3) var(--space-4); border: 1px solid var(--border-default); border-radius: var(--radius-lg); background: var(--bg-elevated); }
.system-strip > div { min-width: 0; display: flex; align-items: baseline; gap: var(--space-2); }
.system-strip span, .system-identity small { overflow: hidden; color: var(--text-muted); font-size: var(--text-xs); text-overflow: ellipsis; white-space: nowrap; }
.system-strip strong { color: var(--text-primary); font-variant-numeric: tabular-nums; }
.system-strip .metric-status { justify-content: flex-end; align-items: center; }
.recommendations-link { display: flex; align-items: center; justify-content: space-between; gap: var(--space-6); margin-bottom: var(--space-5); padding: var(--space-4); border: 1px solid var(--border-default); border-radius: var(--radius-lg); background: var(--bg-elevated); }
.recommendations-link h2, .home-section h2 { margin: var(--space-1) 0 var(--space-2); color: var(--text-primary); font-size: var(--text-xl); }
.recommendations-link p, .home-section p { margin: 0; color: var(--text-secondary); line-height: var(--leading-normal); }
.eyebrow { color: var(--accent) !important; font-size: var(--text-xs); font-weight: var(--weight-semibold); letter-spacing: var(--tracking-wide); text-transform: uppercase; }
.home-grid { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: var(--space-4); margin-bottom: var(--space-4); }
.home-section { display: flex; min-width: 0; align-items: flex-end; justify-content: space-between; gap: var(--space-4); padding: var(--space-4); border: 1px solid var(--border-default); border-radius: var(--radius-lg); background: var(--bg-elevated); }
.system-identity { display: flex; width: 100%; min-width: 0; align-items: center; justify-content: space-between; gap: var(--space-4); padding: var(--space-3) var(--space-4); border: 1px solid var(--border-default); border-radius: var(--radius-lg); background: transparent; color: var(--text-secondary); text-align: left; cursor: pointer; }
.system-identity:hover { border-color: var(--accent); color: var(--accent); }
.system-identity span:first-child { display: flex; min-width: 0; align-items: center; gap: var(--space-3); }
.system-identity strong { color: var(--text-primary); white-space: nowrap; }
.system-identity small { max-width: 520px; }
@media (max-width: 760px) {
  .system-strip { grid-template-columns: repeat(3, 1fr); }
  .system-strip .metric-status { grid-column: 1 / -1; justify-content: space-between; }
  .recommendations-link, .home-section { align-items: stretch; flex-direction: column; }
  .home-grid { grid-template-columns: 1fr; }
}
@media (max-width: 520px) {
  .system-strip { grid-template-columns: 1fr; }
  .system-strip .metric-status { grid-column: auto; align-items: flex-start; flex-direction: column; }
  .system-identity span:last-child { display: none; }
}
</style>
