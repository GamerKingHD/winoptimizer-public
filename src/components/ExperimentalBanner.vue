<script setup lang="ts">
withDefaults(defineProps<{
  title?: string
  message?: string
}>(), {
  title: 'Experimental feature',
  message: 'This workflow is available for review, but its evidence is not yet release-proven.',
})
</script>

<template>
  <aside class="safety-banner experimental-banner" role="note">
    <strong>{{ title }}</strong>
    <span>{{ message }}</span>
  </aside>
</template>

<style scoped>
.safety-banner { display: flex; flex-wrap: wrap; gap: var(--space-2); align-items: baseline; padding: var(--space-3); border: 1px solid var(--border-subtle); border-radius: var(--radius-lg); color: var(--text-secondary); font-size: var(--text-xs); }
.experimental-banner { background: var(--color-warning-bg); }
strong { color: var(--text-primary); }
</style>
