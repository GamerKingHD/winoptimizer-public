<script setup lang="ts">
import { computed } from 'vue'

const props = withDefaults(defineProps<{
  value: number          // 0-100
  size?: 'sm' | 'md' | 'lg'
  variant?: 'accent' | 'success' | 'warning' | 'danger' | 'info'
  showLabel?: boolean
  label?: string
  max?: number
  animated?: boolean
  indeterminate?: boolean
}>(), {
  size: 'md',
  variant: 'accent',
  showLabel: false,
  label: '',
  max: 100,
  animated: true,
  indeterminate: false,
})

const percent = computed(() => Math.min(100, Math.max(0, (props.value / props.max) * 100)))
</script>

<template>
  <div class="progress-wrapper">
    <div v-if="showLabel && label" class="progress-label">
      <span>{{ label }}</span>
      <span v-if="!indeterminate" class="progress-value">{{ Math.round(percent) }}%</span>
    </div>
    <div class="progress-track" :class="[`size-${size}`, { 'is-indeterminate': indeterminate }]">
      <div
        class="progress-fill"
        :class="[`variant-${variant}`, { 'is-animated': animated }]"
        :style="{ width: indeterminate ? '40%' : percent + '%' }"
      />
    </div>
  </div>
</template>

<style scoped>
.progress-wrapper {
  width: 100%;
}

.progress-label {
  display: flex;
  justify-content: space-between;
  align-items: center;
  margin-bottom: var(--space-1);
  font-size: var(--text-xs);
  color: var(--text-secondary);
}

.progress-value {
  font-weight: var(--weight-semibold);
  color: var(--text-primary);
  font-variant-numeric: tabular-nums;
}

.progress-track {
  width: 100%;
  background: var(--bg-hover);
  border-radius: var(--radius-full);
  overflow: hidden;
  position: relative;
}

.size-sm { height: 4px; }
.size-md { height: 6px; }
.size-lg { height: 10px; }

.progress-fill {
  height: 100%;
  border-radius: var(--radius-full);
  transition: width var(--duration-slow) var(--ease-out);
  position: relative;
}

.progress-fill.variant-accent { background: var(--accent-gradient); }
.progress-fill.variant-success { background: var(--success); }
.progress-fill.variant-warning { background: var(--warning); }
.progress-fill.variant-danger { background: var(--danger); }
.progress-fill.variant-info { background: var(--info); }

.is-animated {
  animation: progressFill var(--duration-slow) var(--ease-out);
}

/* Indeterminate animation */
.is-indeterminate {
  background: var(--bg-hover);
}

.is-indeterminate .progress-fill {
  width: 40% !important;
  animation: indeterminateProgress 1.5s ease-in-out infinite;
}

@keyframes indeterminateProgress {
  0% { transform: translateX(-100%); }
  50% { transform: translateX(200%); }
  100% { transform: translateX(350%); }
}

/* Shimmer effect on fill */
</style>
