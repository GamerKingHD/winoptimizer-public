<script setup lang="ts">
import { computed, onActivated, ref, watch } from 'vue'
import type { ContextMenuEntry } from '../api/tauri'
import * as api from '../api/tauri'
import type { OperationPlan } from '../api/operation'
import { confirmAndRequestElevation } from '../api/operation'
import type { SystemTweakTarget, TweakKind, TweakPreset } from '../api/systemTweaks'
import {
  createTweakPresetPlan,
  createSystemTweakPlan,
  executeTweakPresetPlan,
  executeSystemTweakPlan,
  listSystemTweakTargets,
  listTweakPresets,
} from '../api/systemTweaks'
import AppHeader from '../components/AppHeader.vue'
import BaseCard from '../components/BaseCard.vue'
import BaseButton from '../components/BaseButton.vue'
import BaseTable from '../components/BaseTable.vue'
import BaseTabBar from '../components/BaseTabBar.vue'
import BaseBadge from '../components/BaseBadge.vue'
import BaseToggle from '../components/BaseToggle.vue'
import EmptyState from '../components/EmptyState.vue'
import ConfirmDialog from '../components/ConfirmDialog.vue'
import ExperimentalBanner from '../components/ExperimentalBanner.vue'
import LabsBanner from '../components/LabsBanner.vue'
import OperationPreview from '../components/OperationPreview.vue'
import ResultsBanner from '../components/ResultsBanner.vue'
import { failure, success, type ActionFeedback } from '../utils/feedback'

const activeTab = ref<string>('privacy')

const tabs = [
  { id: 'presets', label: 'Presets', icon: 'sliders' },
  { id: 'privacy', label: 'Privacy', icon: 'shield' },
  { id: 'performance', label: 'Performance', icon: 'zap' },
  { id: 'customization', label: 'Customization', icon: 'sliders' },
  { id: 'context', label: 'Context Menu', icon: 'layers' },
]

const privacyTweaks = ref<SystemTweakTarget[]>([])
const perfTweaks = ref<SystemTweakTarget[]>([])
const customizationTweaks = ref<SystemTweakTarget[]>([])
const contextEntries = ref<ContextMenuEntry[]>([])
const presets = ref<TweakPreset[]>([])
const presetSelections = ref<Record<string, string[]>>({})
const loading = ref(false)
const feedback = ref<ActionFeedback | null>(null)
const loadedTabs = ref(new Set<string>())

// Confirmation dialog state
const showTweakConfirm = ref(false)
const pendingTweakAction = ref<'apply' | 'revert'>('apply')
const pendingTweakName = ref('')
const pendingTweakDesc = ref('')
const pendingTweakRisk = ref('low')
const pendingTweakPlan = ref<OperationPlan | null>(null)
const pendingTweakItems = computed(() => {
  const plan = pendingTweakPlan.value
  if (!plan) return []
  return [
    {
      label: 'Action',
      value: pendingTweakAction.value === 'apply' ? 'Enable' : 'Disable',
      risk: pendingTweakRisk.value === 'critical' ? 'high' : pendingTweakRisk.value,
    },
    {
      label: 'Administrator',
      value: plan.requires_elevation ? 'Required' : 'Not required',
      risk: plan.requires_elevation ? 'medium' : 'low',
    },
    {
      label: 'Restart',
      value: plan.requires_restart ? 'Required' : 'Not required',
      risk: plan.requires_restart ? 'medium' : 'info',
    },
    {
      label: 'Recovery',
      value: recoveryLabel(plan),
    },
  ]
})
const showPresetConfirm = ref(false)
const pendingPresetName = ref('')
const pendingPresetPlan = ref<OperationPlan | null>(null)
const pendingPresetItems = computed(() => {
  const plan = pendingPresetPlan.value
  if (!plan) return []
  return plan.targets.map(target => ({
    label: target.description,
    value: target.target_id,
    risk: plan.descriptor.risk === 'critical' ? 'high' : plan.descriptor.risk,
  }))
})

