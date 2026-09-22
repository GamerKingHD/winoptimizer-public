<script setup lang="ts">
import { useId } from 'vue'
import BaseBadge from './BaseBadge.vue'

type EvidenceItem = {
  label: string
  detail?: string
  status?: 'verified' | 'unverified' | 'warning' | 'info'
}

withDefaults(defineProps<{
  items: EvidenceItem[]
  heading?: string
  emptyLabel?: string
}>(), {
  heading: '',
  emptyLabel: 'No evidence recorded.',
})

const headingId = `evidence-list-${useId()}`
</script>

<template>
  <section class="evidence-list" :aria-labelledby="heading ? headingId : undefined">
    <h3 v-if="heading" :id="headingId">{{ heading }}</h3>
    <ul v-if="items.length > 0">
      <li v-for="item in items" :key="`${item.label}:${item.detail ?? ''}`">
        <BaseBadge
          v-if="item.status"
          :variant="item.status === 'verified' ? 'success' : item.status === 'warning' ? 'warning' : item.status === 'unverified' ? 'danger' : 'info'"
          size="sm"
        >
          {{ item.status }}
        </BaseBadge>
        <span>
          <strong>{{ item.label }}</strong>
          <span v-if="item.detail"> — {{ item.detail }}</span>
        </span>
      </li>
    </ul>
    <p v-else class="evidence-empty">{{ emptyLabel }}</p>
  </section>
</template>

<style scoped>
.evidence-list { display: grid; gap: var(--space-2); }
h3 { margin: 0; font-size: var(--text-sm); }
ul { display: grid; gap: var(--space-2); margin: 0; padding-left: var(--space-5); color: var(--text-secondary); }
li { display: flex; align-items: flex-start; gap: var(--space-2); }
li::marker { color: var(--accent); }
li strong { color: var(--text-primary); font-weight: var(--weight-semibold); }
.evidence-empty { margin: 0; color: var(--text-secondary); font-size: var(--text-sm); }
</style>
