<script setup lang="ts">
import { computed, onActivated, ref } from 'vue'
import type { HostEntry } from '../api/tauri'
import * as api from '../api/tauri'
import type { OperationPlan } from '../api/operation'
import { confirmAndRequestElevation } from '../api/operation'
import { createHostsPlan, executeHostsPlan } from '../api/hosts'
import AppHeader from '../components/AppHeader.vue'
import BaseCard from '../components/BaseCard.vue'
import BaseButton from '../components/BaseButton.vue'
import BaseTable from '../components/BaseTable.vue'
import BaseBadge from '../components/BaseBadge.vue'
import BaseToggle from '../components/BaseToggle.vue'
import EmptyState from '../components/EmptyState.vue'
import ResultsBanner from '../components/ResultsBanner.vue'
import ConfirmDialog from '../components/ConfirmDialog.vue'
import { errorMessage } from '../utils/feedback'

const entries = ref<HostEntry[]>([])
const loading = ref(false)
const saving = ref(false)
const saveResult = ref<'success' | 'warning' | null>(null)
const errorDetail = ref('')

// New entry form
const newIp = ref('')
const newHostname = ref('')
const newComment = ref('')
const showAddForm = ref(false)
const editingId = ref<number | null>(null)
const editIp = ref('')
const editHostname = ref('')
const editComment = ref('')
const showSaveConfirm = ref(false)
const pendingPlan = ref<OperationPlan | null>(null)

const columns = [
  { key: 'ip', label: 'IP Address', sortable: true, width: '160px' },
  { key: 'hostnames', label: 'Hostname(s)', sortable: true },
  { key: 'comment', label: 'Comment', sortable: true },
  { key: 'enabled', label: 'Active', width: '90px', align: 'center' as const },
]

const tableRows = computed(() =>
  entries.value.map(e => ({
    _id: e.id,
    ip: e.ip,
    hostnames: e.hostnames.join(' '),
    comment: e.comment || '',
    enabled: e.enabled,
  }))
)

async function loadEntries() {
  loading.value = true
  try {
    entries.value = await api.listHostsEntries()
    errorDetail.value = ''
  } catch (e) {
    entries.value = []
    errorDetail.value = errorMessage(e)
  } finally {
    loading.value = false
  }
}

const hasLoaded = ref(false)
onActivated(() => {
  if (hasLoaded.value) return
  hasLoaded.value = true
  void loadEntries()
})

function toggleEntry(entry: HostEntry) {
  entry.enabled = !entry.enabled
}

async function saveFile() {
  showSaveConfirm.value = false
  const plan = pendingPlan.value
  if (!plan) return
  saving.value = true
  saveResult.value = null
  try {
    if (!(await api.checkElevated())) {
      await confirmAndRequestElevation(plan)
      errorDetail.value = 'Administrator approval requested. The exact confirmed save will continue after elevation.'
      saveResult.value = 'success'
      return
    }
    const execution = await executeHostsPlan(plan)
    if (execution.result.failed > 0) {
      throw new Error(execution.message)
    }
    saveResult.value = 'success'
    errorDetail.value = ''
    setTimeout(() => { saveResult.value = null }, 4000)
  } catch (e) {
    errorDetail.value = errorMessage(e)
    saveResult.value = 'warning'
  } finally {
    saving.value = false
    pendingPlan.value = null
  }
}

async function reviewSave() {
  saving.value = true
  saveResult.value = null
  try {
    pendingPlan.value = await createHostsPlan(entries.value)
    showSaveConfirm.value = true
  } catch (e) {
    errorDetail.value = errorMessage(e)
    saveResult.value = 'warning'
  } finally {
    saving.value = false
  }
}

function addEntry() {
  const ip = newIp.value.trim()
  const hostname = newHostname.value.trim()
  if (!ip || !hostname) return

  const maxId = entries.value.reduce((max, e) => Math.max(max, e.id), 0)
  entries.value.push({
    id: maxId + 1,
    ip,
    hostnames: hostname.split(/\s+/).filter(Boolean),
    comment: newComment.value.trim() || null,
    enabled: true,
    raw_line: null,
  })

  newIp.value = ''
  newHostname.value = ''
  newComment.value = ''
  showAddForm.value = false
}