const riskVariant = (risk: string) => {
  switch (risk.toLowerCase()) {
    case 'low': return 'success'
    case 'medium': return 'warning'
    case 'high': return 'danger'
    default: return 'default'
  }
}

function recoveryLabel(plan: OperationPlan): string {
  switch (plan.descriptor.recovery) {
    case 'exact': return 'Exact recovery'
    case 'best_effort': return 'Best-effort recovery'
    case 'none': return 'No automatic recovery'
  }
  switch (plan.reversibility) {
    case 'fully_reversible': return 'Fully reversible'
    case 'best_effort': return 'Best-effort recovery'
    case 'not_reversible': return 'Not reversible'
    default: return 'Recovery not declared'
  }
}

const contextCols = [
  { key: 'name', label: 'Entry Name', sortable: true },
  { key: 'command', label: 'Command', sortable: true },
  { key: 'location', label: 'Location', sortable: true, width: '120px' },
]

const contextRows = computed(() =>
  contextEntries.value.map(e => ({ ...e, _id: e.name + e.location }))
)

async function loadPrivacyTweaks(force = false) {
  if (!force && loadedTabs.value.has('privacy')) return
  loading.value = true
  try {
    privacyTweaks.value = (await listSystemTweakTargets()).filter(
      tweak => tweak.kind === 'privacy',
    )
    loadedTabs.value.add('privacy')
    feedback.value = null
  } catch (e) {
    feedback.value = failure('Could not load privacy tweaks', e)
    privacyTweaks.value = []
    loadedTabs.value.delete('privacy')
  } finally {
    loading.value = false
  }
}

async function loadPerformanceTweaks(force = false) {
  if (!force && loadedTabs.value.has('performance')) return
  loading.value = true
  try {
    perfTweaks.value = (await listSystemTweakTargets()).filter(
      tweak => tweak.kind === 'performance',
    )
    loadedTabs.value.add('performance')
    feedback.value = null
  } catch (e) {
    feedback.value = failure('Could not load performance tweaks', e)
    perfTweaks.value = []
    loadedTabs.value.delete('performance')
  } finally {
    loading.value = false
  }
}

async function loadCustomizationTweaks(force = false) {
  if (!force && loadedTabs.value.has('customization')) return
  loading.value = true
  try {
    customizationTweaks.value = (await listSystemTweakTargets()).filter(
      tweak => tweak.kind === 'customization',
    )
    loadedTabs.value.add('customization')
    feedback.value = null
  } catch (e) {
    feedback.value = failure('Could not load customization tweaks', e)
    customizationTweaks.value = []
    loadedTabs.value.delete('customization')
  } finally {
    loading.value = false
  }
}

async function loadContextEntries(force = false) {
  if (!force && loadedTabs.value.has('context')) return
  loading.value = true
  try {
    contextEntries.value = await api.listContextMenuEntries()
    loadedTabs.value.add('context')
    feedback.value = null
  } catch (e) {
    feedback.value = failure('Could not load context menu entries', e)
    contextEntries.value = []
    loadedTabs.value.delete('context')
  } finally {
    loading.value = false
  }
}

async function loadPresets(force = false) {
  if (!force && loadedTabs.value.has('presets')) return
  loading.value = true
  try {
    presets.value = await listTweakPresets()
    presetSelections.value = Object.fromEntries(
      presets.value.map(preset => [preset.id, [...preset.target_ids]]),
    )
    loadedTabs.value.add('presets')
    feedback.value = null
  } catch (e) {
    feedback.value = failure('Could not load presets', e)
    presets.value = []
    loadedTabs.value.delete('presets')
  } finally {
    loading.value = false
  }
}

async function refresh() {
  if (activeTab.value === 'presets') await loadPresets(true)
  if (activeTab.value === 'privacy') await loadPrivacyTweaks(true)
  if (activeTab.value === 'performance') await loadPerformanceTweaks(true)
  if (activeTab.value === 'customization') await loadCustomizationTweaks(true)
  if (activeTab.value === 'context') await loadContextEntries(true)
}

