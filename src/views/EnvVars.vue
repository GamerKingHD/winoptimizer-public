<script setup lang="ts">
import { computed, onActivated, ref } from 'vue'
import type { EnvScope, EnvVar, EnvVarType } from '../api/tauri'
import * as api from '../api/tauri'
import type { OperationPlan } from '../api/operation'
import { confirmAndRequestElevation } from '../api/operation'
import type { EnvAction } from '../api/environment'
import { createEnvPlan, executeEnvPlan } from '../api/environment'
import AppHeader from '../components/AppHeader.vue'
import BaseCard from '../components/BaseCard.vue'
import BaseButton from '../components/BaseButton.vue'
import BaseTable from '../components/BaseTable.vue'
import BaseBadge from '../components/BaseBadge.vue'
import EmptyState from '../components/EmptyState.vue'
import ResultsBanner from '../components/ResultsBanner.vue'
import ConfirmDialog from '../components/ConfirmDialog.vue'
import { errorMessage } from '../utils/feedback'

const vars = ref<EnvVar[]>([])
const loading = ref(false)
const saving = ref(false)
const result = ref<'success' | 'warning' | null>(null)
const errorDetail = ref('')

// Editing state
const editingName = ref<string | null>(null)
const editValue = ref('')

// New entry
const showAdd = ref(false)
const newName = ref('')
const newValue = ref('')
const newScope = ref<EnvScope>('user')
const newType = ref<EnvVarType>('REG_SZ')
const showEnvConfirm = ref(false)
const pendingEnvAction = ref<{
  title: string
  description: string
  confirmLabel: string
  variant: 'warning' | 'danger'
  items: Array<{ label: string; value?: string; risk?: 'low' | 'medium' | 'high' | 'info' }>
  plan: OperationPlan
} | null>(null)

const columns = [
  { key: 'name', label: 'Name', sortable: true },
  { key: 'value', label: 'Value', sortable: true },
  { key: 'scope', label: 'Scope', sortable: true, width: '90px' },
  { key: 'type', label: 'Type', sortable: true, width: '120px' },
]

const tableRows = computed(() =>
  vars.value.map(v => ({
    _id: v.scope + '\\' + v.name,
    name: v.name,
    value: v.value,
    scope: v.scope,
    type: v.var_type,
  }))
)

async function loadVars() {
  loading.value = true
  try {
    vars.value = await api.listEnvVars()
    errorDetail.value = ''
  } catch (e) {
    vars.value = []
    errorDetail.value = errorMessage(e)
  } finally {
    loading.value = false
  }
}

const hasLoaded = ref(false)
onActivated(() => {
  if (hasLoaded.value) return
  hasLoaded.value = true
  void loadVars()
})

function startEdit(env: EnvVar) {
  editingName.value = env.name
  editValue.value = env.value
}

async function prepareEnvPlan(action: EnvAction) {
  try {
    return await createEnvPlan(action)
  } catch (e) {
    errorDetail.value = errorMessage(e)
    result.value = 'warning'
    return null
  }
}

async function saveEdit(row: any) {
  const env = vars.value.find(v => v.name === row.name && v.scope === row.scope)
  if (!env) return
  const nextEnv = { ...env, value: editValue.value }
  const plan = await prepareEnvPlan({ kind: 'set', variable: nextEnv })
  if (!plan) return
  pendingEnvAction.value = {
    title: 'Save Environment Variable?',
    description: 'This writes the new value to the Windows environment registry. The previous value is recorded in History.',
    confirmLabel: 'Save Variable',
    variant: env.scope === 'system' ? 'warning' : 'warning',
    items: [
      { label: env.name, value: env.scope, risk: env.scope === 'system' ? 'medium' : 'low' },
      { label: 'Previous value', value: env.value, risk: 'info' },
      { label: 'New value', value: nextEnv.value, risk: env.scope === 'system' ? 'medium' : 'low' },
    ],
    plan,
  }
  showEnvConfirm.value = true
}

async function deleteVar(env: EnvVar) {
  const plan = await prepareEnvPlan({ kind: 'delete', name: env.name, scope: env.scope })
  if (!plan) return
  pendingEnvAction.value = {
    title: 'Delete Environment Variable?',
    description: 'This removes the variable from the Windows environment registry. The current value is recorded in History before deletion.',
    confirmLabel: 'Delete Variable',
    variant: 'danger',
    items: [
      { label: env.name, value: env.scope, risk: env.scope === 'system' ? 'high' : 'medium' },
      { label: 'Current value', value: env.value, risk: 'info' },
    ],
    plan,
  }
  showEnvConfirm.value = true
}

