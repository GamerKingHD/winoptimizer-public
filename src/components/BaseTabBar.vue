<script setup lang="ts">
import { nextTick, ref } from 'vue'

const props = withDefaults(defineProps<{
  tabs: { id: string; label: string; badge?: string | number; icon?: string }[]
  modelValue: string
  variant?: 'underline' | 'pills' | 'glass'
  size?: 'sm' | 'md'
}>(), {
  variant: 'glass',
  size: 'md',
})

const emit = defineEmits<{
  'update:modelValue': [id: string]
}>()

const tabButtons = ref<(HTMLButtonElement | null)[]>([])

function select(id: string) {
  emit('update:modelValue', id)
}

function setTabButton(element: unknown, index: number) {
  tabButtons.value[index] = element instanceof HTMLButtonElement ? element : null
}

function handleKeydown(event: KeyboardEvent, index: number) {
  if (props.tabs.length === 0) return
  const direction = event.key === 'ArrowRight' ? 1 : event.key === 'ArrowLeft' ? -1 : 0
  const nextIndex = event.key === 'Home'
    ? 0
    : event.key === 'End'
      ? props.tabs.length - 1
      : direction
        ? (index + direction + props.tabs.length) % props.tabs.length
        : -1
  if (nextIndex < 0) return
  event.preventDefault()
  select(props.tabs[nextIndex].id)
  void nextTick(() => tabButtons.value[nextIndex]?.focus())
}
</script>

<template>
  <div class="tab-bar" :class="[`variant-${props.variant}`, `size-${props.size}`]" role="tablist" aria-label="Sections">
    <button
      v-for="tab in props.tabs"
      :key="tab.id"
      type="button"
      class="tab"
      :class="{ 'is-active': props.modelValue === tab.id }"
      role="tab"
      :aria-selected="props.modelValue === tab.id"
      :tabindex="props.modelValue === tab.id ? 0 : -1"
      :ref="element => setTabButton(element, props.tabs.indexOf(tab))"
      @click="select(tab.id)"
      @keydown="handleKeydown($event, props.tabs.indexOf(tab))"
    >
      <svg v-if="tab.icon" class="tab-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" aria-hidden="true">
        <path v-if="tab.icon === 'folder'" d="M22 19a2 2 0 01-2 2H4a2 2 0 01-2-2V5a2 2 0 012-2h5l2 3h9a2 2 0 012 2z" />
        <path v-if="tab.icon === 'globe'" d="M12 2a10 10 0 1010 10A10 10 0 0012 2zM2 12h20M12 2a15.3 15.3 0 014 10 15.3 15.3 0 01-4 10 15.3 15.3 0 01-4-10 15.3 15.3 0 014-10z" />
        <path v-if="tab.icon === 'refresh-cw'" d="M23 4v6h-6M1 20v-6h6" />
        <path v-if="tab.icon === 'refresh-cw'" d="M3.51 9a9 9 0 0114.85-3.36L23 10M1 14l4.64 4.36A9 9 0 0020.49 15" />
        <path v-if="tab.icon === 'file-text'" d="M14 2H6a2 2 0 00-2 2v16a2 2 0 002 2h12a2 2 0 002-2V8zM14 2v6h6M16 13H8M16 17H8M10 9H8" />
        <path v-if="tab.icon === 'zap'" d="M13 2L3 14h9l-1 8 10-12h-9l1-8z" />
        <path v-if="tab.icon === 'bar-chart'" d="M12 20V10M18 20V4M6 20v-4" />
        <path v-if="tab.icon === 'copy'" d="M8 16H6a2 2 0 01-2-2V6a2 2 0 012-2h8a2 2 0 012 2v2m-6 12h8a2 2 0 002-2v-8a2 2 0 00-2-2h-8a2 2 0 00-2 2v8a2 2 0 002 2z" />
        <path v-if="tab.icon === 'cpu'" d="M9 9h6v6H9zM3 12h2M19 12h2M12 3v2M12 19v2" />
        <path v-if="tab.icon === 'layers'" d="M12 2L2 7l10 5 10-5-10-5zM2 17l10 5 10-5M2 12l10 5 10-5" />
        <path v-if="tab.icon === 'sliders'" d="M4 21v-7M4 10V3M12 21v-9M12 6V3M20 21v-5M20 10V3M1 14h6M9 6h6M17 16h6" />
        <path v-if="tab.icon === 'shield'" d="M12 22s8-4 8-10V5l-8-3-8 3v7c0 6 8 10 8 10z" />
        <path v-if="tab.icon === 'clock'" d="M12 22c5.523 0 10-4.477 10-10S17.523 2 12 2 2 6.477 2 12s4.477 10 10 10zM12 6v6l4 2" />
        <path v-if="tab.icon === 'users'" d="M17 21v-2a4 4 0 00-4-4H5a4 4 0 00-4 4v2M23 21v-2a4 4 0 00-3-3.87M9 3a4 4 0 100 8 4 4 0 000-8zM16 3a4 4 0 100 8 4 4 0 000-8z" />
      </svg>
      <span class="tab-label">{{ tab.label }}</span>
      <span v-if="tab.badge" class="tab-badge">{{ tab.badge }}</span>
    </button>
  </div>
