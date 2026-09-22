<script setup lang="ts">
import { computed } from 'vue'
import type { OperationDescriptor } from '../api/operation'
import BaseBadge from './BaseBadge.vue'

const props = withDefaults(defineProps<{
  risk?: OperationDescriptor['risk']
  size?: 'sm' | 'md'
}>(), {
  risk: 'none',
  size: 'sm',
})

const label = computed(() => ({
  none: 'No',
  low: 'Low',
  medium: 'Medium',
  high: 'High',
  critical: 'Critical',
}[props.risk] ?? 'Unknown'))

const variant = computed<'default' | 'success' | 'warning' | 'danger' | 'info'>(() => {
  if (props.risk === 'low') return 'success'
  if (props.risk === 'medium') return 'warning'
  if (props.risk === 'high' || props.risk === 'critical') return 'danger'
  if (props.risk === 'none') return 'info'
  return 'default'
})
</script>

<template>
  <BaseBadge :variant="variant" :size="size" dot>{{ label }} risk</BaseBadge>
</template>
