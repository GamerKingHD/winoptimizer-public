import { createPinia, setActivePinia } from 'pinia'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { getHardwareInfo, getOsInfo } from '../api/tauri'
import { SYSTEM_SNAPSHOT_TTL_MS, useSystemSnapshotStore } from '../stores/systemSnapshot'

vi.mock('../api/tauri', () => ({
  getHardwareInfo: vi.fn(),
  getOsInfo: vi.fn(),
}))

const hardware = {
  cpu: { name: 'Test CPU', cores: 8, logical_processors: 16, architecture: 'x64', usage_percent: 25 },
  memory: { total_bytes: 16_000, available_bytes: 8_000, in_use_percent: 50 },
  disks: [],
  gpu: null,
}

const os = {
  version: 'Windows 11',
  edition: 'Pro',
  build: '26100',
  arch: 'x64',
  release_id: null,
  install_date: null,
  last_boot: null,
  user_name: 'Test',
  domain: null,
}

describe('system snapshot store', () => {
  beforeEach(() => {
    setActivePinia(createPinia())
    vi.clearAllMocks()
    vi.mocked(getHardwareInfo).mockResolvedValue(hardware)
    vi.mocked(getOsInfo).mockResolvedValue(os)
  })

  it('shares an in-flight read and caches a fresh complete snapshot', async () => {
    const store = useSystemSnapshotStore()

    await Promise.all([store.refresh(), store.refresh()])
    await store.refresh()

    expect(getHardwareInfo).toHaveBeenCalledTimes(1)
    expect(getOsInfo).toHaveBeenCalledTimes(1)
    expect(store.isFresh()).toBe(true)
  })

  it('marks a cached snapshot stale when its freshness window expires', async () => {
    vi.useFakeTimers()
    try {
      const store = useSystemSnapshotStore()
      await store.refresh()

      expect(store.stale).toBe(false)
      vi.advanceTimersByTime(SYSTEM_SNAPSHOT_TTL_MS)
      expect(store.stale).toBe(true)
      expect(store.isFresh()).toBe(false)
    } finally {
      vi.useRealTimers()
    }
  })

  it('ignores a superseded response and exposes partial failures as stale state', async () => {
    let resolveFirstHardware!: (value: typeof hardware) => void
    let resolveFirstOs!: (value: typeof os) => void
    const firstHardware = new Promise<typeof hardware>(resolve => { resolveFirstHardware = resolve })
    const firstOs = new Promise<typeof os>(resolve => { resolveFirstOs = resolve })
    vi.mocked(getHardwareInfo)
      .mockReturnValueOnce(firstHardware)
      .mockResolvedValueOnce({ ...hardware, cpu: { ...hardware.cpu, usage_percent: 80 } })
    vi.mocked(getOsInfo)
      .mockReturnValueOnce(firstOs)
      .mockResolvedValueOnce(os)

    const store = useSystemSnapshotStore()
    const first = store.refresh({ force: true })
    const second = store.refresh({ force: true })
    await second

    resolveFirstHardware(hardware)
    resolveFirstOs(os)
    await first

    expect(store.hardware?.cpu.usage_percent).toBe(80)
    expect(store.hardwareError).toBeNull()

    vi.mocked(getHardwareInfo).mockRejectedValueOnce(new Error('collector unavailable'))
    await store.refresh({ force: true })
    expect(store.hardware).toBeNull()
    expect(store.hardwareError).toBe('collector unavailable')
    expect(store.isFresh()).toBe(false)
  })
})
