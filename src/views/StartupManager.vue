<script setup lang="ts">
import { computed, onActivated, ref } from 'vue'
import type {
  GuidanceAction,
  GuidanceSourceType,
  StartupProgram,
  ServiceInfo,
  ScheduledTask,
} from '../api/tauri'
import * as api from '../api/tauri'
import type { OperationPlan } from '../api/operation'
import { confirmAndRequestElevation } from '../api/operation'
import type { ServiceAction } from '../api/startup'
import {
  createServicePlan,
  createScheduledTaskPlan,
  createScheduledTasksGuidancePlan,
  createServicesGuidancePlan,
  createStartupProgramPlan,
  createStartupProgramEnablePlan,
  executeScheduledTaskPlan,
  executeScheduledTasksGuidancePlan,
  executeServicePlan,
  executeServicesGuidancePlan,
  executeStartupProgramPlan,
  executeStartupProgramEnablePlan,
  scanScheduledTasks,
} from '../api/startup'
import AppHeader from '../components/AppHeader.vue'
import BaseButton from '../components/BaseButton.vue'
import BaseTable from '../components/BaseTable.vue'
import BaseTabBar from '../components/BaseTabBar.vue'
import BaseBadge from '../components/BaseBadge.vue'
import BaseToggle from '../components/BaseToggle.vue'
import BaseSearchInput from '../components/BaseSearchInput.vue'
import EmptyState from '../components/EmptyState.vue'
import ConfirmDialog from '../components/ConfirmDialog.vue'
import ResultsBanner from '../components/ResultsBanner.vue'
import { failure, success, type ActionFeedback } from '../utils/feedback'

const activeTab = ref<string>('programs')

const tabs = [
  { id: 'programs', label: 'Startup Programs', icon: 'zap' },
  { id: 'services', label: 'Services', icon: 'cpu' },
  { id: 'tasks', label: 'Scheduled Tasks', icon: 'clock' },
]

const programs = ref<StartupProgram[]>([])
const services = ref<ServiceInfo[]>([])
const tasks = ref<ScheduledTask[]>([])
const taskCollectedAt = ref('')
const loading = ref(false)
const feedback = ref<ActionFeedback | null>(null)
const serviceSearch = ref('')
const taskSearch = ref('')
const serviceGroupFilter = ref<GuidanceAction | 'all'>('all')
const taskGroupFilter = ref<GuidanceAction | 'all'>('all')
const serviceSourceFilter = ref<GuidanceSourceType | 'all'>('all')
const taskSourceFilter = ref<GuidanceSourceType | 'all'>('all')
const selectedServiceNames = ref(new Set<string>())
const selectedTaskNames = ref(new Set<string>())

const guidanceGroups: Array<{ value: GuidanceAction; label: string }> = [
  { value: 'keep_enabled', label: 'Keep enabled' },
  { value: 'set_manual', label: 'Set to Manual' },
  { value: 'disable_if_unused', label: 'Disable if unused' },
  { value: 'review_first', label: 'Review first' },
]

const guidanceSources: Array<{ value: GuidanceSourceType; label: string }> = [
  { value: 'microsoft', label: 'Microsoft guidance' },
  { value: 'vendor', label: 'Vendor guidance' },
  { value: 'community', label: 'Community' },
  { value: 'policy', label: 'Protected policy' },
  { value: 'unknown', label: 'Unknown' },
]

const programsCols = [
  { key: 'name', label: 'Name', sortable: true },
  { key: 'command', label: 'Command', sortable: true },
  { key: 'location', label: 'Location', sortable: true, width: '120px' },
  { key: 'enabled', label: 'Status', width: '100px', align: 'center' as const },
]

const servicesCols = [
  { key: 'select', label: '', width: '42px', sortable: false, align: 'center' as const },
  { key: 'name', label: 'Name', sortable: true, width: '19%' },
  { key: 'display_name', label: 'Display Name', sortable: true, width: '23%' },
  { key: 'status', label: 'Status', sortable: true, width: '12%' },
  { key: 'start_type', label: 'Start Type', sortable: true, width: '15%' },
  { key: 'guidance', label: 'Guidance', sortable: true, width: '18%' },
  { key: 'actions', label: 'Actions', width: '13%', sortable: false },
]

const tasksCols = [
  { key: 'select', label: '', width: '42px', sortable: false, align: 'center' as const },
  { key: 'name', label: 'Task Name', sortable: true, width: '30%' },
  { key: 'trigger', label: 'Trigger', sortable: true, width: '15%' },
  { key: 'status', label: 'Status', sortable: true, width: '12%' },
  { key: 'classification', label: 'Guidance', sortable: true, width: '26%' },
  { key: 'actions', label: 'Actions', width: '17%', sortable: false },
]

type StartupImpact = 'high' | 'medium' | 'low' | 'protected'

interface StartupGuideEntry {
  software: string
  matches: string[]
  impact: StartupImpact
  delayedBenefit: string
  improvement: string
  recommendation: string
}

