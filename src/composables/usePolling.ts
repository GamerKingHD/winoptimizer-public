import { ref, onMounted, onUnmounted, onActivated, onDeactivated, type Ref } from 'vue'

export interface UsePolling {
  isPolling: Ref<boolean>
  start: () => void
  stop: () => void
}

/**
 * Polls a function on a regular interval. Auto-pauses when the tab is hidden
 * or the keep-alive component is deactivated. Auto-stops on unmount.
 */
export function usePolling(
  fn: () => Promise<void>,
  intervalMs: number,
  opts?: { immediate?: boolean },
): UsePolling {
  const isPolling = ref(false)
  let timer: ReturnType<typeof setInterval> | null = null
  let pausedByVisibility = false
  let pausedByDeactivate = false

  function clear() {
    if (timer !== null) {
      clearInterval(timer)
      timer = null
    }
    isPolling.value = false
  }

  function run() {
    if (timer !== null) return // already running
    isPolling.value = true
    if (opts?.immediate) {
      fn().catch(() => { /* swallow */ })
    }
    timer = setInterval(() => {
      fn().catch(() => { /* swallow - keep polling on error */ })
    }, intervalMs)
  }

  function resume() {
    if (pausedByVisibility || pausedByDeactivate) return
    run()
  }

  // Pause when the tab is not visible
  function onVisibilityChange() {
    if (document.visibilityState === 'hidden') {
      pausedByVisibility = true
      clear()
    } else {
      pausedByVisibility = false
      resume()
    }
  }

  onMounted(() => {
    document.addEventListener('visibilitychange', onVisibilityChange)
    run()
  })

  onUnmounted(() => {
    document.removeEventListener('visibilitychange', onVisibilityChange)
    clear()
  })

  // For keep-alive views: pause on deactivate, resume on activate
  onActivated(() => {
    pausedByDeactivate = false
    resume()
  })

  onDeactivated(() => {
    pausedByDeactivate = true
    clear()
  })

  return {
    isPolling,
    start: run,
    stop: clear,
  }
}