</template>

<style scoped>
.tab-bar {
  display: flex;
  gap: 2px;
  max-width: 100%;
}

/* Glass variant */
.variant-glass {
  background: var(--glass-bg);
  border: 1px solid var(--glass-border);
  border-radius: var(--radius-xl);
  padding: 3px;
  backdrop-filter: blur(var(--glass-blur));
  -webkit-backdrop-filter: blur(var(--glass-blur));
}

.variant-glass .tab {
  flex: 1;
  display: flex;
  align-items: center;
  justify-content: center;
  gap: var(--space-2);
  padding: var(--space-2) var(--space-4);
  border: none;
  background: transparent;
  color: var(--text-muted);
  font-family: var(--font-sans);
  font-size: var(--text-sm);
  font-weight: var(--weight-medium);
  border-radius: calc(var(--radius-xl) - 3px);
  cursor: pointer;
  transition: all var(--duration-fast) var(--ease-out);
  white-space: nowrap;
  position: relative;
}

.variant-glass .tab:hover {
  color: var(--text-secondary);
  background: var(--glass-bg-hover);
}

.variant-glass .tab.is-active {
  background: var(--bg-hover);
  color: var(--text-primary);
  border: 1px solid var(--border-default);
  box-shadow: var(--shadow-sm);
}

/* Underline variant */
.variant-underline {
  border-bottom: 1px solid var(--border-default);
  gap: 0;
  padding: 0;
}

.variant-underline .tab {
  padding: var(--space-3) var(--space-5);
  border: none;
  background: transparent;
  color: var(--text-muted);
  font-family: var(--font-sans);
  font-size: var(--text-sm);
  font-weight: var(--weight-medium);
  cursor: pointer;
  position: relative;
  transition: color var(--duration-fast);
}

.variant-underline .tab::after {
  content: '';
  position: absolute;
  bottom: -1px;
  left: var(--space-5);
  right: var(--space-5);
  height: 2px;
  background: var(--accent);
  border-radius: 1px;
  transform: scaleX(0);
  transition: transform var(--duration-normal) var(--ease-out);
}

.variant-underline .tab.is-active {
  color: var(--text-primary);
}

.variant-underline .tab.is-active::after {
  transform: scaleX(1);
}

/* Pills variant */
.variant-pills {
  gap: var(--space-1);
}

.variant-pills .tab {
  padding: var(--space-2) var(--space-4);
  border: none;
  background: transparent;
  color: var(--text-muted);
  font-family: var(--font-sans);
  font-size: var(--text-sm);
  font-weight: var(--weight-medium);
  border-radius: var(--radius-full);
  cursor: pointer;
  transition: all var(--duration-fast);
}

.variant-pills .tab:hover {
  background: var(--glass-bg-hover);
}

.variant-pills .tab.is-active {
  background: var(--accent);
  color: var(--text-inverse);
}

/* Size */
.size-sm .tab {
  padding: var(--space-1) var(--space-3);
  font-size: var(--text-xs);
}

.size-sm .tab-icon {
  width: 14px;
  height: 14px;
}

.tab-icon {
  width: 16px;
  height: 16px;
  flex-shrink: 0;
}

.tab-badge {
  font-size: var(--text-xs);
  padding: 1px 6px;
  border-radius: var(--radius-full);
  background: var(--bg-hover);
  min-width: 18px;
  text-align: center;
}

.tab.is-active .tab-badge {
  background: var(--color-primary-700);
}

@media (max-width: 700px) {
  .tab-bar {
    overflow: hidden;
  }

  .variant-glass .tab,
  .variant-pills .tab {
    flex: 1 1 0;
    min-width: 0;
  }

  .variant-glass .tab {
    min-width: 0;
    padding-inline: var(--space-3);
  }

  .tab-label {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
  }
}
</style>