const startupGuide: StartupGuideEntry[] = [
  {
    software: 'Microsoft Teams / Slack / Zoom',
    matches: ['teams', 'ms-teams', 'slack', 'zoom'],
    impact: 'high',
    delayedBenefit: 'High',
    improvement: 'Often noticeable: the desktop becomes usable before the client loads.',
    recommendation: 'Delay unless you need chat or calls immediately after sign-in.',
  },
  {
    software: 'Cloud sync: OneDrive / Dropbox / Google Drive',
    matches: ['onedrive', 'dropbox', 'google drive', 'googledrivesync'],
    impact: 'medium',
    delayedBenefit: 'Medium',
    improvement: 'Usually modest: sign-in gets more CPU and disk headroom for a short period.',
    recommendation: 'Delay on slower disks; keep enabled if files must sync immediately.',
  },
  {
    software: 'Game launchers: Steam / Epic / Battle.net',
    matches: ['steam', 'epicgameslauncher', 'epic games', 'battle.net', 'blizzard'],
    impact: 'medium',
    delayedBenefit: 'Medium',
    improvement: 'Can reduce sign-in disk activity, especially when launchers check updates.',
    recommendation: 'Delay unless you want the launcher ready before opening a game.',
  },
  {
    software: 'Adobe Creative Cloud',
    matches: ['creative cloud', 'adobe gc invoker', 'adobe creative'],
    impact: 'medium',
    delayedBenefit: 'Medium',
    improvement: 'Often frees a noticeable amount of background disk and CPU work at sign-in.',
    recommendation: 'Delay if Adobe apps are not part of every session.',
  },
  {
    software: 'Discord / Spotify',
    matches: ['discord', 'spotify'],
    impact: 'medium',
    delayedBenefit: 'Low–medium',
    improvement: 'Usually a small improvement; the benefit is larger on older hardware.',
    recommendation: 'Delay unless voice chat or music is an immediate requirement.',
  },
  {
    software: 'Razer Synapse / Logitech G HUB',
    matches: ['razer', 'synapse', 'logitech g hub', 'lghub'],
    impact: 'medium',
    delayedBenefit: 'Medium',
    improvement: 'Can shorten the busy period after sign-in while keeping the device usable.',
    recommendation: 'Delay only if lighting or custom profiles are not needed immediately.',
  },
  {
    software: 'NVIDIA / AMD companion apps',
    matches: ['geforce experience', 'nvidia app', 'amd software', 'radeon'],
    impact: 'low',
    delayedBenefit: 'Low',
    improvement: 'Usually small; graphics drivers and core services should remain enabled.',
    recommendation: 'Prefer delaying the companion UI, never the display or driver service.',
  },
  {
    software: 'Windows Security / antivirus',
    matches: ['windows security', 'windows defender', 'securityhealth', 'antivirus'],
    impact: 'protected',
    delayedBenefit: 'None',
    improvement: 'Do not trade security coverage for a faster sign-in.',
    recommendation: 'Keep enabled. WinOptimizer protects these entries from disabling.',
  },
]

function guidanceForProgram(program: StartupProgram): StartupGuideEntry | null {
  const identity = (program.name + ' ' + program.command).toLowerCase()
  return startupGuide.find(entry => entry.matches.some(match => identity.includes(match))) || null
}

const detectedGuidance = computed(() => {
  const matches = new Map<StartupGuideEntry, string[]>()

  for (const program of programs.value) {
    if (!program.enabled) continue
    const entry = guidanceForProgram(program)
    if (!entry) continue

    const names = matches.get(entry) || []
    if (!names.includes(program.name)) names.push(program.name)
    matches.set(entry, names)
  }

  return Array.from(matches, ([entry, names]) => ({ ...entry, software: names.join(', ') }))
})

const programsRows = computed(() =>
  programs.value.map(p => ({
    ...p,
    _id: p.name + p.location,
    guidance: guidanceForProgram(p),
    protected: p.protected !== false,
  }))
)

function guidanceGroupLabel(group: GuidanceAction) {
  return guidanceGroups.find(item => item.value === group)?.label || 'Review first'
}

function guidanceSourceLabel(source: GuidanceSourceType) {
  return guidanceSources.find(item => item.value === source)?.label || 'Unknown'
}

function guidanceBadgeVariant(group: GuidanceAction) {
  if (group === 'keep_enabled') return 'success'
  if (group === 'set_manual') return 'info'
  if (group === 'disable_if_unused') return 'accent'
  return 'warning'
}

function guidanceSourceVariant(source: GuidanceSourceType) {
  if (source === 'microsoft' || source === 'vendor') return 'info'
  if (source === 'community') return 'accent'
  if (source === 'policy') return 'warning'
  return 'default'
}

function guidanceDesiredStartType(service: ServiceInfo): 'demand' | 'disabled' | null {
  if (service.guidance.action === 'set_manual') return 'demand'
  if (service.guidance.action === 'disable_if_unused') return 'disabled'
  return null
}

function guidanceCanBeSelected(guidance: ServiceInfo['guidance'] | ScheduledTask['guidance']) {
  return !guidance.protected
    && guidance.source_type !== 'unknown'
    && guidance.source_type !== 'policy'
    && (guidance.action === 'set_manual' || guidance.action === 'disable_if_unused')
}

function guidanceIsQuickSelectable(guidance: ServiceInfo['guidance'] | ScheduledTask['guidance']) {
  return guidanceCanBeSelected(guidance)
    && (guidance.source_type === 'microsoft' || guidance.source_type === 'vendor')
}

const servicesRows = computed(() =>
  services.value.map(s => ({
    ...s,
    _id: s.name,
    status: s.status,
    start_type: s.start_type,
    selectable: guidanceCanBeSelected(s.guidance),
  }))
)

const tasksRows = computed(() =>
  tasks.value.map(t => ({
    ...t,
    _id: t.name,
    selectable: guidanceCanBeSelected(t.guidance),
  }))
)

const filteredServicesRows = computed(() => {
  const search = serviceSearch.value.trim().toLowerCase()
  return servicesRows.value.filter(service => {
    const matchesSearch = !search || `${service.name} ${service.display_name} ${service.guidance.rationale}`.toLowerCase().includes(search)
    const matchesGroup = serviceGroupFilter.value === 'all' || service.guidance.group === serviceGroupFilter.value
    const matchesSource = serviceSourceFilter.value === 'all' || service.guidance.source_type === serviceSourceFilter.value
    return matchesSearch && matchesGroup && matchesSource
  })
})

const filteredTasksRows = computed(() => {
  const search = taskSearch.value.trim().toLowerCase()
  return tasksRows.value.filter(task => {
    const matchesSearch = !search || `${task.name} ${task.path} ${task.guidance.rationale}`.toLowerCase().includes(search)
    const matchesGroup = taskGroupFilter.value === 'all' || task.guidance.group === taskGroupFilter.value
    const matchesSource = taskSourceFilter.value === 'all' || task.guidance.source_type === taskSourceFilter.value
    return matchesSearch && matchesGroup && matchesSource
  })
})

function groupCounts(items: Array<ServiceInfo | ScheduledTask>) {
  const counts = {
    keep_enabled: 0,
    set_manual: 0,
    disable_if_unused: 0,
    review_first: 0,
  }
  for (const item of items) counts[item.guidance.group] += 1
  return counts
}

