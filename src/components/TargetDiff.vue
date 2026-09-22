<script setup lang="ts">
import { computed } from 'vue'
import type { StateSnapshot } from '../api/operation'

const props = withDefaults(defineProps<{
  before?: StateSnapshot | null
  after?: StateSnapshot | null
  emptyLabel?: string
}>(), {
  before: null,
  after: null,
  emptyLabel: 'State was not captured.',
})

const rows = computed(() => {
  const keys = new Set([
    ...Object.keys(props.before?.properties ?? {}),
    ...Object.keys(props.after?.properties ?? {}),
  ])
  return [...keys].sort().map(key => {
    const before = props.before?.properties[key] ?? 'Not present'
    const after = props.after?.properties[key] ?? 'Not present'
    return { key, before, after, changed: before !== after }
  })
})
</script>

<template>
  <div class="target-diff">
    <div v-if="before?.summary || after?.summary" class="diff-summaries">
      <span><strong>Before:</strong> {{ before?.summary || 'Not captured' }}</span>
      <span><strong>After:</strong> {{ after?.summary || 'Not captured' }}</span>
    </div>
    <table v-if="rows.length > 0">
      <caption class="sr-only">Target state differences</caption>
      <thead><tr><th>Property</th><th>Before</th><th>After</th></tr></thead>
      <tbody>
        <tr v-for="row in rows" :key="row.key" :class="{ changed: row.changed }">
          <th scope="row">{{ row.key }}</th>
          <td>{{ row.before }}</td>
          <td>{{ row.after }}</td>
        </tr>
      </tbody>
    </table>
    <p v-else class="diff-empty">{{ emptyLabel }}</p>
  </div>
</template>

<style scoped>
.target-diff { display: grid; gap: var(--space-2); }
.diff-summaries { display: grid; gap: var(--space-1); color: var(--text-secondary); font-size: var(--text-xs); }
.diff-summaries strong { color: var(--text-primary); }
table { width: 100%; border-collapse: collapse; font-size: var(--text-xs); }
th, td { padding: var(--space-2); border-bottom: 1px solid var(--border-subtle); text-align: left; vertical-align: top; overflow-wrap: anywhere; }
thead th { color: var(--text-muted); font-weight: var(--weight-semibold); }
tbody th { color: var(--text-secondary); font-weight: var(--weight-semibold); }
tr.changed td, tr.changed th { background: color-mix(in srgb, var(--color-warning-bg) 35%, transparent); }
.diff-empty { margin: 0; color: var(--text-secondary); }
.sr-only { position: absolute; width: 1px; height: 1px; padding: 0; margin: -1px; overflow: hidden; clip: rect(0, 0, 0, 0); white-space: nowrap; border: 0; }
</style>
