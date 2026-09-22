<script setup lang="ts">
withDefaults(defineProps<{
  variant?: 'primary' | 'secondary' | 'accent' | 'ghost' | 'danger' | 'success'
  size?: 'sm' | 'md' | 'lg'
  loading?: boolean
  disabled?: boolean
  fullWidth?: boolean
  icon?: string
}>(), {
  variant: 'primary',
  size: 'md',
  loading: false,
  disabled: false,
  fullWidth: false,
  icon: '',
})

const emit = defineEmits<{
  click: [e: MouseEvent]
}>()
</script>

<template>
  <button
    class="base-btn"
    :class="[
      `variant-${variant}`,
      `size-${size}`,
      { 'is-loading': loading, 'is-full-width': fullWidth, 'has-icon': icon }
    ]"
    :disabled="disabled || loading"
    @click="emit('click', $event)"
  >
    <span v-if="loading" class="btn-spinner" />
    <svg v-else-if="icon" class="btn-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
      <path v-if="icon === 'play'" d="M5 3l14 9-14 9V3z" />
      <path v-if="icon === 'refresh'" d="M1 4v6h6M23 20v-6h-6" />
      <path v-if="icon === 'refresh'" d="M20.49 9A9 9 0 005.64 5.64L1 10m22 4l-4.64 4.36A9 9 0 013.51 15" />
      <path v-if="icon === 'trash'" d="M3 6h18M19 6v14a2 2 0 01-2 2H7a2 2 0 01-2-2V6m3 0V4a2 2 0 012-2h4a2 2 0 012 2v2" />
      <path v-if="icon === 'x'" d="M18 6L6 18M6 6l12 12" />
      <path v-if="icon === 'check'" d="M20 6L9 17l-5-5" />
      <path v-if="icon === 'search'" d="M10 2a8 8 0 100 16 8 8 0 000-16zM21 21l-4.35-4.35" />
      <path v-if="icon === 'undo'" d="M1 4v6h6M3.51 15a9 9 0 102.13-9.36L1 10" />
      <path v-if="icon === 'back'" d="M19 12H5M12 19l-7-7 7-7" />
      <path v-if="icon === 'settings'" d="M12 15a3 3 0 100-6 3 3 0 000 6z" />
      <path v-if="icon === 'settings'" d="M19.4 15a1.65 1.65 0 00.33 1.82l.06.06a2 2 0 01-2.83 2.83l-.06-.06a1.65 1.65 0 00-1.82-.33 1.65 1.65 0 00-1 1.51V21a2 2 0 01-4 0v-.09A1.65 1.65 0 009 19.4a1.65 1.65 0 00-1.82.33l-.06.06a2 2 0 01-2.83-2.83l.06-.06A1.65 1.65 0 004.68 15a1.65 1.65 0 00-1.51-1H3a2 2 0 010-4h.09A1.65 1.65 0 004.6 9a1.65 1.65 0 00-.33-1.82l-.06-.06a2 2 0 012.83-2.83l.06.06A1.65 1.65 0 009 4.68a1.65 1.65 0 001-1.51V3a2 2 0 014 0v.09a1.65 1.65 0 001 1.51 1.65 1.65 0 001.82-.33l.06-.06a2 2 0 012.83 2.83l-.06.06A1.65 1.65 0 0019.4 9a1.65 1.65 0 001.51 1H21a2 2 0 010 4h-.09a1.65 1.65 0 00-1.51 1z" />
      <path v-if="icon === 'alert-triangle'" d="M12 9v4M12 17h.01M10.29 3.86l-8.1 14c-.19.33-.2.73-.01 1.07.18.33.53.54.91.54h16.62c.38 0 .73-.21.91-.54.19-.34.18-.74-.01-1.07l-8.1-14A.98.98 0 0012 3.5c-.38 0-.73.21-.91.54z" />
      <path v-if="icon === 'info'" d="M12 16v-4M12 8h.01M12 22c5.523 0 10-4.477 10-10S17.523 2 12 2 2 6.477 2 12s4.477 10 10 10z" />
    </svg>
    <span v-if="$slots.default" class="btn-label"><slot /></span>
  </button>
</template>

<style scoped>
.base-btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  gap: var(--space-2);
  border: none;
  border-radius: var(--radius-lg);
  font-family: var(--font-sans);
  font-weight: var(--weight-medium);
  cursor: pointer;
  transition: all var(--duration-fast) var(--ease-out);
  position: relative;
  overflow: hidden;
  white-space: nowrap;
  user-select: none;
  -webkit-user-select: none;
}

.base-btn:disabled {
  opacity: 0.4;
  cursor: not-allowed;
  pointer-events: none;
}

/* Sizes */
.size-sm { padding: var(--space-1) var(--space-3); font-size: var(--text-xs); height: 28px; }
.size-md { padding: var(--space-2) var(--space-5); font-size: var(--text-sm); height: 34px; }
.size-lg { padding: var(--space-3) var(--space-6); font-size: var(--text-md); height: 40px; }

/* Variants */
.variant-primary {
  background: var(--bg-hover);
  color: var(--text-primary);
  border: 1px solid var(--border-default);
}
.variant-primary:hover:not(:disabled) {
  background: var(--bg-active);
  border-color: var(--border-strong);
}

.variant-secondary {
  background: transparent;
  color: var(--text-secondary);
  border: 1px solid var(--border-default);
}
.variant-secondary:hover:not(:disabled) {
  background: var(--glass-bg-hover);
  color: var(--text-primary);
  border-color: var(--glass-border-hover);
}

.variant-accent {
  background: var(--accent);
  color: var(--text-inverse);
  font-weight: var(--weight-semibold);
}
.variant-accent:hover:not(:disabled) {
  background: var(--accent-600);
}
.variant-accent:active:not(:disabled) {
  transform: translateY(0);
}

.variant-ghost {
  background: transparent;
  color: var(--text-secondary);
  border: 1px solid transparent;
}
.variant-ghost:hover:not(:disabled) {
  background: var(--glass-bg-hover);
  color: var(--text-primary);
}

.variant-danger {
  background: var(--danger);
  color: var(--text-inverse);
  font-weight: var(--weight-semibold);
}
.variant-danger:hover:not(:disabled) {
  filter: brightness(.94);
}

.variant-success {
  background: var(--success);
  color: var(--text-inverse);
  font-weight: var(--weight-semibold);
}
.variant-success:hover:not(:disabled) {
  filter: brightness(.94);
}

/* Loading */
.is-loading {
  pointer-events: none;
}

.btn-spinner {
  width: 14px;
  height: 14px;
  border: 2px solid transparent;
  border-top-color: currentColor;
  border-radius: var(--radius-full);
  animation: spin 0.6s linear infinite;
}

.btn-icon {
  width: 16px;
  height: 16px;
  flex-shrink: 0;
}

.btn-label {
  display: inline-flex;
  align-items: center;
  gap: var(--space-1);
}

.is-full-width {
  width: 100%;
}

</style>
