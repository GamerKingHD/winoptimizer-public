<script setup lang="ts">
import { ref, computed, shallowRef, nextTick, onBeforeUnmount, onMounted, watch } from 'vue'
import { isTauri } from '@tauri-apps/api/core'
import type { DiskEntryDetails, DiskNode, DiskScanResult, LargestDiskFile, ScanStatus } from '../api/tauri'
import * as api from '../api/tauri'
import { onProgress, type ProgressEvent } from '../api/tauri'
import AppHeader from '../components/AppHeader.vue'
import BaseCard from '../components/BaseCard.vue'
import BaseButton from '../components/BaseButton.vue'
import BaseTabBar from '../components/BaseTabBar.vue'
import BaseBadge from '../components/BaseBadge.vue'
import BaseProgress from '../components/BaseProgress.vue'
import BaseSearchInput from '../components/BaseSearchInput.vue'
import EmptyState from '../components/EmptyState.vue'
import ResultsBanner from '../components/ResultsBanner.vue'
import ConfirmDialog from '../components/ConfirmDialog.vue'
import { failure, type ActionFeedback } from '../utils/feedback'
import { formatBytes } from '../utils/format'
import { describeScanScope } from '../utils/scanScope'
const activeTab = ref<string>('usage')
const scanPath = ref('C:\\')
const diskTree = shallowRef<DiskNode | null>(null)
const diskResult = shallowRef<DiskScanResult | null>(null)
const diskChildren = shallowRef<DiskNode[]>([])
const nestedChildren = shallowRef<Record<number, DiskNode[]>>({})
const nestedHasMoreChildren = shallowRef<Record<number, boolean>>({})
const nestedNextOffsets = shallowRef<Record<number, number>>({})
const expandedDirectoryIds = ref<Set<number>>(new Set())
const nestedLoadingIds = shallowRef<Set<number>>(new Set())
const treemapChildren = shallowRef<DiskNode[]>([])
const currentParentId = ref<number | null>(0)
const currentParentNode = shallowRef<DiskNode | null>(null)
const childOffset = ref(0)
const hasMoreChildren = ref(false)
const childLoading = ref(false)
const sortMode = ref<'allocated' | 'logical' | 'unique' | 'name'>('allocated')
const displayMode = ref<'allocated' | 'logical' | 'unique'>('allocated')
const entryDetails = shallowRef<DiskEntryDetails | null>(null)
const detailsLoading = ref(false)
const largestFiles = shallowRef<LargestDiskFile[]>([])
const largestFilesLoading = ref(false)
const largestFileSort = ref<'allocated' | 'logical' | 'unique'>('allocated')
const allocationIsEstimate = computed(() => diskResult.value?.engine === 'win32_fallback')
const allocationLabel = computed(() =>
  allocationIsEstimate.value ? 'Allocated (estimated)' : 'Allocated',
)
// The backend only selects the whole-volume MFT engine for an exact drive root;
// any other path is a subtree walked by the Win32 fallback. The label must
// describe that scope instead of always claiming "Entire volume".
const diskScopeLabel = computed(() => describeScanScope(scanPath.value))
const loading = ref(false)
const cancelling = ref(false)
const minSizeMb = ref(1)
const DISK_CHILD_PAGE_SIZE = 200
const TREEMAP_CHILD_LIMIT = 1000
const MAX_CACHED_CHILDREN_PER_PARENT = 1000
const MAX_RENDERED_TREE_ROWS = 500
const DISK_SCAN_BUDGET_MS = 30_000
const GUI_RENDER_BUDGET_MS = 2_000
const feedback = ref<ActionFeedback | null>(null)
const scanProgress = ref(0)
const scanMessage = ref('')
const scanPhase = ref('')
const guiScanElapsedMs = ref<number | null>(null)
const treemapHost = ref<HTMLElement | null>(null)
const treemapCanvas = ref<HTMLCanvasElement | null>(null)
const treemapBlocks = shallowRef<TreemapBlock[]>([])
const hoveredTreemapIndex = ref<number | null>(null)
let treemapResizeObserver: ResizeObserver | null = null
let childRequestSequence = 0
let latestNavigationChildRequest = 0
const latestNestedChildRequests = new Map<number, number>()
let largestFilesRequestSequence = 0
let detailsRequestSequence = 0
let diskScanRequestSequence = 0
interface TreemapBlock {
  node: DiskNode | null
  label: string
  value: number
  isOther: boolean
  index: number
  x: number
  y: number
  width: number
  height: number
  color: string
}

interface DiskTreeRow {
  kind: 'node' | 'load_more'
  node: DiskNode | null
  depth: number
  parentId: number | null
}
const tabs = [
  { id: 'usage', label: 'Disk Usage', icon: 'bar-chart' },
]

// Colors for treemap bars
const barColors = [
  '#00d4aa', '#00b4d8', '#6c63ff', '#f59e0b',
  '#ef4444', '#3b82f6', '#22c55e', '#ec4899',
  '#14b8a6', '#8b5cf6', '#f97316', '#06b6d4',
]

function getColor(index: number): string {
  return barColors[index % barColors.length]
}

function statusVariant(status: ScanStatus): 'success' | 'warning' | 'info' | 'danger' {
  if (status === 'complete') return 'success'
  if (status === 'partial') return 'warning'
  if (status === 'failed') return 'danger'
  return 'info'
}

function statusLabel(status: ScanStatus): string {
  return status.replace('_', ' ')
}

function scanPhaseLabel(phase: string): string {
  return phase.replace(/_/g, ' ')
}

function budgetLabel(elapsedMs: number, budgetMs: number): string {
  return elapsedMs <= budgetMs ? 'within budget' : 'over budget'
}

function showScanStatus(title: string, status: ScanStatus, details: string) {
  if (status === 'complete') return
  feedback.value = {
    type: status === 'partial' ? 'warning' : 'info',
    title: `${title} ${statusLabel(status)}`,
    message: details,
  }
}
function barWidth(size: number, total: number): string {
  return total > 0 ? `${Math.min(100, (size / total) * 100).toFixed(1)}%` : '0%'
}

function nodeMetric(node: DiskNode): number {
  if (displayMode.value === 'logical') return node.logical_size_bytes
  if (displayMode.value === 'unique') return node.unique_allocated_size_bytes
  return node.allocated_size_bytes
}

function summaryMetric(): number {
  if (!diskResult.value) return 0
  if (displayMode.value === 'logical') return diskResult.value.summary.logical_size_bytes
  if (displayMode.value === 'unique') return diskResult.value.summary.unique_allocated_size_bytes
  return diskResult.value.summary.allocated_size_bytes
}

function currentParentMetric(): number {
  return currentParentNode.value ? nodeMetric(currentParentNode.value) : summaryMetric()
}

function diskNodeAttributes(flags: number): string {
  const attributes: string[] = []
  if ((flags & 4) !== 0) attributes.push('Compressed')
  if ((flags & 2) !== 0) attributes.push('Sparse')
  if ((flags & 1) !== 0) attributes.push('Reparse point')
  return attributes.join(' · ') || 'Standard'
}

function childCacheKey(parentId: number | null): number {
  return parentId ?? -1
}

