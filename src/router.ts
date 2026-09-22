import { createRouter, createWebHistory } from 'vue-router'
import Dashboard from './views/Dashboard.vue'

// Prefetch-on-hover: wrap lazy imports so hovering a link preloads the component
function prefetchable(importFn: () => Promise<any>): () => Promise<any> {
  let promise: Promise<any> | null = null
  const load = () => {
    if (!promise) promise = importFn()
    return promise
  }
  ;(load as any).__prefetch = () => { if (!promise) promise = importFn() }
  return load
}

const routes = [
  {
    path: '/',
    name: 'Home',
    component: Dashboard,
    meta: { title: 'Home', subtitle: 'Live system status and recent activity', category: 'Overview', requiresAdmin: false, risk: 'low', mutatesSystem: false, commonTool: true },
  },
  {
    path: '/recommendations',
    name: 'Recommendations',
    component: prefetchable(() => import('./views/Recommendations.vue')),
    meta: { title: 'Recommendations', subtitle: 'Review optional guidance before opening a workflow', category: 'Overview', requiresAdmin: false, risk: 'low', mutatesSystem: false, commonTool: true },
  },
  {
    path: '/disk-cleaner',
    name: 'DiskCleaner',
    component: prefetchable(() => import('./views/CleanupReference.vue')),
    meta: { title: 'Clean Up', subtitle: 'Review temporary files, caches, and logs before removing them', category: 'Clean', requiresAdmin: false, risk: 'medium', mutatesSystem: true, commonTool: true },
  },
  {
    path: '/performance',
    name: 'Performance',
    component: prefetchable(() => import('./views/Performance.vue')),
    meta: { title: 'Performance', subtitle: 'A live view of current system usage', category: 'Overview', requiresAdmin: false, risk: 'low', mutatesSystem: false, commonTool: true },
  },
  {
    path: '/tools',
    name: 'Tools',
    component: prefetchable(() => import('./views/Tools.vue')),
    meta: { title: 'Tools', subtitle: 'Choose a focused Windows utility', category: 'Tools', requiresAdmin: false, risk: 'low', mutatesSystem: false, commonTool: true },
  },
  {
    path: '/settings',
    name: 'Settings',
    component: prefetchable(() => import('./views/Settings.vue')),
    meta: { title: 'System & History', subtitle: 'Open system details or review recorded changes', category: 'Overview', requiresAdmin: false, risk: 'low', mutatesSystem: false, commonTool: true },
  },
  {
    path: '/privacy',
    name: 'Privacy',
    component: prefetchable(() => import('./views/Privacy.vue')),
    meta: { title: 'Privacy & Updates', subtitle: 'Local storage, offline behavior, and opt-in updates', category: 'Overview', requiresAdmin: false, risk: 'low', mutatesSystem: false, commonTool: true },
  },
  {
    path: '/repair-center',
    name: 'RepairCenter',
    component: prefetchable(() => import('./views/RepairCenter.vue')),
    meta: { title: 'Repair Center', subtitle: 'Guided Windows repair workflows', category: 'Repair', requiresAdmin: true, risk: 'medium', mutatesSystem: true, commonTool: true },
  },
  {
    path: '/startup-manager',
    name: 'StartupManager',
    component: prefetchable(() => import('./views/StartupManager.vue')),
    meta: { title: 'Startup Manager', subtitle: 'Control what runs when Windows starts', category: 'Control', requiresAdmin: false, risk: 'medium', mutatesSystem: true, commonTool: true },
  },
  {
    path: '/system-tweaks',
    name: 'SystemTweaks',
    component: prefetchable(() => import('./views/SystemTweaks.vue')),
    meta: { title: 'System Tweaks', subtitle: 'Privacy, performance, and customization', category: 'Customize', requiresAdmin: true, risk: 'medium', mutatesSystem: true, commonTool: true },
  },
  {
    path: '/hosts-editor',
    name: 'HostsEditor',
    component: prefetchable(() => import('./views/HostsEditor.vue')),
    meta: { title: 'Hosts Editor', subtitle: 'Manage Windows hosts file entries', category: 'Network', requiresAdmin: true, risk: 'high', mutatesSystem: true, commonTool: false },
  },
  {
    path: '/env-vars',
    name: 'EnvVars',
    component: prefetchable(() => import('./views/EnvVars.vue')),
    meta: { title: 'Environment Variables', subtitle: 'Edit user & system environment variables', category: 'System', requiresAdmin: false, risk: 'medium', mutatesSystem: true, commonTool: false },
  },
  {
    path: '/appx-manager',
    name: 'AppxManager',
    component: prefetchable(() => import('./views/AppxManager.vue')),
    meta: { title: 'Preinstalled Apps Policy', subtitle: 'Review policy for pre-installed modern apps', category: 'Clean', requiresAdmin: true, risk: 'medium', mutatesSystem: true, commonTool: false },
  },
  {
    path: '/network',
    name: 'NetworkMonitor',
    component: prefetchable(() => import('./views/NetworkMonitor.vue')),
    meta: { title: 'Network Monitor', subtitle: 'Adapters & active TCP connections', category: 'Inspect', requiresAdmin: false, risk: 'low', mutatesSystem: false, commonTool: false },
  },
  {
    path: '/disk-analyzer',
    name: 'DiskAnalyzer',
    component: prefetchable(() => import('./views/DiskAnalyzer.vue')),
    meta: { title: 'Disk Analyzer', subtitle: 'Visualize usage and find duplicates', category: 'Inspect', requiresAdmin: false, risk: 'low', mutatesSystem: false, commonTool: true },
  },
  {
    path: '/process-manager',
    name: 'ProcessManager',
    component: prefetchable(() => import('./views/ProcessManager.vue')),
    meta: { title: 'Process Manager', subtitle: 'Monitor and manage running processes', category: 'Control', requiresAdmin: false, risk: 'high', mutatesSystem: true, commonTool: true },
  },
  {
    path: '/system-info',
    name: 'SystemInfo',
    component: prefetchable(() => import('./views/SystemInfo.vue')),
    meta: { title: 'System Info', subtitle: 'Hardware and software inventory', category: 'Inspect', requiresAdmin: false, risk: 'low', mutatesSystem: false, commonTool: true },
  },
  {
    path: '/history',
    name: 'History',
    component: prefetchable(() => import('./views/History.vue')),
    meta: { title: 'History & Undo', subtitle: 'Review and reverse past actions', category: 'Safety', requiresAdmin: false, risk: 'medium', mutatesSystem: true, commonTool: true },
  },
  {
    path: '/:pathMatch(.*)*',
    redirect: '/',
  },
]

const router = createRouter({
  history: createWebHistory(),
  routes,
})

export default router
