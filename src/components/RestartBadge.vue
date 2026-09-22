<script setup lang="ts">
import { computed } from 'vue'
import type { OperationDescriptor } from '../api/operation'
import BaseBadge from './BaseBadge.vue'

const props = withDefaults(defineProps<{
  policy?: OperationDescriptor['reboot_policy']
  requiresRestart?: boolean
  size?: 'sm' | 'md'
}>(), {
  policy: 'not_required',
  requiresRestart: false,
  size: 'sm',
})

const label = computed(() => {
  if (props.requiresRestart || props.policy === 'required') return 'Required'
  if (props.policy === 'may_be_required') return 'May be required'
  return 'Not required'
})
</script>

<template>
  <BaseBadge :variant="label === 'Not required' ? 'default' : 'warning'" :size="size">
    {{ label }}
  </BaseBadge>
</template>