async function loadDiskChildren(parentId: number | null = 0, append = false, updateNavigation = true) {
  if (!diskResult.value) return
  const cacheKey = childCacheKey(parentId)
  const snapshotId = diskResult.value.snapshot_id
  const requestId = ++childRequestSequence
  if (updateNavigation) {
    latestNavigationChildRequest = requestId
    childLoading.value = true
  } else {
    latestNestedChildRequests.set(cacheKey, requestId)
    nestedLoadingIds.value = new Set(nestedLoadingIds.value).add(cacheKey)
  }
  try {
    const cachedChildren = nestedChildren.value[cacheKey] ?? []
    const offset = append
      ? (updateNavigation ? childOffset.value : (nestedNextOffsets.value[cacheKey] ?? cachedChildren.length))
      : 0
    const children = await api.listDiskChildren(
      snapshotId,
      parentId,
      sortMode.value,
      offset,
      DISK_CHILD_PAGE_SIZE,
    )
    const isCurrentRequest = snapshotId === diskResult.value?.snapshot_id
      && (updateNavigation
        ? latestNavigationChildRequest === requestId
        : latestNestedChildRequests.get(cacheKey) === requestId)
    if (!isCurrentRequest) return
    const nextChildren = append ? [...cachedChildren, ...children] : children
    const boundedChildren = nextChildren.length > MAX_CACHED_CHILDREN_PER_PARENT
      ? nextChildren.slice(-MAX_CACHED_CHILDREN_PER_PARENT)
      : nextChildren
    nestedChildren.value = { ...nestedChildren.value, [cacheKey]: boundedChildren }
    nestedHasMoreChildren.value = {
      ...nestedHasMoreChildren.value,
      [cacheKey]: children.length === DISK_CHILD_PAGE_SIZE,
    }
    nestedNextOffsets.value = {
      ...nestedNextOffsets.value,
      [cacheKey]: offset + children.length,
    }
    if (!updateNavigation) return
    diskChildren.value = boundedChildren
    childOffset.value = offset + children.length
    hasMoreChildren.value = children.length === DISK_CHILD_PAGE_SIZE
    currentParentId.value = parentId
    if (!append) {
      currentParentNode.value = parentId === diskResult.value.root.id
        ? diskResult.value.root
        : currentParentNode.value
      const nextTreemapChildren = await api.listDiskChildren(
        snapshotId,
        parentId,
        displayMode.value,
        0,
        TREEMAP_CHILD_LIMIT,
      )
      if (snapshotId !== diskResult.value?.snapshot_id || latestNavigationChildRequest !== requestId) return
      treemapChildren.value = nextTreemapChildren
    }
    await nextTick()
    redrawTreemap()
  } catch (e) {
    const isCurrentRequest = snapshotId === diskResult.value?.snapshot_id
      && (updateNavigation
        ? latestNavigationChildRequest === requestId
        : latestNestedChildRequests.get(cacheKey) === requestId)
    if (isCurrentRequest) feedback.value = failure('Unable to load disk entries', e)
  } finally {
    if (updateNavigation && latestNavigationChildRequest === requestId) {
      childLoading.value = false
    }
    if (!updateNavigation && latestNestedChildRequests.get(cacheKey) === requestId) {
      const nextLoading = new Set(nestedLoadingIds.value)
      nextLoading.delete(cacheKey)
      nestedLoadingIds.value = nextLoading
    }
  }
}

async function openDirectory(node: DiskNode) {
  if (node.is_directory) {
    currentParentNode.value = node
    await loadDiskChildren(node.id)
  }
}

async function toggleDirectory(node: DiskNode) {
  if (!node.is_directory) return
  const nextExpanded = new Set(expandedDirectoryIds.value)
  if (nextExpanded.has(node.id)) {
    nextExpanded.delete(node.id)
  } else {
    nextExpanded.add(node.id)
    if (!(node.id in nestedChildren.value)) {
      await loadDiskChildren(node.id, false, false)
    }
  }
  expandedDirectoryIds.value = nextExpanded
}

async function loadMoreNestedChildren(parentId: number) {
  await loadDiskChildren(parentId, true, false)
}

const visibleTree = computed(() => {
  const rows: DiskTreeRow[] = []
  let truncated = false
  const append = (nodes: DiskNode[], depth: number, parentId: number | null) => {
    for (const node of nodes) {
      if (rows.length >= MAX_RENDERED_TREE_ROWS) {
        truncated = true
        return
      }
      rows.push({ kind: 'node', node, depth, parentId })
      if (node.is_directory && expandedDirectoryIds.value.has(node.id)) {
        const children = nestedChildren.value[node.id] ?? []
        append(children, depth + 1, node.id)
        if (truncated) return
        if (nestedHasMoreChildren.value[node.id]) {
          if (rows.length >= MAX_RENDERED_TREE_ROWS) {
            truncated = true
            return
          }
          rows.push({ kind: 'load_more', node: null, depth: depth + 1, parentId: node.id })
        }
      }
    }
  }
  append(diskChildren.value, 0, currentParentId.value)
  return { rows, truncated }
})

const visibleChildren = computed(() => visibleTree.value.rows)
const treeRowsTruncated = computed(() => visibleTree.value.truncated)

async function loadLargestFiles() {
  if (!diskResult.value) return
  const requestId = ++largestFilesRequestSequence
  const snapshotId = diskResult.value.snapshot_id
  largestFilesLoading.value = true
  try {
    const files = await api.listLargestDiskFiles(
      snapshotId,
      largestFileSort.value,
      50,
    )
    if (requestId === largestFilesRequestSequence && snapshotId === diskResult.value?.snapshot_id) {
      largestFiles.value = files
    }
  } catch (e) {
    if (requestId === largestFilesRequestSequence && snapshotId === diskResult.value?.snapshot_id) {
      feedback.value = failure('Unable to load largest files', e)
    }
  } finally {
    if (requestId === largestFilesRequestSequence) largestFilesLoading.value = false
  }
}

async function showDetailsForId(nodeId: number) {
  if (!diskResult.value) return
  const requestId = ++detailsRequestSequence
  const snapshotId = diskResult.value.snapshot_id
  entryDetails.value = null
  detailsLoading.value = true
  try {
    const details = await api.getDiskEntryDetails(snapshotId, nodeId)
    if (requestId !== detailsRequestSequence || snapshotId !== diskResult.value?.snapshot_id) return
    entryDetails.value = details
    await nextTick()
    document.querySelector('.details-card')?.scrollIntoView({ behavior: 'smooth', block: 'nearest' })
  } catch (e) {
    if (requestId === detailsRequestSequence && snapshotId === diskResult.value?.snapshot_id) {
      feedback.value = failure('Unable to load entry details', e)
    }
  } finally {
    if (requestId === detailsRequestSequence) detailsLoading.value = false
  }
}

async function showDetails(node: DiskNode) {
  await showDetailsForId(node.id)
}

async function analyzeDisk() {
  const requestId = ++diskScanRequestSequence
  childRequestSequence += 1
  latestNavigationChildRequest = childRequestSequence
  latestNestedChildRequests.clear()
  const guiScanStartedAt = performance.now()
  performance.clearMarks('winoptimizer-disk-scan-start')
  performance.clearMarks('winoptimizer-disk-scan-rendered')
  performance.clearMeasures('winoptimizer-disk-scan-gui')
  performance.mark('winoptimizer-disk-scan-start')
  loading.value = true
  scanProgress.value = 0
  scanPhase.value = 'starting'
  scanMessage.value = 'Starting scan...'
  guiScanElapsedMs.value = null
  duplicateScanElapsedMs.value = null
  feedback.value = null
  const unlisten = await onProgress((event: ProgressEvent) => {
    if (event.operation === 'disk_usage_scan') {
      if (requestId !== diskScanRequestSequence) return
      if (event.phase && event.phase !== 'progress') scanPhase.value = event.phase
      if (event.percent != null) scanProgress.value = event.percent
      if (event.message) scanMessage.value = event.message
    }
  })
  try {
    const result = await api.scanDiskUsage(scanPath.value)
    if (requestId !== diskScanRequestSequence) return
    diskResult.value = result
    diskTree.value = result.root
    largestFiles.value = []
    entryDetails.value = null
    nestedChildren.value = {}
    nestedHasMoreChildren.value = {}
    nestedNextOffsets.value = {}
    expandedDirectoryIds.value = new Set()
    treemapChildren.value = []
    currentParentNode.value = result.root
    await Promise.all([loadDiskChildren(result.root.id), loadLargestFiles()])
    if (requestId !== diskScanRequestSequence) return
    loading.value = false
    await nextTick()
    redrawTreemap()
    await new Promise<void>((resolve) => {
      if (typeof window.requestAnimationFrame === 'function') {
        window.requestAnimationFrame(() => resolve())
      } else {
        window.setTimeout(resolve, 0)
      }
    })
    guiScanElapsedMs.value = Math.round(performance.now() - guiScanStartedAt)
    performance.mark('winoptimizer-disk-scan-rendered')
    performance.measure(
      'winoptimizer-disk-scan-gui',
      'winoptimizer-disk-scan-start',
      'winoptimizer-disk-scan-rendered',
    )
    showScanStatus(
      'Disk scan',
      result.status,
      `${result.summary.file_count} files indexed; ${result.diagnostics.skipped_records} records were skipped.`,
    )
  } catch (e) {
    feedback.value = failure('Disk analysis failed', e)
  } finally {
    if (requestId === diskScanRequestSequence) {
      loading.value = false
      cancelling.value = false
    }
    unlisten()
  }
}
async function cancelCurrentScan() {
  cancelling.value = true
  try {
    const op = activeTab.value === 'usage' ? 'disk_usage' : 'find_duplicates'
    await api.cancelScan(op)
  } catch {
    cancelling.value = false
  }
}

function runActiveScan() {
  void analyzeDisk()
}