const serviceGroupCounts = computed(() => groupCounts(services.value))
const taskGroupCounts = computed(() => groupCounts(tasks.value))
const selectedServiceCount = computed(() => selectedServiceNames.value.size)
const selectedTaskCount = computed(() => selectedTaskNames.value.size)
const selectedServiceHasCommunity = computed(() => services.value.some(service =>
  selectedServiceNames.value.has(service.name) && service.guidance.source_type === 'community',
))
const selectedTaskHasCommunity = computed(() => tasks.value.some(task =>
  selectedTaskNames.value.has(task.name) && task.guidance.source_type === 'community',
))

const selectedServiceTargets = computed(() => services.value
  .filter(service => selectedServiceNames.value.has(service.name))
  .map(service => ({
    serviceName: service.name,
    desiredStartType: guidanceDesiredStartType(service),
  }))
  .filter((target): target is { serviceName: string; desiredStartType: 'demand' | 'disabled' } => target.desiredStartType !== null)
)

const selectedTaskTargets = computed(() => tasks.value
  .filter(task => selectedTaskNames.value.has(task.name))
  .map(task => ({ taskName: task.name, enabled: false }))
)

const loadedTabs = ref(new Set<string>())
const serviceActionLoading = ref<string | null>(null) // service name being acted upon
const taskActionLoading = ref<string | null>(null)
const bulkActionLoading = ref(false)
const showActionConfirm = ref(false)
const pendingAction = ref<{
  kind: 'program' | 'service-control' | 'service-start-type' | 'services-guidance' | 'scheduled-task' | 'scheduled-tasks-guidance'
  title: string
  description: string
  confirmLabel: string
  items: Array<{ label: string; value?: string; risk?: 'low' | 'medium' | 'high' | 'info' }>
  run: () => Promise<void>
} | null>(null)

async function loadPrograms(force = false) {
  if (!force && loadedTabs.value.has('programs')) return
  loading.value = true
  try {
    programs.value = await api.listStartupPrograms()
    loadedTabs.value.add('programs')
    feedback.value = null
  } catch (e) {
    feedback.value = failure('Could not load startup programs', e)
  } finally {
    loading.value = false
  }
}

async function loadServices(force = false) {
  if (!force && loadedTabs.value.has('services')) return
  loading.value = true
  try {
    services.value = await api.listServices()
    selectedServiceNames.value = new Set()
    loadedTabs.value.add('services')
    feedback.value = null
  } catch (e) {
    feedback.value = failure('Could not load services', e)
  } finally {
    loading.value = false
  }
}

async function loadTasks(force = false) {
  if (!force && loadedTabs.value.has('tasks')) return
  loading.value = true
  try {
    const scan = await scanScheduledTasks()
    tasks.value = scan.tasks
    selectedTaskNames.value = new Set()
    taskCollectedAt.value = scan.collected_at
    loadedTabs.value.add('tasks')
    feedback.value = scan.error
      ? failure('Could not collect scheduled tasks', scan.error)
      : null
  } catch (e) {
    feedback.value = failure('Could not load scheduled tasks', e)
  } finally {
    loading.value = false
  }
}

async function refresh() {
  if (activeTab.value === 'programs') await loadPrograms(true)
  if (activeTab.value === 'services') await loadServices(true)
  if (activeTab.value === 'tasks') await loadTasks(true)
}

function selectTab(tab: string) {
  activeTab.value = tab
  if (tab === 'programs') void loadPrograms()
  if (tab === 'services') void loadServices()
  if (tab === 'tasks') void loadTasks()
}

onActivated(() => {
  if (activeTab.value === 'programs') void loadPrograms()
  if (activeTab.value === 'services') void loadServices()
  if (activeTab.value === 'tasks') void loadTasks()
})

async function toggleProgram(prog: StartupProgram) {
  if (programsRows.value.find(row => row.id === prog.id)?.protected) return
  const enabling = !prog.enabled
  loading.value = true
  try {
    const plan = enabling
      ? await createStartupProgramEnablePlan(prog.id)
      : await createStartupProgramPlan(prog.id)
    pendingAction.value = {
      kind: 'program',
      title: `${enabling ? 'Enable' : 'Disable'} Startup Program?`,
      description: enabling
        ? 'Review the exact saved startup entry. WinOptimizer restores it only if the recovery record is still current, verifies re-enumeration, and completes the original recovery record.'
        : 'Review the exact confirmed startup entry. WinOptimizer rejects it if the entry changes before execution, verifies removal, and records recovery in History.',
      confirmLabel: enabling ? 'Enable Program' : 'Disable Program',
      items: [
        { label: prog.name, value: prog.location, risk: plan.descriptor.risk === 'critical' ? 'high' : plan.descriptor.risk },
        { label: 'Command', value: prog.command, risk: 'info' },
        { label: 'Administrator', value: plan.requires_elevation ? 'Required' : 'Not required', risk: plan.requires_elevation ? 'medium' : 'low' },
        { label: 'Recovery', value: enabling ? 'Original recovery record will be completed after verification' : 'Exact original entry is recorded before the change', risk: 'info' },
      ],
      run: () => applyProgramToggle(prog, plan),
    }
    showActionConfirm.value = true
  } catch (e) {
    feedback.value = failure(`Could not review ${prog.name}`, e)
    await loadPrograms(true)
  } finally {
    loading.value = false
  }
}

async function applyProgramToggle(prog: StartupProgram, plan: OperationPlan) {
  loading.value = true
  try {
    if (plan.requires_elevation && !(await api.checkElevated())) {
      await confirmAndRequestElevation(plan)
      feedback.value = success(
        'Administrator approval requested',
        'The exact confirmed entry will continue after elevation and reopen in History.',
      )
      return
    }
    const execution = prog.enabled
      ? await executeStartupProgramPlan(plan)
      : await executeStartupProgramEnablePlan(plan)
    await loadPrograms(true)
    feedback.value = success(
      prog.enabled ? 'Startup program disabled and verified' : 'Startup program enabled and verified',
      prog.enabled ? `${execution.message} You can restore it from History.` : execution.message,
    )
  } catch (e) {
    feedback.value = failure(`Could not update ${prog.name}`, e)
    await loadPrograms(true)
  } finally {
    loading.value = false
  }
}

function startService(svc: ServiceInfo) {
  void reviewService(svc, { kind: 'start' })
}