async function addVar() {
  const name = newName.value.trim()
  const value = newValue.value.trim()
  if (!name || !value) return

  const env: EnvVar = { name, value, scope: newScope.value, var_type: newType.value }
  const plan = await prepareEnvPlan({ kind: 'set', variable: env })
  if (!plan) return
  pendingEnvAction.value = {
    title: 'Add Environment Variable?',
    description: 'This creates a new Windows environment variable. Creation is recorded in History so it can be undone.',
    confirmLabel: 'Add Variable',
    variant: newScope.value === 'system' ? 'warning' : 'warning',
    items: [
      { label: name, value: newScope.value, risk: newScope.value === 'system' ? 'medium' : 'low' },
      { label: 'Value', value, risk: 'info' },
    ],
    plan,
  }
  showEnvConfirm.value = true
}

async function onConfirmEnvAction() {
  const action = pendingEnvAction.value
  if (!action) return
  showEnvConfirm.value = false
  saving.value = true
  try {
    if (action.plan.requires_elevation && !(await api.checkElevated())) {
      await confirmAndRequestElevation(action.plan)
      errorDetail.value = 'Administrator approval requested. The exact confirmed change will continue after elevation.'
      result.value = 'success'
      return
    }
    const execution = await executeEnvPlan(action.plan)
    if (execution.result.failed > 0) throw new Error(execution.message)
    result.value = 'success'
    errorDetail.value = ''
    editingName.value = null
    newName.value = ''
    newValue.value = ''
    newType.value = 'REG_SZ'
    showAdd.value = false
    await loadVars()
  } catch (e) {
    errorDetail.value = errorMessage(e)
    result.value = 'warning'
  } finally {
    saving.value = false
    pendingEnvAction.value = null
  }
}

function onCancelEnvAction() {
  showEnvConfirm.value = false
  pendingEnvAction.value = null
}
</script>

<template>
  <div class="env-vars">
    <AppHeader />

    <div class="toolbar">
      <div class="toolbar-left">
        <BaseButton variant="primary" icon="refresh" @click="loadVars" :loading="loading" size="sm">
          Refresh
        </BaseButton>
        <BaseButton variant="accent" icon="plus" @click="showAdd = !showAdd" size="sm">
          Add Variable
        </BaseButton>
        <BaseBadge variant="info" size="sm">{{ vars.length }} variables</BaseBadge>
        <BaseBadge variant="warning" size="sm">Partially proven · VM acceptance pending</BaseBadge>
        <BaseBadge variant="info" size="sm">Exact registry recovery</BaseBadge>
      </div>
    </div>

    <!-- Add form -->
    <div v-if="showAdd" class="add-form">
      <BaseCard variant="elevated" padding="md">
        <div class="form-row">
          <div class="form-field">
            <label class="form-label">Variable Name</label>
            <input v-model="newName" class="form-input" placeholder="JAVA_HOME" @keyup.enter="addVar" />
          </div>
          <div class="form-field flex">
            <label class="form-label">Value</label>
            <input v-model="newValue" class="form-input" placeholder="C:\\Program Files\\Java\\jdk-21" @keyup.enter="addVar" />
          </div>
          <div class="form-field sm">
            <label class="form-label">Scope</label>
            <select v-model="newScope" class="form-select">
              <option value="user">User</option>
              <option value="system">System</option>
            </select>
          </div>
          <div class="form-field sm">
            <label class="form-label">Registry type</label>
            <select v-model="newType" class="form-select">
              <option value="REG_SZ">REG_SZ</option>
              <option value="REG_EXPAND_SZ">REG_EXPAND_SZ</option>
            </select>
          </div>
          <div class="form-actions">
            <BaseButton variant="ghost" size="sm" @click="showAdd = false">Cancel</BaseButton>
            <BaseButton variant="primary" size="sm" @click="addVar" :loading="saving">Add</BaseButton>
          </div>
        </div>
      </BaseCard>
    </div>

    <!-- Result banner -->
    <ResultsBanner
      v-if="result === 'success'"
      variant="success"
      title="Environment variable saved"
      description="Changes take effect for new processes. WM_SETTINGCHANGE was sent and an exact registry recovery record was written to History."
      dismissable
      class="result-banner"
    />
    <ResultsBanner
      v-if="result === 'warning'"
      variant="warning"
      title="Operation failed"
      :description="errorDetail || 'Could not write to registry. Try running WinOptimizer as Administrator for system variables.'"
      dismissable
      class="result-banner"
    />

    <!-- Table -->
    <BaseTable
      v-if="vars.length > 0"
      :columns="columns"
      :rows="tableRows"
      :loading="loading"
      max-height="520px"
      compact
    >
      <template #cell-name="{ row }">
        <span class="env-name">{{ row.name }}</span>
      </template>
      <template #cell-value="{ row }">
        <input
          v-if="editingName === row.name"
          v-model="editValue"
          class="inline-input"
          @keyup.enter="saveEdit(row)"
          @keyup.escape="editingName = null"
        />
        <span v-else class="env-value">{{ row.value }}</span>
      </template>
      <template #cell-scope="{ row }">
        <BaseBadge :variant="row.scope === 'system' ? 'warning' : 'default'" size="sm">
          {{ row.scope }}
        </BaseBadge>
      </template>
      <template #cell-type="{ row }">
        <span class="env-type">{{ row.type }}</span>
      </template>
      <template #row-actions="{ row }">
        <div class="row-actions">
          <template v-if="editingName === row.name">
            <BaseButton variant="primary" size="sm" icon="check" @click="saveEdit(row)" :loading="saving">Save</BaseButton>
          </template>
          <template v-else>
            <BaseButton variant="ghost" size="sm" icon="edit" @click="startEdit(row)">Edit</BaseButton>
            <BaseButton variant="danger" size="sm" icon="x" @click="deleteVar(row)" :loading="saving">Delete</BaseButton>
          </template>
        </div>
      </template>
    </BaseTable>

    <EmptyState
      v-else-if="!loading"
      icon="zap"
      title="No Variables Loaded"
      description="Click Refresh to load environment variables."
    />

    <ConfirmDialog
      :open="showEnvConfirm"
      :operation-plan="pendingEnvAction?.plan"
      :title="pendingEnvAction?.title || ''"
      :description="pendingEnvAction?.description || ''"
      :items="pendingEnvAction?.items || []"
      :confirm-label="pendingEnvAction?.confirmLabel || 'Confirm'"
      cancel-label="Cancel"
      :variant="pendingEnvAction?.variant || 'warning'"
      icon="shield"
      :loading="saving"
      @confirm="onConfirmEnvAction"
      @cancel="onCancelEnvAction"
    />
  </div>
