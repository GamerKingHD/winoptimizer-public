<script setup lang="ts">
import { computed, ref } from 'vue'
import type { CleanItem } from '../api/tauri'
import { formatBytes } from '../utils/format'
import BaseBadge from './BaseBadge.vue'

const props = defineProps<{
  items: CleanItem[]
}>()

const emit = defineEmits<{
  'update:selected': [paths: Set<string>]
}>()

const selected = ref<Set<string>>(new Set())
const expandedGroups = ref<Set<string>>(new Set())
const expandedSubGroups = ref<Set<string>>(new Set())

// Group items by group_key → sub_group
const groups = computed(() => {
  const map = new Map<string, Map<string, CleanItem[]>>()
  for (const item of props.items) {
    const gk = item.group_key || 'Other'
    const sg = item.sub_group || 'Root'
    if (!map.has(gk)) map.set(gk, new Map())
    const subMap = map.get(gk)!
    if (!subMap.has(sg)) subMap.set(sg, [])
    subMap.get(sg)!.push(item)
  }
  // Sort groups and sub-groups by total size descending
  const sorted = new Map<string, Map<string, CleanItem[]>>()
  const groupEntries = [...map.entries()].map(([gk, subMap]) => {
    const groupTotal = [...subMap.values()].flat().reduce((s, i) => s + i.size_bytes, 0)
    return { gk, subMap, groupTotal }
  })
  groupEntries.sort((a, b) => b.groupTotal - a.groupTotal)
  for (const { gk, subMap } of groupEntries) {
    const sortedSub = new Map<string, CleanItem[]>()
    const subEntries = [...subMap.entries()].map(([sg, items]) => {
      return { sg, items, total: items.reduce((s, i) => s + i.size_bytes, 0) }
    })
    subEntries.sort((a, b) => b.total - a.total)
    for (const { sg, items } of subEntries) {
      items.sort((a, b) => b.size_bytes - a.size_bytes)
      sortedSub.set(sg, items)
    }
    sorted.set(gk, sortedSub)
  }
  return sorted
})

// Derived: total counts/sizes per group
function groupStats(subMap: Map<string, CleanItem[]>) {
  let count = 0
  let size = 0
  for (const items of subMap.values()) {
    count += items.length
    size += items.reduce((s, i) => s + i.size_bytes, 0)
  }
  return { count, size }
}

function subGroupStats(items: CleanItem[]) {
  return {
    count: items.length,
    size: items.reduce((s, i) => s + i.size_bytes, 0),
  }
}

// Checkbox state helpers
function groupCheckedState(gk: string, subMap: Map<string, CleanItem[]>): 'all' | 'some' | 'none' {
  let total = 0
  let checked = 0
  for (const items of subMap.values()) {
    for (const item of items) {
      total++
      if (selected.value.has(item.path)) checked++
    }
  }
  if (checked === 0) return 'none'
  if (checked === total) return 'all'
  return 'some'
}

function subGroupCheckedState(items: CleanItem[]): 'all' | 'some' | 'none' {
  let checked = 0
  for (const item of items) {
    if (selected.value.has(item.path)) checked++
  }
  if (checked === 0) return 'none'
  if (checked === items.length) return 'all'
  return 'some'
}

function toggleGroup(gk: string, subMap: Map<string, CleanItem[]>) {
  const state = groupCheckedState(gk, subMap)
  const newSet = new Set(selected.value)
  if (state === 'all') {
    for (const items of subMap.values()) {
      for (const item of items) newSet.delete(item.path)
    }
  } else {
    for (const items of subMap.values()) {
      for (const item of items) newSet.add(item.path)
    }
  }
  selected.value = newSet
  emitUpdate()
}

function toggleSubGroup(items: CleanItem[]) {
  const state = subGroupCheckedState(items)
  const newSet = new Set(selected.value)
  if (state === 'all') {
    for (const item of items) newSet.delete(item.path)
  } else {
    for (const item of items) newSet.add(item.path)
  }
  selected.value = newSet
  emitUpdate()
}

