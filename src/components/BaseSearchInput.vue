<script setup lang="ts">
const model = defineModel<string>({ default: '' })

withDefaults(defineProps<{
  placeholder?: string
  size?: 'sm' | 'md'
  debounce?: number
}>(), {
  placeholder: 'Search...',
  size: 'md',
  debounce: 0,
})

const emit = defineEmits<{
  search: [value: string]
}>()

function onInput(e: Event) {
  const target = e.target as HTMLInputElement
  model.value = target.value
  emit('search', model.value)
}

function clear() {
  model.value = ''
  emit('search', '')
}
</script>

<template>
  <div class="search-input" :class="`size-${size}`">
    <svg class="search-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
      <circle cx="11" cy="11" r="8" />
      <path d="M21 21l-4.35-4.35" />
    </svg>
    <input
      :value="model"
      @input="onInput"
      :placeholder="placeholder"
      class="search-field"
      type="text"
    />
    <button v-if="model" class="search-clear" @click="clear" tabindex="-1" type="button">
      <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" width="14" height="14">
        <path d="M18 6L6 18M6 6l12 12" />
      </svg>
    </button>
  </div>
</template>

<style scoped>
.search-input {
  display: flex;
  align-items: center;
  gap: var(--space-2);
  background: var(--bg-elevated);
  border: 1px solid var(--border-default);
  border-radius: var(--radius-lg);
  transition: all var(--duration-fast) var(--ease-out);
  position: relative;
}

.search-input:focus-within {
  border-color: var(--accent);
  box-shadow: 0 0 0 3px var(--color-primary-100);
}

.size-sm {
  padding: var(--space-1) var(--space-3);
  height: 30px;
}

.size-md {
  padding: var(--space-2) var(--space-3);
  height: 36px;
}

.search-icon {
  width: 16px;
  height: 16px;
  color: var(--text-muted);
  flex-shrink: 0;
}

.search-field {
  flex: 1;
  border: none;
  background: transparent;
  color: var(--text-primary);
  font-family: var(--font-sans);
  font-size: var(--text-sm);
  outline: none;
  min-width: 0;
}

.search-field::placeholder {
  color: var(--text-muted);
}

.search-clear {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 18px;
  height: 18px;
  border: none;
  background: var(--bg-hover);
  border-radius: var(--radius-full);
  color: var(--text-muted);
  cursor: pointer;
  flex-shrink: 0;
  transition: all var(--duration-fast);
  padding: 0;
}

.search-clear:hover {
  background: var(--bg-active);
  color: var(--text-secondary);
}
</style>
