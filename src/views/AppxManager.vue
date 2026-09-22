<script setup lang="ts">
import { computed, onActivated, ref } from 'vue'
import type { AppxPackage } from '../api/tauri'
import * as api from '../api/tauri'
import type { OperationPlan } from '../api/operation'
import { confirmAndRequestElevation } from '../api/operation'
import type { AppxAction } from '../api/appx'
import { createAppxPlan, executeAppxPlan } from '../api/appx'
import AppHeader from '../components/AppHeader.vue'
import BaseCard from '../components/BaseCard.vue'
import BaseButton from '../components/BaseButton.vue'
import BaseBadge from '../components/BaseBadge.vue'
import ConfirmDialog from '../components/ConfirmDialog.vue'
import ResultsBanner from '../components/ResultsBanner.vue'
import EmptyState from '../components/EmptyState.vue'
import { failure, success, type ActionFeedback } from '../utils/feedback'

const packages = ref<AppxPackage[]>([])
const policyEnabled = ref(false)
const loading = ref(false)
const operating = ref(false)
const feedback = ref<ActionFeedback | null>(null)
const showConfirm = ref(false)
const pendingAction = ref<{
  title: string
  description: string
  confirmLabel: string
  variant: 'warning' | 'danger'
  plan: OperationPlan
} | null>(null)

const selectedFamilyNames = ref<Set<string>>(new Set())

const columns = [
  { key: 'select', label: '', width: '40px', sortable: false },
  { key: 'display_name', label: 'App Name', sortable: true },
  { key: 'risk', label: 'Removal profile', width: '130px', sortable: false },
  { key: 'family_name', label: 'Package Family Name', sortable: true },
  { key: 'status', label: 'Status', width: '140px', sortable: false },
]

const tableRows = computed(() =>
  packages.value.map(p => ({
    _id: p.family_name,
    select: p.is_protected
      ? null
      : selectedFamilyNames.value.has(p.family_name),
    display_name: p.display_name,
    risk: p.risk,
    family_name: p.family_name,
    status: statusBadge(p),
    _is_protected: p.is_protected,
    _is_marked: p.is_marked_for_removal,
  }))
)

function statusBadge(pkg: AppxPackage): string {
  if (pkg.is_protected) return 'Protected'
  if (pkg.is_marked_for_removal) return 'Marked for removal'
  if (!isAvailable(pkg)) return 'Not detected'
  return 'Available'
}

function statusVariant(pkg: AppxPackage): 'info' | 'danger' | 'success' {
  if (pkg.is_protected) return 'info'
  if (pkg.is_marked_for_removal) return 'danger'
  if (!isAvailable(pkg)) return 'info'
  return 'success'
}

function riskVariant(risk: AppxPackage['risk']): 'success' | 'warning' | 'danger' {
  if (risk === 'safe') return 'success'
  if (risk === 'caution') return 'warning'
  return 'danger'
}

function isAvailable(pkg: AppxPackage): boolean {
  return Boolean(pkg.installed_package_full_name || pkg.provisioned_package_name)
}

async function loadPackages() {
  loading.value = true
  feedback.value = null
  try {
    const result = await api.listAppxCandidates()
    packages.value = result
    policyEnabled.value = await api.getAppxPolicyStatus()
    selectedFamilyNames.value = new Set()
  } catch (e) {
    feedback.value = failure('Could not load AppX packages', e)
  } finally {
    loading.value = false
  }
}

const hasLoaded = ref(false)
onActivated(() => {
  if (hasLoaded.value) return
  hasLoaded.value = true
  void loadPackages()
})

function toggleSelect(familyName: string) {
  const next = new Set(selectedFamilyNames.value)
  if (next.has(familyName)) {
    next.delete(familyName)
  } else {
    next.add(familyName)
  }
  selectedFamilyNames.value = next
}

function toggleAll() {
  const selectable = packages.value.filter(p => !p.is_protected && isAvailable(p))
  const allSelected = selectable.every(p => selectedFamilyNames.value.has(p.family_name))
  const next = new Set(selectedFamilyNames.value)
  if (allSelected) {
    for (const p of selectable) next.delete(p.family_name)
  } else {
    for (const p of selectable) next.add(p.family_name)
  }
  selectedFamilyNames.value = next
}

async function confirmMarkSelected() {
  const names = Array.from(selectedFamilyNames.value)
  if (names.length === 0) return
  await reviewAction(
    { kind: 'mark', family_names: names },
    `Mark ${names.length} package${names.length > 1 ? 's' : ''} for removal?`,
    `Mark ${names.length} for removal`,
    'warning',
  )
}

function confirmUnmark(pkg: AppxPackage) {
  void reviewAction(
    { kind: 'unmark', family_name: pkg.family_name },
    `Remove "${pkg.display_name}" from de-provisioning?`,
    'Remove from list',
    'warning',
  )
}