const scanDisabled = computed(() => {
  if (cancelling.value) return true
  return false
})
const hoveredTreemap = computed(() =>
  hoveredTreemapIndex.value === null ? null : treemapBlocks.value[hoveredTreemapIndex.value] ?? null,
)

const treemapItems = computed(() => {
  const parentTotal = currentParentNode.value ? nodeMetric(currentParentNode.value) : 0
  const items = treemapChildren.value
    .map((node, index) => ({ node, label: node.name, index, value: nodeMetric(node), isOther: false }))
    .filter(item => item.value > 0)
    .slice(0, 96)
  const shownTotal = items.reduce((total, item) => total + item.value, 0)
  const remainder = Math.max(0, parentTotal - shownTotal)
  if (remainder > Math.max(4096, parentTotal * 0.001)) {
    items.push({ node: null, label: 'Other items', index: items.length, value: remainder, isOther: true })
  }
  return items
})

function layoutTreemap(
  items: Array<{ node: DiskNode | null; label: string; index: number; value: number; isOther: boolean }>,
  x: number,
  y: number,
  width: number,
  height: number,
): TreemapBlock[] {
  const blocks: TreemapBlock[] = []

  function place(
    group: typeof items,
    left: number,
    top: number,
    groupWidth: number,
    groupHeight: number,
    groupTotal: number,
  ) {
    if (!group.length || groupWidth <= 0 || groupHeight <= 0 || groupTotal <= 0) return
    if (group.length === 1) {
      const item = group[0]
      blocks.push({
        node: item.node,
        label: item.label,
        value: item.value,
        isOther: item.isOther,
        index: item.index,
        x: left,
        y: top,
        width: groupWidth,
        height: groupHeight,
        color: getColor(item.index),
      })
      return
    }

    let split = 1
    let running = group[0].value
    for (let index = 1; index < group.length; index += 1) {
      const next = running + group[index].value
      if (Math.abs(groupTotal / 2 - next) > Math.abs(groupTotal / 2 - running)) break
      running = next
      split = index + 1
    }
    const first = group.slice(0, split)
    const second = group.slice(split)
    const firstRatio = running / groupTotal
    if (groupWidth >= groupHeight) {
      const firstWidth = groupWidth * firstRatio
      place(first, left, top, firstWidth, groupHeight, running)
      place(second, left + firstWidth, top, groupWidth - firstWidth, groupHeight, groupTotal - running)
    } else {
      const firstHeight = groupHeight * firstRatio
      place(first, left, top, groupWidth, firstHeight, running)
      place(second, left, top + firstHeight, groupWidth, groupHeight - firstHeight, groupTotal - running)
    }
  }

  place(items, x, y, width, height, items.reduce((total, item) => total + item.value, 0))
  return blocks
}

function redrawTreemap() {
  const host = treemapHost.value
  const canvas = treemapCanvas.value
  if (!host || !canvas) return
  const bounds = host.getBoundingClientRect()
  const width = Math.max(1, Math.floor(bounds.width))
  const height = 360
  const dpr = Math.min(window.devicePixelRatio || 1, 2)
  canvas.width = Math.floor(width * dpr)
  canvas.height = Math.floor(height * dpr)
  canvas.style.height = `${height}px`
  const blocks = layoutTreemap(treemapItems.value, 0, 0, width, height)
  treemapBlocks.value = blocks
  const context = canvas.getContext('2d')
  if (!context) return
  context.setTransform(dpr, 0, 0, dpr, 0, 0)
  context.clearRect(0, 0, width, height)
  for (const [blockIndex, block] of blocks.entries()) {
    context.fillStyle = block.isOther ? '#64748b' : block.color
    context.fillRect(block.x + 1, block.y + 1, Math.max(0, block.width - 2), Math.max(0, block.height - 2))
    context.strokeStyle = blockIndex === hoveredTreemapIndex.value ? '#ffffff' : 'rgba(5, 10, 18, 0.72)'
    context.lineWidth = blockIndex === hoveredTreemapIndex.value ? 2 : 1
    context.strokeRect(block.x + 1, block.y + 1, Math.max(0, block.width - 2), Math.max(0, block.height - 2))
    if (block.width >= 56 && block.height >= 24) {
      context.fillStyle = '#ffffff'
      context.font = '12px system-ui, sans-serif'
      context.fillText(block.label.slice(0, Math.max(4, Math.floor(block.width / 8))), block.x + 8, block.y + 17)
    }
  }
}

function treemapBlockAt(event: MouseEvent): TreemapBlock | null {
  const canvas = treemapCanvas.value
  if (!canvas) return null
  const bounds = canvas.getBoundingClientRect()
  const x = event.clientX - bounds.left
  const y = event.clientY - bounds.top
  return treemapBlocks.value.find(block =>
    x >= block.x && x <= block.x + block.width && y >= block.y && y <= block.y + block.height,
  ) ?? null
}

function handleTreemapMove(event: MouseEvent) {
  const block = treemapBlockAt(event)
  const nextIndex = block ? treemapBlocks.value.indexOf(block) : null
  if (nextIndex !== hoveredTreemapIndex.value) {
    hoveredTreemapIndex.value = nextIndex
    redrawTreemap()
  }
}

function handleTreemapBlockEnter(index: number) {
  if (index !== hoveredTreemapIndex.value) {
    hoveredTreemapIndex.value = index
    redrawTreemap()
  }
}

function handleTreemapLeave() {
  if (hoveredTreemapIndex.value !== null) {
    hoveredTreemapIndex.value = null
    redrawTreemap()
  }
}

async function handleTreemapClick() {
  const block = hoveredTreemap.value
  if (block?.node?.is_directory) await openDirectory(block.node)
}

async function handleTreemapBlockClick(block: TreemapBlock) {
  if (block.node?.is_directory) await openDirectory(block.node)
}

onMounted(() => {
  if (treemapHost.value && typeof ResizeObserver !== 'undefined') {
    treemapResizeObserver = new ResizeObserver(() => redrawTreemap())
    treemapResizeObserver.observe(treemapHost.value)
  } else {
    window.addEventListener('resize', redrawTreemap)
  }
})

onBeforeUnmount(() => {
  diskScanRequestSequence += 1
  childRequestSequence += 1
  latestNavigationChildRequest = childRequestSequence
  latestNestedChildRequests.clear()
  largestFilesRequestSequence += 1
  detailsRequestSequence += 1
  treemapResizeObserver?.disconnect()
  window.removeEventListener('resize', redrawTreemap)
})

watch([diskChildren, displayMode], () => {
  void nextTick(redrawTreemap)
})

watch(treemapHost, host => {
  if (host) void nextTick(redrawTreemap)
})
watch(displayMode, () => {
  if (diskResult.value && currentParentId.value !== null) {
    void loadDiskChildren(currentParentId.value)
  }
})
</script>

