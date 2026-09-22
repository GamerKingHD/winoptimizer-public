<script setup lang="ts">
withDefaults(defineProps<{
  title?: string
  message?: string
}>(), {
  title: 'Labs',
  message: 'This change is optional and remains separate from stable recommendations.',
})
</script>

<template>
  <aside class="safety-banner labs-banner" role="note">
    <strong>{{ title }}</strong>
    <span>{{ message }}</span>
  </aside>
</template>

<style scoped>
.safety-banner { display: flex; flex-wrap: wrap; gap: var(--space-2); align-items: baseline; padding: var(--space-3); border: 1px solid var(--border-subtle); border-radius: var(--radius-lg); color: var(--text-secondary); font-size: var(--text-xs); }
.labs-banner { background: var(--color-info-bg); }
strong { color: var(--text-primary); }
</style>