onActivated(() => {
  if (activeTab.value === 'presets') void loadPresets()
  if (activeTab.value === 'privacy') void loadPrivacyTweaks()
  if (activeTab.value === 'performance') void loadPerformanceTweaks()
  if (activeTab.value === 'customization') void loadCustomizationTweaks()
  if (activeTab.value === 'context') void loadContextEntries()
})
watch(activeTab, () => {
  if (activeTab.value === 'presets') void loadPresets()
  if (activeTab.value === 'privacy') void loadPrivacyTweaks()
  if (activeTab.value === 'performance') void loadPerformanceTweaks()
  if (activeTab.value === 'customization') void loadCustomizationTweaks()
  if (activeTab.value === 'context') void loadContextEntries()
})

async function reviewTweak(tweak: SystemTweakTarget, kind: TweakKind) {
  loading.value = true
  try {
    const plan = await createSystemTweakPlan(kind, tweak.id)
    pendingTweakAction.value = tweak.is_applied ? 'revert' : 'apply'
    pendingTweakName.value = tweak.name
    pendingTweakDesc.value = tweak.description
    pendingTweakRisk.value = tweak.risk
    pendingTweakPlan.value = plan
    showTweakConfirm.value = true
  } catch (e) {
    feedback.value = failure(`Could not review ${tweak.name}`, e)
    await refresh()
  } finally {
    loading.value = false
  }
}

function togglePrivacy(tweak: SystemTweakTarget) {
  void reviewTweak(tweak, 'privacy')
}

function togglePerf(tweak: SystemTweakTarget) {
  void reviewTweak(tweak, 'performance')
}

function toggleCustomization(tweak: SystemTweakTarget) {
  void reviewTweak(tweak, 'customization')
}

async function onConfirmTweak() {
  showTweakConfirm.value = false
  const plan = pendingTweakPlan.value
  if (!plan) return
  loading.value = true
  try {
    if (plan.requires_elevation && !(await api.checkElevated())) {
      await confirmAndRequestElevation(plan)
      feedback.value = success(
        'Administrator approval requested',
        'The exact confirmed tweak will continue after elevation and reopen in History.',
      )
      return
    }
    const execution = await executeSystemTweakPlan(plan)
    await refresh()
    feedback.value = success('Tweak updated and verified', execution.message)
  } catch (e) {
    feedback.value = failure(`Could not update ${pendingTweakName.value}`, e)
  } finally {
    loading.value = false
    pendingTweakPlan.value = null
  }
}

async function reviewPreset(preset: TweakPreset) {
  const selectedTargetIds = presetSelections.value[preset.id] ?? []
  if (selectedTargetIds.length === 0) {
    feedback.value = failure(
      'Select at least one target for ' + preset.name,
      'A preset cannot be applied with no selected constituents.',
    )
    return
  }
  loading.value = true
  try {
    pendingPresetPlan.value = await createTweakPresetPlan(preset.id, selectedTargetIds)
    pendingPresetName.value = preset.name
    showPresetConfirm.value = true
  } catch (e) {
    feedback.value = failure(`Could not review ${preset.name}`, e)
  } finally {
    loading.value = false
  }
}

function togglePresetTarget(preset: TweakPreset, targetId: string) {
  const current = new Set(presetSelections.value[preset.id] ?? preset.target_ids)
  if (current.has(targetId)) current.delete(targetId)
  else current.add(targetId)
  presetSelections.value = {
    ...presetSelections.value,
    [preset.id]: preset.target_ids.filter(id => current.has(id)),
  }
}

async function onConfirmPreset() {
  const plan = pendingPresetPlan.value
  showPresetConfirm.value = false
  if (!plan) return
  loading.value = true
  try {
    if (plan.requires_elevation && !(await api.checkElevated())) {
      await confirmAndRequestElevation(plan)
      feedback.value = success(
        'Administrator approval requested',
        'The exact visible preset targets will continue after elevation and reopen in History.',
      )
      return
    }
    const execution = await executeTweakPresetPlan(plan)
    feedback.value = execution.result.failed === 0
      ? success('Preset applied and verified', execution.message)
      : failure('Preset partially applied', execution.message)
  } catch (e) {
    feedback.value = failure(`Could not apply ${pendingPresetName.value}`, e)
  } finally {
    loading.value = false
    pendingPresetPlan.value = null
  }
}

