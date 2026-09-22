<script setup lang="ts">
import { ref, computed } from 'vue'
import { useScanStore } from '../stores/scan'
import AppHeader from '../components/AppHeader.vue'
import BaseCard from '../components/BaseCard.vue'
import BaseButton from '../components/BaseButton.vue'
import BaseTabBar from '../components/BaseTabBar.vue'
import BaseBadge from '../components/BaseBadge.vue'
import BaseProgress from '../components/BaseProgress.vue'
import ResultsBanner from '../components/ResultsBanner.vue'
import EmptyState from '../components/EmptyState.vue'
import CleanupTree from '../components/CleanupTree.vue'

const scanStore = useScanStore()

const activeTab = ref<string>('temp')

const tabs = [
  { id: 'temp', label: 'Temp Files', icon: 'folder' },
  { id: 'browsers', label: 'Browser Cache', icon: 'globe' },
  { id: 'update', label: 'Windows Update', icon: 'refresh-cw' },
  { id: 'logs', label: 'System Logs', icon: 'file-text' },
]

const currentItems = computed(() => {
  switch (activeTab.value) {
    case 'temp': return scanStore.tempFiles
    case 'browsers': return scanStore.browserCache
    case 'update': return scanStore.updateCache
    case 'logs': return scanStore.systemLogs
  }
})

const currentResult = computed(() => {
  switch (activeTab.value) {
    case 'temp': return scanStore.tempResult
    case 'browsers': return scanStore.browserResult
    case 'update': return scanStore.updateResult
    case 'logs': return scanStore.logsResult
  }
})

const tabTotals = computed(() => {
  const totals: Record<string, number> = {}
  for (const item of scanStore.tempFiles) totals.temp = (totals.temp || 0) + item.size_bytes
  for (const item of scanStore.browserCache) totals.browsers = (totals.browsers || 0) + item.size_bytes
  for (const item of scanStore.updateCache) totals.update = (totals.update || 0) + item.size_bytes
  for (const item of scanStore.systemLogs) totals.logs = (totals.logs || 0) + item.size_bytes
  return totals
})

const totalForCurrentTab = computed(() => tabTotals.value[activeTab.value] || 0)

</script>

<template>
  <div class="disk-cleaner">
    <AppHeader />

    <!-- Action Bar -->
    <div class="action-bar">
      <div class="action-left">
        <BaseButton variant="primary" icon="refresh" @click="scanStore.scanAll()" :loading="scanStore.loading">
          Scan All
        </BaseButton>
        <BaseBadge variant="warning" size="md">Read-only legacy scan</BaseBadge>
      </div>
      <div class="action-right">
        <BaseBadge variant="accent" size="md">
          {{ scanStore.formatBytes(totalForCurrentTab) }} in this tab
        </BaseBadge>
      </div>
    </div>

    <!-- Tabs -->
    <BaseTabBar
      :tabs="tabs.map(t => ({ ...t, badge: scanStore.formatBytes(tabTotals[t.id] || 0) }))"
      v-model="activeTab"
      variant="glass"
      class="disk-tabs"
    />

    <!-- Results: Tree View -->
    <div v-if="currentItems.length > 0" class="results-section">
      <CleanupTree
        :key="activeTab"
        :items="currentItems"
      />

      <!-- Total bar -->
      <div class="total-bar">
        <span class="total-label">Total for {{ tabs.find(t => t.id === activeTab)?.label || activeTab }}</span>
        <BaseProgress :value="totalForCurrentTab" :max="scanStore.totalScannableBytes || 1" variant="accent" size="sm" />
        <span class="total-value">{{ scanStore.formatBytes(totalForCurrentTab) }}</span>
      </div>
    </div>

    <!-- Empty state -->
    <BaseCard v-else-if="!scanStore.loading" variant="default" class="empty-section">
      <EmptyState
        icon="search"
        title="No items found"
        description="Click Scan All to check for cleanable files in each category."
        action-label="Scan All"
        @action="scanStore.scanAll()"
      />
    </BaseCard>

    <!-- Loading state -->
    <BaseCard v-if="scanStore.loading && currentItems.length === 0" variant="glass" class="loading-section">
      <div class="scan-animation">
        <div class="scan-ring" />
        <div class="scan-ring scan-ring-2" />
        <div class="scan-content">
          <span class="scan-text">Scanning {{ tabs.find(t => t.id === activeTab)?.label || activeTab }}...</span>
        </div>
      </div>
    </BaseCard>

    <!-- Result Banner -->
    <ResultsBanner
      v-if="currentResult"
      :variant="(currentResult.failures.length === 0) ? 'success' : 'warning'"
      :title="(currentResult.failures.length === 0) ? 'Cleanup Complete' : 'Cleanup Completed with Warnings'"
      :description="`Cleaned ${currentResult.items_cleaned} items (${scanStore.formatBytes(currentResult.bytes_freed)})`"
      :items="currentResult.failures.length > 0 ? [{ label: 'Failures', value: currentResult.failures.length }] : undefined"
      dismissable
      class="result-banner"
    />

  </div>
</template>

<style scoped>
.disk-cleaner { width: 100%; min-width: 0; }

.action-bar {
  display: flex;
  justify-content: space-between;
  align-items: center;
  margin-bottom: var(--space-4);
  gap: var(--space-3);
}
.action-left { display: flex; gap: var(--space-2); }
.action-right { display: flex; align-items: center; }
.disk-tabs { margin-bottom: var(--space-4); }
.results-section { animation: fadeInUp var(--duration-normal) var(--ease-out); }

/* Total bar */
.total-bar {
  display: flex;
  align-items: center;
  gap: var(--space-3);
  padding: var(--space-3) var(--space-4);
  background: var(--bg-surface);
  border: 1px solid var(--border-subtle);
  border-top: none;
  border-radius: 0 0 var(--radius-xl) var(--radius-xl);
}
.total-label {
  font-size: var(--text-xs);
  color: var(--text-muted);
  white-space: nowrap;
}
.total-value {
  font-size: var(--text-sm);
  font-weight: var(--weight-semibold);
  color: var(--text-primary);
  white-space: nowrap;
  font-variant-numeric: tabular-nums;
}

/* Loading animation */
.loading-section {
  min-height: 200px;
  display: flex;
  align-items: center;
  justify-content: center;
}
.scan-animation {
  position: relative;
  display: flex;
  align-items: center;
  justify-content: center;
  width: 120px;
  height: 120px;
}
.scan-ring {
  position: absolute;
  inset: 0;
  border-radius: var(--radius-full);
  border: 3px solid transparent;
  border-top-color: var(--accent);
  border-right-color: var(--accent-300);
  animation: spin 1s linear infinite;
}
.scan-ring-2 {
  inset: 12px;
  border-width: 2px;
  border-top-color: var(--accent-600);
  border-left-color: var(--accent);
  animation-duration: 1.5s;
  animation-direction: reverse;
}
.scan-content { text-align: center; }
.scan-text {
  font-size: var(--text-sm);
  color: var(--text-secondary);
  animation: pulse 1.5s ease-in-out infinite;
}

.empty-section { min-height: 200px; }
.result-banner { margin-top: var(--space-4); }

@media (max-width: 700px) {
  .action-bar,
  .action-left,
  .action-right {
    flex-direction: column;
    align-items: stretch;
  }
  .action-right :deep(.base-badge) {
    justify-content: center;
    max-width: 100%;
    overflow: hidden;
    text-overflow: ellipsis;
  }
}
</style>