</template>

<style scoped>
.env-vars {
  width: 100%;
  min-width: 0;
}

.toolbar {
  display: flex;
  justify-content: space-between;
  align-items: center;
  margin-bottom: var(--space-4);
  gap: var(--space-3);
}

.toolbar-left {
  display: flex;
  align-items: center;
  gap: var(--space-2);
}

.add-form {
  margin-bottom: var(--space-4);
}

.form-row {
  display: flex;
  align-items: flex-end;
  gap: var(--space-3);
  flex-wrap: wrap;
}

.form-field {
  display: flex;
  flex-direction: column;
  gap: var(--space-1);
  flex: 0 0 160px;
}

.form-field.flex {
  flex: 1;
  min-width: 200px;
}

.form-field.sm {
  flex: 0 0 100px;
}

.form-label {
  font-size: var(--text-xs);
  color: var(--text-muted);
  text-transform: uppercase;
  letter-spacing: var(--tracking-wide);
}

.form-input {
  padding: var(--space-1) var(--space-2);
  background: var(--bg-surface);
  border: 1px solid var(--border-default);
  border-radius: var(--radius-md);
  color: var(--text-primary);
  font-size: var(--text-sm);
  font-family: var(--font-mono);
  outline: none;
  transition: border-color var(--duration-fast);
}

.form-input:focus {
  border-color: var(--accent);
}

.form-select {
  padding: var(--space-1) var(--space-2);
  background: var(--bg-surface);
  border: 1px solid var(--border-default);
  border-radius: var(--radius-md);
  color: var(--text-primary);
  font-size: var(--text-sm);
  outline: none;
  cursor: pointer;
}

.form-actions {
  display: flex;
  gap: var(--space-2);
  align-items: center;
  padding-bottom: 1px;
}

.env-name {
  font-weight: var(--weight-medium);
  font-size: var(--text-sm);
  color: var(--text-primary);
  font-family: var(--font-mono);
}

.env-value {
  font-size: var(--text-xs);
  color: var(--text-secondary);
  font-family: var(--font-mono);
  word-break: break-all;
}

.env-type {
  font-size: var(--text-xs);
  color: var(--text-muted);
}

.inline-input {
  padding: var(--space-05) var(--space-2);
  background: var(--bg-surface);
  border: 1px solid var(--accent);
  border-radius: var(--radius-md);
  color: var(--text-primary);
  font-size: var(--text-xs);
  font-family: var(--font-mono);
  outline: none;
  width: 100%;
  box-sizing: border-box;
}

.row-actions {
  display: flex;
  gap: var(--space-1);
}

.result-banner {
  margin-bottom: var(--space-4);
}
</style>