</script>

<template>
  <div class="system-tweaks" :aria-busy="loading">
    <AppHeader />

    <p v-if="loading" class="sr-only" role="status" aria-live="polite">Reading current machine state…</p>

    <ResultsBanner
      v-if="feedback"
      :variant="feedback.type"
      :title="feedback.title"
      :description="feedback.message"
      class="tweaks-feedback"
      @dismiss="feedback = null"
    />

    <BaseTabBar
      :tabs="tabs"
      v-model="activeTab"
      variant="glass"
      class="tweaks-tabs"
    />

    <ExperimentalBanner
      v-if="activeTab === 'privacy' || activeTab === 'performance'"
      title="System-sensitive changes"
      message="Review the exact target and evidence status before applying any system-sensitive tweak."
    />
    <LabsBanner
      v-if="activeTab === 'performance'"
      message="Performance changes are optional experiments; WinOptimizer does not assume they make every machine faster."
    />

    <div v-if="activeTab === 'presets'" class="tab-content">
      <div class="toolbar">
        <BaseButton variant="primary" icon="refresh" @click="refresh" :loading="loading" size="sm">
          Refresh
        </BaseButton>
        <BaseBadge variant="accent" size="sm">{{ presets.length }} review presets</BaseBadge>
      </div>

      <div v-if="presets.length > 0" class="tweak-cards">
        <BaseCard
          v-for="preset in presets"
          :key="preset.id"
          variant="default"
          class="tweak-card"
        >
          <div class="tweak-body">
            <div class="tweak-info">
              <div class="tweak-header">
                <h4 class="tweak-name">{{ preset.name }}</h4>
                <BaseBadge variant="info" size="sm">No Labs targets</BaseBadge>
              </div>
              <p class="tweak-desc">{{ preset.description }}</p>
              <p class="tweak-desc">
                {{ (presetSelections[preset.id] ?? []).length }} of {{ preset.target_ids.length }} visible constituent operations selected
              </p>
              <div class="preset-targets" aria-label="Preset constituents">
                <label v-for="targetId in preset.target_ids" :key="targetId" class="preset-target">
                  <input
                    type="checkbox"
                    :checked="(presetSelections[preset.id] ?? []).includes(targetId)"
                    @change="togglePresetTarget(preset, targetId)"
                  >
                  <span>{{ targetId }}</span>
                </label>
              </div>
            </div>
            <BaseButton size="sm" :disabled="(presetSelections[preset.id] ?? []).length === 0" @click="reviewPreset(preset)">
              Review exact targets
            </BaseButton>
          </div>
        </BaseCard>
      </div>

      <EmptyState
        v-else-if="!loading"
        icon="sliders"
        title="No Presets"
        description="No reviewable non-Labs presets are available."
      />
    </div>

    <!-- Privacy Tweaks -->
    <div v-if="activeTab === 'privacy'" class="tab-content">
      <div class="toolbar">
        <BaseButton variant="primary" icon="refresh" @click="refresh" :loading="loading" size="sm">
          Refresh
        </BaseButton>
        <BaseBadge variant="accent" size="sm">{{ privacyTweaks.length }} tweaks</BaseBadge>
      </div>

      <div v-if="privacyTweaks.length > 0" class="tweak-cards">
        <BaseCard
          v-for="tweak in privacyTweaks"
          :key="tweak.id"
          :variant="tweak.is_applied ? 'accent' : 'default'"
          class="tweak-card"
        >
          <div class="tweak-body">
            <div class="tweak-info">
              <div class="tweak-header">
                <h4 class="tweak-name">{{ tweak.name }}</h4>
                <BaseBadge :variant="riskVariant(tweak.risk)" size="sm">
                  {{ tweak.risk }} risk
                </BaseBadge>
                <BaseBadge :variant="tweak.is_applied ? 'success' : 'default'" size="sm">
                  {{ tweak.is_applied ? 'Applied' : 'Not applied' }}
                </BaseBadge>
                <BaseBadge v-if="tweak.experimental" variant="warning" size="sm">
                  Experimental
                </BaseBadge>
                <BaseBadge v-if="tweak.evidence_status === 'pending_native_validation'" variant="warning" size="sm">
                  Native validation pending
                </BaseBadge>
                <BaseBadge v-else-if="tweak.evidence_status === 'confirmed_conditional'" variant="info" size="sm">
                  Conditional support
                </BaseBadge>
              </div>
              <p class="tweak-desc">{{ tweak.description }}</p>
            </div>
            <div class="tweak-action">
              <BaseToggle
                :modelValue="tweak.is_applied"
                :aria-label="`${tweak.is_applied ? 'Revert' : 'Apply'} ${tweak.name}`"
                :disabled="loading"
                @update:modelValue="togglePrivacy(tweak)"
                size="md"
              />
            </div>
          </div>
        </BaseCard>
      </div>

      <EmptyState
        v-else-if="!loading"
        icon="shield"
        title="No Privacy Tweaks"
        description="Click Refresh to load privacy tweaks."
      />
    </div>

    <!-- Performance Tweaks -->
    <div v-if="activeTab === 'performance'" class="tab-content">
      <div class="toolbar">
        <BaseButton variant="primary" icon="refresh" @click="refresh" :loading="loading" size="sm">
          Refresh
        </BaseButton>
        <BaseBadge variant="accent" size="sm">{{ perfTweaks.length }} tweaks</BaseBadge>
      </div>

      <div v-if="perfTweaks.length > 0" class="tweak-cards">
        <BaseCard
          v-for="tweak in perfTweaks"
          :key="tweak.id"
          :variant="tweak.is_applied ? 'success' : 'default'"
          class="tweak-card"
        >
          <div class="tweak-body">
            <div class="tweak-info">
              <div class="tweak-header">
                <h4 class="tweak-name">{{ tweak.name }}</h4>
                <BaseBadge :variant="riskVariant(tweak.risk)" size="sm">
                  {{ tweak.risk }} risk
                </BaseBadge>
                <BaseBadge :variant="tweak.is_applied ? 'success' : 'default'" size="sm">
                  {{ tweak.is_applied ? 'Applied' : 'Not applied' }}
                </BaseBadge>
                <BaseBadge v-if="tweak.experimental" variant="warning" size="sm">
                  Experimental
                </BaseBadge>
                <BaseBadge v-if="tweak.evidence_status === 'pending_native_validation'" variant="warning" size="sm">
                  Native validation pending
                </BaseBadge>
                <BaseBadge v-else-if="tweak.evidence_status === 'confirmed_conditional'" variant="info" size="sm">
                  Conditional support
                </BaseBadge>
                <BaseBadge v-if="tweak.requires_restart" variant="warning" size="sm">
                  Restart required
                </BaseBadge>
              </div>
              <p class="tweak-desc">{{ tweak.description }}</p>
            </div>
            <div class="tweak-action">
              <BaseToggle
                :modelValue="tweak.is_applied"
                :aria-label="`${tweak.is_applied ? 'Revert' : 'Apply'} ${tweak.name}`"
                :disabled="loading"
                @update:modelValue="togglePerf(tweak)"
                size="md"
              />
            </div>
          </div>
        </BaseCard>
      </div>

      <EmptyState
        v-else-if="!loading"
        icon="zap"
        title="No Performance Tweaks"
        description="Click Refresh to load performance tweaks."
      />
    </div>

    <!-- Customization Tweaks -->
    <div v-if="activeTab === 'customization'" class="tab-content">
      <div class="toolbar">
        <BaseButton variant="primary" icon="refresh" @click="refresh" :loading="loading" size="sm">
          Refresh
        </BaseButton>
        <BaseBadge variant="accent" size="sm">{{ customizationTweaks.length }} tweaks</BaseBadge>
      </div>

      <div v-if="customizationTweaks.length > 0" class="tweak-cards">
        <BaseCard
          v-for="tweak in customizationTweaks"
          :key="tweak.id"
          :variant="tweak.is_applied ? 'accent' : 'default'"
          class="tweak-card"
        >
          <div class="tweak-body">
            <div class="tweak-info">
              <div class="tweak-header">
                <h4 class="tweak-name">{{ tweak.name }}</h4>
                <BaseBadge :variant="riskVariant(tweak.risk)" size="sm">
                  {{ tweak.risk }} risk
                </BaseBadge>
                <BaseBadge :variant="tweak.is_applied ? 'success' : 'default'" size="sm">
                  {{ tweak.is_applied ? 'Applied' : 'Not applied' }}
                </BaseBadge>
                <BaseBadge v-if="tweak.requires_elevation" variant="warning" size="sm">
                  Administrator
                </BaseBadge>
                <BaseBadge v-if="tweak.evidence_status === 'pending_native_validation'" variant="warning" size="sm">
                  Native validation pending
                </BaseBadge>
                <BaseBadge v-else-if="tweak.evidence_status === 'confirmed_conditional'" variant="info" size="sm">
                  Conditional support
                </BaseBadge>
              </div>
              <p class="tweak-desc">{{ tweak.description }}</p>
            </div>
            <div class="tweak-action">
              <BaseToggle
                :modelValue="tweak.is_applied"
                :aria-label="`${tweak.is_applied ? 'Revert' : 'Apply'} ${tweak.name}`"
                :disabled="loading"
                @update:modelValue="toggleCustomization(tweak)"
                size="md"
              />
            </div>
          </div>
        </BaseCard>
      </div>

      <EmptyState
        v-else-if="!loading"
        icon="sliders"
        title="No Customization Tweaks"
        description="Click Refresh to load Explorer and Windows usability tweaks."
      />
    </div>

    <!-- Context Menu -->
    <div v-if="activeTab === 'context'" class="tab-content">
      <div class="toolbar">
        <BaseButton variant="primary" icon="refresh" @click="refresh" :loading="loading" size="sm">
          Refresh
        </BaseButton>
        <BaseBadge variant="accent" size="sm">{{ contextEntries.length }} entries</BaseBadge>
        <BaseBadge variant="warning" size="sm">Read-only inventory</BaseBadge>
      </div>

      <BaseTable
        v-if="contextEntries.length > 0"
        :columns="contextCols"
        :rows="contextRows"
        :loading="loading"
        max-height="520px"
        compact
      >
        <template #cell-name="{ row }">
          <span class="ctx-name">{{ row.name }}</span>
        </template>
        <template #cell-command="{ row }">
          <span class="ctx-cmd" :title="row.command">{{ row.command.substring(0, 60) }}{{ row.command.length > 60 ? '...' : '' }}</span>
        </template>
        <template #cell-location="{ row }">
          <BaseBadge variant="info" size="sm">{{ row.location }}</BaseBadge>
        </template>
        <template #row-actions="{ row }">
          <span class="ctx-readonly">Review only</span>
        </template>
      </BaseTable>

      <EmptyState
        v-else-if="!loading"
        icon="layers"
        title="No Context Menu Entries"
        description="Click Refresh to load context menu entries."
      />
    </div>

    <OperationPreview
      v-if="showPresetConfirm && pendingPresetPlan"
      :plan="pendingPresetPlan"
      :title="`${pendingPresetName} preview`"
      action-label="Apply exact targets"
      class="operation-preview-card"
    />

    <OperationPreview
      v-if="showTweakConfirm && pendingTweakPlan"
      :plan="pendingTweakPlan"
      :title="`${pendingTweakAction === 'apply' ? 'Apply' : 'Revert'} ${pendingTweakName} preview`"
      :action-label="pendingTweakAction === 'apply' ? 'Apply change' : 'Revert change'"
      class="operation-preview-card"
    />

    <ConfirmDialog
      :open="showPresetConfirm"
      :title="'Apply ' + pendingPresetName + '?'"
      description="This preset contains only the checked visible non-Labs operations below. Native validation status remains visible for each target. Already-applied targets were omitted and no target can be added after confirmation."
      :items="pendingPresetItems as any"
      confirm-label="Apply Exact Targets"
      cancel-label="Cancel"
      variant="warning"
      icon="sliders"
      :loading="loading"
      @confirm="onConfirmPreset"
      @cancel="showPresetConfirm = false"
    />

    <!-- Tweak Confirmation Dialog -->
    <ConfirmDialog
      :open="showTweakConfirm"
      :title="pendingTweakAction === 'apply' ? 'Apply ' + pendingTweakName + '?' : 'Revert ' + pendingTweakName + '?'"
      :description="pendingTweakDesc"
      :items="pendingTweakItems as any"
      :confirm-label="pendingTweakAction === 'apply' ? 'Apply Change' : 'Revert Change'"
      cancel-label="Cancel"
      :variant="pendingTweakRisk === 'high' ? 'danger' : pendingTweakRisk === 'medium' ? 'warning' : 'primary'"
      :icon="pendingTweakAction === 'apply' ? 'shield' : 'zap'"
      :loading="loading"
      @confirm="onConfirmTweak"
      @cancel="showTweakConfirm = false"
    />

  </div>