function confirmDisablePolicy() {
  void reviewAction(
    { kind: 'disable_policy' },
    'Disable AppX de-provisioning policy?',
    'Disable policy',
    'danger',
  )
}

async function reviewAction(
  action: AppxAction,
  title: string,
  confirmLabel: string,
  variant: 'warning' | 'danger',
) {
  operating.value = true
  feedback.value = null
  try {
    const plan = await createAppxPlan(action)
    pendingAction.value = {
      title,
      description: 'Review the exact policy targets. This Experimental change is verified by registry read-back and may take effect at the next sign-in.',
      confirmLabel,
      variant,
      plan,
    }
    showConfirm.value = true
  } catch (e) {
    feedback.value = failure('Could not review AppX policy change', e)
  } finally {
    operating.value = false
  }
}

async function performConfirmed() {
  const pending = pendingAction.value
  showConfirm.value = false
  if (!pending) return
  operating.value = true
  feedback.value = null
  try {
    if (!(await api.checkElevated())) {
      await confirmAndRequestElevation(pending.plan)
      feedback.value = success(
        'Administrator approval requested',
        'The exact confirmed AppX policy change will continue after elevation and reopen in History.',
      )
      return
    }
    const execution = await executeAppxPlan(pending.plan)
    feedback.value = execution.result.failed === 0
      ? success('AppX policy updated and verified', execution.message)
      : failure('AppX policy partially updated', execution.message)
    selectedFamilyNames.value = new Set()
    await loadPackages()
  } catch (e) {
    feedback.value = failure('Could not update AppX policy', e)
  } finally {
    operating.value = false
    pendingAction.value = null
  }
}

function cancelConfirm() {
  showConfirm.value = false
  pendingAction.value = null
}
</script>

<template>
  <div class="appx-page">
    <AppHeader
      title="Preinstalled Apps Policy"
      subtitle="Review the policy for pre-installed Windows apps before the next sign-in"
    >
      <template #actions>
        <BaseButton variant="primary" icon="refresh" @click="loadPackages" :loading="loading" size="sm">
          Refresh
        </BaseButton>
      </template>
    </AppHeader>

    <ResultsBanner
      v-if="feedback"
      :variant="feedback.type"
      :title="feedback.title"
      :description="feedback.message"
      class="appx-banner"
      @dismiss="feedback = null"
    />

    <!-- Policy Status -->
    <BaseCard variant="glass" class="appx-policy-card">
      <div class="policy-status">
        <span class="policy-label">Preinstalled Apps Policy:</span>
        <BaseBadge :variant="policyEnabled ? 'danger' : 'info'">
          {{ policyEnabled ? 'Enabled' : 'Disabled' }}
        </BaseBadge>
        <BaseBadge variant="warning">Experimental</BaseBadge>
        <span class="policy-hint" v-if="policyEnabled">
          Marked packages will be removed on next login
        </span>
        <span class="policy-hint" v-else>
          No packages are currently scheduled for removal
        </span>
        <p class="policy-explanation">
          This changes Windows policy for future sign-ins; it does not uninstall packages immediately.
          Only packages detected as installed or provisioned are selectable. Microsoft Store and Gaming infrastructure remains protected.
        </p>
      </div>
      <div class="policy-actions" v-if="policyEnabled">
        <BaseButton variant="danger" size="sm" icon="trash" @click="confirmDisablePolicy" :disabled="operating">
          Disable Policy
        </BaseButton>
      </div>
    </BaseCard>

    <!-- Package list -->
    <BaseCard variant="glass" class="appx-list-card">
      <div class="list-toolbar" v-if="packages.length > 0">
        <BaseButton
          variant="secondary"
          size="sm"
          @click="toggleAll"
          :disabled="loading || operating"
        >
          {{ packages.filter(p => !p.is_protected && isAvailable(p)).every(p => selectedFamilyNames.has(p.family_name))
              ? 'Deselect All' : 'Select All' }}
        </BaseButton>
        <BaseButton
          variant="warning"
          size="sm"
          icon="trash"
          :disabled="selectedFamilyNames.size === 0 || loading || operating"
          @click="confirmMarkSelected"
        >
          Mark Selected ({{ selectedFamilyNames.size }})
        </BaseButton>
      </div>

      <div v-if="loading" class="loading-state">
        <p>Loading packages...</p>
      </div>

      <EmptyState
        v-else-if="!loading && packages.length === 0"
        title="No AppX Packages"
        description="Click Refresh to load the list of available packages."
      >
        <BaseButton variant="primary" @click="loadPackages">Refresh</BaseButton>
      </EmptyState>

      <table v-else class="appx-table">
        <thead>
          <tr>
            <th class="col-select"></th>
            <th class="col-name">App Name</th>
            <th class="col-family">Package Family Name</th>
            <th class="col-status">Status</th>
            <th class="col-action">Action</th>
          </tr>
        </thead>
        <tbody>
          <tr v-for="pkg in packages" :key="pkg.family_name"
            :class="{ 'is-protected': pkg.is_protected, 'is-marked': pkg.is_marked_for_removal, 'is-unavailable': !isAvailable(pkg) && !pkg.is_marked_for_removal }">
            <td class="col-select">
              <input
                type="checkbox"
                v-if="!pkg.is_protected && isAvailable(pkg)"
                :checked="selectedFamilyNames.has(pkg.family_name)"
                @change="toggleSelect(pkg.family_name)"
                :disabled="operating"
              />
            </td>
            <td class="col-name">{{ pkg.display_name }}</td>
            <td class="col-risk">
              <BaseBadge :variant="riskVariant(pkg.risk)">{{ pkg.risk }}</BaseBadge>
            </td>
            <td class="col-family code">{{ pkg.family_name }}</td>
            <td class="col-status">
              <BaseBadge :variant="statusVariant(pkg)">
                {{ statusBadge(pkg) }}
              </BaseBadge>
            </td>
            <td class="col-action">
              <BaseButton
                v-if="pkg.is_marked_for_removal && !pkg.is_protected"
                variant="secondary"
                size="xs"
                :disabled="operating"
                @click="confirmUnmark(pkg)"
              >
                Remove
              </BaseButton>
            </td>
          </tr>
        </tbody>
      </table>
    </BaseCard>

    <ConfirmDialog
      v-if="showConfirm && pendingAction"
      :open="showConfirm"
      :operation-plan="pendingAction.plan"
      :title="pendingAction.title"
      :description="pendingAction.description"
      :items="[
        ...pendingAction.plan.targets.map(target => ({
          label: target.description,
          value: target.target_id,
          risk: 'high'
        })),
        { label: 'Administrator', value: 'Required', risk: 'medium' },
        { label: 'Undo', value: 'Unavailable while Experimental', risk: 'high' },
        { label: 'Activation', value: 'Next sign-in or profile creation may be required', risk: 'medium' }
      ]"
      :confirm-label="pendingAction.confirmLabel"
      :variant="pendingAction.variant"
      :loading="operating"
      @confirm="performConfirmed"
      @cancel="cancelConfirm"
    />
  </div>
