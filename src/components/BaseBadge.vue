<script setup lang="ts">
withDefaults(defineProps<{
  variant?: 'default' | 'success' | 'warning' | 'danger' | 'info' | 'accent'
  size?: 'sm' | 'md'
  dot?: boolean
  pulse?: boolean
}>(), {
  variant: 'default',
  size: 'sm',
  dot: false,
  pulse: false,
})
</script>

<template>
  <span class="base-badge" :class="[`variant-${variant}`, `size-${size}`, { 'has-dot': dot, 'is-pulsing': pulse }]">
    <span v-if="dot" class="badge-dot" />
    <slot />
  </span>
</template>

<style scoped>
.base-badge {
  display: inline-flex;
  align-items: center;
  gap: var(--space-1);
  font-family: var(--font-sans);
  font-weight: var(--weight-medium);
  border-radius: var(--radius-full);
  white-space: nowrap;
  line-height: 1;
}

.size-sm {
  padding: 2px 8px;
  font-size: var(--text-xs);
  letter-spacing: var(--tracking-wide);
  text-transform: uppercase;
}

.size-md {
  padding: 4px 12px;
  font-size: var(--text-sm);
}

/* Variants */
.variant-default {
  background: var(--bg-hover);
  color: var(--text-muted);
  border: 1px solid var(--border-subtle);
}

.variant-success {
  background: var(--color-success-bg);
  color: var(--success);
}

.variant-warning {
  background: var(--color-warning-bg);
  color: var(--warning);
}

.variant-danger {
  background: var(--color-danger-bg);
  color: var(--danger);
}

.variant-info {
  background: var(--color-info-bg);
  color: var(--info);
}

.variant-accent {
  background: var(--color-primary-100);
  color: var(--accent);
}

/* Dot */
.badge-dot {
  width: 6px;
  height: 6px;
  border-radius: var(--radius-full);
  background: currentColor;
}

/* Pulse animation for live indicators */
.is-pulsing .badge-dot {
  animation: pulse 2s ease-in-out infinite;
}
</style>