function toggleItem(path: string) {
  const newSet = new Set(selected.value)
  if (newSet.has(path)) newSet.delete(path)
  else newSet.add(path)
  selected.value = newSet
  emitUpdate()
}

function emitUpdate() {
  emit('update:selected', selected.value)
}

function toggleExpandGroup(gk: string) {
  const newSet = new Set(expandedGroups.value)
  if (newSet.has(gk)) newSet.delete(gk)
  else newSet.add(gk)
  expandedGroups.value = newSet
}

function toggleExpandSubGroup(key: string) {
  const newSet = new Set(expandedSubGroups.value)
  if (newSet.has(key)) newSet.delete(key)
  else newSet.add(key)
  expandedSubGroups.value = newSet
}

// Expose reset for external clearing
function resetSelection() {
  selected.value = new Set()
  emitUpdate()
}

defineExpose({ resetSelection, selected })
</script>

<template>
  <div class="cleanup-tree">
    <template v-for="[gk, subMap] in groups" :key="gk">
      <!-- Group header -->
      <div class="tree-group">
        <button
          class="tree-row group-row"
          @click="toggleExpandGroup(gk)"
          :aria-expanded="expandedGroups.has(gk)"
        >
          <span class="expand-arrow" :class="{ open: expandedGroups.has(gk) }">
            <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" width="14" height="14">
              <path d="M9 18l6-6-6-6" />
            </svg>
          </span>
          <label class="tree-check" @click.stop>
            <input
              type="checkbox"
              :checked="groupCheckedState(gk, subMap) === 'all'"
              :indeterminate="groupCheckedState(gk, subMap) === 'some'"
              @change="toggleGroup(gk, subMap)"
            />
          </label>
          <span class="tree-label">{{ gk }}</span>
          <span class="tree-meta">
            <BaseBadge variant="default" size="sm">{{ groupStats(subMap).count }} files</BaseBadge>
            <BaseBadge variant="accent" size="sm">{{ formatBytes(groupStats(subMap).size) }}</BaseBadge>
          </span>
        </button>

        <!-- Sub-groups -->
        <div v-if="expandedGroups.has(gk)" class="tree-children">
          <template v-for="[sg, items] in subMap" :key="`${gk}/${sg}`">
            <div class="tree-subgroup">
              <button
                class="tree-row subgroup-row"
                @click="toggleExpandSubGroup(`${gk}/${sg}`)"
                :aria-expanded="expandedSubGroups.has(`${gk}/${sg}`)"
              >
                <span class="expand-arrow sub-arrow" :class="{ open: expandedSubGroups.has(`${gk}/${sg}`) }">
                  <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" width="12" height="12">
                    <path d="M9 18l6-6-6-6" />
                  </svg>
                </span>
                <label class="tree-check" @click.stop>
                  <input
                    type="checkbox"
                    :checked="subGroupCheckedState(items) === 'all'"
                    :indeterminate="subGroupCheckedState(items) === 'some'"
                    @change="toggleSubGroup(items)"
                  />
                </label>
                <span class="tree-label sub-label">{{ sg }}</span>
                <span class="tree-meta">
                  <BaseBadge variant="default" size="sm">{{ subGroupStats(items).count }} files</BaseBadge>
                  <BaseBadge variant="accent" size="sm">{{ formatBytes(subGroupStats(items).size) }}</BaseBadge>
                </span>
              </button>

              <!-- Individual files -->
              <div v-if="expandedSubGroups.has(`${gk}/${sg}`)" class="tree-children file-list">
                <div
                  v-for="item in items.slice(0, 200)"
                  :key="item.path"
                  class="tree-row file-row"
                  @click="toggleItem(item.path)"
                >
                  <label class="tree-check" @click.stop>
                    <input
                      type="checkbox"
                      :checked="selected.has(item.path)"
                      @change="toggleItem(item.path)"
                    />
                  </label>
                  <span class="file-name" :title="item.path">{{ item.path.split('\\').pop() || item.path }}</span>
                  <span class="file-path">{{ item.path }}</span>
                  <span class="file-size">{{ formatBytes(item.size_bytes) }}</span>
                  <span class="file-date">{{ item.last_accessed ? new Date(item.last_accessed).toLocaleDateString() : '-' }}</span>
                </div>
                <div v-if="items.length > 200" class="file-truncated">
                  +{{ items.length - 200 }} more files (use Search to narrow down)
                </div>
              </div>
            </div>
          </template>
        </div>
      </div>
    </template>
  </div>
