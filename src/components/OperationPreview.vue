<script setup lang="ts">
import { computed, ref, useId, watch } from 'vue'
import type { OperationPlan } from '../api/operation'
import BaseBadge from './BaseBadge.vue'
import BaseCard from './BaseCard.vue'
import OperationSafetySummary from './OperationSafetySummary.vue'

const props = withDefaults(defineProps<{
  plan: OperationPlan
  title?: string
  description?: string
  actionLabel?: string
}>(), {
  title: 'Exact operation preview',
  description: 'Review the exact plan before confirming. The target list is fixed for this operation.',
  actionLabel: '',
})

const instanceId = useId()
const headingId = `operation-preview-${instanceId}`
const targetsId = `${headingId}-targets`
const targetCount = computed(() => props.plan.targets?.length ?? 0)
const targetPageSize = 50
const targetPage = ref(1)
const targetPageCount = computed(() => Math.max(1, Math.ceil(targetCount.value / targetPageSize)))
const visibleTargets = computed(() => {
  const start = (targetPage.value - 1) * targetPageSize
  return props.plan.targets.slice(start, start + targetPageSize)
})
const targetPageStart = computed(() => (targetPage.value - 1) * targetPageSize + 1)
const targetPageEnd = computed(() => Math.min(targetPage.value * targetPageSize, targetCount.value))

watch(targetCount, () => {
  targetPage.value = Math.min(targetPage.value, targetPageCount.value)
})

watch(() => props.plan.id, () => {
  targetPage.value = 1
})
</script>

<template>
  <BaseCard as="section" class="operation-preview" :aria-labelledby="headingId">
    <div class="preview-header">
      <div>
        <p class="preview-kicker">Review before confirmation</p>
        <h2 :id="headingId">{{ title }}</h2>
        <p class="preview-description">{{ description }}</p>
        <p v-if="actionLabel" class="preview-action"><strong>Action:</strong> {{ actionLabel }}</p>
      </div>
      <BaseBadge variant="info" size="sm">{{ targetCount }} targets</BaseBadge>
    </div>

    <OperationSafetySummary :plan="plan" title="Safety summary" compact />

    <section class="target-section" :aria-labelledby="targetsId">
      <div class="target-heading">
        <h3 :id="targetsId">Exact targets</h3>
        <span>{{ targetCount }} {{ targetCount === 1 ? 'target' : 'targets' }}</span>
      </div>
      <ol v-if="targetCount > 0" class="target-list">
        <li v-for="target in visibleTargets" :key="target.target_id">
          <span class="target-description">{{ target.description }}</span>
          <code>{{ target.target_id }}</code>
        </li>
      </ol>
      <p v-else class="empty-targets">No targets are included in this plan.</p>
      <nav v-if="targetPageCount > 1" class="target-pagination" aria-label="Exact target pages">
        <span aria-live="polite">Showing {{ targetPageStart }}–{{ targetPageEnd }} of {{ targetCount }}</span>
        <div class="target-pagination-actions">
          <button type="button" :disabled="targetPage === 1" aria-label="Previous target page" @click="targetPage -= 1">Previous</button>
          <span>Page {{ targetPage }} of {{ targetPageCount }}</span>
          <button type="button" :disabled="targetPage === targetPageCount" aria-label="Next target page" @click="targetPage += 1">Next</button>
        </div>
      </nav>
    </section>
  </BaseCard>
</template>

<style scoped>
.operation-preview { display: grid; gap: var(--space-5); }
.preview-header { display: flex; align-items: flex-start; justify-content: space-between; gap: var(--space-4); }
.preview-kicker { margin: 0 0 var(--space-1); color: var(--text-muted); font-size: var(--text-xs); font-weight: var(--weight-semibold); letter-spacing: var(--tracking-wide); text-transform: uppercase; }
h2, h3 { margin: 0; }
h2 { font-size: var(--text-xl); }
.preview-description, .preview-action { margin: var(--space-1) 0 0; color: var(--text-secondary); font-size: var(--text-sm); }
.preview-action strong { color: var(--text-primary); }
.target-section { display: grid; gap: var(--space-2); }
.target-heading { display: flex; align-items: baseline; justify-content: space-between; gap: var(--space-3); }
.target-heading h3 { font-size: var(--text-md); }
.target-heading span { color: var(--text-muted); font-size: var(--text-xs); }
.target-list { display: grid; gap: var(--space-2); max-height: 260px; margin: 0; padding: 0; overflow-y: auto; list-style: none; }
.target-list li { display: grid; gap: var(--space-1); padding: var(--space-3); border: 1px solid var(--border-subtle); border-radius: var(--radius-md); background: var(--bg-surface); }
.target-pagination { display: flex; align-items: center; justify-content: space-between; gap: var(--space-3); color: var(--text-muted); font-size: var(--text-xs); }
.target-pagination-actions { display: flex; align-items: center; gap: var(--space-2); }
.target-pagination button { padding: var(--space-1) var(--space-2); border: 1px solid var(--border-subtle); border-radius: var(--radius-sm); background: var(--bg-surface); color: var(--text-primary); font: inherit; cursor: pointer; }
.target-pagination button:disabled { cursor: not-allowed; opacity: 0.5; }
.target-description { color: var(--text-primary); font-size: var(--text-sm); font-weight: var(--weight-semibold); }
code { color: var(--text-secondary); font-family: var(--font-mono); font-size: var(--text-xs); overflow-wrap: anywhere; }
.empty-targets { margin: 0; color: var(--text-secondary); font-size: var(--text-sm); }
@media (max-width: 560px) {
  .preview-header { display: grid; }
}
</style>
