<script setup lang="ts">
import { computed } from 'vue'
import type { OperationDescriptor } from '../api/operation'
import BaseBadge from './BaseBadge.vue'

const props = withDefaults(defineProps<{
  privilege?: OperationDescriptor['privilege']
  requiresElevation?: boolean
  size?: 'sm' | 'md'
}>(), {
  privilege: 'standard_user',
  requiresElevation: false,
  size: 'sm',
})

const requiresAdmin = computed(() => props.requiresElevation || props.privilege === 'administrator')
</script>

<template>
  <BaseBadge :variant="requiresAdmin ? 'warning' : 'default'" :size="size">
    {{ requiresAdmin ? 'Administrator approval' : 'Standard user' }}
  </BaseBadge>
</template>
