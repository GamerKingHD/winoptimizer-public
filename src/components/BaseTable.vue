<script setup lang="ts">
import { ref, computed, useSlots } from 'vue'

const props = withDefaults(defineProps<{
  columns: { key: string; label: string; sortable?: boolean; width?: string; align?: 'left' | 'right' | 'center' }[]
  rows: Record<string, any>[]
  maxHeight?: string
  striped?: boolean
  hoverable?: boolean
  compact?: boolean
  loading?: boolean
  emptyText?: string
}>(), {
  maxHeight: '500px',
  striped: true,
  hoverable: true,
  compact: false,
  loading: false,
  emptyText: 'No data available',
})

const emit = defineEmits<{
  'row-click': [row: Record<string, any>, index: number]
}>()

const sortKey = ref<string>('')
const sortDir = ref<'asc' | 'desc'>('asc')

function toggleSort(key: string) {
  if (sortKey.value === key) {
    sortDir.value = sortDir.value === 'asc' ? 'desc' : 'asc'
  } else {
    sortKey.value = key
    sortDir.value = 'asc'
  }
}

const sortedRows = computed(() => {
  if (!sortKey.value) return props.rows
  return [...props.rows].sort((a, b) => {
    const aVal = a[sortKey.value]
    const bVal = b[sortKey.value]
    if (aVal == null) return 1
    if (bVal == null) return -1
    if (typeof aVal === 'number' && typeof bVal === 'number') {
      return sortDir.value === 'asc' ? aVal - bVal : bVal - aVal
    }
    const cmp = String(aVal).localeCompare(String(bVal))
    return sortDir.value === 'asc' ? cmp : -cmp
  })
})

const slots = useSlots()
</script>

<template>
  <div class="table-container" :style="{ maxHeight }" :aria-busy="loading">
    <div class="table-wrapper">
      <!-- Loading overlay -->
      <div v-if="loading" class="table-loading" role="status" aria-live="polite">
        <div class="loading-spinner" />
        <span>Loading...</span>
      </div>

      <!-- Table -->
      <table class="base-table" :class="{ 'is-compact': compact, 'is-striped': striped, 'is-hoverable': hoverable }">
        <thead>
          <tr>
            <th
              v-for="col in columns"
              :key="col.key"
              :style="{ textAlign: col.align || 'left', width: col.width }"
              :class="{ 'is-sortable': col.sortable, [`align-${col.align || 'left'}`]: true }"
              :aria-sort="col.sortable
                ? sortKey === col.key ? (sortDir === 'asc' ? 'ascending' : 'descending') : 'none'
                : undefined"
            >
              <button v-if="col.sortable" type="button" class="sort-button" :aria-label="`Sort by ${col.label}`" @click="toggleSort(col.key)">
                {{ col.label }}
                <span class="sort-icon" aria-hidden="true">
                  <svg v-if="sortKey === col.key && sortDir === 'asc'" width="10" height="10" viewBox="0 0 10 10" fill="currentColor">
                    <path d="M5 2l4 6H1z"/>
                  </svg>
                  <svg v-else-if="sortKey === col.key && sortDir === 'desc'" width="10" height="10" viewBox="0 0 10 10" fill="currentColor">
                    <path d="M5 8l4-6H1z"/>
                  </svg>
                  <svg v-else width="10" height="10" viewBox="0 0 10 10" fill="none" stroke="currentColor" stroke-width="1.5" opacity="0.3">
                    <path d="M3 7l2-4 2 4"/>
                  </svg>
                </span>
              </button>
              <template v-else>{{ col.label }}</template>
            </th>
            <th v-if="slots['row-actions']" class="actions-col" />
          </tr>
        </thead>
        <tbody>
          <tr v-if="sortedRows.length === 0 && !loading">
            <td :colspan="columns.length + (slots['row-actions'] ? 1 : 0)" class="empty-row">
              {{ emptyText }}
            </td>
          </tr>
          <tr
            v-for="(row, idx) in sortedRows"
            :key="row._id || idx"
            @click="emit('row-click', row, idx)"
          >
            <td
              v-for="col in columns"
              :key="col.key"
              :style="{ textAlign: col.align || 'left' }"
              :class="`align-${col.align || 'left'}`"
            >
              <slot :name="`cell-${col.key}`" :row="row" :value="row[col.key]" :index="idx">
                {{ row[col.key] ?? '-' }}
              </slot>
            </td>
            <td v-if="slots['row-actions']" class="actions-cell" @click.stop>
              <slot name="row-actions" :row="row" :index="idx" />
            </td>
          </tr>
        </tbody>
      </table>
    </div>
  </div>
