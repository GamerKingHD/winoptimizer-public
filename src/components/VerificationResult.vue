<script setup lang="ts">
import { computed } from 'vue'
import type { TargetOutcome } from '../api/operation'
import BaseBadge from './BaseBadge.vue'

const props = withDefaults(defineProps<{
  status?: TargetOutcome['verification']
  message?: string | null
}>(), {
  status: 'not_applicable',
  message: null,
})

const label = computed(() => props.status.replaceAll('_', ' '))
const variant = computed<'default' | 'success' | 'warning' | 'danger'>(() => {
  if (props.status === 'verified') return 'success'
  if (props.status === 'unverified' || props.status === 'not_applicable') return 'warning'
  return 'danger'
})
</script>

<template>
  <span class="verification-result">
    <BaseBadge :variant="variant" size="sm" dot>{{ label }}</BaseBadge>
    <span v-if="message" class="verification-message">{{ message }}</span>
  </span>
</template>

<style scoped>
.verification-result { display: inline-flex; align-items: center; flex-wrap: wrap; gap: var(--space-2); }
.verification-message { color: var(--text-secondary); }
</style>