function startEdit(entry: HostEntry) {
  editingId.value = entry.id
  editIp.value = entry.ip
  editHostname.value = entry.hostnames.join(' ')
  editComment.value = entry.comment || ''
}

function saveEdit() {
  const id = editingId.value
  if (id === null) return
  const entry = entries.value.find(e => e.id === id)
  if (!entry) return
  entry.ip = editIp.value.trim()
  entry.hostnames = editHostname.value.trim().split(/\s+/).filter(Boolean)
  entry.comment = editComment.value.trim() || null
  editingId.value = null
}

function removeEntry(entry: HostEntry) {
  entries.value = entries.value.filter(e => e.id !== entry.id)
}

</script>

<template>
  <div class="hosts-editor">
    <AppHeader />

    <div class="toolbar">
      <div class="toolbar-left">
        <BaseButton variant="primary" icon="refresh" @click="loadEntries" :loading="loading" size="sm">
          Refresh
        </BaseButton>
        <BaseButton variant="accent" icon="plus" @click="showAddForm = !showAddForm" size="sm">
          Add Entry
        </BaseButton>
        <BaseBadge variant="info" size="sm">{{ entries.length }} entries</BaseBadge>
      </div>
      <div class="toolbar-right">
        <BaseButton variant="accent" icon="save" @click="reviewSave" :loading="saving" :disabled="entries.length === 0">
          Save to File
        </BaseButton>
        <BaseBadge variant="warning" size="sm">Experimental</BaseBadge>
      </div>
    </div>

    <!-- Add Entry Form -->
    <div v-if="showAddForm" class="add-form">
      <BaseCard variant="elevated" padding="md">
        <div class="form-row">
          <div class="form-field">
            <label class="form-label">IP Address</label>
            <input v-model="newIp" class="form-input" placeholder="127.0.0.1" @keyup.enter="addEntry" />
          </div>
          <div class="form-field">
            <label class="form-label">Hostname</label>
            <input v-model="newHostname" class="form-input" placeholder="myapp.local" @keyup.enter="addEntry" />
          </div>
          <div class="form-field flex">
            <label class="form-label">Comment</label>
            <input v-model="newComment" class="form-input" placeholder="Optional comment" @keyup.enter="addEntry" />
          </div>
          <div class="form-actions">
            <BaseButton variant="ghost" size="sm" @click="showAddForm = false">Cancel</BaseButton>
            <BaseButton variant="primary" size="sm" @click="addEntry">Add</BaseButton>
          </div>
        </div>
      </BaseCard>
    </div>

    <!-- Table -->
    <BaseTable
      v-if="entries.length > 0"
      :columns="columns"
      :rows="tableRows"
      :loading="loading"
      max-height="480px"
      compact
    >
      <template #cell-ip="{ row }">
        <template v-if="editingId === row._id">
          <input v-model="editIp" class="inline-input" @keyup.enter="saveEdit" @keyup.escape="editingId = null" />
        </template>
        <span v-else class="entry-ip">{{ row.ip }}</span>
      </template>
      <template #cell-hostnames="{ row }">
        <template v-if="editingId === row._id">
          <input v-model="editHostname" class="inline-input" placeholder="host1 host2" @keyup.enter="saveEdit" @keyup.escape="editingId = null" />
        </template>
        <span v-else class="entry-hostname">{{ row.hostnames }}</span>
      </template>
      <template #cell-comment="{ row }">
        <template v-if="editingId === row._id">
          <input v-model="editComment" class="inline-input" placeholder="Comment" @keyup.enter="saveEdit" @keyup.escape="editingId = null" />
        </template>
        <span v-else class="entry-comment">{{ row.comment || '-' }}</span>
      </template>
      <template #cell-enabled="{ row }">
        <BaseToggle :modelValue="row.enabled" @update:modelValue="toggleEntry(row)" size="sm" />
      </template>
      <template #row-actions="{ row }">
        <div class="row-actions">
          <template v-if="editingId === row._id">
            <BaseButton variant="primary" size="sm" icon="check" @click="saveEdit">Save</BaseButton>
          </template>
          <template v-else>
            <BaseButton variant="ghost" size="sm" icon="edit" @click="startEdit(row)">Edit</BaseButton>
            <BaseButton variant="danger" size="sm" icon="x" @click="removeEntry(row)">Remove</BaseButton>
          </template>
        </div>
      </template>
    </BaseTable>

    <EmptyState
      v-else-if="!loading"
      icon="info"
      title="No Entries"
      description="Click Refresh to load the hosts file, or add a new entry before saving."
      action-label="Add Entry"
      @action="showAddForm = true"
    />

    <!-- Save Result Banner -->
    <ResultsBanner
      v-if="saveResult === 'success'"
      variant="success"
      title="Hosts file saved"
      :description="errorDetail || 'Changes were written to the canonical hosts file and verified by re-read.'"
      dismissable
      class="save-banner"
    />
    <ResultsBanner
      v-if="saveResult === 'warning'"
      variant="warning"
      title="Save failed"
      :description="errorDetail || 'Could not write to the hosts file. Try running WinOptimizer as Administrator.'"
      dismissable
      class="save-banner"
    />

    <ConfirmDialog
      :open="showSaveConfirm"
      :operation-plan="pendingPlan"
      title="Save Hosts File?"
      description="This immutable Experimental plan writes the canonical hosts file, rejects changed-since-preview content, and verifies the result. Exact ACL and timestamp restoration is not yet proven."
      :items="[
        { label: 'Target file', value: 'C:\\Windows\\System32\\drivers\\etc\\hosts', risk: 'high' },
        { label: 'Entries to write', value: String(entries.length), risk: 'medium' },
        { label: 'Undo', value: 'Unavailable until metadata restoration is proven', risk: 'high' },
        { label: 'DNS cache', value: 'Existing cached results may remain temporarily', risk: 'medium' },
      ]"
      confirm-label="Save Hosts File"
      cancel-label="Cancel"
      variant="danger"
      icon="shield"
      :loading="saving"
      @confirm="saveFile"
      @cancel="showSaveConfirm = false; pendingPlan = null"
    />
  </div>
