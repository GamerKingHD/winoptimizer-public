<script setup lang="ts">
import AppHeader from '../components/AppHeader.vue'

const tools = [
  { to: '/process-manager', title: 'Process Manager', description: 'Review running processes and their resource use.', icon: 'activity' },
  { to: '/repair-center', title: 'Repair Center', description: 'Review and run guided Windows repair actions.', icon: 'wrench' },
  { to: '/network', title: 'Network Monitor', description: 'Inspect adapters and active connections.', icon: 'network' },
  { to: '/hosts-editor', title: 'Hosts Editor', description: 'Manage entries in the Windows hosts file.', icon: 'globe' },
  { to: '/env-vars', title: 'Environment Variables', description: 'View and edit user or system variables.', icon: 'code' },
  { to: '/appx-manager', title: 'Preinstalled Apps Policy', description: 'Review pre-installed Windows apps and the policy applied at next sign-in.', icon: 'apps' },
]
</script>

<template>
  <div class="tools-page">
    <AppHeader />

    <section class="intro" aria-labelledby="tools-heading">
      <h2 id="tools-heading">Choose a tool</h2>
      <p>Open a focused tool to inspect or manage a specific part of your PC.</p>
    </section>

    <nav class="tool-grid" aria-label="Tools">
      <RouterLink v-for="tool in tools" :key="tool.to" :to="tool.to" class="tool-card">
        <span class="tool-icon" aria-hidden="true">
          <svg v-if="tool.icon === 'disk'" viewBox="0 0 24 24"><ellipse cx="12" cy="5" rx="8" ry="3" /><path d="M4 5v14c0 1.7 3.6 3 8 3s8-1.3 8-3V5" /><path d="M4 12c0 1.7 3.6 3 8 3s8-1.3 8-3" /></svg>
          <svg v-else-if="tool.icon === 'activity'" viewBox="0 0 24 24"><path d="M3 12h4l3-7 4 14 2-7h5" /></svg>
          <svg v-else-if="tool.icon === 'wrench'" viewBox="0 0 24 24"><path d="M14.7 6.3a4 4 0 0 0-5 5L4 17v3h3l5.7-5.7a4 4 0 0 0 5-5l-3 3-3-3 3-3Z" /></svg>
          <svg v-else-if="tool.icon === 'network'" viewBox="0 0 24 24"><circle cx="12" cy="12" r="3" /><path d="M12 2v3M12 19v3M2 12h3M19 12h3M4.9 4.9 7 7M17 17l2.1 2.1M19.1 4.9 17 7M7 17l-2.1 2.1" /></svg>
          <svg v-else-if="tool.icon === 'globe'" viewBox="0 0 24 24"><circle cx="12" cy="12" r="9" /><path d="M3 12h18M12 3c2.4 2.5 3.6 5.5 3.6 9S14.4 18.5 12 21c-2.4-2.5-3.6-5.5-3.6-9S9.6 5.5 12 3Z" /></svg>
          <svg v-else-if="tool.icon === 'code'" viewBox="0 0 24 24"><path d="m8 9-3 3 3 3M16 9l3 3-3 3M14 5l-4 14" /></svg>
          <svg v-else viewBox="0 0 24 24"><rect x="4" y="4" width="6" height="6" rx="1" /><rect x="14" y="4" width="6" height="6" rx="1" /><rect x="4" y="14" width="6" height="6" rx="1" /><rect x="14" y="14" width="6" height="6" rx="1" /></svg>
        </span>
        <span class="tool-copy"><strong>{{ tool.title }}</strong><span>{{ tool.description }}</span></span>
        <span class="tool-arrow" aria-hidden="true">→</span>
      </RouterLink>
    </nav>
  </div>
</template>

<style scoped>
.tools-page { width: 100%; min-width: 0; }
.intro { margin-bottom: var(--space-5); }
.intro h2 { margin: 0 0 var(--space-1); color: var(--text-primary); font-size: var(--text-lg); }
.intro p { margin: 0; color: var(--text-secondary); font-size: var(--text-sm); }
.tool-grid { display: grid; grid-template-columns: repeat(auto-fit, minmax(250px, 1fr)); gap: var(--space-3); }
.tool-card { display: grid; grid-template-columns: auto 1fr auto; align-items: center; gap: var(--space-3); min-height: 112px; padding: var(--space-4); color: inherit; text-decoration: none; background: var(--bg-elevated); border: 1px solid var(--border-default); border-radius: var(--radius-xl); transition: border-color var(--duration-fast), box-shadow var(--duration-fast), transform var(--duration-fast); }
.tool-card:hover { border-color: var(--accent); box-shadow: var(--shadow-sm); transform: translateY(-1px); }
.tool-icon { display: grid; place-items: center; width: 40px; height: 40px; color: var(--accent); background: var(--bg-hover); border-radius: var(--radius-lg); }
.tool-icon svg { width: 20px; height: 20px; fill: none; stroke: currentColor; stroke-width: 1.8; stroke-linecap: round; stroke-linejoin: round; }
.tool-copy { display: grid; gap: var(--space-1); min-width: 0; }
.tool-copy strong { color: var(--text-primary); font-size: var(--text-sm); font-weight: var(--weight-semibold); }
.tool-copy span { color: var(--text-secondary); font-size: var(--text-xs); line-height: var(--leading-relaxed); }
.tool-arrow { color: var(--text-muted); font-size: var(--text-lg); }
@media (max-width: 600px) { .tool-grid { grid-template-columns: 1fr; } }
</style>