<template>
  <div class="disk-analyzer">
    <AppHeader />

    <ResultsBanner
      v-if="feedback"
      :variant="feedback.type"
      :title="feedback.title"
      :description="feedback.message"
      class="analyzer-feedback"
      @dismiss="feedback = null"
    />

    <BaseTabBar
      :tabs="tabs"
      v-model="activeTab"
      variant="glass"
      class="analyzer-tabs"
    />
    <div class="analyzer-safety">
      <BaseBadge variant="info" size="sm">Read-only scan · no volume writes or deletion</BaseBadge>
      <BaseBadge
        v-if="activeTab === 'usage' && diskResult"
        :variant="statusVariant(diskResult.status)"
        size="sm"
      >
        {{ statusLabel(diskResult.status) }} · {{ diskResult.engine.replace('_', ' ') }}
      </BaseBadge>
    </div>

    <!-- Scan config -->
    <div class="scan-config">
      <div class="config-row">
        <div class="config-field">
          <label class="field-label">Path</label>
          <div class="path-input">
            <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" width="16" height="16" class="path-icon">
              <path d="M22 19a2 2 0 01-2 2H4a2 2 0 01-2-2V5a2 2 0 012-2h5l2 3h9a2 2 0 012 2z" />
            </svg>
            <input v-model="scanPath" class="input" placeholder="C:\" @keyup.enter="runActiveScan" />
          </div>
        </div>
        <div v-if="activeTab === 'usage'" class="config-field sm scan-scope">
          <label class="field-label">Scope</label>
          <span class="scan-scope-value">{{ diskScopeLabel }}</span>
        </div>
        <div v-if="activeTab === 'usage'" class="config-field sm">
          <label class="field-label">Display</label>
          <select v-model="displayMode" class="input">
            <option value="allocated">{{ allocationLabel }}</option>
            <option value="logical">Logical</option>
            <option value="unique">Unique allocated</option>
          </select>
        </div>
        <div v-if="activeTab === 'usage'" class="config-field sm">
          <label class="field-label">Sort</label>
          <select v-model="sortMode" class="input" @change="loadDiskChildren(currentParentId)">
            <option value="allocated">{{ allocationLabel }}</option>
            <option value="logical">Logical</option>
            <option value="unique">Unique allocated</option>
            <option value="name">Name</option>
          </select>
        </div>
        <div class="config-action">
          <BaseButton
            variant="accent"
            icon="play"
            @click="runActiveScan"
            :loading="loading"
            :disabled="scanDisabled"
          >
            Scan
          </BaseButton>
          <BaseButton
            v-if="loading"
            variant="danger"
            icon="close"
            size="sm"
            @click="cancelCurrentScan"
            :loading="cancelling"
            class="cancel-btn"
          >
            Cancel
          </BaseButton>
        </div>
      </div>
    </div>

    <!-- Disk Usage -->
    <div v-if="activeTab === 'usage' && diskTree && diskResult && !loading" class="usage-section">
      <div class="disk-summary-grid">
        <BaseCard variant="elevated" class="usage-summary">
          <div class="summary-header">
            <div class="summary-icon">
              <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" width="20" height="20">
                <path d="M22 12H2M2 12l2.5-7.5A2 2 0 016.44 3h11.12a2 2 0 011.94 1.5L22 12zM2 12v5a2 2 0 002 2h16a2 2 0 002-2v-5" />
              </svg>
            </div>
            <div class="summary-text">
              <h3 class="summary-title">{{ diskTree.name }}</h3>
              <span class="summary-sub">{{ formatBytes(summaryMetric()) }} · {{ diskResult.summary.file_count }} files</span>
            </div>
          </div>
        </BaseCard>
        <BaseCard variant="glass" class="usage-summary metric-card">
          <span class="metric-label">Logical</span>
          <strong>{{ formatBytes(diskResult.summary.logical_size_bytes) }}</strong>
        </BaseCard>
        <BaseCard variant="glass" class="usage-summary metric-card">
          <span class="metric-label">{{ allocationLabel }}</span>
          <strong>{{ formatBytes(diskResult.summary.allocated_size_bytes) }}</strong>
        </BaseCard>
        <BaseCard variant="glass" class="usage-summary metric-card">
          <span class="metric-label">Unique allocation</span>
          <strong>{{ formatBytes(diskResult.summary.unique_allocated_size_bytes) }}</strong>
        </BaseCard>
        <BaseCard variant="glass" class="usage-summary metric-card">
          <span class="metric-label">Metadata</span>
          <strong>{{ formatBytes(diskResult.summary.metadata_size_bytes) }}</strong>
        </BaseCard>
        <BaseCard v-if="diskResult.summary.unaccounted_size_bytes !== null" variant="glass" class="usage-summary metric-card">
          <span class="metric-label">Unaccounted</span>
          <strong>{{ formatBytes(diskResult.summary.unaccounted_size_bytes) }}</strong>
        </BaseCard>
      </div>

      <BaseCard variant="glass" class="diagnostics-card">
        <div class="diagnostics-row">
          <BaseBadge variant="info" size="sm">{{ diskResult.source }} · cache {{ diskResult.diagnostics.cache_source }}</BaseBadge>
          <BaseBadge variant="info" size="sm">Scan {{ diskResult.diagnostics.elapsed_ms }} ms</BaseBadge>
          <BaseBadge v-if="guiScanElapsedMs !== null" variant="success" size="sm">GUI {{ guiScanElapsedMs }} ms</BaseBadge>
          <BaseBadge v-if="diskResult.diagnostics.reconciliation !== 'unavailable'" variant="success" size="sm">reconciled</BaseBadge>
          <BaseBadge v-if="diskResult.diagnostics.partial_scan" variant="warning" size="sm">partial scan</BaseBadge>
          <BaseBadge v-if="diskResult.diagnostics.access_denied" variant="warning" size="sm">access denied</BaseBadge>
          <span v-if="diskResult.diagnostics.messages.length" class="diagnostic-message">
            {{ diskResult.diagnostics.messages[0] }}
          </span>
        </div>
        <details v-if="diskResult.diagnostics.timings" class="developer-timings">
          <summary>Developer timings · local only</summary>
          <dl class="timing-grid">
            <dt>Validation</dt><dd>{{ diskResult.diagnostics.timings.validation_ms }} ms</dd>
            <dt>Scan</dt><dd>{{ diskResult.diagnostics.timings.scan_ms }} ms · {{ budgetLabel(diskResult.diagnostics.timings.scan_ms, DISK_SCAN_BUDGET_MS) }}</dd>
            <dt>Result finalization</dt><dd>{{ diskResult.diagnostics.timings.result_finalization_ms }} ms</dd>
            <dt>Index finalization</dt><dd>{{ diskResult.diagnostics.timings.index_finalization_ms }} ms</dd>
            <template v-if="guiScanElapsedMs !== null">
              <dt>GUI render</dt><dd>{{ guiScanElapsedMs }} ms · {{ budgetLabel(guiScanElapsedMs, GUI_RENDER_BUDGET_MS) }}</dd>
            </template>
          </dl>
          <p>Internal diagnostic budgets: scan ≤ {{ DISK_SCAN_BUDGET_MS }} ms and GUI render ≤ {{ GUI_RENDER_BUDGET_MS }} ms. Profiling stays local; no telemetry is sent.</p>
        </details>
      </BaseCard>

      <BaseCard variant="elevated" class="entries-card">
        <div class="entries-header">
          <div>
            <span class="treemap-title">{{ currentParentNode?.name ?? 'Volume' }} contents</span>
            <span class="entries-subtitle">Full scan · paginated navigation</span>
          </div>
          <BaseButton v-if="currentParentId !== 0" variant="ghost" size="sm" @click="loadDiskChildren(0)">Root</BaseButton>
        </div>
        <div class="chart-section virtual-rows">
          <div
            v-for="(row, idx) in visibleChildren"
            :key="row.kind === 'node' ? row.node?.id : `load-more-${row.parentId}`"
            class="chart-row entry-row"
            :class="{ 'nested-load-more': row.kind === 'load_more' }"
          >
            <template v-if="row.kind === 'node' && row.node">
              <button
                class="entry-name-button"
                type="button"
                :style="{ paddingLeft: `${row.depth * 20}px` }"
                :aria-expanded="row.node.is_directory ? expandedDirectoryIds.has(row.node.id) : undefined"
                @click="toggleDirectory(row.node)"
              >
                <span class="entry-name-line">
                  <span class="entry-expander" :class="{ empty: !row.node.is_directory }">{{ row.node.is_directory ? (expandedDirectoryIds.has(row.node.id) ? '▾' : '▸') : '·' }}</span>
                  <span class="chart-name" :title="row.node.name">{{ row.node.name }}</span>
                </span>
                <span class="chart-count">{{ row.node.is_directory ? `${row.node.directory_count} directories · ${row.node.file_count} files` : 'file' }}</span>
              </button>
              <div class="chart-bar-track">
                <div
                  class="chart-bar-fill"
                  :style="{ width: barWidth(nodeMetric(row.node), currentParentMetric()), background: getColor(idx) }"
                />
              </div>
              <div class="chart-value">
                <strong>{{ formatBytes(nodeMetric(row.node)) }}</strong>
                <small>{{ formatBytes(row.node.logical_size_bytes) }} logical · {{ formatBytes(row.node.allocated_size_bytes) }} allocated</small>
              </div>
              <BaseButton variant="ghost" size="sm" @click.stop="showDetails(row.node)">Details</BaseButton>
            </template>
            <template v-else>
              <div class="nested-load-more-control" :style="{ paddingLeft: `${row.depth * 20 + 20}px` }">
                <BaseButton variant="secondary" size="sm" @click="loadMoreNestedChildren(row.parentId ?? 0)" :loading="nestedLoadingIds.has(row.parentId ?? -1)">
                  Load more in this folder
                </BaseButton>
              </div>
            </template>
          </div>
          <div v-if="!visibleChildren.length && !childLoading" class="entries-empty">No entries at this level.</div>
        </div>
        <div class="entries-footer">
          <span>{{ diskChildren.length }} loaded<span v-if="treeRowsTruncated"> · showing at most {{ MAX_RENDERED_TREE_ROWS }} rendered rows</span></span>
          <BaseButton v-if="hasMoreChildren" variant="secondary" size="sm" @click="loadDiskChildren(currentParentId, true)" :loading="childLoading">
            Load more
          </BaseButton>
        </div>
      </BaseCard>

      <BaseCard variant="glass" class="treemap-card">
        <div class="treemap-header">
          <div>
            <span class="treemap-title">Space map</span>
            <span class="entries-subtitle">Proportional to {{ displayMode }} space</span>
          </div>
          <span class="treemap-hint">Click a folder to drill down</span>
        </div>
        <div ref="treemapHost" class="treemap-canvas-host">
          <canvas
            ref="treemapCanvas"
            class="treemap-canvas"
            role="img"
            aria-label="Color-coded disk usage treemap"
            @mousemove="handleTreemapMove"
            @mouseleave="handleTreemapLeave"
            @click="handleTreemapClick"
          />
          <div v-if="treemapBlocks.length" class="treemap-block-layer" aria-label="Space map folders">
            <button
              v-for="(block, blockIndex) in treemapBlocks"
              :key="`${block.label}-${block.index}`"
              type="button"
              class="treemap-block"
              :class="{ hovered: blockIndex === hoveredTreemapIndex, other: block.isOther }"
              :style="{ left: `${block.x}px`, top: `${block.y}px`, width: `${block.width}px`, height: `${block.height}px`, background: block.isOther ? '#64748b' : block.color }"
              :disabled="block.isOther || !block.node?.is_directory"
              :title="`${block.label} · ${formatBytes(block.value)}`"
              @mouseenter="handleTreemapBlockEnter(blockIndex)"
              @mouseleave="handleTreemapLeave"
              @click.stop="handleTreemapBlockClick(block)"
            >
              <span v-if="block.width >= 56 && block.height >= 24">{{ block.label }}</span>
            </button>
          </div>
          <div v-else class="treemap-empty">No allocated entries were returned for this level.</div>
        </div>
        <div v-if="hoveredTreemap" class="treemap-hover" role="status">
          <strong>{{ hoveredTreemap.label }}</strong>
          <span>{{ formatBytes(hoveredTreemap.value) }}<template v-if="hoveredTreemap.node"> · {{ hoveredTreemap.node.file_count }} files</template></span>
        </div>
      </BaseCard>

      <BaseCard variant="elevated" class="largest-files-card">
        <div class="entries-header">
          <div>
            <span class="treemap-title">Largest files</span>
            <span class="entries-subtitle">Fastest way to find individual space hogs</span>
          </div>
          <select v-model="largestFileSort" class="input largest-sort" @change="loadLargestFiles">
            <option value="allocated">Allocated space</option>
            <option value="logical">Logical size</option>
            <option value="unique">Unique allocation</option>
          </select>
        </div>
        <div v-if="largestFilesLoading" class="entries-empty">Loading largest files…</div>
        <div v-else-if="largestFiles.length" class="largest-files-list">
          <div v-for="(file, index) in largestFiles" :key="file.node_id" class="largest-file-row">
            <span class="largest-file-rank">{{ index + 1 }}</span>
            <button class="largest-file-path" type="button" :title="file.path" @click="showDetailsForId(file.node_id)">
              <strong>{{ file.name }}</strong>
              <span>{{ file.path }}</span>
            </button>
            <span class="largest-file-size">
              <strong>{{ formatBytes(largestFileSort === 'logical' ? file.logical_size_bytes : largestFileSort === 'unique' ? file.unique_allocated_size_bytes : file.allocated_size_bytes) }}</strong>
              <small>{{ formatBytes(file.logical_size_bytes) }} logical · {{ formatBytes(file.allocated_size_bytes) }} allocated</small>
            </span>
            <BaseButton variant="ghost" size="sm" @click="showDetailsForId(file.node_id)">Details</BaseButton>
          </div>
        </div>
        <div v-else class="entries-empty">No files were indexed.</div>
      </BaseCard>

      <BaseCard v-if="entryDetails || detailsLoading" variant="elevated" class="details-card">
        <div class="entries-header">
          <div>
            <span class="treemap-title">Entry details</span>
            <span class="entries-subtitle">{{ entryDetails?.path ?? 'Loading selected entry…' }}</span>
          </div>
          <BaseButton v-if="entryDetails" variant="ghost" size="sm" @click="entryDetails = null">Close</BaseButton>
        </div>
        <div v-if="detailsLoading" class="entries-empty">Loading file-system details…</div>
        <template v-else-if="entryDetails">
          <div class="details-size-grid">
            <div><span>Actual / logical size</span><strong>{{ formatBytes(entryDetails.logical_size_bytes) }}</strong></div>
            <div><span>{{ allocationLabel }} on disk</span><strong>{{ formatBytes(entryDetails.allocated_size_bytes) }}</strong></div>
            <div><span>Unique allocation</span><strong>{{ formatBytes(entryDetails.unique_allocated_size_bytes) }}</strong></div>
          </div>
          <div class="details-grid">
            <span>MFT record</span><strong>{{ entryDetails.mft_record_number ?? 'Unavailable' }}</strong>
            <span>Sequence</span><strong>{{ entryDetails.sequence ?? 'Unavailable' }}</strong>
            <span>Links</span><strong>{{ entryDetails.link_count }}</strong>
            <span>Attributes</span><strong>{{ diskNodeAttributes(entryDetails.flags) }}</strong>
            <span>Reparse tag</span><strong>{{ entryDetails.reparse_tag ?? 'None' }}</strong>
          </div>
          <div v-for="stream in entryDetails.streams" :key="stream.name" class="stream-row">
            <span>{{ stream.name || '$DATA' }}</span>
            <span>{{ formatBytes(stream.logical_size_bytes) }} logical · {{ formatBytes(stream.allocated_size_bytes) }} allocated</span>
          </div>
        </template>
      </BaseCard>
    </div>
    <!-- Loading state -->
    <BaseCard v-if="loading" variant="glass" class="loading-section">
      <div class="scan-animation">
        <div class="scan-ring" />
        <div class="scan-content">
          <span class="scan-text">Analyzing {{ activeTab === 'usage' ? 'disk usage' : 'for duplicates' }}...</span>
          <span v-if="scanPhase" class="scan-phase" aria-live="polite">Phase: {{ scanPhaseLabel(scanPhase) }}</span>
          <BaseProgress
            :value="scanProgress"
            :indeterminate="scanProgress === 0"
            :show-label="scanProgress > 0"
            :label="scanMessage"
            size="sm"
          />
        </div>
      </div>
    </BaseCard>
  </div>
