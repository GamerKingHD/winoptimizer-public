<script setup lang="ts">
import { computed } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import BaseButton from './BaseButton.vue'

const props = defineProps<{
  title?: string
  subtitle?: string
}>()

const route = useRoute()
const router = useRouter()
const toolPaths = new Set(['/process-manager', '/repair-center', '/network', '/hosts-editor', '/env-vars', '/appx-manager'])
const showToolsBack = computed(() => toolPaths.has(route.path))
</script>

<template>
  <header class="app-header">
    <div class="header-left">
      <h1 class="page-title">{{ props.title || route.meta.title || 'WinOptimizer' }}</h1>
      <p v-if="props.subtitle || route.meta.subtitle" class="page-subtitle">{{ props.subtitle || route.meta.subtitle }}</p>
    </div>
    <div class="header-right">
      <BaseButton v-if="showToolsBack" variant="secondary" size="sm" icon="back" @click="router.push('/tools')">Back to tools</BaseButton>
      <slot name="actions" />
    </div>
  </header>
</template>

<style scoped>
.app-header {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: var(--space-4);
  margin-bottom: var(--space-6);
  padding-bottom: var(--space-5);
  border-bottom: 1px solid var(--border-subtle);
}

.header-left {
  display: flex;
  flex-direction: column;
  gap: var(--space-1);
}

.page-title {
  margin: 0;
  font-size: var(--text-3xl);
  font-weight: var(--weight-bold);
  color: var(--text-primary);
  line-height: var(--leading-tight);
  letter-spacing: var(--tracking-tight);
}

.page-subtitle {
  margin: 0;
  font-size: var(--text-sm);
  color: var(--text-muted);
  line-height: var(--leading-normal);
}

.header-right {
  display: flex;
  align-items: center;
  gap: var(--space-2);
  flex-shrink: 0;
  padding-top: 2px;
}
</style>
