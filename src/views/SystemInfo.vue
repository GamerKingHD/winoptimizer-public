<script setup lang="ts">
import { computed, ref } from 'vue'
import { storeToRefs } from 'pinia'
import type { UpdateInfo } from '../api/tauri'
import * as api from '../api/tauri'
import AppHeader from '../components/AppHeader.vue'
import BaseCard from '../components/BaseCard.vue'
import BaseButton from '../components/BaseButton.vue'
import BaseStatCard from '../components/BaseStatCard.vue'
import BaseBadge from '../components/BaseBadge.vue'
import BaseProgress from '../components/BaseProgress.vue'
import LoadingSpinner from '../components/LoadingSpinner.vue'
import EmptyState from '../components/EmptyState.vue'
import ResultsBanner from '../components/ResultsBanner.vue'
import { failure, type ActionFeedback } from '../utils/feedback'
import { formatBytes, formatPercent } from '../utils/format'
import { usePolling } from '../composables/usePolling'
import { useSystemSnapshotStore } from '../stores/systemSnapshot'

const snapshotStore = useSystemSnapshotStore()
const { hardware: hw, os, hardwareError, osError, stale } = storeToRefs(snapshotStore)
const updates = ref<UpdateInfo[]>([])
const battery = ref<api.BatteryInfo | null>(null)
const loading = ref(false)
const updatesLoading = ref(false)
const feedback = ref<ActionFeedback | null>(null)

const memUsedPercent = computed(() => {
  if (!hw.value) return 0
  return hw.value.memory.in_use_percent
})

const diskUsagePercent = (disk: { available_bytes: number; total_bytes: number }) => {
  return ((1 - disk.available_bytes / disk.total_bytes) * 100).toFixed(1)
}

async function refreshSystemInfo() {
  loading.value = true
  await snapshotStore.refresh({ force: true })
  const snapshotError = hardwareError.value || osError.value
  feedback.value = snapshotError
    ? failure('Could not gather system information', snapshotError)
    : null
  loading.value = false
  void loadOptionalSystemInfo()
}

async function loadOptionalSystemInfo() {
  updatesLoading.value = true
  const [updatesData, batteryData] = await Promise.all([
    api.getInstalledUpdates().catch(() => [] as UpdateInfo[]),
    api.getBatteryInfo().catch(() => null),
  ])
  updates.value = updatesData
  battery.value = batteryData
  updatesLoading.value = false
}

usePolling(refreshSystemInfo, 30000, { immediate: true })
</script>

