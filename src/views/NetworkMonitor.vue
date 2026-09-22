<script setup lang="ts">
import { computed, ref } from 'vue'
import type { NetworkInfo, NetworkAdapter, TcpConnection } from '../api/tauri'
import * as api from '../api/tauri'
import AppHeader from '../components/AppHeader.vue'
import BaseCard from '../components/BaseCard.vue'
import BaseButton from '../components/BaseButton.vue'
import BaseTable from '../components/BaseTable.vue'
import BaseBadge from '../components/BaseBadge.vue'
import LoadingSpinner from '../components/LoadingSpinner.vue'
import ResultsBanner from '../components/ResultsBanner.vue'
import { failure, type ActionFeedback } from '../utils/feedback'
import { usePolling } from '../composables/usePolling'

const info = ref<NetworkInfo | null>(null)
const loading = ref(false)
const feedback = ref<ActionFeedback | null>(null)

const connColumns = [
  { key: 'local', label: 'Local Address', sortable: true },
  { key: 'remote', label: 'Remote Address', sortable: true },
  { key: 'state', label: 'State', sortable: true, width: '100px' },
  { key: 'process', label: 'Process', sortable: true },
]

const connRows = computed(() =>
  (info.value?.connections || []).map(c => ({
    _id: `${c.local_addr}:${c.local_port}-${c.remote_addr}:${c.remote_port}-${c.pid}`,
    local: `${c.local_addr}:${c.local_port}`,
    remote: c.remote_port > 0 ? `${c.remote_addr}:${c.remote_port}` : '*',
    state: c.state,
    process: c.process_name,
    pid: c.pid,
  }))
)

async function refresh() {
  loading.value = true
  try {
    info.value = await api.getNetworkInfo()
    feedback.value = null
  } catch (e) {
    info.value = null
    feedback.value = failure('Could not load network information', e)
  } finally {
    loading.value = false
  }
}

const filteredConnections = computed(() => connRows.value.slice(0, 100))

usePolling(refresh, 5000, { immediate: true })
</script>

<template>
  <div class="network-monitor">
    <AppHeader />

    <ResultsBanner
      v-if="feedback"
      :variant="feedback.type"
      :title="feedback.title"
      :description="feedback.message"
      class="network-feedback"
      @dismiss="feedback = null"
    />

    <div class="toolbar">
      <BaseButton variant="primary" icon="refresh" @click="refresh" :loading="loading" size="sm">
        Refresh
      </BaseButton>
      <BaseBadge variant="accent" size="sm">
        {{ info?.adapters.length || 0 }} adapters / {{ info?.connections.length || 0 }} connections
      </BaseBadge>
    </div>

    <LoadingSpinner v-if="loading && !info" />

    <BaseCard v-if="!loading && !info" variant="default" class="empty-card">
      <p class="empty-text">Click Refresh to load adapters and active TCP connections.</p>
    </BaseCard>

    <!-- Adapters -->
    <div class="adapters-grid" v-if="info">
      <BaseCard
        v-for="adapter in info.adapters"
        :key="adapter.name"
        :variant="adapter.operational ? 'default' : 'glass'"
        class="adapter-card"
      >
        <div class="adapter-header">
          <div class="adapter-info">
            <h4 class="adapter-name">{{ adapter.description || adapter.name }}</h4>
            <p class="adapter-mac" v-if="adapter.mac_address">{{ adapter.mac_address }}</p>
          </div>
          <BaseBadge
            :variant="adapter.operational ? 'success' : 'warning'"
            size="sm"
            :dot="adapter.operational"
          >
            {{ adapter.operational ? 'Up' : 'Down' }}
          </BaseBadge>
        </div>

        <div class="adapter-details" v-if="adapter.operational">
          <div class="detail-row" v-for="ip in adapter.ip_addresses" :key="ip">
            <span class="detail-label">IP</span>
            <span class="detail-value mono">{{ ip }}</span>
          </div>
          <div class="detail-row" v-if="adapter.gateway">
            <span class="detail-label">Gateway</span>
            <span class="detail-value mono">{{ adapter.gateway }}</span>
          </div>
          <div class="detail-row" v-if="adapter.dhcp_server">
            <span class="detail-label">DHCP</span>
            <span class="detail-value mono">{{ adapter.dhcp_server }}</span>
          </div>
          <div class="detail-row" v-if="adapter.dns_servers.length > 0">
            <span class="detail-label">DNS</span>
            <span class="detail-value mono">{{ adapter.dns_servers.join(', ') }}</span>
          </div>
          <div class="detail-row" v-if="adapter.link_speed_mbps > 0">
            <span class="detail-label">Speed</span>
            <span class="detail-value">{{ adapter.link_speed_mbps >= 1000 ? (adapter.link_speed_mbps / 1000).toFixed(1) + ' Gbps' : adapter.link_speed_mbps + ' Mbps' }}</span>
          </div>
          <div class="detail-row" v-if="adapter.mtu > 0">
            <span class="detail-label">MTU</span>
            <span class="detail-value">{{ adapter.mtu }}</span>
          </div>
        </div>
      </BaseCard>
    </div>

    <!-- Active Connections -->
    <div class="connections-section" v-if="info && info.connections.length > 0">
      <h2 class="section-title">Active TCP Connections ({{ info.connections.length }})</h2>
      <BaseTable
        :columns="connColumns"
        :rows="filteredConnections"
        max-height="350px"
        compact
      >
        <template #cell-local="{ row }">
          <span class="conn-addr">{{ row.local }}</span>
        </template>
        <template #cell-remote="{ row }">
          <span class="conn-addr">{{ row.remote }}</span>
        </template>
        <template #cell-state="{ row }">
          <BaseBadge
            :variant="row.state === 'LISTENING' ? 'info' : row.state === 'ESTABLISHED' ? 'success' : 'default'"
            size="sm"
          >
            {{ row.state }}
          </BaseBadge>
        </template>
        <template #cell-process="{ row }">
          <span class="conn-proc">{{ row.process }}</span>
          <span class="conn-pid" v-if="row.pid > 0">({{ row.pid }})</span>
        </template>
      </BaseTable>
    </div>

    <div class="empty-state" v-if="info && info.connections.length === 0">
      <p class="empty-text">No active connections found.</p>
    </div>
  </div>
