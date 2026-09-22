<script setup lang="ts">
withDefaults(defineProps<{
  size?: 'sm' | 'md' | 'lg'
  label?: string
  overlay?: boolean
  variant?: 'accent' | 'default'
}>(), {
  size: 'md',
  label: '',
  overlay: false,
  variant: 'accent',
})
</script>

<template>
  <div v-if="overlay" class="spinner-overlay">
    <div class="spinner-inner">
      <div class="spinner" :class="[`size-${size}`, `variant-${variant}`]">
        <div class="spinner-ring" />
        <div class="spinner-ring spinner-ring-2" />
      </div>
      <p v-if="label" class="spinner-label">{{ label }}</p>
    </div>
  </div>
  <div v-else class="spinner-inline">
    <div class="spinner" :class="[`size-${size}`, `variant-${variant}`]">
      <div class="spinner-ring" />
      <div class="spinner-ring spinner-ring-2" />
    </div>
    <span v-if="label" class="spinner-label">{{ label }}</span>
  </div>
</template>

<style scoped>
/* Dual-ring spinner */
.spinner {
  position: relative;
  display: inline-flex;
  align-items: center;
  justify-content: center;
}

.size-sm {
  width: 20px;
  height: 20px;
}

.size-md {
  width: 32px;
  height: 32px;
}

.size-lg {
  width: 48px;
  height: 48px;
}

.spinner-ring {
  position: absolute;
  inset: 0;
  border-radius: var(--radius-full);
  border: 2px solid transparent;
}

.variant-accent .spinner-ring {
  border-top-color: var(--accent);
  animation: spin 0.8s linear infinite;
}

.variant-default .spinner-ring {
  border-top-color: var(--text-muted);
  animation: spin 0.8s linear infinite;
}

.spinner-ring-2 {
  inset: 3px;
  border-width: 2px;
  animation-duration: 1.2s;
  animation-direction: reverse;
}

.variant-accent .spinner-ring-2 {
  border-right-color: var(--accent-300);
}

.variant-default .spinner-ring-2 {
  border-right-color: var(--text-secondary);
}

/* Overlay mode */
.spinner-overlay {
  position: fixed;
  inset: 0;
  display: flex;
  align-items: center;
  justify-content: center;
  background: rgba(25, 27, 37, .32);
  backdrop-filter: blur(8px);
  z-index: var(--z-modal-backdrop);
}

.spinner-inner {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: var(--space-4);
}

/* Inline mode */
.spinner-inline {
  display: inline-flex;
  align-items: center;
  gap: var(--space-3);
}

.spinner-label {
  font-size: var(--text-sm);
  color: var(--text-secondary);
  margin: 0;
  animation: pulse 2s ease-in-out infinite;
}
</style>