function stopService(svc: ServiceInfo) {
  void reviewService(svc, { kind: 'stop' })
}

async function reviewService(svc: ServiceInfo, action: ServiceAction) {
  serviceActionLoading.value = svc.name
  try {
    const plan = await createServicePlan(svc.name, action)
    const actionLabel = action.kind === 'start'
      ? 'Start'
      : action.kind === 'stop'
        ? 'Stop'
        : `Set startup to ${action.start_type}`
    pendingAction.value = {
      kind: action.kind === 'set_start_type' ? 'service-start-type' : 'service-control',
      title: `${actionLabel} Service?`,
      description: 'This exact service operation remains Experimental. WinOptimizer rejects protected or changed services and verifies Service Control Manager state after execution.',
      confirmLabel: actionLabel,
      items: [
        { label: svc.display_name || svc.name, value: svc.name, risk: 'high' },
        { label: 'Current status', value: svc.status, risk: 'info' },
        { label: 'Current start type', value: svc.start_type, risk: 'info' },
        { label: 'Administrator', value: 'Required', risk: 'medium' },
        { label: 'Safety status', value: 'Experimental; exact undo unavailable', risk: 'high' },
      ],
      run: () => applyServicePlan(svc, plan),
    }
    showActionConfirm.value = true
  } catch (e) {
    feedback.value = failure(`Could not review ${svc.display_name || svc.name}`, e)
    await loadServices(true)
  } finally {
    serviceActionLoading.value = null
  }
}

async function applyServicePlan(svc: ServiceInfo, plan: OperationPlan) {
  serviceActionLoading.value = svc.name
  try {
    if (!(await api.checkElevated())) {
      await confirmAndRequestElevation(plan)
      feedback.value = success(
        'Administrator approval requested',
        'The exact confirmed service operation will continue after elevation and reopen in History.',
      )
      return
    }
    const execution = await executeServicePlan(plan)
    await loadServices(true)
    feedback.value = success('Service updated and verified', execution.message)
  } catch (e) {
    feedback.value = failure(`Could not update ${svc.display_name || svc.name}`, e)
    await loadServices(true)
  } finally {
    serviceActionLoading.value = null
  }
}

function changeServiceStartType(svc: ServiceInfo, startType: string) {
  if (startType !== 'auto' && startType !== 'demand' && startType !== 'disabled') return
  void reviewService(svc, { kind: 'set_start_type', start_type: startType })
}

function toggleServiceSelection(name: string, checked: boolean) {
  const next = new Set(selectedServiceNames.value)
  if (checked) next.add(name)
  else next.delete(name)
  selectedServiceNames.value = next
}

function toggleTaskSelection(name: string, checked: boolean) {
  const next = new Set(selectedTaskNames.value)
  if (checked) next.add(name)
  else next.delete(name)
  selectedTaskNames.value = next
}

function selectServiceRecommendations() {
  selectedServiceNames.value = new Set(
    services.value
      .filter(service => guidanceIsQuickSelectable(service.guidance))
      .map(service => service.name),
  )
}

function selectTaskRecommendations() {
  selectedTaskNames.value = new Set(
    tasks.value
      .filter(task => guidanceIsQuickSelectable(task.guidance))
      .map(task => task.name),
  )
}

async function reviewSelectedServiceChanges() {
  if (selectedServiceTargets.value.length === 0) return
  bulkActionLoading.value = true
  try {
    const plan = await createServicesGuidancePlan(selectedServiceTargets.value)
    const communityWarning = selectedServiceHasCommunity.value
      ? ' Community-sourced items are explicitly selected; verify their purpose and dependencies.'
      : ''
    pendingAction.value = {
      kind: 'services-guidance',
      title: 'Review selected service changes',
      description: `Only the ${selectedServiceTargets.value.length} selected service target${selectedServiceTargets.value.length === 1 ? '' : 's'} will be previewed.${communityWarning}`,
      confirmLabel: 'Review Service Changes',
      items: [
        { label: 'Selected services', value: String(plan.targets.length), risk: 'high' },
        { label: 'Guidance scope', value: 'Windows 10/11 desktop; conditional recommendations', risk: 'info' },
        { label: 'Administrator', value: 'Required', risk: 'medium' },
        { label: 'Verification', value: 'Each selected service is re-read from Service Control Manager', risk: 'info' },
        { label: 'Recovery', value: 'Exact dependent-service restoration is not available yet', risk: 'high' },
      ],
      run: () => applyServicesGuidancePlan(plan),
    }
    showActionConfirm.value = true
  } catch (e) {
    feedback.value = failure('Could not prepare the selected service changes', e)
  } finally {
    bulkActionLoading.value = false
  }
}

async function applyServicesGuidancePlan(plan: OperationPlan) {
  bulkActionLoading.value = true
  try {
    if (!(await api.checkElevated())) {
      await confirmAndRequestElevation(plan)
      feedback.value = success(
        'Administrator approval requested',
        'The reviewed selected service operation will continue after elevation and reopen in History.',
      )
      return
    }
    const execution = await executeServicesGuidancePlan(plan)
    await loadServices(true)
    feedback.value = success('Selected services updated and verified', execution.message)
  } catch (e) {
    feedback.value = failure('Could not apply selected service changes', e)
    await loadServices(true)
  } finally {
    bulkActionLoading.value = false
  }
}

function taskIsEnabled(task: ScheduledTask) {
  return !task.status.toLowerCase().includes('disabled')
}

function taskGuidanceLabel(task: ScheduledTask) {
  return guidanceGroupLabel(task.guidance.group)
}

function taskGuidanceVariant(task: ScheduledTask) {
  return guidanceBadgeVariant(task.guidance.group)
}

async function toggleTask(task: ScheduledTask) {
  if (!task.can_modify) return
  const enabled = !taskIsEnabled(task)
  taskActionLoading.value = task.name
  try {
    const plan = await createScheduledTaskPlan(task.name, enabled)
    pendingAction.value = {
      kind: 'scheduled-task',
      title: (enabled ? 'Enable' : 'Disable') + ' scheduled task?',
      description: task.recommendation,
      confirmLabel: enabled ? 'Enable Task' : 'Disable Task',
      items: [
        { label: 'Task', value: task.name, risk: plan.descriptor.risk === 'high' ? 'high' : 'medium' },
        { label: 'Current state', value: task.status, risk: 'info' },
        { label: 'New state', value: enabled ? 'Enabled' : 'Disabled', risk: enabled ? 'low' : 'medium' },
        { label: 'Verification', value: 'Task Scheduler state is re-queried after the change', risk: 'info' },
      ],
      run: () => applyScheduledTaskPlan(task, plan),
    }
    showActionConfirm.value = true
  } catch (e) {
    feedback.value = failure('Could not review ' + task.name, e)
    await loadTasks(true)
  } finally {
    taskActionLoading.value = null
  }
}

