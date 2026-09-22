<script setup lang="ts">
withDefaults(defineProps<{
  icon?: string
  title?: string
  description?: string
  actionLabel?: string
  compact?: boolean
}>(), {
  icon: 'inbox',
  title: 'Nothing here yet',
  description: 'Data will appear here once available.',
  actionLabel: '',
  compact: false,
})

const emit = defineEmits<{
  action: []
}>()
</script>

<template>
  <div class="empty-state" :class="{ 'is-compact': compact }">
    <div class="empty-icon" aria-hidden="true">
      <svg v-if="icon === 'inbox'" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5">
        <path d="M22 12h-6l-2 3H10l-2-3H2" />
        <path d="M5.45 5.11L2 12v6a2 2 0 002 2h16a2 2 0 002-2v-6l-3.45-6.89A2 2 0 0016.76 4H7.24a2 2 0 00-1.79 1.11z" />
      </svg>
      <svg v-else-if="icon === 'search'" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5">
        <circle cx="11" cy="11" r="8" />
        <path d="M21 21l-4.35-4.35" />
      </svg>
      <svg v-else-if="icon === 'check-circle'" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5">
        <path d="M22 11.08V12a10 10 0 11-5.93-9.14" />
        <path d="M22 4L12 14.01l-3-3" />
      </svg>
      <svg v-else-if="icon === 'shield'" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5">
        <path d="M12 22s8-4 8-10V5l-8-3-8 3v7c0 6 8 10 8 10z" />
      </svg>
      <svg v-else viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5">
        <circle cx="12" cy="12" r="10" />
        <path d="M12 8v4M12 16h.01" />
      </svg>
    </div>
    <h3 class="empty-title">{{ title }}</h3>
    <p class="empty-description">{{ description }}</p>
    <button v-if="actionLabel" type="button" class="empty-action" @click="emit('action')">
      {{ actionLabel }}
    </button>
    <slot />
  </div>
</template>

<style scoped>
.empty-state {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  padding: var(--space-12) var(--space-6);
  text-align: center;
  min-height: 200px;
}

.is-compact {
  padding: var(--space-8);
  min-height: auto;
}

.empty-icon {
  width: 64px;
  height: 64px;
  margin-bottom: var(--space-4);
  color: var(--text-muted);
  opacity: 0.4;
}

.empty-icon svg {
  width: 100%;
  height: 100%;
}

.empty-title {
  margin: 0 0 var(--space-2);
  font-size: var(--text-lg);
  font-weight: var(--weight-semibold);
  color: var(--text-secondary);
}

.empty-description {
  margin: 0 0 var(--space-5);
  font-size: var(--text-sm);
  color: var(--text-muted);
  max-width: 360px;
  line-height: var(--leading-relaxed);
}

.empty-action {
  padding: var(--space-2) var(--space-5);
  background: var(--accent);
  color: var(--text-inverse);
  border: none;
  border-radius: var(--radius-lg);
  font-size: var(--text-sm);
  font-weight: var(--weight-semibold);
  cursor: pointer;
  transition: all var(--duration-fast);
}

.empty-action:hover {
  background: var(--accent-600);
}
</style>