</template>

<style scoped>
.network-monitor {
  width: 100%;
  min-width: 0;
}

.network-feedback {
  margin-bottom: var(--space-5);
}

.toolbar {
  display: flex;
  align-items: center;
  gap: var(--space-3);
  margin-bottom: var(--space-4);
}

/* Adapter grid */
.adapters-grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(320px, 1fr));
  gap: var(--space-4);
  margin-bottom: var(--space-6);
}

.adapter-card {
  padding: var(--space-4) !important;
}

.adapter-header {
  display: flex;
  justify-content: space-between;
  align-items: flex-start;
  gap: var(--space-3);
  margin-bottom: var(--space-3);
}

.adapter-name {
  margin: 0;
  font-size: var(--text-sm);
  font-weight: var(--weight-semibold);
  color: var(--text-primary);
  line-height: 1.3;
}

.adapter-mac {
  margin: 2px 0 0;
  font-size: var(--text-xs);
  color: var(--text-muted);
  font-family: var(--font-mono);
}

.adapter-details {
  display: flex;
  flex-direction: column;
  gap: var(--space-1);
}

.detail-row {
  display: flex;
  gap: var(--space-3);
  font-size: var(--text-xs);
}

.detail-label {
  color: var(--text-muted);
  min-width: 55px;
  flex-shrink: 0;
}

.detail-value {
  color: var(--text-secondary);
  word-break: break-all;
}

.detail-value.mono {
  font-family: var(--font-mono);
}

/* Connections */
.connections-section {
  margin-top: var(--space-4);
}

.section-title {
  font-size: var(--text-base);
  font-weight: var(--weight-semibold);
  color: var(--text-primary);
  margin: 0 0 var(--space-3);
}

.conn-addr {
  font-family: var(--font-mono);
  font-size: var(--text-xs);
  color: var(--text-secondary);
}

.conn-proc {
  font-size: var(--text-xs);
  color: var(--text-primary);
}

.conn-pid {
  font-size: var(--text-xs);
  color: var(--text-muted);
  margin-left: var(--space-1);
}

.empty-state {
  padding: var(--space-8);
  text-align: center;
}

.empty-card {
  padding: var(--space-8) !important;
  text-align: center;
}

.empty-text {
  color: var(--text-muted);
  font-size: var(--text-sm);
}
</style>