<template>
  <div class="system-info">
    <AppHeader>
      <template #actions>
        <BaseBadge v-if="hw" :variant="stale ? 'warning' : 'success'" size="sm" dot>
          {{ stale ? 'Snapshot stale' : 'Snapshot current' }}
        </BaseBadge>
        <BaseButton variant="primary" icon="refresh" @click="refreshSystemInfo" :loading="loading || updatesLoading" size="sm">
          Refresh
        </BaseButton>
      </template>
    </AppHeader>

    <ResultsBanner
      v-if="feedback"
      :variant="feedback.type"
      :title="feedback.title"
      :description="feedback.message"
      class="system-info-feedback"
      @dismiss="feedback = null"
    />

    <!-- Loading -->
    <LoadingSpinner v-if="loading" label="Gathering system information..." overlay />

    <!-- Content -->
    <div v-if="!loading && (hw || os)" class="info-content">
      <!-- OS + Quick Stats -->
      <div class="section">
        <div class="section-header">
          <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" class="section-icon">
            <rect x="2" y="3" width="20" height="14" rx="2" ry="2" /><path d="M8 21h8M12 17v4" />
          </svg>
          <h2 class="section-title">Operating System</h2>
        </div>
        <div class="os-cards" v-if="os">
          <BaseStatCard title="Edition" :value="os.edition" icon="server" />
          <BaseStatCard title="Build" :value="os.build" :subtitle="os.version" icon="cpu" />
          <BaseStatCard title="Architecture" :value="os.arch" icon="cpu" />
          <BaseStatCard title="User" :value="os.user_name" icon="users" />
          <BaseStatCard v-if="os.last_boot" title="Last Boot" :value="new Date(os.last_boot).toLocaleDateString()" icon="clock" />
        </div>
      </div>

      <!-- CPU -->
      <div class="section" v-if="hw">
        <div class="section-header">
          <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" class="section-icon">
            <rect x="4" y="4" width="16" height="16" rx="2" /><path d="M9 9h6v6H9z" /><path d="M3 12h2M19 12h2M12 3v2M12 19v2" />
          </svg>
          <h2 class="section-title">Processor</h2>
        </div>
        <div class="detail-grid">
          <div class="detail-item">
            <span class="detail-label">Name</span>
            <span class="detail-value">{{ hw.cpu.name }}</span>
          </div>
          <div class="detail-item">
            <span class="detail-label">Cores</span>
            <span class="detail-value">{{ hw.cpu.cores }} physical / {{ hw.cpu.logical_processors }} logical</span>
          </div>
          <div class="detail-item">
            <span class="detail-label">Architecture</span>
            <span class="detail-value">{{ hw.cpu.architecture }}</span>
          </div>
        </div>
      </div>

      <!-- Memory -->
      <div class="section" v-if="hw">
        <div class="section-header">
          <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" class="section-icon">
            <rect x="2" y="2" width="20" height="20" rx="2" /><path d="M2 12h20M12 2v20" />
          </svg>
          <h2 class="section-title">Memory</h2>
        </div>
        <BaseCard variant="elevated" class="memory-card">
          <div class="memory-stats">
            <div class="memory-stat">
              <span class="mem-label">Total</span>
              <span class="mem-value">{{ formatBytes(hw.memory.total_bytes) }}</span>
            </div>
            <div class="memory-stat">
              <span class="mem-label">Available</span>
              <span class="mem-value success">{{ formatBytes(hw.memory.available_bytes) }}</span>
            </div>
            <div class="memory-stat">
              <span class="mem-label">In Use</span>
              <span class="mem-value ">{{ formatPercent(hw.memory.in_use_percent) }}</span>
            </div>
          </div>
          <BaseProgress :value="memUsedPercent" variant="accent" size="lg" show-label label="Usage" />
        </BaseCard>
      </div>

      <!-- Battery -->
      <div class="section" v-if="battery && battery.is_laptop">
        <div class="section-header">
          <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" class="section-icon">
            <rect x="1" y="6" width="18" height="12" rx="2" /><path d="M23 13v-2" />
            <!-- Charge bars -->
            <path d="M4 10h2v4H4z" fill="currentColor" />
            <path d="M8 9h2v6H8z" fill="currentColor" />
            <path d="M12 8h2v8h-2z" fill="currentColor" />
          </svg>
          <h2 class="section-title">Battery</h2>
          <BaseBadge :variant="battery.charger_connected ? 'info' : battery.charge_percent > 20 ? 'success' : 'warning'" size="sm" dot>
            {{ battery.charger_connected ? 'Plugged In' : battery.charge_percent > 20 ? 'On Battery' : 'Low Battery' }}
          </BaseBadge>
        </div>
        <BaseCard variant="elevated" class="battery-card">
          <div class="battery-stats">
            <div class="battery-charge">
              <svg viewBox="0 0 36 36" class="battery-gauge">
                <circle cx="18" cy="18" r="15" fill="none" stroke="var(--bg-hover)" stroke-width="3" />
                <circle
                  cx="18" cy="18" r="15"
                  fill="none"
                  :stroke="battery.charge_percent > 50 ? 'var(--accent)' : battery.charge_percent > 20 ? 'var(--warning)' : 'var(--danger)'"
                  stroke-width="3"
                  stroke-linecap="round"
                  :stroke-dasharray="2 * Math.PI * 15"
                  :stroke-dashoffset="2 * Math.PI * 15 * (1 - battery.charge_percent / 100)"
                  transform="rotate(-90 18 18)"
                />
                <text x="18" y="19" text-anchor="middle" :fill="battery.charge_percent > 50 ? 'var(--accent)' : battery.charge_percent > 20 ? 'var(--warning)' : 'var(--danger)'" font-size="8" font-weight="600">
                  {{ battery.charge_percent.toFixed(0) }}%
                </text>
              </svg>
            </div>
            <div class="battery-labels">
              <div class="battery-label">
                <span class="bl-key">{{ battery.charger_connected ? (battery.charging ? 'Charging' : 'Fully charged') : 'Discharging' }}</span>
              </div>
              <div class="battery-label" v-if="battery.time_remaining_minutes > 0">
                <span class="bl-key">Est. remaining</span>
                <span class="bl-val">{{ Math.floor(battery.time_remaining_minutes / 60) }}h {{ battery.time_remaining_minutes % 60 }}m</span>
              </div>
              <div class="battery-label" v-if="battery.saving_mode">
                <BaseBadge variant="warning" size="sm">Battery Saver Active</BaseBadge>
              </div>
            </div>
          </div>
        </BaseCard>
      </div>

      <!-- Disks -->
      <div class="section" v-if="hw">
        <div class="section-header">
          <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" class="section-icon">
            <ellipse cx="12" cy="5" rx="9" ry="3" /><path d="M21 12c0 1.66-4 3-9 3s-9-1.34-9-3" /><path d="M3 5v14c0 1.66 4 3 9 3s9-1.34 9-3V5" />
          </svg>
          <h2 class="section-title">Storage</h2>
        </div>
        <div class="disks-list">
          <BaseCard
            v-for="disk in hw.disks"
            :key="disk.mount_point"
            :variant="parseFloat(diskUsagePercent(disk)) > 90 ? 'danger' : parseFloat(diskUsagePercent(disk)) > 75 ? 'warning' : 'default'"
            class="disk-card"
          >
            <div class="disk-header">
              <div class="disk-label">
                <span class="disk-name">{{ disk.name }}</span>
                <span class="disk-mount">{{ disk.mount_point }}</span>
              </div>
              <div class="disk-meta">
                <BaseBadge :variant="disk.is_ssd ? 'info' : 'default'" size="sm">
                  {{ disk.disk_type }}
                </BaseBadge>
                <span class="disk-filesystem">{{ disk.file_system }}</span>
              </div>
            </div>
            <BaseProgress
              :value="parseFloat(diskUsagePercent(disk))"
              :variant="parseFloat(diskUsagePercent(disk)) > 90 ? 'danger' : parseFloat(diskUsagePercent(disk)) > 75 ? 'warning' : 'accent'"
              size="md"
              show-label
              :label="`${formatBytes(disk.available_bytes)} free of ${formatBytes(disk.total_bytes)}`"
            />
          </BaseCard>
        </div>
      </div>

      <!-- GPU -->
      <div class="section" v-if="hw && hw.gpu">
        <div class="section-header">
          <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" class="section-icon">
            <rect x="2" y="3" width="20" height="14" rx="2" ry="2" /><path d="M8 21h8M12 17v4" />
          </svg>
          <h2 class="section-title">Graphics</h2>
        </div>
        <div class="detail-grid">
          <div class="detail-item">
            <span class="detail-label">GPU</span>
            <span class="detail-value">{{ hw.gpu.name }}</span>
          </div>
          <div class="detail-item">
            <span class="detail-label">VRAM</span>
            <span class="detail-value">{{ formatBytes(hw.gpu.adapter_ram) }}</span>
          </div>
          <div class="detail-item">
            <span class="detail-label">Driver</span>
            <span class="detail-value">{{ hw.gpu.driver_version }}</span>
          </div>
        </div>
      </div>

      <!-- Updates -->
      <div class="section">
        <div class="section-header">
          <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" class="section-icon">
            <path d="M23 4v6h-6M1 20v-6h6" /><path d="M3.51 9a9 9 0 0114.85-3.36L23 10M1 14l4.64 4.36A9 9 0 0020.49 15" />
          </svg>
          <h2 class="section-title">Installed Updates ({{ updatesLoading ? 'Loading' : updates.length }})</h2>
        </div>
        <BaseCard variant="elevated" padding="none" class="updates-card">
          <div v-if="updatesLoading" class="updates-empty">
            <span class="updates-empty-text">Loading update history...</span>
          </div>
          <div v-else-if="updates.length > 0" class="updates-list">
            <div v-for="update in updates" :key="update.title" class="update-item">
              <span v-if="update.kb_number" class="update-kb">{{ update.kb_number }}</span>
              <span class="update-title" :title="update.title">{{ update.title.substring(0, 100) }}{{ update.title.length > 100 ? '...' : '' }}</span>
              <span v-if="update.installed_date" class="update-date">{{ new Date(update.installed_date).toLocaleDateString() }}</span>
            </div>
          </div>
          <div v-else class="updates-empty">
            <span class="updates-empty-text">No update history available</span>
          </div>
        </BaseCard>
      </div>
    </div>

    <EmptyState
      v-if="!loading && !hw && !os"
      icon="info"
      title="No System Information Loaded"
      description="Click Refresh to gather hardware and Windows information."
    />
  </div>
