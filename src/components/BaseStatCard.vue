<script setup lang="ts">
withDefaults(defineProps<{
  title: string
  value: string | number
  subtitle?: string
  icon?: string
  trend?: 'up' | 'down' | 'neutral'
  trendValue?: string
  variant?: 'default' | 'accent' | 'success' | 'warning' | 'danger' | 'info'
  loading?: boolean
}>(), {
  subtitle: '',
  icon: '',
  trend: 'neutral',
  trendValue: '',
  variant: 'default',
  loading: false,
})
</script>

<template>
  <div class="stat-card" :class="`variant-${variant}`">
    <div class="stat-body">
      <div class="stat-header">
        <svg v-if="icon" class="stat-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
          <path v-if="icon === 'hard-drive'" d="M22 12H2M2 12l2.5-7.5A2 2 0 016.44 3h11.12a2 2 0 011.94 1.5L22 12zM2 12v5a2 2 0 002 2h16a2 2 0 002-2v-5" />
          <path v-if="icon === 'cpu'" d="M9 9h6v6H9zM3 12h2M19 12h2M12 3v2M12 19v2" />
          <path v-if="icon === 'zap'" d="M13 2L3 14h9l-1 8 10-12h-9l1-8z" />
          <path v-if="icon === 'trash-2'" d="M3 6h18M19 6v14a2 2 0 01-2 2H7a2 2 0 01-2-2V6m3 0V4a2 2 0 012-2h4a2 2 0 012 2v2M10 11v6M14 11v6" />
          <path v-if="icon === 'clock'" d="M12 22c5.523 0 10-4.477 10-10S17.523 2 12 2 2 6.477 2 12s4.477 10 10 10zM12 6v6l4 2" />
          <path v-if="icon === 'shield'" d="M12 22s8-4 8-10V5l-8-3-8 3v7c0 6 8 10 8 10z" />
          <path v-if="icon === 'activity'" d="M22 12h-4l-3 9-4-18-3 9H2" />
          <path v-if="icon === 'database'" d="M12 2C8.13 2 5 4.69 5 8v1c0 3.31 3.13 6 7 6s7-2.69 7-6V8c0-3.31-3.13-6-7-6zM5 12v1c0 3.31 3.13 6 7 6s7-2.69 7-6v-1" />
          <path v-if="icon === 'file' || icon === 'folder'" d="M22 19a2 2 0 01-2 2H4a2 2 0 01-2-2V5a2 2 0 012-2h5l2 3h9a2 2 0 012 2z" />
          <path v-if="icon === 'users'" d="M17 21v-2a4 4 0 00-4-4H5a4 4 0 00-4 4v2M23 21v-2a4 4 0 00-3-3.87M9 3a4 4 0 100 8 4 4 0 000-8zM16 3a4 4 0 100 8 4 4 0 000-8z" />
          <path v-if="icon === 'server'" d="M2 2h20v20H2zM2 12h20M2 18h20M12 2v20" />
          <path v-if="icon === 'bar-chart-2'" d="M18 20V10M12 20V4M6 20v-4" />
        </svg>
        <span class="stat-title">{{ title }}</span>
      </div>
      <div v-if="loading" class="stat-skeleton">
        <div class="skeleton-line skeleton-value" />
        <div class="skeleton-line skeleton-sub" />
      </div>
      <template v-else>
        <div class="stat-value">{{ value }}</div>
        <div v-if="subtitle" class="stat-subtitle">{{ subtitle }}</div>
      </template>
    </div>
    <div v-if="trendValue" class="stat-trend" :class="`trend-${trend}`">
      <svg v-if="trend === 'up'" width="12" height="12" viewBox="0 0 12 12" fill="currentColor">
        <path d="M6 2l5 6H1z"/>
      </svg>
      <svg v-else-if="trend === 'down'" width="12" height="12" viewBox="0 0 12 12" fill="currentColor">
        <path d="M6 10l5-6H1z"/>
      </svg>
      <span>{{ trendValue }}</span>
    </div>
  </div>
</template>

<style scoped>
.stat-card {
  position: relative;
  background: var(--bg-elevated);
  border: 1px solid var(--border-default);
  border-radius: var(--radius-xl);
  padding: var(--space-5);
  display: flex;
  flex-direction: column;
  justify-content: space-between;
  min-height: 120px;
  transition: all var(--duration-normal) var(--ease-out);
  overflow: hidden;
}


.stat-card:hover {
  transform: translateY(-2px);
  box-shadow: var(--shadow-md);
}

/* Variant accent bar on left */
.variant-accent {
  border-left: 3px solid var(--accent);
}
.variant-success {
  border-left: 3px solid var(--success);
}
.variant-warning {
  border-left: 3px solid var(--warning);
}
.variant-danger {
  border-left: 3px solid var(--danger);
}
.variant-info {
  border-left: 3px solid var(--info);
}

.stat-body {
  z-index: 1;
}

.stat-header {
  display: flex;
  align-items: center;
  gap: var(--space-2);
  margin-bottom: var(--space-3);
}

.stat-icon {
  width: 18px;
  height: 18px;
  color: var(--text-muted);
  flex-shrink: 0;
}

.variant-accent .stat-icon { color: var(--accent); }
.variant-success .stat-icon { color: var(--success); }
.variant-warning .stat-icon { color: var(--warning); }
.variant-danger .stat-icon { color: var(--danger); }
.variant-info .stat-icon { color: var(--info); }

.stat-title {
  font-size: var(--text-xs);
  font-weight: var(--weight-medium);
  color: var(--text-muted);
  text-transform: uppercase;
  letter-spacing: var(--tracking-wide);
}

.stat-value {
  font-size: var(--text-4xl);
  font-weight: var(--weight-bold);
  color: var(--text-primary);
  line-height: 1.1;
  font-variant-numeric: tabular-nums;
}

.stat-subtitle {
  font-size: var(--text-sm);
  color: var(--text-muted);
  margin-top: var(--space-1);
}

.stat-trend {
  display: flex;
  align-items: center;
  gap: var(--space-1);
  font-size: var(--text-xs);
  font-weight: var(--weight-medium);
  margin-top: var(--space-2);
}

.trend-up { color: var(--success); }
.trend-down { color: var(--danger); }
.trend-neutral { color: var(--text-muted); }

/* Skeleton loading */
.stat-skeleton {
  display: flex;
  flex-direction: column;
  gap: var(--space-2);
}

.skeleton-line {
  border-radius: var(--radius-sm);
  background: var(--bg-hover);
}

.skeleton-value {
  height: 32px;
  width: 70%;
}

.skeleton-sub {
  height: 14px;
  width: 50%;
}
</style>
