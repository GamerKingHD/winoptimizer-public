<script setup lang="ts">
import { computed } from 'vue'
import type { OperationProofStatus } from '../api/operation'
import BaseBadge from './BaseBadge.vue'

const props = withDefaults(defineProps<{
  proof?: OperationProofStatus
  experimental?: boolean
  size?: 'sm' | 'md'
}>(), {
  proof: undefined,
  experimental: false,
  size: 'sm',
})

const label = computed(() => {
  if (props.proof === 'proven') return 'Proven'
  if (props.proof === 'partially_proven') return 'Partially proven'
  if (props.proof === 'experimental' || props.experimental) return 'Experimental'
  return 'Proof not declared'
})

const variant = computed<'default' | 'success' | 'warning' | 'danger'>(() => {
  if (props.proof === 'proven') return 'success'
  if (props.proof === 'partially_proven') return 'warning'
  if (props.proof === 'experimental' || props.experimental) return 'danger'
  return 'default'
})
</script>

<template>
  <BaseBadge :variant="variant" :size="size">{{ label }}</BaseBadge>
</template>