</template>

<style scoped>
.system-tweaks {
  width: 100%;
  min-width: 0;
}

.sr-only {
  position: absolute;
  width: 1px;
  height: 1px;
  padding: 0;
  margin: -1px;
  overflow: hidden;
  clip: rect(0, 0, 0, 0);
  white-space: nowrap;
  border: 0;
}

.tweaks-feedback {
  margin-bottom: var(--space-5);
}

.operation-preview-card {
  margin-bottom: var(--space-4);
}

.tweaks-tabs {
  margin-bottom: var(--space-4);
}

.tab-content {
  animation: fadeInUp var(--duration-normal) var(--ease-out);
}

.toolbar {
  display: flex;
  align-items: center;
  gap: var(--space-3);
  margin-bottom: var(--space-4);
}

/* Tweak Cards */
.tweak-cards {
  display: flex;
  flex-direction: column;
  gap: var(--space-3);
}

.tweak-card {
  transition: all var(--duration-normal) var(--ease-out);
}

.tweak-body {
  display: flex;
  justify-content: space-between;
  align-items: flex-start;
  gap: var(--space-4);
}

.tweak-info {
  flex: 1;
  min-width: 0;
}

.tweak-header {
  display: flex;
  align-items: center;
  gap: var(--space-3);
  margin-bottom: var(--space-1);
}

