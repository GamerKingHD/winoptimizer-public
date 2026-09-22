<script setup lang="ts">
import { ref, onMounted } from 'vue'

const props = withDefaults(defineProps<{
  variant: 'success' | 'error' | 'warning' | 'info'
  title: string
  description?: string
  items?: { label: string; value: string | number }[]
  dismissable?: boolean
  autoDismiss?: number
}>(), {
  description: '',
  items: () => [],
  dismissable: true,
  autoDismiss: 0,
})

const emit = defineEmits<{
  dismiss: []
}>()

const visible = ref(true)

onMounted(() => {
  if (props.autoDismiss > 0) {
    setTimeout(() => {
      visible.value = false
      emit('dismiss')
    }, props.autoDismiss)
  }
})

function dismiss() {
  visible.value = false
  emit('dismiss')
}
</script>

<template>
  <div v-if="visible" class="results-banner" :class="`variant-${variant}`" :role="variant === 'error' ? 'alert' : 'status'" :aria-live="variant === 'error' ? 'assertive' : 'polite'">
    <div class="banner-content">
      <div class="banner-header">
        <svg class="banner-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" aria-hidden="true">
          <path v-if="variant === 'success'" d="M22 11.08V12a10 10 0 11-5.93-9.14" />
          <path v-if="variant === 'success'" d="M22 4L12 14.01l-3-3" />
          <path v-if="variant === 'error'" d="M12 9v4M12 17h.01M12 22c5.523 0 10-4.477 10-10S17.523 2 12 2 2 6.477 2 12s4.477 10 10 10z" />
          <path v-if="variant === 'warning'" d="M10.29 3.86L1.82 18a2 2 0 001.71 3h16.94a2 2 0 001.71-3L13.71 3.86a2 2 0 00-3.42 0zM12 9v4M12 17h.01" />
          <path v-if="variant === 'info'" d="M12 16v-4M12 8h.01M12 22c5.523 0 10-4.477 10-10S17.523 2 12 2 2 6.477 2 12s4.477 10 10 10z" />
        </svg>
        <div class="banner-text">
          <span class="banner-title">{{ title }}</span>
          <span v-if="description" class="banner-desc">{{ description }}</span>
        </div>
        <button v-if="dismissable" type="button" class="banner-dismiss" @click="dismiss" aria-label="Dismiss">
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" width="16" height="16" aria-hidden="true">
            <path d="M18 6L6 18M6 6l12 12" />
          </svg>
        </button>
      </div>
      <div v-if="items.length" class="banner-items">
        <div v-for="(item, idx) in items" :key="idx" class="banner-item">
          <span class="item-label">{{ item.label }}</span>
          <span class="item-value">{{ item.value }}</span>
        </div>
      </div>
      <slot />
    </div>
  </div>
</template>

<style scoped>
.results-banner {
  border-radius: var(--radius-xl);
  border: 1px solid;
  overflow: hidden;
  animation: fadeInUp var(--duration-normal) var(--ease-out);
}

.variant-success {
  background: var(--color-success-bg);
  border-color: var(--success);
}

.variant-error {
  background: var(--color-danger-bg);
  border-color: var(--danger);
}

.variant-warning {
  background: var(--color-warning-bg);
  border-color: var(--warning);
}

.variant-info {
  background: var(--color-info-bg);
  border-color: var(--info);
}

.banner-content {
  padding: var(--space-4) var(--space-5);
}

.banner-header {
  display: flex;
  align-items: flex-start;
  gap: var(--space-3);
}

.banner-icon {
  width: 20px;
  height: 20px;
  flex-shrink: 0;
  margin-top: 1px;
}

.variant-success .banner-icon { color: var(--success); }
.variant-error .banner-icon { color: var(--danger); }
.variant-warning .banner-icon { color: var(--warning); }
.variant-info .banner-icon { color: var(--info); }

.banner-text {
  flex: 1;
  display: flex;
  flex-direction: column;
  gap: var(--space-1);
}

.banner-title {
  font-size: var(--text-sm);
  font-weight: var(--weight-semibold);
  color: var(--text-primary);
}

.banner-desc {
  font-size: var(--text-xs);
  color: var(--text-secondary);
  line-height: var(--leading-relaxed);
}

.banner-dismiss {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 24px;
  height: 24px;
  border: none;
  background: transparent;
  color: var(--text-muted);
  cursor: pointer;
  border-radius: var(--radius-md);
  flex-shrink: 0;
  transition: all var(--duration-fast);
}

.banner-dismiss:hover {
  background: var(--bg-hover);
  color: var(--text-secondary);
}

.banner-items {
  display: flex;
  flex-wrap: wrap;
  gap: var(--space-3);
  margin-top: var(--space-3);
  padding-top: var(--space-3);
  border-top: 1px solid var(--border-subtle);
}

.banner-item {
  display: flex;
  align-items: center;
  gap: var(--space-2);
  font-size: var(--text-xs);
}

.item-label {
  color: var(--text-muted);
}

.item-value {
  color: var(--text-primary);
  font-weight: var(--weight-semibold);
  font-variant-numeric: tabular-nums;
}

.variant-success .item-value { color: var(--success); }
.variant-error .item-value { color: var(--danger); }
</style>