</template>

<style scoped>
.disk-analyzer {
  width: 100%;
  min-width: 0;
  box-sizing: border-box;
}

.analyzer-feedback {
  margin-bottom: var(--space-5);
}

.analyzer-tabs {
  margin-bottom: var(--space-4);
}

.analyzer-safety {
  display: flex;
  flex-wrap: wrap;
  gap: var(--space-2);
  margin-bottom: var(--space-4);
}

/* Scan config */
.scan-config {
  margin-bottom: var(--space-5);
  padding: var(--space-4);
  background: var(--bg-elevated);
  border: 1px solid var(--border-default);
  border-radius: var(--radius-xl);
}

.config-row {
  display: flex;
  align-items: flex-end;
  gap: var(--space-4);
  flex-wrap: wrap;
}

.config-field {
  display: flex;
  flex-direction: column;
  gap: var(--space-1);
  flex: 1;
  min-width: 180px;
}

.config-field.sm {
  flex: 0 0 auto;
  min-width: 100px;
}

.scan-scope {
  min-width: 135px;
}

.scan-scope-value {
  display: flex;
  align-items: center;
  min-height: 32px;
  color: var(--text-primary);
  font-size: var(--text-sm);
}

.field-label {
  font-size: var(--text-xs);
  color: var(--text-muted);
  font-weight: var(--weight-medium);
  text-transform: uppercase;
  letter-spacing: var(--tracking-wide);
}

.path-input {
  display: flex;
  align-items: center;
  gap: var(--space-2);
  background: var(--bg-surface);
  border: 1px solid var(--border-default);
  border-radius: var(--radius-lg);
  padding: var(--space-1) var(--space-3);
  transition: border-color var(--duration-fast);
}

.path-input:focus-within {
  border-color: var(--accent);
  box-shadow: 0 0 0 3px var(--color-primary-100);
}

.path-icon {
  color: var(--text-muted);
  flex-shrink: 0;
}

.input {
  flex: 1;
  border: none;
  background: transparent;
  color: var(--text-primary);
  font-family: var(--font-sans);
  font-size: var(--text-sm);
  padding: var(--space-1) 0;
  outline: none;
  width: 100%;
}

select.input {
  cursor: pointer;
}