.tweak-name {
  margin: 0;
  font-size: var(--text-md);
  font-weight: var(--weight-semibold);
  color: var(--text-primary);
}

.tweak-desc {
  margin: 0;
  font-size: var(--text-sm);
  color: var(--text-muted);
  line-height: var(--leading-relaxed);
}

.preset-targets {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(230px, 1fr));
  gap: var(--space-2);
  margin-top: var(--space-3);
}

.preset-target {
  display: flex;
  align-items: flex-start;
  gap: var(--space-2);
  color: var(--text-secondary);
  font-size: var(--text-xs);
  line-height: var(--leading-relaxed);
}

.tweak-action {
  flex-shrink: 0;
  padding-top: 2px;
}

/* Context menu */
.ctx-name {
  font-weight: var(--weight-medium);
  color: var(--text-primary);
  font-size: var(--text-sm);
}

.ctx-cmd {
  font-family: var(--font-mono);
  font-size: var(--text-xs);
  color: var(--text-muted);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  display: block;
  max-width: 350px;
}

.ctx-readonly {
  color: var(--text-muted);
  font-size: var(--text-xs);
}

@media (max-width: 700px) {
  .tweak-body,
  .tweak-header {
    align-items: flex-start;
  }

  .tweak-body,
  .toolbar {
    flex-direction: column;
  }

  .toolbar {
    align-items: stretch;
  }
}
</style>