</template>

<style scoped>
.system-info {
  width: 100%;
  min-width: 0;
}

.system-info-feedback {
  margin-bottom: var(--space-5);
}

/* Sections */
.section {
  margin-bottom: var(--space-6);
  animation: fadeInUp calc(var(--duration-normal) * 1.2) var(--ease-out);
}

.section-header {
  display: flex;
  align-items: center;
  gap: var(--space-2);
  margin-bottom: var(--space-4);
}

.section-icon {
  color: var(--accent);
  flex-shrink: 0;
}

.section-title {
  margin: 0;
  font-size: var(--text-lg);
  font-weight: var(--weight-semibold);
  color: var(--text-primary);
}

/* OS Cards */
.os-cards {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(160px, 1fr));
  gap: var(--space-3);
}

/* Detail Grid */
.detail-grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(280px, 1fr));
  gap: var(--space-2);
  padding: var(--space-4);
  background: var(--bg-elevated);
  border: 1px solid var(--border-default);
  border-radius: var(--radius-xl);
}

.detail-item {
  display: flex;
  flex-direction: column;
  gap: 2px;
  padding: var(--space-2) 0;
  border-bottom: 1px solid var(--border-subtle);
}

.detail-item:last-child {
  border-bottom: none;
}

.detail-label {
  font-size: var(--text-xs);
  color: var(--text-muted);
  text-transform: uppercase;
  letter-spacing: var(--tracking-wide);
}