</template>

<style scoped>
.hosts-editor {
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

.toolbar-left, .toolbar-right {
  display: flex;
  align-items: center;
  gap: var(--space-2);
}

/* Add form */
.add-form {
  margin-bottom: var(--space-4);
  animation: fadeInUp var(--duration-normal) var(--ease-out);
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
  flex: 0 0 140px;
}

.form-field.flex {
  flex: 1;
  min-width: 150px;
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

.form-actions {
  display: flex;
  gap: var(--space-2);
  align-items: center;
  padding-bottom: 1px;
}

/* Table styling */
.entry-ip {
  font-family: var(--font-mono);
  font-size: var(--text-sm);
  color: var(--accent);
  font-weight: var(--weight-medium);
}

.entry-hostname {
  font-weight: var(--weight-medium);
  font-size: var(--text-sm);
  color: var(--text-primary);
}

.entry-comment {
  font-size: var(--text-xs);
  color: var(--text-muted);
}

.inline-input {
  padding: var(--space-05) var(--space-2);
  background: var(--bg-surface);
  border: 1px solid var(--accent);
  border-radius: var(--radius-md);
  color: var(--text-primary);
  font-size: var(--text-sm);
  font-family: var(--font-mono);
  outline: none;
  width: 100%;
  box-sizing: border-box;
}

.row-actions {
  display: flex;
  gap: var(--space-1);
}

.save-banner {
  margin-top: var(--space-4);
}

@media (max-width: 700px) {
  .toolbar,
  .toolbar-left,
  .toolbar-right {
    flex-direction: column;
    align-items: stretch;
  }

  .form-field,
  .form-field.flex {
    flex: 1 1 100%;
    min-width: 0;
  }
}
</style>