async function applyScheduledTaskPlan(task: ScheduledTask, plan: OperationPlan) {
  taskActionLoading.value = task.name
  try {
    if (!(await api.checkElevated())) {
      await confirmAndRequestElevation(plan)
      feedback.value = success(
        'Administrator approval requested',
        'The reviewed scheduled-task change will continue after elevation and reopen in History.',
      )
      return
    }
    const execution = await executeScheduledTaskPlan(plan)
    await loadTasks(true)
    feedback.value = success('Scheduled task updated and verified', execution.message)
  } catch (e) {
    feedback.value = failure('Could not update ' + task.name, e)
    await loadTasks(true)
  } finally {
    taskActionLoading.value = null
  }
}

async function reviewSelectedTaskChanges() {
  if (selectedTaskTargets.value.length === 0) return
  bulkActionLoading.value = true
  try {
    const plan = await createScheduledTasksGuidancePlan(selectedTaskTargets.value)
    const communityWarning = selectedTaskHasCommunity.value
      ? ' Community-sourced items are explicitly selected; verify their purpose and dependencies.'
      : ''
    pendingAction.value = {
      kind: 'scheduled-tasks-guidance',
      title: 'Review selected scheduled-task changes',
      description: `Only the ${selectedTaskTargets.value.length} selected task${selectedTaskTargets.value.length === 1 ? '' : 's'} will be previewed.${communityWarning}`,
      confirmLabel: 'Review Task Changes',
      items: [
        { label: 'Selected tasks', value: String(plan.targets.length), risk: 'high' },
        { label: 'Guidance scope', value: 'Windows 10/11 desktop; conditional recommendations', risk: 'info' },
        { label: 'Administrator', value: 'Required', risk: 'medium' },
        { label: 'Verification', value: 'Each selected task is re-read from Task Scheduler', risk: 'info' },
        { label: 'Recovery', value: 'Full scheduled-task XML restoration is not available yet', risk: 'high' },
      ],
      run: () => applyScheduledTasksGuidancePlan(plan),
    }
    showActionConfirm.value = true
  } catch (e) {
    feedback.value = failure('Could not prepare the selected scheduled-task changes', e)
  } finally {
    bulkActionLoading.value = false
  }
}

async function applyScheduledTasksGuidancePlan(plan: OperationPlan) {
  bulkActionLoading.value = true
  try {
    if (!(await api.checkElevated())) {
      await confirmAndRequestElevation(plan)
      feedback.value = success(
        'Administrator approval requested',
        'The reviewed selected scheduled-task operation will continue after elevation and reopen in History.',
      )
      return
    }
    const execution = await executeScheduledTasksGuidancePlan(plan)
    await loadTasks(true)
    feedback.value = success('Selected scheduled tasks updated and verified', execution.message)
  } catch (e) {
    feedback.value = failure('Could not apply selected scheduled-task changes', e)
    await loadTasks(true)
  } finally {
    bulkActionLoading.value = false
  }
}

async function onConfirmAction() {
  const action = pendingAction.value
  if (!action) return
  showActionConfirm.value = false
  await action.run()
  pendingAction.value = null
}

function onCancelAction() {
  showActionConfirm.value = false
  pendingAction.value = null
}
</script>

