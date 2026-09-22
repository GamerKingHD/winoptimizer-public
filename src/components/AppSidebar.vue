<script setup lang="ts">
import { ref, onMounted } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { checkElevated } from '../api/tauri'

const router = useRouter()
const route = useRoute()
const collapsed = ref(false)
const isElevated = ref(true)

const items = [
  { name: 'Home', path: '/', icon: 'home' },
  { name: 'Clean Up', path: '/disk-cleaner', icon: 'clean' },
  { name: 'Disk Analyzer', path: '/disk-analyzer', icon: 'disk' },
  { name: 'Performance', path: '/performance', icon: 'performance' },
  { name: 'Startup Manager', path: '/startup-manager', icon: 'startup' },
  { name: 'Tools', path: '/tools', icon: 'tools' },
  { name: 'System Tweaks', path: '/system-tweaks', icon: 'tweaks' },
  { name: 'History & Undo', path: '/history', icon: 'history' },
  { name: 'Privacy & Updates', path: '/privacy', icon: 'privacy' },
]

onMounted(async () => {
  try { isElevated.value = await checkElevated() } catch { isElevated.value = false }
})

function navigate(path: string) { router.push(path) }
function prefetch(path: string) {
  const component = router.resolve(path).matched[0]?.components?.default as { __prefetch?: () => void } | undefined
  component?.__prefetch?.()
}
</script>

<template>
  <aside class="sidebar" :class="{ 'is-collapsed': collapsed }">
    <div class="sidebar-brand">
      <div class="brand-logo" aria-hidden="true">W</div>
      <span v-show="!collapsed" class="brand-name">WinOptimizer</span>
      <button class="collapse-button" type="button" :aria-label="collapsed ? 'Expand navigation' : 'Collapse navigation'" @click="collapsed = !collapsed">
        {{ collapsed ? '›' : '‹' }}
      </button>
    </div>

    <nav class="sidebar-nav" aria-label="Main navigation">
      <button v-for="item in items" :key="item.path" class="nav-item" :class="{ active: route.path === item.path }" :aria-current="route.path === item.path ? 'page' : undefined" type="button" :title="collapsed ? item.name : undefined" @click="navigate(item.path)" @mouseenter="prefetch(item.path)">
        <svg class="nav-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" aria-hidden="true">
          <path v-if="item.icon === 'home'" d="m3 10 9-7 9 7v10a1 1 0 0 1-1 1h-5v-6H9v6H4a1 1 0 0 1-1-1z" />
          <path v-else-if="item.icon === 'clean'" d="M4 7h16M9 7V4h6v3m3 0-1 13H7L6 7m4 4v5m4-5v5" />
          <path v-else-if="item.icon === 'disk'" d="M5 6c0-1.1 3.1-2 7-2s7 .9 7 2-3.1 2-7 2-7-.9-7-2Zm0 0v6c0 1.1 3.1 2 7 2s7-.9 7-2V6m-14 6v6c0 1.1 3.1 2 7 2s7-.9 7-2v-6" />
          <path v-else-if="item.icon === 'performance'" d="M4 19V5m0 14h16M8 15l3-4 3 2 5-6" />
          <path v-else-if="item.icon === 'startup'" d="M5 19c1-5 4-8 9-10l2-4 3 3-4 2c-2 5-5 8-10 9Zm7-8h.01" />
          <path v-else-if="item.icon === 'tools'" d="M4 5h6v6H4zm10 0h6v6h-6zM4 15h6v5H4zm10 0h6v5h-6z" />
          <path v-else-if="item.icon === 'tweaks'" d="M4 6h16M7 6v5m-3 7h16m-4-5v5m-6-1h10m-13-7v4" />
          <path v-else-if="item.icon === 'privacy'" d="M12 3 19 6v5c0 4.5-3 8-7 10-4-2-7-5.5-7-10V6l7-3Z" />
          <path v-else d="M12 15.5a3.5 3.5 0 1 0 0-7 3.5 3.5 0 0 0 0 7Zm0-12v2m0 13v2m8.5-8.5h-2m-13 0h-2m14.5-6-1.4 1.4M7.4 16.6 6 18m12 0-1.4-1.4M7.4 7.4 6 6" />
        </svg>
        <span v-show="!collapsed">{{ item.name }}</span>
      </button>
    </nav>

    <div v-if="!isElevated && !collapsed" class="limited-mode">
      <strong>Limited Mode</strong>
      <span>Some actions need administrator access.</span>
    </div>
  </aside>
</template>

<style scoped>
.sidebar { width: var(--sidebar-width); min-width: var(--sidebar-width); height: 100vh; display: flex; flex-direction: column; background: var(--bg-surface); border-right: 1px solid var(--border-default); padding: var(--space-4) var(--space-3); transition: width var(--duration-normal) var(--ease-out); }
.sidebar.is-collapsed { width: var(--sidebar-collapsed-width); min-width: var(--sidebar-collapsed-width); padding-inline: var(--space-2); }
.sidebar-brand { display: flex; align-items: center; gap: var(--space-2); min-height: 40px; padding: 0 var(--space-2); }
.brand-logo { display: grid; place-items: center; width: 24px; height: 24px; border-radius: var(--radius-sm); background: var(--accent); color: var(--text-inverse); font-size: var(--text-xs); font-weight: var(--weight-bold); }
.brand-name { flex: 1; font-size: var(--text-sm); font-weight: var(--weight-bold); white-space: nowrap; }
.collapse-button { margin-left: auto; width: 28px; height: 28px; border: 0; border-radius: var(--radius-sm); background: transparent; color: var(--text-muted); cursor: pointer; }
.collapse-button:hover { background: var(--bg-hover); color: var(--text-primary); }
.sidebar-nav { display: grid; gap: var(--space-1); margin-top: var(--space-7); }
.nav-item { display: flex; align-items: center; gap: var(--space-3); min-height: 40px; width: 100%; padding: 0 var(--space-3); border: 0; border-radius: var(--radius-control, var(--radius-lg)); background: transparent; color: var(--text-secondary); text-align: left; cursor: pointer; white-space: nowrap; }
.nav-item:hover { background: var(--bg-hover); color: var(--text-primary); }
.nav-item.active { background: var(--color-primary-100); color: var(--accent); font-weight: var(--weight-semibold); }
.nav-icon { width: 18px; height: 18px; flex: 0 0 18px; }
.limited-mode { display: grid; gap: var(--space-1); margin-top: auto; padding: var(--space-3); border: 1px solid var(--border-default); border-radius: var(--radius-lg); color: var(--text-secondary); font-size: var(--text-xs); }
.limited-mode strong { color: var(--text-primary); }
@media (max-width: 760px) { .sidebar { width: var(--sidebar-collapsed-width); min-width: var(--sidebar-collapsed-width); padding-inline: var(--space-2); } .brand-name, .collapse-button, .nav-item span { display: none !important; } .sidebar-brand, .nav-item { justify-content: center; padding-inline: 0; } }
</style>