.size-input-group {
  display: flex;
  align-items: center;
  gap: var(--space-1);
}

.size-input {
  width: 60px;
}

.size-unit {
  font-size: var(--text-xs);
  color: var(--text-muted);
}

.config-action {
  flex-shrink: 0;
  display: flex;
  gap: var(--space-sm);
  align-items: center;
}

.duplicate-settings {
  margin-top: var(--space-4);
  padding-top: var(--space-4);
  border-top: 1px solid var(--border-subtle);
}

.duplicate-scope {
  display: grid;
  gap: var(--space-3);
  margin-top: var(--space-4);
  padding-top: var(--space-4);
  border-top: 1px solid var(--border-subtle);
}

.duplicate-scope-heading {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: var(--space-4);
  flex-wrap: wrap;
}

.duplicate-scope-copy {
  margin: var(--space-1) 0 0;
  color: var(--text-muted);
  font-size: var(--text-xs);
}

.duplicate-scope-options {
  display: flex;
  flex-wrap: wrap;
  gap: var(--space-2) var(--space-4);
}

.scope-check {
  display: inline-flex;
  align-items: center;
  gap: var(--space-2);
  color: var(--text-secondary);
  font-size: var(--text-sm);
}

.scope-check input {
  accent-color: var(--accent);
}

.duplicate-scope-error {
  color: var(--color-danger, #ef4444);
  font-size: var(--text-xs);
  font-weight: var(--weight-semibold);
}

.duplicate-entitlement-notice {
  display: flex;
  flex-wrap: wrap;
  gap: var(--space-2);
  align-items: baseline;
  margin-top: var(--space-3);
  padding: var(--space-3);
  border: 1px solid var(--border-subtle);
  border-radius: var(--radius-lg);
  background: var(--bg-surface);
  color: var(--text-secondary);
  font-size: var(--text-xs);
}

.duplicate-entitlement-notice strong {
  color: var(--text-primary);
}

.duplicate-entitlement-inline {
  color: var(--text-muted);
  font-size: var(--text-xs);
}

.duplicate-settings-summary {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--space-4);
}

.duplicate-settings-summary > div {
  display: flex;
  align-items: baseline;
  flex-wrap: wrap;
  gap: var(--space-2);
}

.duplicate-settings-summary strong {
  color: var(--text-primary);
  font-size: var(--text-sm);
}

.duplicate-settings-copy {
  color: var(--text-muted);
  font-size: var(--text-xs);
}

.advanced-toggle {
  border: 1px solid var(--border-default);
  border-radius: var(--radius-lg);
  background: var(--bg-surface);
  color: var(--text-secondary);
  cursor: pointer;
  font-size: var(--text-xs);
  padding: var(--space-2) var(--space-3);
  white-space: nowrap;
}

.advanced-toggle:hover,
.advanced-toggle:focus-visible {
  border-color: var(--accent);
  color: var(--text-primary);
}

.duplicate-advanced-settings {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(min(100%, 240px), 1fr));
  gap: var(--space-3);
  margin-top: var(--space-4);
}

.media-option-group {
  display: flex;
  flex-direction: column;
  gap: var(--space-2);
  min-width: 0;
  padding: var(--space-3);
  border: 1px solid var(--border-subtle);
  border-radius: var(--radius-lg);
}

.media-option-group legend {
  padding: 0 var(--space-1);
  color: var(--text-primary);
  font-size: var(--text-sm);
  font-weight: var(--weight-semibold);
}

.media-option-group .field-label {
  display: flex;
  flex-direction: column;
  gap: var(--space-1);
}

.photo-distance-control output {
  color: var(--text-secondary);
  font-size: var(--text-xs);
  font-variant-numeric: tabular-nums;
}

.photo-distance-control .field-help {
  margin: 0;
  color: var(--text-muted);
  font-size: var(--text-xs);
  line-height: 1.45;
}

.option-check {
  display: flex;
  align-items: flex-start;
  gap: var(--space-2);
  color: var(--text-secondary);
  font-size: var(--text-xs);
  line-height: 1.4;
}

.option-check input {
  margin-top: 2px;
  accent-color: var(--accent);
}

@media (max-width: 1100px) {
  .duplicate-advanced-settings {
    grid-template-columns: repeat(auto-fit, minmax(min(100%, 240px), 1fr));
  }

  .duplicate-advanced-settings > .config-field {
    grid-column: auto;
  }
}

@media (max-width: 680px) {
  .duplicate-settings-summary {
    align-items: flex-start;
    flex-direction: column;
  }

  .duplicate-advanced-settings {
    grid-template-columns: 1fr;
  }
}

/* Usage Summary */
.usage-section {
  display: flex;
  flex-direction: column;
  animation: fadeInUp var(--duration-normal) var(--ease-out);
}

.disk-summary-grid {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(min(100%, 170px), 1fr));
  gap: var(--space-3);
  margin-bottom: var(--space-4);
}

.disk-summary-grid .usage-summary {
  margin-bottom: 0;
}

.metric-card {
  display: flex;
  flex-direction: column;
  justify-content: center;
  gap: var(--space-1);
}

.metric-label,
.entries-subtitle {
  color: var(--text-muted);
  font-size: var(--text-xs);
}

.diagnostics-card,
.entries-card,
.largest-files-card,
.details-card {
  margin-bottom: var(--space-4);
}

.treemap-card { order: 1; }
.entries-card { order: 2; }
.details-card { order: 3; }
.largest-files-card { order: 4; }

.diagnostics-row,
.entries-header,
.entries-footer {
  display: flex;
  align-items: center;
  justify-content: space-between;
  flex-wrap: wrap;
  gap: var(--space-2);
}

.diagnostics-row {
  justify-content: flex-start;
}

.diagnostic-message {
  color: var(--text-secondary);
  font-size: var(--text-xs);
}

.developer-timings {
  margin-top: var(--space-3);
  color: var(--text-secondary);
  font-size: var(--text-xs);
}

.developer-timings summary {
  cursor: pointer;
  color: var(--text-primary);
  font-weight: var(--weight-semibold);
}

.timing-grid {
  display: grid;
  grid-template-columns: minmax(130px, auto) minmax(0, 1fr);
  gap: var(--space-1) var(--space-3);
  margin: var(--space-2) 0 0;
}

.timing-grid dt { color: var(--text-muted); }
.timing-grid dd { margin: 0; }
.developer-timings p { margin: var(--space-2) 0 0; }

.virtual-rows {
  max-height: 520px;
  overflow: auto;
}

.entry-row {
  min-height: 48px;
}

.entry-name-button {
  display: flex;
  flex-direction: column;
  align-items: flex-start;
  min-width: 0;
  padding: 0;
  border: 0;
  background: transparent;
  color: inherit;
  cursor: pointer;
  text-align: left;
}

.entry-name-line {
  display: inline-flex;
  min-width: 0;
  align-items: center;
  gap: var(--space-1);
}

.entry-expander {
  width: 14px;
  flex: 0 0 14px;
  color: var(--accent);
  font-size: var(--text-xs);
  text-align: center;
}

.entry-expander.empty {
  color: var(--text-muted);
}

.entry-name-button:hover .chart-name {
  color: var(--accent);
}

.entries-empty {
  padding: var(--space-5);
  color: var(--text-muted);
  text-align: center;
}

.entries-footer {
  margin-top: var(--space-3);
  color: var(--text-muted);
  font-size: var(--text-xs);
}

.details-grid {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(min(100%, 140px), 1fr));
  gap: var(--space-2);
  margin-bottom: var(--space-3);
  color: var(--text-muted);
  font-size: var(--text-xs);
}

.details-size-grid {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(min(100%, 180px), 1fr));
  gap: var(--space-2);
  margin-bottom: var(--space-3);
}

.details-size-grid div {
  display: flex;
  flex-direction: column;
  gap: 2px;
  padding: var(--space-3);
  border: 1px solid var(--border-subtle);
  border-radius: var(--radius-md);
  background: var(--bg-surface);
}

.details-size-grid span {
  color: var(--text-muted);
  font-size: var(--text-xs);
}

.details-size-grid strong {
  color: var(--text-primary);
  font-size: var(--text-sm);
}

.details-grid strong {
  color: var(--text-primary);
}

.stream-row {
  display: flex;
  justify-content: space-between;
  gap: var(--space-3);
  padding: var(--space-2) 0;
  border-top: 1px solid var(--border-default);
  color: var(--text-secondary);
  font-size: var(--text-xs);
}

.usage-summary {
  margin-bottom: var(--space-4);
}

.summary-header {
  display: flex;
  align-items: center;
  gap: var(--space-3);
}

