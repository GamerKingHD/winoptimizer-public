import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { describe, expect, it } from 'vitest'

describe('frontend bootstrap', () => {
  const main = readFileSync(resolve(__dirname, '../main.ts'), 'utf8')
  const cleanup = readFileSync(resolve(__dirname, '../views/CleanupReference.vue'), 'utf8')
  const dashboard = readFileSync(resolve(__dirname, '../views/Dashboard.vue'), 'utf8')
  const systemInfo = readFileSync(resolve(__dirname, '../views/SystemInfo.vue'), 'utf8')
  const app = readFileSync(resolve(__dirname, '../App.vue'), 'utf8')
  const startup = readFileSync(resolve(__dirname, '../views/StartupManager.vue'), 'utf8')
  const analyzer = readFileSync(resolve(__dirname, '../views/DiskAnalyzer.vue'), 'utf8')

  it('does not call executeResumedOperation on every Tauri start', () => {
    expect(main).toContain('hasResumeOperationArgs')
    const gateCallAt = main.indexOf('hasResumeOperationArgs()')
    const resumeCallAt = main.indexOf('executeResumedOperation()')
    const failAt = main.indexOf("resumed: 'elevation-failed'")
    const tauriMountAt = main.lastIndexOf("app.mount('#app')")
    expect(gateCallAt).toBeGreaterThan(-1)
    expect(resumeCallAt).toBeGreaterThan(gateCallAt)
    expect(failAt).toBeGreaterThan(resumeCallAt)
    expect(failAt).toBeLessThan(tauriMountAt)

    const beforeGate = main.slice(0, gateCallAt)
    expect(beforeGate).not.toContain('executeResumedOperation()')
    expect(beforeGate).not.toContain("resumed: 'elevation-failed'")
  })

  it('uses elevation-failed only after a gated resume attempt fails', () => {
    expect(main).toMatch(
      /if \(await hasResumeOperationArgs\(\)\) \{[\s\S]*executeResumedOperation\(\)[\s\S]*elevation-failed[\s\S]*\}/,
    )
    const unguardedStart = main.replace(
      /if \(await hasResumeOperationArgs\(\)\) \{[\s\S]*?\n  \}/,
      '',
    )
    expect(unguardedStart).not.toContain('executeResumedOperation()')
    expect(unguardedStart).not.toContain("resumed: 'elevation-failed'")
  })

  it('keeps Clean Up idle until Scan', () => {
    expect(cleanup).not.toContain('onMounted(runScan)')
  })

  it('does not start Home as a 1s force-live hardware poller that blocks idle', () => {
    expect(dashboard).not.toContain('usePolling(refreshHardware, 1000, { immediate: true })')
    expect(dashboard).not.toMatch(/usePolling\(\s*refreshHardware\s*,\s*1000/)
  })

  it('keeps list pages from force-refetching on every activate and keeps Disk Analyzer alive', () => {
    expect(startup).not.toContain('onActivated(() => { void refresh() })')
    expect(startup).toContain('void loadPrograms()')
    expect(app).not.toContain("keep-alive :exclude=\"['DiskAnalyzer', 'DiskCleaner']\"")
    expect(app).toContain('<keep-alive>')
    expect(analyzer).not.toMatch(/onMounted\(\(\) => \{[\s\S]*scanDisk|runScan|scan_disk/)
  })

  it('shares the bounded system snapshot between Home and System Info', () => {
    expect(dashboard).toContain('useSystemSnapshotStore')
    expect(systemInfo).toContain('useSystemSnapshotStore')
    expect(dashboard).not.toContain('getHardwareInfo')
    expect(systemInfo).not.toContain('api.getHardwareInfo()')
    expect(dashboard).toContain('Metrics stale')
    expect(systemInfo).toContain('Snapshot stale')
  })
})
