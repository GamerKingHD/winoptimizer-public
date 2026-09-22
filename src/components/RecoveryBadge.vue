<script setup lang="ts">
import { computed } from 'vue'
import type { OperationDescriptor, OperationRecovery } from '../api/operation'
import BaseBadge from './BaseBadge.vue'

const props = withDefaults(defineProps<{
  recovery?: OperationRecovery
  reversibility?: OperationDescriptor['reversibility']
  size?: 'sm' | 'md'
}>(), {
  recovery: undefined,
  reversibility: undefined,
  size: 'sm',
})

const label = computed(() => {
  if (props.recovery === 'exact') return 'Exact recovery'
  if (props.recovery === 'best_effort') return 'Best-effort recovery'
  if (props.recovery === 'none') return 'No automatic recovery'
  if (props.reversibility === 'fully_reversible') return 'Fully reversible'
  if (props.reversibility === 'best_effort') return 'Best-effort recovery'
  if (props.reversibility === 'not_reversible') return 'Not reversible'
  return 'Recovery not declared'
})

const variant = computed<'default' | 'success' | 'warning' | 'danger'>(() => {
  if (props.recovery === 'exact' || props.reversibility === 'fully_reversible') return 'success'
  if (props.recovery === 'best_effort' || props.reversibility === 'best_effort') return 'warning'
  if (props.recovery === 'none' || props.reversibility === 'not_reversible') return 'danger'
  return 'default'
})
</script>

<template>
  <BaseBadge :variant="variant" :size="size">{{ label }}</BaseBadge>
</template>