.summary-icon {
  width: 40px;
  height: 40px;
  display: flex;
  align-items: center;
  justify-content: center;
  background: var(--color-primary-100);
  border-radius: var(--radius-xl);
  color: var(--accent);
  flex-shrink: 0;
}

.summary-text {
  display: flex;
  flex-direction: column;
  gap: 1px;
}

.summary-title {
  margin: 0;
  font-size: var(--text-lg);
  font-weight: var(--weight-semibold);
  color: var(--text-primary);
}

.summary-sub {
  font-size: var(--text-sm);
  color: var(--text-muted);
}

/* Bar Chart */
.chart-section {
  margin-bottom: var(--space-4);
  display: flex;
  flex-direction: column;
  gap: var(--space-2);
  padding: var(--space-4);
  background: var(--bg-elevated);
  border: 1px solid var(--border-default);
  border-radius: var(--radius-xl);
}

.chart-row {
  display: grid;
  grid-template-columns: minmax(0, 1fr) minmax(0, 3fr) auto;
  align-items: center;
  gap: var(--space-3);
}

.chart-label {
  display: flex;
  flex-direction: column;
  gap: 1px;
  min-width: 0;
}

.chart-name {
  font-size: var(--text-sm);
  font-weight: var(--weight-medium);
  color: var(--text-primary);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.chart-count {
  font-size: var(--text-xs);
  color: var(--text-muted);
}

.chart-bar-track {
  height: 20px;
  background: var(--bg-surface);
  border-radius: var(--radius-full);
  overflow: hidden;
}

.chart-bar-fill {
  height: 100%;
  border-radius: var(--radius-full);
  transition: width 0.8s var(--ease-out);
  min-width: 4px;
}

.chart-value {
  display: flex;
  min-width: 150px;
  flex-direction: column;
  gap: 2px;
  font-size: var(--text-sm);
  font-weight: var(--weight-semibold);
  color: var(--text-primary);
  text-align: right;
  font-variant-numeric: tabular-nums;
}

/* Treemap */
.treemap-card {
  margin-bottom: var(--space-4);
}

.chart-value small {
  color: var(--text-muted);
  font-size: var(--text-xs);
  font-weight: var(--weight-normal);
  white-space: nowrap;
}

.nested-load-more-control {
  grid-column: 1 / -1;
}

.treemap-header {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: var(--space-3);
  margin-bottom: var(--space-3);
}

.treemap-title {
  font-size: var(--text-sm);
  font-weight: var(--weight-semibold);
  color: var(--text-secondary);
}

.treemap-hint {
  color: var(--text-muted);
  font-size: var(--text-xs);
}

.treemap-canvas-host {
  position: relative;
  width: 100%;
  min-height: 360px;
  overflow: hidden;
  border: 1px solid var(--border-default);
  border-radius: var(--radius-lg);
  background: var(--bg-surface);
}

.treemap-canvas {
  display: block;
  width: 100%;
  height: 360px;
  cursor: pointer;
}

.treemap-block-layer {
  position: absolute;
  inset: 0;
  height: 360px;
}

.treemap-block {
  position: absolute;
  display: flex;
  align-items: flex-start;
  justify-content: flex-start;
  overflow: hidden;
  margin: 0;
  padding: 6px 7px;
  border: 1px solid rgba(5, 10, 18, 0.72);
  border-radius: 2px;
  color: #fff;
  cursor: pointer;
  font: 12px system-ui, sans-serif;
  text-align: left;
}

.treemap-block span {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.treemap-block:hover,
.treemap-block.hovered {
  z-index: 1;
  border: 2px solid #fff;
  padding: 5px 6px;
}

.treemap-block:disabled {
  cursor: default;
  opacity: 0.82;
}

.treemap-empty {
  position: absolute;
  inset: 0;
  display: grid;
  place-items: center;
  height: 360px;
  color: var(--text-muted);
  font-size: var(--text-xs);
}

.treemap-hover {
  display: flex;
  justify-content: space-between;
  gap: var(--space-3);
  margin-top: var(--space-2);
  color: var(--text-secondary);
  font-size: var(--text-xs);
}

.treemap-hover strong {
  color: var(--text-primary);
}

.largest-sort {
  flex: 0 0 auto;
  width: auto;
  min-width: 150px;
  padding: var(--space-2);
  border: 1px solid var(--border-default);
  border-radius: var(--radius-md);
  background: var(--bg-surface);
}

.largest-files-list {
  display: flex;
  flex-direction: column;
  gap: var(--space-1);
  max-height: 520px;
  overflow: auto;
}

.largest-file-row {
  display: grid;
  grid-template-columns: 28px minmax(0, 1fr) minmax(0, auto) auto;
  align-items: center;
  gap: var(--space-3);
  padding: var(--space-2);
  border-bottom: 1px solid var(--border-subtle);
}

.largest-file-row:last-child {
  border-bottom: 0;
}

.largest-file-rank {
  color: var(--text-muted);
  font-size: var(--text-xs);
  font-variant-numeric: tabular-nums;
  text-align: right;
}

.largest-file-path {
  display: flex;
  min-width: 0;
  flex-direction: column;
  align-items: flex-start;
  padding: 0;
  border: 0;
  background: transparent;
  color: inherit;
  cursor: pointer;
  text-align: left;
}

.largest-file-path strong,
.largest-file-path span {
  max-width: 100%;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.largest-file-path strong {
  color: var(--text-primary);
  font-size: var(--text-sm);
}

.largest-file-path span,
.largest-file-size small {
  color: var(--text-muted);
  font-size: var(--text-xs);
}

.largest-file-size {
  display: flex;
  flex-direction: column;
  align-items: flex-end;
  gap: 2px;
  white-space: nowrap;
  text-align: right;
}

.largest-file-size strong {
  color: var(--text-primary);
  font-size: var(--text-sm);
}

.largest-file-size small {
  color: var(--text-muted);
}

/* Duplicates */
.duplicates-section {
  display: flex;
  flex-direction: column;
  animation: fadeInUp var(--duration-normal) var(--ease-out);
}

.duplicate-saved-projects {
  display: flex;
  flex-direction: column;
  gap: var(--space-3);
  margin-bottom: var(--space-4);
  padding: var(--space-4);
  border: 1px solid var(--border-default);
  border-radius: var(--radius-xl);
  background: var(--bg-elevated);
}

.duplicate-saved-projects-heading {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: var(--space-3);
}

.duplicate-saved-projects-heading h2 {
  margin: var(--space-1) 0;
  color: var(--text-primary);
  font-size: var(--text-base);
}

.duplicate-saved-projects-heading p,
.duplicate-saved-projects-empty,
.duplicate-saved-project span {
  color: var(--text-muted);
  font-size: var(--text-xs);
}

.duplicate-saved-projects-heading p {
  margin: 0;
}

.duplicate-saved-project-list {
  display: grid;
  gap: var(--space-2);
}

.duplicate-saved-project {
  display: flex;
  flex-direction: column;
  align-items: flex-start;
  gap: var(--space-1);
  padding: var(--space-3);
  border: 1px solid var(--border-default);
  border-radius: var(--radius-md);
  background: var(--bg-surface);
  color: inherit;
  cursor: pointer;
  text-align: left;
}

.duplicate-saved-project:hover:not(:disabled),
.duplicate-saved-project:focus-visible {
  border-color: var(--accent);
}

.duplicate-saved-project:disabled {
  cursor: wait;
  opacity: 0.65;
}

.duplicate-saved-project strong {
  max-width: 100%;
  overflow: hidden;
  color: var(--text-primary);
  font-size: var(--text-sm);
  text-overflow: ellipsis;
  white-space: nowrap;
}

.duplicate-review-project {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--space-3);
  margin-bottom: var(--space-4);
  padding: var(--space-3) var(--space-4);
  border: 1px solid var(--border-default);
  border-radius: var(--radius-lg);
  background: var(--bg-elevated);
}

.duplicate-review-project-copy {
  display: flex;
  min-width: 0;
  flex-direction: column;
  gap: var(--space-1);
}

.duplicate-review-project-copy strong {
  color: var(--text-primary);
  font-size: var(--text-sm);
}

.duplicate-review-project-copy > span:last-child {
  color: var(--text-muted);
  font-size: var(--text-xs);
}

.duplicates-toolbar { order: 0; }
.needs-comparison { order: 1; }
.perfect-copies { order: 2; }

.duplicate-section-heading {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: var(--space-4);
  margin-bottom: var(--space-4);
  padding: var(--space-4);
  border: 1px solid var(--border-default);
  border-radius: var(--radius-xl);
  background: var(--bg-elevated);
}

.duplicate-section-heading h2 {
  margin: var(--space-1) 0;
  color: var(--text-primary);
  font-size: var(--text-lg);
}

.duplicate-section-heading p {
  max-width: 72ch;
  margin: 0;
  color: var(--text-muted);
  font-size: var(--text-sm);
}

.duplicate-section-eyebrow {
  color: var(--accent);
  font-size: var(--text-xs);
  font-weight: var(--weight-semibold);
  letter-spacing: var(--tracking-wide);
}

.duplicate-section-summary {
  display: flex;
  flex: 0 0 auto;
  flex-direction: column;
  align-items: flex-end;
  gap: var(--space-1);
  color: var(--text-muted);
  font-size: var(--text-xs);
  white-space: nowrap;
}

.duplicate-section-summary strong {
  color: var(--text-primary);
  font-size: var(--text-sm);
}

.perfect-copy-list {
  display: grid;
  gap: var(--space-3);
  margin-bottom: var(--space-5);
}

.duplicate-pagination {
  display: grid;
  gap: var(--space-2);
  margin-bottom: var(--space-5);
  color: var(--text-muted);
  font-size: var(--text-xs);
}

.duplicate-pagination > div {
  display: flex;
  align-items: center;
  gap: var(--space-2);
}

.duplicate-pagination button {
  padding: var(--space-1) var(--space-2);
  border: 1px solid var(--border-subtle);
  border-radius: var(--radius-sm);
  background: var(--bg-surface);
  color: var(--text-primary);
  font: inherit;
  cursor: pointer;
}

.duplicate-pagination button:disabled {
  cursor: not-allowed;
  opacity: 0.5;
}

.perfect-copy-group {
  min-width: 0;
  padding: var(--space-4);
  border: 1px solid var(--border-default);
  border-radius: var(--radius-lg);
  background: var(--bg-surface);
}

.perfect-copy-group-heading,
.perfect-copy-file {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--space-3);
  flex-wrap: wrap;
}

.perfect-copy-group-heading {
  margin-bottom: var(--space-3);
}

.perfect-copy-group-heading > div {
  display: flex;
  min-width: 0;
  flex-direction: column;
  gap: var(--space-1);
}

.perfect-copy-group-heading strong,
.perfect-copy-path {
  color: var(--text-primary);
}

.perfect-copy-group-heading span {
  color: var(--text-muted);
  font-size: var(--text-xs);
}

.perfect-copy-files {
  display: grid;
  gap: var(--space-1);
}

.perfect-copy-file {
  padding: var(--space-2) 0;
  border-top: 1px solid var(--border-subtle);
}

.perfect-copy-selection {
  display: flex;
  min-width: 0;
  flex: 1 1 360px;
  align-items: flex-start;
  gap: var(--space-2);
  cursor: pointer;
}

.perfect-copy-selection input {
  flex: 0 0 auto;
  margin-top: 3px;
  accent-color: var(--accent);
}

.perfect-copy-path {
  min-width: 0;
  overflow-wrap: anywhere;
  font-family: var(--font-mono);
  font-size: var(--text-xs);
}

.perfect-copy-actions {
  display: flex;
  flex-wrap: wrap;
  gap: var(--space-2);
}

.text-action {
  padding: var(--space-1) var(--space-2);
  border: 1px solid var(--border-default);
  border-radius: var(--radius-md);
  background: transparent;
  color: var(--text-secondary);
  cursor: pointer;
  font-size: var(--text-xs);
}

.text-action:hover,
.text-action:focus-visible,
.text-action.selected {
  border-color: var(--accent);
  color: var(--text-primary);
}

.needs-comparison {
  min-width: 0;
}

.duplicates-toolbar {
  display: flex;
  align-items: center;
  gap: var(--space-3);
  margin-bottom: var(--space-4);
  flex-wrap: wrap;
}

.duplicate-delete-mode {
  margin-left: auto;
  min-width: 145px;
}

.duplicate-groups {
  display: flex;
  flex-direction: column;
  gap: var(--space-3);
}

.dup-group {
  transition: all var(--duration-fast);
}

.dup-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
  margin-bottom: var(--space-3);
  gap: var(--space-3);
}