</template>

<script lang="ts">
import { defineComponent } from 'vue'
export default defineComponent({})
</script>

<style scoped>
.appx-page {
  padding: 0 var(--space-lg);
}

.appx-banner {
  margin-bottom: var(--space-md);
}

.appx-policy-card {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: var(--space-md);
  margin-bottom: var(--space-md);
  flex-wrap: wrap;
  gap: var(--space-sm);
}

.policy-status {
  display: flex;
  align-items: center;
  gap: var(--space-sm);
}

.policy-label {
  font-weight: 600;
  font-size: 0.9rem;
}

.policy-hint {
  font-size: 0.8rem;
  opacity: 0.7;
}

.policy-explanation {
  flex-basis: 100%;
  margin: 0;
  font-size: 0.8rem;
  opacity: 0.75;
}

.policy-actions {
  display: flex;
  gap: var(--space-sm);
}

.appx-list-card {
  padding: var(--space-md);
}

.list-toolbar {
  display: flex;
  gap: var(--space-sm);
  margin-bottom: var(--space-md);
  flex-wrap: wrap;
}

.loading-state {
  text-align: center;
  padding: var(--space-xl);
  opacity: 0.6;
}

.appx-table {
  width: 100%;
  border-collapse: collapse;
  font-size: 0.85rem;
}

.appx-table th {
  text-align: left;
  padding: var(--space-sm) var(--space-sm);
  border-bottom: 1px solid var(--border-default);
  font-weight: 600;
  white-space: nowrap;
}

.appx-table td {
  padding: var(--space-sm) var(--space-sm);
  border-bottom: 1px solid var(--border-subtle);
  vertical-align: middle;
}

.appx-table tr.is-protected {
  opacity: 0.5;
}

.appx-table tr.is-marked {
  background: var(--color-danger-bg);
}

.col-select {
  width: 40px;
  text-align: center;
}

.col-name {
  min-width: 140px;
}

.col-status {
  width: 140px;
}

.col-action {
  width: 100px;
  text-align: right;
}

.code {
  font-family: 'Cascadia Code', 'Fira Code', 'Consolas', monospace;
  font-size: 0.8rem;
  opacity: 0.8;
}

@media (max-width: 700px) {
  .appx-page {
    padding: 0 var(--space-sm);
  }

  .appx-table .col-family {
    display: none;
  }
}
</style>