</template>

<style scoped>
.table-container {
  overflow-y: auto;
  border-radius: var(--radius-xl);
  border: 1px solid var(--border-default);
  background: var(--bg-elevated);
  position: relative;
}

.table-wrapper {
  position: relative;
  overflow-x: auto;
}

.base-table {
  width: 100%;
  border-collapse: collapse;
  font-size: var(--text-sm);
  text-align: left;
}

/* Header */
thead {
  position: sticky;
  top: 0;
  z-index: var(--z-base);
}

th {
  padding: var(--space-3) var(--space-4);
  font-weight: var(--weight-semibold);
  font-size: var(--text-xs);
  color: var(--text-muted);
  text-transform: uppercase;
  letter-spacing: var(--tracking-wide);
  background: var(--bg-surface);
  border-bottom: 1px solid var(--border-default);
  white-space: nowrap;
}

th.is-sortable {
  cursor: pointer;
  user-select: none;
  transition: color var(--duration-fast);
}

th.is-sortable:hover {
  color: var(--text-primary);
}

.sort-button {
  display: inline-flex;
  align-items: center;
  padding: 0;
  border: 0;
  background: transparent;
  color: inherit;
  font: inherit;
  text-transform: inherit;
  letter-spacing: inherit;
  cursor: pointer;
}

.sort-button:focus-visible {
  outline: 2px solid var(--accent);
  outline-offset: 2px;
}

.sort-icon {
  display: inline-flex;
  vertical-align: middle;
  margin-left: var(--space-1);
}

/* Body */
td {
  padding: var(--space-3) var(--space-4);
  border-bottom: 1px solid var(--border-subtle);
  color: var(--text-secondary);
  font-size: var(--text-sm);
  transition: background var(--duration-fast);
}

.align-right { text-align: right; }
.align-center { text-align: center; }

/* Striped rows */
.is-striped tbody tr:nth-child(even) {
  background: var(--color-surface-muted);
}

/* Hover */
.is-hoverable tbody tr:hover {
  background: var(--bg-hover);
}

.is-hoverable tbody tr {
  cursor: pointer;
}

/* Compact */
.is-compact td,
.is-compact th {
  padding: var(--space-2) var(--space-3);
}

/* content-visibility: auto on table rows — browser skips rendering off-screen rows.
   Gives virtual-scroll-like performance without a library. */
.base-table tbody tr {
  content-visibility: auto;
  contain-intrinsic-size: auto 44px;
}

/* Empty state */
.empty-row {
  text-align: center;
  padding: var(--space-12) var(--space-4);
  color: var(--text-muted);
  font-size: var(--text-md);
}

/* Actions */
.actions-col {
  width: 1%;
  white-space: nowrap;
}

.actions-cell {
  white-space: nowrap;
}

/* Loading */
.table-loading {
  position: absolute;
  inset: 0;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: var(--space-3);
  background: var(--color-bg-app);
  z-index: var(--z-sticky);
  color: var(--text-secondary);
  font-size: var(--text-sm);
}

.loading-spinner {
  width: 24px;
  height: 24px;
  border: 2px solid var(--border-default);
  border-top-color: var(--accent);
  border-radius: var(--radius-full);
  animation: spin 0.6s linear infinite;
}
</style>
