import { defineStore } from 'pinia'
import { ref, computed, shallowRef } from 'vue'
import type { CleanItem, CleanResult, BrowserInfo } from '../api/tauri'
import * as api from '../api/tauri'
import { formatBytes } from '../utils/format'

export const useScanStore = defineStore('scan', () => {
  // Temp files
  const tempFiles = shallowRef<CleanItem[]>([])
  const tempResult = ref<CleanResult | null>(null)

  // Browser cache
  const browsers = ref<BrowserInfo[]>([])
  const browserCache = shallowRef<CleanItem[]>([])
  const browserResult = ref<CleanResult | null>(null)

  // Update cache
  const updateCache = shallowRef<CleanItem[]>([])
  const updateResult = ref<CleanResult | null>(null)

  // System logs
  const systemLogs = shallowRef<CleanItem[]>([])
  const logsResult = ref<CleanResult | null>(null)

  // Loading states
  const loading = ref(false)
  const scanningType = ref<string | null>(null)

  // Per-category error states
  const tempError = ref<string | null>(null)
  const browserError = ref<string | null>(null)
  const updateError = ref<string | null>(null)
  const logsError = ref<string | null>(null)

  // Computed totals
  const totalScannableBytes = computed(() => {
    let total = 0
    for (const item of tempFiles.value) total += item.size_bytes
    for (const item of browserCache.value) total += item.size_bytes
    for (const item of updateCache.value) total += item.size_bytes
    for (const item of systemLogs.value) total += item.size_bytes
    return total
  })

  const totalFreedBytes = computed(() => {
    let total = 0
    if (tempResult.value) total += tempResult.value.bytes_freed
    if (browserResult.value) total += browserResult.value.bytes_freed
    if (updateResult.value) total += updateResult.value.bytes_freed
    if (logsResult.value) total += logsResult.value.bytes_freed
    return total
  })

  // Actions
  async function scanAll() {
    loading.value = true
    try {
      scanningType.value = 'all'
      const results = await Promise.allSettled([
        api.scanTempFiles(),
        api.scanBrowserCache(),
        api.scanWindowsUpdateCache(),
        api.scanSystemLogs(),
      ])

      const [tempR, browserR, updateR, logsR] = results
      if (tempR.status === 'fulfilled') { tempFiles.value = tempR.value; tempError.value = null }
      else { tempError.value = String(tempR.reason) }
      if (browserR.status === 'fulfilled') { browserCache.value = browserR.value; browserError.value = null }
      else { browserError.value = String(browserR.reason) }
      if (updateR.status === 'fulfilled') { updateCache.value = updateR.value; updateError.value = null }
      else { updateError.value = String(updateR.reason) }
      if (logsR.status === 'fulfilled') { systemLogs.value = logsR.value; logsError.value = null }
      else { logsError.value = String(logsR.reason) }
    } finally {
      loading.value = false
      scanningType.value = null
    }
  }

  function formatBytesWrapper(bytes: number | null | undefined): string {
    return formatBytes(bytes)
  }

  return {
    tempFiles,
    tempResult,
    tempError,
    browsers,
    browserCache,
    browserResult,
    browserError,
    updateCache,
    updateResult,
    updateError,
    systemLogs,
    logsResult,
    logsError,
    loading,
    scanningType,
    totalScannableBytes,
    totalFreedBytes,
    scanAll,
    formatBytes: formatBytesWrapper,
  }
})