</template>

<style scoped>
.cleanup-tree {
  background: var(--bg-elevated);
  border: 1px solid var(--border-default);
  border-radius: var(--radius-xl);
  overflow: hidden;
}

.tree-group {
  border-bottom: 1px solid var(--border-subtle);
}
.tree-group:last-child { border-bottom: none; }

.tree-row {
  display: flex;
  align-items: center;
  gap: var(--space-2);
  width: 100%;
  padding: var(--space-2) var(--space-3);
  border: none;
  background: transparent;
  cursor: pointer;
  text-align: left;
  color: var(--text-primary);
  font-size: var(--text-sm);
  transition: background var(--duration-fast);
}
.tree-row:hover { background: var(--bg-hover); }

.group-row {
  padding: var(--space-3) var(--space-3);
  font-weight: var(--weight-semibold);
  background: var(--bg-surface);
}

.subgroup-row {
  padding: var(--space-2) var(--space-3) var(--space-2) var(--space-6);
}

.file-row {
  padding: var(--space-1) var(--space-3) var(--space-1) calc(var(--space-10) + var(--space-2));
  font-size: var(--text-xs);
  cursor: pointer;
}

.expand-arrow {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 18px;
  height: 18px;
  color: var(--text-muted);
  transition: transform var(--duration-fast);
  flex-shrink: 0;
}
.expand-arrow.open {
  transform: rotate(90deg);
}
.sub-arrow { width: 14px; height: 14px; }

.tree-check {
  display: flex;
  align-items: center;
  flex-shrink: 0;
}
.tree-check input[type="checkbox"] {
  width: 16px;
  height: 16px;
  accent-color: var(--accent);
  cursor: pointer;
}

.tree-label {
  flex: 0 0 auto;
  max-width: 280px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.sub-label { font-weight: var(--weight-medium); }

.tree-meta {
  display: flex;
  gap: var(--space-2);
  margin-left: auto;
  flex-shrink: 0;
}

.tree-children {
  animation: slideDown var(--duration-fast) var(--ease-out);
}

.file-list { max-height: 420px; overflow-y: auto; }

.file-name {
  flex: 0 0 auto;
  max-width: 200px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-family: var(--font-mono);
  font-size: var(--text-xs);
  color: var(--text-primary);
}
.file-path {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  color: var(--text-muted);
  font-family: var(--font-mono);
  font-size: var(--text-xs);
}
.file-size {
  flex-shrink: 0;
  width: 80px;
  text-align: right;
  font-variant-numeric: tabular-nums;
  color: var(--text-primary);
  font-size: var(--text-xs);
}
.file-date {
  flex-shrink: 0;
  width: 90px;
  text-align: right;
  color: var(--text-muted);
  font-size: var(--text-xs);
}

.file-truncated {
  padding: var(--space-2) var(--space-3) var(--space-2) calc(var(--space-10) + var(--space-2));
  color: var(--text-muted);
  font-size: var(--text-xs);
  font-style: italic;
}

@keyframes slideDown {
  from { opacity: 0; transform: translateY(-4px); }
  to { opacity: 1; transform: translateY(0); }
}
</style>
