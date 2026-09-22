import { defineStore } from 'pinia'
import { ref, shallowRef } from 'vue'
import { getHardwareInfo, getOsInfo, type HardwareInfo, type OsInfo } from '../api/tauri'

export const SYSTEM_SNAPSHOT_TTL_MS = 30_000

function describeError(error: unknown): string {
  return error instanceof Error ? error.message : String(error)
}

export const useSystemSnapshotStore = defineStore('systemSnapshot', () => {
  const hardware = shallowRef<HardwareInfo | null>(null)
  const os = shallowRef<OsInfo | null>(null)
  const hardwareError = ref<string | null>(null)
  const osError = ref<string | null>(null)
  const hardwareUpdatedAt = ref<Date | null>(null)
  const osUpdatedAt = ref<Date | null>(null)
  const stale = ref(true)
  const loading = ref(false)

  let requestSequence = 0
  let inFlight: Promise<void> | null = null
  let freshnessTimer: ReturnType<typeof setTimeout> | null = null

  function clearFreshnessTimer() {
    if (freshnessTimer !== null) {
      clearTimeout(freshnessTimer)
      freshnessTimer = null
    }
  }

  function markStaleAfter(timestamp: number) {
    clearFreshnessTimer()
    freshnessTimer = setTimeout(() => {
      if (
        hardwareUpdatedAt.value?.getTime() === timestamp &&
        osUpdatedAt.value?.getTime() === timestamp
      ) {
        stale.value = true
      }
      freshnessTimer = null
    }, SYSTEM_SNAPSHOT_TTL_MS)
  }

  function isFresh(): boolean {
    const now = Date.now()
    return Boolean(
      hardware.value &&
      os.value &&
      hardwareUpdatedAt.value &&
      osUpdatedAt.value &&
      !stale.value &&
      now - hardwareUpdatedAt.value.getTime() < SYSTEM_SNAPSHOT_TTL_MS &&
      now - osUpdatedAt.value.getTime() < SYSTEM_SNAPSHOT_TTL_MS &&
      !hardwareError.value &&
      !osError.value,
    )
  }

  function refresh(options?: { force?: boolean }): Promise<void> {
    const force = options?.force ?? false
    if (!force) {
      if (inFlight) return inFlight
      if (isFresh()) return Promise.resolve()
    }

    const requestId = ++requestSequence
    loading.value = true
    let request: Promise<void>
    request = Promise.allSettled([getHardwareInfo(), getOsInfo()])
      .then(([hardwareResult, osResult]) => {
        if (requestId !== requestSequence) return

        if (hardwareResult.status === 'fulfilled') {
          hardware.value = hardwareResult.value
          hardwareError.value = null
        } else {
          hardware.value = null
          hardwareError.value = describeError(hardwareResult.reason)
          hardwareUpdatedAt.value = null
        }

        if (osResult.status === 'fulfilled') {
          os.value = osResult.value
          osError.value = null
        } else {
          os.value = null
          osError.value = describeError(osResult.reason)
          osUpdatedAt.value = null
        }

        if (hardwareResult.status === 'fulfilled' && osResult.status === 'fulfilled') {
          const updatedAt = new Date()
          hardwareUpdatedAt.value = updatedAt
          osUpdatedAt.value = updatedAt
          stale.value = false
          markStaleAfter(updatedAt.getTime())
        } else {
          stale.value = true
          clearFreshnessTimer()
        }
      })
      .finally(() => {
        if (requestId !== requestSequence) return
        loading.value = false
        if (inFlight === request) inFlight = null
      })

    inFlight = request
    return request
  }

  return {
    hardware,
    os,
    hardwareError,
    osError,
    hardwareUpdatedAt,
    osUpdatedAt,
    stale,
    loading,
    isFresh,
    refresh,
  }
})