.dup-hash-info {
  display: flex;
  align-items: center;
  gap: var(--space-2);
}

.dup-hash-label {
  font-size: var(--text-xs);
  color: var(--text-muted);
  text-transform: uppercase;
  letter-spacing: var(--tracking-wide);
}

.dup-hash-value {
  font-family: var(--font-mono);
  font-size: var(--text-xs);
  color: var(--text-secondary);
}

.dup-reason {
  margin: 0 0 var(--space-3);
  color: var(--text-muted);
  font-size: var(--text-xs);
}

.dup-meta {
  display: flex;
  gap: var(--space-2);
}

.dup-files {
  display: flex;
  flex-wrap: wrap;
  gap: var(--space-1);
}

.dup-file {
  display: flex;
  align-items: center;
  gap: var(--space-1);
  background: var(--bg-surface);
  padding: var(--space-1) var(--space-2);
  border-radius: var(--radius-md);
  font-size: var(--text-xs);
  color: var(--text-secondary);
  white-space: nowrap;
  max-width: 180px;
  overflow: hidden;
  text-overflow: ellipsis;
}

.dup-file input[type='checkbox'] {
  flex: 0 0 auto;
}

.dup-file-copy {
  display: flex;
  flex-direction: column;
  min-width: 0;
}

.dup-file-copy strong,
.dup-file-copy small {
  overflow: hidden;
  text-overflow: ellipsis;
}

.dup-file-copy small {
  color: var(--text-muted);
  font-size: 10px;
}

.dup-more {
  display: flex;
  align-items: center;
  padding: var(--space-1) var(--space-2);
  font-size: var(--text-xs);
  color: var(--text-muted);
}

/* Loading */
.loading-section {
  min-height: 160px;
  display: flex;
  align-items: center;
  justify-content: center;
}

.scan-animation {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: var(--space-4);
}

.scan-ring {
  width: 40px;
  height: 40px;
  border-radius: var(--radius-full);
  border: 3px solid var(--glass-border);
  border-top-color: var(--accent);
  animation: spin 1s linear infinite;
}

.scan-content {
  text-align: center;
  width: min(420px, 100%);
}

.chart-row.entry-row {
  grid-template-columns: minmax(0, 1.5fr) minmax(80px, 3fr) auto auto;
}

.scan-text {
  font-size: var(--text-sm);
  color: var(--text-secondary);
  animation: pulse 1.5s ease-in-out infinite;
}

.scan-phase {
  display: block;
  margin-top: var(--space-1);
  color: var(--text-muted);
  font-size: var(--text-xs);
  text-transform: capitalize;
}

@media (max-width: 900px) {
  .config-row {
    align-items: stretch;
  }

  .config-field,
  .config-field.sm,
  .scan-scope {
    min-width: min(100%, 180px);
    flex: 1 1 180px;
  }

  .config-action {
    flex: 1 1 100%;
  }

  .config-action > * {
    flex: 1 1 auto;
  }

  .duplicate-section-heading {
    align-items: stretch;
    flex-direction: column;
  }

  .duplicate-section-summary {
    align-items: flex-start;
  }

  .largest-file-row {
    grid-template-columns: 28px minmax(0, 1fr);
  }

  .chart-row.entry-row {
    grid-template-columns: minmax(0, 1fr) auto;
  }

  .chart-row.entry-row .chart-bar-track {
    grid-column: 1 / -1;
    grid-row: 2;
  }

  .chart-row.entry-row .chart-value {
    grid-column: 2;
    grid-row: 1;
  }

  .chart-row.entry-row > :last-child {
    grid-column: 2;
    grid-row: 3;
  }

  .largest-file-size,
  .largest-file-row > :last-child {
    grid-column: 2;
    justify-self: start;
  }

  .largest-file-size {
    align-items: flex-start;
    text-align: left;
  }
}

@media (max-width: 560px) {
  .scan-config {
    padding: var(--space-3);
  }

  .config-field,
  .config-field.sm,
  .scan-scope {
    min-width: 100%;
    flex-basis: 100%;
  }

  .duplicate-settings-summary {
    gap: var(--space-3);
  }

  .duplicate-scope-heading {
    gap: var(--space-2);
  }

  .perfect-copy-group {
    padding: var(--space-3);
  }

  .perfect-copy-selection {
    flex-basis: 100%;
  }

  .duplicate-section-heading,
  .perfect-copy-group {
    border-radius: var(--radius-lg);
  }
}
</style>
