<script setup lang="ts">
withDefaults(defineProps<{
  variant?: 'default' | 'accent' | 'elevated' | 'glass' | 'danger' | 'success' | 'warning'
  padding?: 'sm' | 'md' | 'lg' | 'none'
  hover?: boolean
  as?: string
}>(), {
  variant: 'default',
  padding: 'md',
  hover: false,
  as: 'div',
})
</script>

<template>
  <component
    :is="as"
    class="base-card"
    :class="[
      `variant-${variant}`,
      `padding-${padding}`,
      { 'is-hoverable': hover },
    ]"
  >
    <slot />
  </component>
</template>

<style scoped>
.base-card {
  background: var(--bg-elevated);
  border: 1px solid var(--border-default);
  border-radius: var(--radius-xl);
  transition: all var(--duration-normal) var(--ease-out);
  position: relative;
  overflow: hidden;
}

/* Variants */
.variant-default {
  background: var(--bg-elevated);
}

.variant-accent { background: var(--color-primary-50); border-color: var(--color-primary-200); }

.variant-elevated {
  background: var(--bg-elevated);
  box-shadow: var(--shadow-md);
  border-color: var(--border-strong);
}

.variant-glass { background: var(--bg-elevated); border-color: var(--border-default); }

.variant-danger { background: var(--color-danger-bg); border-color: var(--danger); }

.variant-success { background: var(--color-success-bg); border-color: var(--success); }

.variant-warning { background: var(--color-warning-bg); border-color: var(--warning); }

/* Padding */
.padding-sm { padding: var(--space-3); }
.padding-md { padding: var(--space-5); }
.padding-lg { padding: var(--space-8); }
.padding-none { padding: 0; }

/* Hover effect */
.is-hoverable {
  cursor: pointer;
}

.is-hoverable:hover {
  box-shadow: var(--shadow-md);
  border-color: var(--accent);
}

.is-hoverable:active {
  transform: translateY(0);
  box-shadow: var(--shadow-sm);
}

</style>