.detail-value {
  font-size: var(--text-sm);
  color: var(--text-primary);
  font-weight: var(--weight-medium);
}

/* Memory Card */
.memory-card {
  padding: var(--space-5);
}

.memory-stats {
  display: flex;
  gap: var(--space-6);
  margin-bottom: var(--space-4);
}

.memory-stat {
  display: flex;
  flex-direction: column;
  gap: 2px;
}

.mem-label {
  font-size: var(--text-xs);
  color: var(--text-muted);
  text-transform: uppercase;
}

.mem-value {
  font-size: var(--text-xl);
  font-weight: var(--weight-bold);
  color: var(--text-primary);
  font-variant-numeric: tabular-nums;
}

.mem-value.success {
  color: var(--success);
}

/* Disks */
.disks-list {
  display: flex;
  flex-direction: column;
  gap: var(--space-3);
}

.disk-card {
  padding: var(--space-4);
}

.disk-header {
  display: flex;
  justify-content: space-between;
  align-items: flex-start;
  margin-bottom: var(--space-3);
  gap: var(--space-3);
}

.disk-label {
  display: flex;
  flex-direction: column;
  gap: 1px;
}

.disk-name {
  font-size: var(--text-md);
  font-weight: var(--weight-semibold);
  color: var(--text-primary);
}

.disk-mount {
  font-size: var(--text-xs);
  color: var(--text-muted);
  font-family: var(--font-mono);
}

.disk-meta {
  display: flex;
  align-items: center;
  gap: var(--space-2);
}

.disk-filesystem {
  font-size: var(--text-xs);
  color: var(--text-muted);
  font-family: var(--font-mono);
}

/* Updates */
.updates-card {
  max-height: 400px;
  overflow-y: auto;
}

.updates-list {
  display: flex;
  flex-direction: column;
}

.update-item {
  display: flex;
  align-items: center;
  gap: var(--space-3);
  padding: var(--space-2) var(--space-4);
  border-bottom: 1px solid var(--border-subtle);
  font-size: var(--text-sm);
}

.update-item:last-child {
  border-bottom: none;
}

.update-item:hover {
  background: var(--glass-bg-hover);
}

.update-kb {
  color: var(--accent);
  font-family: var(--font-mono);
  font-size: var(--text-xs);
  font-weight: var(--weight-semibold);
  min-width: 70px;
}

.update-title {
  flex: 1;
  color: var(--text-secondary);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.update-date {
  font-size: var(--text-xs);
  color: var(--text-muted);
  white-space: nowrap;
}

.updates-empty {
  padding: var(--space-8);
  text-align: center;
}

.updates-empty-text {
  color: var(--text-muted);
  font-size: var(--text-sm);
}

/* Battery */
.battery-card {
  padding: var(--space-4) !important;
}

.battery-stats {
  display: flex;
  align-items: center;
  gap: var(--space-6);
}

.battery-gauge {
  width: 100px;
  height: 100px;
  flex-shrink: 0;
}

.battery-labels {
  display: flex;
  flex-direction: column;
  gap: var(--space-2);
}

.battery-label {
  font-size: var(--text-sm);
  color: var(--text-secondary);
}

.bl-key {
  font-weight: var(--weight-medium);
}

.bl-val {
  margin-left: var(--space-2);
  color: var(--text-primary);
  font-weight: var(--weight-semibold);
}

@media (max-width: 700px) {
  .memory-stats,
  .battery-stats,
  .disk-header,
  .update-item {
    flex-direction: column;
    align-items: stretch;
  }

  .detail-grid {
    grid-template-columns: 1fr;
  }

  .disk-meta {
    flex-wrap: wrap;
  }
}
</style>