<template>
  <div class="startup-manager">
    <AppHeader />

    <ResultsBanner
      v-if="feedback"
      :variant="feedback.type"
      :title="feedback.title"
      :description="feedback.message"
      class="startup-feedback"
      @dismiss="feedback = null"
    />

    <BaseTabBar
      :tabs="tabs"
      :model-value="activeTab"
      variant="glass"
      class="startup-tabs"
      @update:model-value="selectTab"
    />

    <!-- Programs -->
    <div v-if="activeTab === 'programs'" class="tab-content">
      <div class="toolbar">
        <BaseButton variant="primary" icon="refresh" @click="refresh" :loading="loading" size="sm">
          Refresh
        </BaseButton>
        <BaseBadge variant="accent" size="sm">{{ programs.length }} programs</BaseBadge>
      </div>

      <div v-if="detectedGuidance.length > 0" class="startup-guide">
        <div class="guide-heading">
          <div>
            <strong>Startup guidance</strong>
            <span>Recommendations matched to enabled startup entries detected on this PC.</span>
          </div>
          <BaseBadge variant="info" size="sm">Delay before disabling</BaseBadge>
        </div>
        <div class="guide-grid">
          <div v-for="entry in detectedGuidance" :key="entry.software" class="guide-item">
            <div class="guide-item-heading">
              <strong>{{ entry.software }}</strong>
              <BaseBadge
                :variant="entry.impact === 'high' ? 'danger' : entry.impact === 'protected' ? 'warning' : entry.impact === 'medium' ? 'accent' : 'default'"
                size="sm"
              >
                {{ entry.impact === 'protected' ? 'Keep' : entry.impact }} impact
              </BaseBadge>
            </div>
            <span class="guide-benefit">Delayed start: {{ entry.delayedBenefit }} · {{ entry.improvement }}</span>
            <span class="guide-recommendation">{{ entry.recommendation }}</span>
          </div>
        </div>
      </div>

      <BaseTable
        v-if="programs.length > 0 || loading"
        :columns="programsCols"
        :rows="programsRows"
        :loading="loading"
        max-height="none"
        compact
      >
        <template #cell-name="{ row }">
          <div class="program-name-cell">
            <span class="prog-name">{{ row.name }}</span>
            <BaseBadge
              v-if="row.guidance"
              :variant="row.guidance.impact === 'high' ? 'danger' : row.guidance.impact === 'protected' ? 'warning' : 'info'"
              size="sm"
            >
              {{ row.guidance.impact === 'protected' ? 'Keep' : row.guidance.impact }}
            </BaseBadge>
          </div>
        </template>
        <template #cell-command="{ row }">
          <span class="prog-cmd" :title="row.command">{{ row.command.substring(0, 60) }}{{ row.command.length > 60 ? '...' : '' }}</span>
        </template>
        <template #cell-location="{ row }">
          <BaseBadge :variant="row.location === 'HKLM' || row.location === 'HKLM Run' ? 'info' : 'default'" size="sm">
            {{ row.location }}
          </BaseBadge>
        </template>
        <template #cell-enabled="{ row }">
          <div class="toggle-cell">
            <BaseToggle
              :modelValue="row.enabled"
              @update:modelValue="toggleProgram(row)"
              :disabled="row.protected"
              size="sm"
            />
            <BaseBadge :variant="row.protected ? 'warning' : row.enabled ? 'success' : 'default'" size="sm" dot>
              {{ row.protected ? 'Protected' : row.enabled ? 'Enabled' : 'Disabled' }}
            </BaseBadge>
          </div>
        </template>
      </BaseTable>
      <EmptyState
        v-else
        icon="zap"
        title="No Startup Programs Loaded"
        description="Click Refresh to load startup programs."
      />
    </div>

    <!-- Services -->
    <div v-if="activeTab === 'services'" class="tab-content">
      <div class="toolbar">
        <BaseButton variant="primary" icon="refresh" @click="refresh" :loading="loading" size="sm">
          Refresh
        </BaseButton>
        <BaseBadge variant="accent" size="sm">{{ services.length }} services</BaseBadge>
        <BaseBadge variant="warning" size="sm">Mutations Experimental</BaseBadge>
      </div>

      <div class="guidance-controls">
        <BaseSearchInput v-model="serviceSearch" placeholder="Search services or guidance..." size="sm" />
        <select v-model="serviceGroupFilter" class="guidance-filter" aria-label="Filter services by guidance group">
          <option value="all">All action groups</option>
          <option v-for="group in guidanceGroups" :key="group.value" :value="group.value">
            {{ group.label }}
          </option>
        </select>
        <select v-model="serviceSourceFilter" class="guidance-filter" aria-label="Filter services by source">
          <option value="all">All sources</option>
          <option v-for="source in guidanceSources" :key="source.value" :value="source.value">
            {{ source.label }}
          </option>
        </select>
        <BaseButton
          variant="secondary"
          size="sm"
          :disabled="loading || serviceActionLoading !== null"
          @click="selectServiceRecommendations"
        >
          Select recommendations
        </BaseButton>
        <BaseButton
          variant="warning"
          icon="settings"
          size="sm"
          :loading="bulkActionLoading"
          :disabled="loading || serviceActionLoading !== null || selectedServiceCount === 0"
          @click="reviewSelectedServiceChanges"
        >
          Review selected changes ({{ selectedServiceCount }})
        </BaseButton>
      </div>
      <div class="guidance-summary" aria-label="Service guidance group counts">
        <BaseBadge v-for="group in guidanceGroups" :key="group.value" :variant="guidanceBadgeVariant(group.value)" size="sm">
          {{ group.label }}: {{ serviceGroupCounts[group.value] }}
        </BaseBadge>
      </div>
      <p v-if="selectedServiceHasCommunity" class="community-warning">
        Community recommendations are selected explicitly. Review their source and local dependencies before applying them.
      </p>

      <BaseTable
        v-if="filteredServicesRows.length > 0 || loading"
        :columns="servicesCols"
        :rows="filteredServicesRows"
        :loading="loading"
        max-height="none"
        compact
        class="services-table"
      >
        <template #cell-select="{ row }">
          <input
            v-if="row.selectable"
            class="guidance-checkbox"
            type="checkbox"
            :checked="selectedServiceNames.has(row.name)"
            :disabled="serviceActionLoading !== null || bulkActionLoading"
            :title="row.guidance.source_type === 'community' ? 'Explicitly select this community recommendation' : 'Select recommendation'"
            @change="toggleServiceSelection(row.name, ($event.target as HTMLInputElement).checked)"
          />
        </template>
        <template #cell-name="{ row }">
          <span class="svc-name" :title="row.name">{{ row.name }}</span>
        </template>
        <template #cell-display_name="{ row }">
          <span class="svc-display-name" :title="row.display_name">{{ row.display_name || '-' }}</span>
        </template>
        <template #cell-status="{ row }">
          <BaseBadge :variant="row.status === 'Running' ? 'success' : 'default'" size="sm" :dot="row.status === 'Running'">
            {{ row.status }}
          </BaseBadge>
        </template>
        <template #cell-start_type="{ row }">
          <select
            class="start-type-select"
            :value="row.start_type"
            :disabled="row.guidance.protected || serviceActionLoading === row._id || bulkActionLoading"
            @change="changeServiceStartType(row, ($event.target as HTMLSelectElement).value)"
          >
            <option value="auto">Automatic</option>
            <option value="demand">Manual</option>
            <option value="disabled">Disabled</option>
          </select>
        </template>
        <template #cell-guidance="{ row }">
          <div class="guidance-cell">
            <BaseBadge :variant="guidanceBadgeVariant(row.guidance.group)" size="sm">
              {{ guidanceGroupLabel(row.guidance.group) }}
            </BaseBadge>
            <span :title="row.guidance.source_title">{{ guidanceSourceLabel(row.guidance.source_type) }}</span>
          </div>
        </template>
        <template #cell-actions="{ row }">
          <div class="svc-actions">
            <BaseButton
              v-if="row.status === 'Stopped'"
              variant="success"
              size="sm"
              icon="play"
              :loading="serviceActionLoading === row._id"
              :disabled="row.guidance.protected || serviceActionLoading !== null || bulkActionLoading"
              @click="startService(row)"
            >
              Start
            </BaseButton>
            <BaseButton
              v-if="row.status === 'Running'"
              variant="danger"
              size="sm"
              icon="x"
              :loading="serviceActionLoading === row._id"
              :disabled="row.guidance.protected || serviceActionLoading !== null || bulkActionLoading"
              @click="stopService(row)"
            >
              Stop
            </BaseButton>
          </div>
        </template>
      </BaseTable>
      <EmptyState
        v-else-if="services.length > 0"
        icon="search"
        title="No Matching Services"
        description="Adjust the search or guidance filters to see more services."
      />
      <EmptyState
        v-else
        icon="cpu"
        title="No Services Loaded"
        description="Click Refresh to load services."
      />
    </div>

    <!-- Tasks -->
    <div v-if="activeTab === 'tasks'" class="tab-content">
      <div class="toolbar">
        <BaseButton variant="primary" icon="refresh" @click="refresh" :loading="loading" size="sm">
          Refresh
        </BaseButton>
        <BaseBadge variant="accent" size="sm">{{ tasks.length }} tasks</BaseBadge>
        <BaseBadge variant="info" size="sm">Protected Windows tasks locked</BaseBadge>
        <BaseBadge v-if="taskCollectedAt" variant="default" size="sm">
          Updated {{ new Date(taskCollectedAt).toLocaleTimeString() }}
        </BaseBadge>
      </div>

      <div class="guidance-controls">
        <BaseSearchInput v-model="taskSearch" placeholder="Search tasks or guidance..." size="sm" />
        <select v-model="taskGroupFilter" class="guidance-filter" aria-label="Filter tasks by guidance group">
          <option value="all">All action groups</option>
          <option v-for="group in guidanceGroups" :key="group.value" :value="group.value">
            {{ group.label }}
          </option>
        </select>
        <select v-model="taskSourceFilter" class="guidance-filter" aria-label="Filter tasks by source">
          <option value="all">All sources</option>
          <option v-for="source in guidanceSources" :key="source.value" :value="source.value">
            {{ source.label }}
          </option>
        </select>
        <BaseButton
          variant="secondary"
          size="sm"
          :disabled="loading || taskActionLoading !== null"
          @click="selectTaskRecommendations"
        >
          Select recommendations
        </BaseButton>
        <BaseButton
          variant="warning"
          icon="settings"
          size="sm"
          :loading="bulkActionLoading"
          :disabled="loading || taskActionLoading !== null || selectedTaskCount === 0"
          @click="reviewSelectedTaskChanges"
        >
          Review selected changes ({{ selectedTaskCount }})
        </BaseButton>
      </div>
      <div class="guidance-summary" aria-label="Scheduled task guidance group counts">
        <BaseBadge v-for="group in guidanceGroups" :key="group.value" :variant="guidanceBadgeVariant(group.value)" size="sm">
          {{ group.label }}: {{ taskGroupCounts[group.value] }}
        </BaseBadge>
      </div>
      <p v-if="selectedTaskHasCommunity" class="community-warning">
        Community recommendations are selected explicitly. Review their source and local dependencies before applying them.
      </p>

      <BaseTable
        v-if="filteredTasksRows.length > 0 || loading"
        :columns="tasksCols"
        :rows="filteredTasksRows"
        :loading="loading"
        max-height="none"
        compact
        class="tasks-table"
      >
        <template #cell-select="{ row }">
          <input
            v-if="row.selectable"
            class="guidance-checkbox"
            type="checkbox"
            :checked="selectedTaskNames.has(row.name)"
            :disabled="taskActionLoading !== null || bulkActionLoading"
            :title="row.guidance.source_type === 'community' ? 'Explicitly select this community recommendation' : 'Select recommendation'"
            @change="toggleTaskSelection(row.name, ($event.target as HTMLInputElement).checked)"
          />
        </template>
        <template #cell-name="{ row }">
          <span class="task-name" :title="row.name">{{ row.name }}</span>
        </template>
        <template #cell-trigger="{ row }">
          <span class="task-trigger">{{ row.trigger || 'Task Scheduler trigger' }}</span>
        </template>
        <template #cell-status="{ row }">
          <BaseBadge :variant="row.status === 'Ready' ? 'success' : row.status === 'Running' ? 'info' : 'default'" size="sm" :dot="row.status === 'Running'">
            {{ row.status }}
          </BaseBadge>
        </template>
        <template #cell-classification="{ row }">
          <div class="task-guidance">
            <BaseBadge :variant="taskGuidanceVariant(row)" size="sm">
              {{ taskGuidanceLabel(row) }}
            </BaseBadge>
            <BaseBadge :variant="guidanceSourceVariant(row.guidance.source_type)" size="sm">
              {{ guidanceSourceLabel(row.guidance.source_type) }}
            </BaseBadge>
            <span :title="row.guidance.source_title">{{ row.guidance.rationale }}</span>
          </div>
        </template>
        <template #cell-actions="{ row }">
          <BaseButton
            v-if="row.can_modify"
            :variant="row.guidance.action === 'disable_if_unused' && taskIsEnabled(row) ? 'warning' : 'secondary'"
            size="sm"
            :icon="taskIsEnabled(row) ? 'x' : 'check'"
            :loading="taskActionLoading === row._id"
            :disabled="taskActionLoading !== null || bulkActionLoading"
            @click="toggleTask(row)"
          >
            {{ taskIsEnabled(row) ? 'Disable' : 'Enable' }}
          </BaseButton>
          <BaseBadge v-else variant="warning" size="sm">Protected</BaseBadge>
        </template>
      </BaseTable>
      <EmptyState
        v-else-if="tasks.length > 0"
        icon="search"
        title="No Matching Scheduled Tasks"
        description="Adjust the search or guidance filters to see more tasks."
      />
      <EmptyState
        v-else
        icon="clock"
        title="No Scheduled Tasks"
        description="No tasks were returned. Collection failures are shown above rather than treated as an empty successful result."
      />
    </div>

    <ConfirmDialog
      :open="showActionConfirm"
      :operation-plan="pendingAction?.plan"
      :title="pendingAction?.title || ''"
      :description="pendingAction?.description || ''"
      :items="pendingAction?.items || []"
      :confirm-label="pendingAction?.confirmLabel || 'Confirm'"
      cancel-label="Cancel"
      :variant="pendingAction?.items.some(item => item.risk === 'high') ? 'danger' : 'warning'"
      icon="shield"
      :loading="loading || serviceActionLoading !== null || taskActionLoading !== null || bulkActionLoading"
      @confirm="onConfirmAction"
      @cancel="onCancelAction"
    />
  </div>
</template>

<style scoped>
.startup-manager {
  width: 100%;
  min-width: 0;
}

.startup-feedback {
  margin-bottom: var(--space-5);
}

.startup-tabs {
  margin-bottom: var(--space-4);
}

.tab-content {
  animation: fadeInUp var(--duration-normal) var(--ease-out);
}

.toolbar {
  display: flex;
  align-items: center;
  gap: var(--space-3);
  margin-bottom: var(--space-3);
  flex-wrap: wrap;
}

.guidance-controls {
  display: flex;
  align-items: center;
  gap: var(--space-2);
  margin-bottom: var(--space-2);
  flex-wrap: wrap;
}

.guidance-controls :deep(.search-input) {
  min-width: 240px;
  flex: 1 1 260px;
}

.guidance-filter {
  min-height: 34px;
  max-width: 190px;
  padding: var(--space-1) var(--space-2);
  color: var(--text-primary);
  background: var(--bg-surface);
  border: 1px solid var(--border-default);
  border-radius: var(--radius-md);
  font-size: var(--text-xs);
}

.guidance-summary {
  display: flex;
  gap: var(--space-2);
  margin-bottom: var(--space-3);
  flex-wrap: wrap;
}

.community-warning {
  margin: 0 0 var(--space-3);
  color: var(--text-warning, var(--text-secondary));
  font-size: var(--text-xs);
}

.guidance-checkbox {
  width: 15px;
  height: 15px;
  accent-color: var(--accent);
  cursor: pointer;
}

.guidance-checkbox:disabled {
  cursor: not-allowed;
  opacity: 0.45;
}

.guidance-cell {
  display: grid;
  min-width: 0;
  gap: var(--space-1);
}

.guidance-cell > span {
  overflow: hidden;
  color: var(--text-muted);
  font-size: var(--text-xs);
  text-overflow: ellipsis;
  white-space: nowrap;
}

.startup-guide {
  margin-bottom: var(--space-4);
  padding: var(--space-4);
  border: 1px solid var(--border-default);
  border-radius: var(--radius-xl);
  background: var(--bg-elevated);
}

.guide-heading,
.guide-item-heading {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: var(--space-3);
}

.guide-heading strong {
  display: block;
  color: var(--text-primary);
}

.guide-heading span,
.guide-benefit,
.guide-recommendation {
  display: block;
  color: var(--text-muted);
  font-size: var(--text-xs);
}

.guide-grid {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: var(--space-3);
  margin-top: var(--space-3);
}

.guide-item {
  min-width: 0;
  padding: var(--space-3);
  border: 1px solid var(--border-subtle);
  border-radius: var(--radius-lg);
}

.guide-item-heading strong {
  min-width: 0;
  color: var(--text-primary);
  font-size: var(--text-sm);
}

.guide-benefit {
  margin-top: var(--space-2);
  line-height: 1.45;
}

.guide-recommendation {
  margin-top: var(--space-1);
  color: var(--text-secondary);
  line-height: 1.45;
}

.toggle-cell {
  display: flex;
  align-items: center;
  gap: var(--space-2);
  justify-content: center;
}

.prog-name {
  font-weight: var(--weight-medium);
  color: var(--text-primary);
  font-size: var(--text-sm);
}

.program-name-cell {
  display: flex;
  align-items: center;
  gap: var(--space-2);
  min-width: 0;
}

.prog-cmd {
  font-family: var(--font-mono);
  font-size: var(--text-xs);
  color: var(--text-muted);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  display: block;
  max-width: 350px;
}

.svc-name {
  display: block;
  overflow-wrap: anywhere;
  word-break: break-word;
  font-weight: var(--weight-medium);
  color: var(--text-primary);
  font-size: var(--text-sm);
}

.svc-display-name {
  display: block;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  color: var(--text-secondary);
  font-size: var(--text-sm);
}

.services-table :deep(.table-wrapper),
.tasks-table :deep(.table-wrapper) {
  overflow-x: hidden;
}

.services-table :deep(.base-table),
.tasks-table :deep(.base-table) {
  width: 100%;
  table-layout: fixed;
}

.services-table :deep(th),
.services-table :deep(td),
.tasks-table :deep(th),
.tasks-table :deep(td) {
  min-width: 0;
}

.start-type-select {
  width: 100%;
  min-width: 0;
  font-size: var(--text-xs);
  color: var(--text-primary);
  background: var(--bg-surface);
  border: 1px solid var(--border-default);
  border-radius: var(--radius-md);
  padding: var(--space-1) var(--space-2);
  cursor: pointer;
  outline: none;
  transition: border-color var(--duration-fast);
  max-width: 110px;
}

.start-type-select:focus {
  border-color: var(--accent);
}

.start-type-select:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}

.svc-actions {
  display: flex;
  flex-wrap: wrap;
  gap: var(--space-1);
}

.task-name {
  display: block;
  overflow-wrap: anywhere;
  word-break: break-word;
  font-weight: var(--weight-medium);
  color: var(--text-primary);
  font-size: var(--text-sm);
}

.task-trigger {
  display: block;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-size: var(--text-sm);
  color: var(--text-secondary);
}

.task-guidance {
  display: grid;
  min-width: 0;
  gap: var(--space-1);
}

.task-guidance span {
  overflow: hidden;
  color: var(--text-muted);
  font-size: var(--text-xs);
  line-height: 1.35;
  text-overflow: ellipsis;
  white-space: nowrap;
}

@media (max-width: 900px) {
  .services-table :deep(th:nth-child(6)),
  .services-table :deep(td:nth-child(6)) {
    display: none;
  }
}

@media (max-width: 700px) {
  .guide-grid {
    grid-template-columns: 1fr;
  }

  .toolbar {
    flex-direction: column;
    align-items: stretch;
  }

  .guidance-controls {
    align-items: stretch;
    flex-direction: column;
  }

  .guidance-controls :deep(.search-input),
  .guidance-filter {
    max-width: none;
    width: 100%;
  }

  .toggle-cell,
  .svc-actions {
    justify-content: flex-start;
  }

  .services-table :deep(th),
  .services-table :deep(td),
  .tasks-table :deep(th),
  .tasks-table :deep(td) {
    padding-inline: var(--space-2);
  }
}
</style>
