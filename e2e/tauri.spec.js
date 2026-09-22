import { expect } from 'chai'
import { spawn } from 'child_process'
import path from 'path'
import os from 'os'
import { fileURLToPath } from 'url'
import fs from 'fs'
import net from 'net'
import { randomUUID } from 'crypto'

const __dirname = path.dirname(fileURLToPath(import.meta.url))
const SCREENSHOT_DIR = path.resolve(process.cwd(), 'test-results', 'tauri')
const APP_BINARY =
  process.env.WINOPTIMIZER_TAURI_APP ||
  path.resolve(__dirname, '..', 'target', 'release', 'winoptimizer.exe')

let tauriDriverProcess
let sessionId
let initialUrl

async function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms))
}

async function waitForPort(port, timeoutMs = 30000) {
  const deadline = Date.now() + timeoutMs
  while (Date.now() < deadline) {
    const connected = await new Promise((resolve) => {
      const socket = net.createConnection({ host: '127.0.0.1', port })
      socket.once('connect', () => {
        socket.destroy()
        resolve(true)
      })
      socket.once('error', () => resolve(false))
    })
    if (connected) return
    await sleep(250)
  }
  throw new Error(`Timed out waiting for tauri-driver on port ${port}`)
}

async function webdriver(method, route, body) {
  const response = await fetch(`http://127.0.0.1:4444${route}`, {
    method,
    headers: body ? { 'content-type': 'application/json' } : undefined,
    body: body ? JSON.stringify(body) : undefined,
    signal: AbortSignal.timeout(30000),
  })
  const payload = await response.json()
  if (!response.ok || payload.value?.error) {
    throw new Error(`WebDriver ${method} ${route} failed: ${JSON.stringify(payload)}`)
  }
  return payload.value
}

async function invokeTauri(command, args = {}) {
  const response = await webdriver('POST', `/session/${sessionId}/execute/async`, {
    script: `
      const command = arguments[0];
      const args = arguments[1];
      const done = arguments[arguments.length - 1];
      window.__TAURI_INTERNALS__.invoke(command, args)
        .then((value) => done({ ok: true, value }))
        .catch((error) => done({ ok: false, error }));
    `,
    args: [command, args],
  })
  if (!response.ok) {
    throw new Error(`${command}: ${JSON.stringify(response.error)}`)
  }
  return response.value
}

async function waitForBundledUrl(timeoutMs = 15000) {
  const deadline = Date.now() + timeoutMs
  let currentUrl = ''
  while (Date.now() < deadline) {
    currentUrl = await webdriver('GET', `/session/${sessionId}/url`)
    if (/^https?:\/\/tauri\.localhost\/?/.test(currentUrl)) return currentUrl
    await sleep(100)
  }
  throw new Error(`Packaged app did not load its bundled frontend; last URL: ${currentUrl}`)
}

before(async function () {
  this.timeout(120000)

  if (!fs.existsSync(APP_BINARY)) {
    throw new Error(`App binary not found at ${APP_BINARY}`)
  }

  const nativeDriverBin = process.env.WINOPTIMIZER_NATIVE_DRIVER ||
    path.resolve(__dirname, '..', 'msedgedriver.exe')
  if (!fs.existsSync(nativeDriverBin)) {
    throw new Error(`Native WebDriver binary not found at ${nativeDriverBin}`)
  }

  fs.mkdirSync(SCREENSHOT_DIR, { recursive: true })
  const tauriDriverBin = path.resolve(os.homedir(), '.cargo', 'bin', 'tauri-driver.exe')
  tauriDriverProcess = spawn(tauriDriverBin, ['--native-driver', nativeDriverBin], {
    stdio: ['ignore', 'pipe', 'pipe'],
    windowsHide: true,
  })
  tauriDriverProcess.stdout.on('data', (data) => process.stdout.write(`[tauri-driver] ${data}`))
  tauriDriverProcess.stderr.on('data', (data) => process.stderr.write(`[tauri-driver] ${data}`))
  tauriDriverProcess.on('error', (error) => { throw error })

  await waitForPort(4444)
  const value = await webdriver('POST', '/session', {
    capabilities: {
      alwaysMatch: {
        browserName: 'wry',
        'tauri:options': { application: APP_BINARY },
      },
    },
  })
  sessionId = value.sessionId
  if (!sessionId) throw new Error('WebDriver did not return a session id')
  initialUrl = await waitForBundledUrl()
})

after(async function () {
  if (sessionId) {
    try { await webdriver('DELETE', `/session/${sessionId}`) } catch {}
  }
  if (tauriDriverProcess) tauriDriverProcess.kill()
})

describe('WinOptimizer packaged-artifact Tauri E2E', () => {
  it('loads the bundled frontend without Vite', () => {
    expect(initialUrl).to.match(/^https?:\/\/tauri\.localhost\/?/)
  })

  it('renders the app shell', async () => {
    const element = await webdriver('POST', `/session/${sessionId}/element`, {
      using: 'css selector',
      value: '#app',
    })
    expect(element).to.have.property('element-6066-11e4-a52e-4f735466cecf')
  })

  it('shows a non-empty title', async () => {
    const title = await webdriver('GET', `/session/${sessionId}/title`)
    expect(title).to.be.a('string').and.not.empty
  })

  it('captures a dashboard screenshot', async () => {
    const screenshot = await webdriver('GET', `/session/${sessionId}/screenshot`)
    fs.writeFileSync(path.join(SCREENSHOT_DIR, 'tauri-dashboard.png'), screenshot, 'base64')
  })

  it('executes and exactly undoes a typed user environment operation', async () => {
    const name = `WINOPTIMIZER_PACKAGED_${randomUUID().replaceAll('-', '_')}`
    const plan = await invokeTauri('create_env_plan', {
      action: {
        kind: 'set',
        variable: {
          name,
          value: '%TEMP%\\packaged',
          scope: 'user',
          var_type: 'REG_EXPAND_SZ',
        },
      },
    })
    const execution = await invokeTauri('execute_env_plan', { confirmedPlan: plan })
    expect(execution.result.failed).to.equal(0)
    expect(execution.result.verified).to.equal(1)

    const journal = await invokeTauri('get_operation_journal')
    const record = journal.find((entry) => entry.operation_id === plan.id)
    expect(record?.undo_eligibility.eligible).to.equal(true)
    const preview = await invokeTauri('preview_operation_undo', {
      journalId: record.journal_id,
    })
    const undo = await invokeTauri('execute_operation_undo', {
      journalId: record.journal_id,
      confirmedPlan: preview.plan,
    })
    expect(undo.target_outcomes[0].verification).to.equal('verified')
    const variables = await invokeTauri('list_env_vars')
    expect(variables.some((variable) => variable.name === name && variable.scope === 'user'))
      .to.equal(false)
  })

  it('terminates only the confirmed disposable process identity', async () => {
    const disposable = spawn('cmd.exe', ['/c', 'ping', '127.0.0.1', '-t'], {
      stdio: 'ignore',
      windowsHide: true,
    })
    try {
      await sleep(300)
      const processes = await invokeTauri('process_snapshot')
      const process = processes.find((entry) => entry.pid === disposable.pid)
      expect(process).to.exist
      const plan = await invokeTauri('create_process_plan', { process })
      const execution = await invokeTauri('execute_process_plan', { confirmedPlan: plan })
      expect(execution.result.failed).to.equal(0)
      expect(execution.result.verified).to.equal(1)
    } finally {
      disposable.kill()
    }
  })

  it('exposes fail-closed supporting and Experimental workflow contracts', async () => {
    const hosts = await invokeTauri('list_hosts_entries')
    const hostsPlan = await invokeTauri('create_hosts_plan', { entries: hosts })
    expect(hostsPlan.descriptor.id).to.equal('hosts.save')
    expect(hostsPlan.descriptor.experimental).to.equal(true)

    const tasks = await invokeTauri('scan_scheduled_tasks')
    expect(tasks).to.have.property('collected_at')
    expect(tasks).to.have.property('tasks')

    const tweaks = await invokeTauri('list_system_tweak_targets')
    const stableTweak = tweaks.find((entry) => !entry.experimental)
    expect(stableTweak).to.exist
    const tweakPlan = await invokeTauri('create_system_tweak_plan', {
      kind: stableTweak.kind,
      tweakId: stableTweak.id,
    })
    expect(tweakPlan.descriptor.experimental).to.equal(false)
    expect(tweakPlan.targets).to.have.length(1)

    const presets = await invokeTauri('list_tweak_presets')
    expect(presets.map((preset) => preset.id)).to.include('privacy_essentials')
    expect(presets.map((preset) => preset.id)).to.include('desktop_responsiveness')

    const repairs = await invokeTauri('list_repair_targets')
    const diagnostic = repairs.find(
      (entry) => entry.execution_available && !entry.mutates_system && !entry.requires_admin
    )
    expect(diagnostic).to.exist
    const repairPlan = await invokeTauri('create_repair_plan', {
      actionId: diagnostic.id,
    })
    const repairExecution = await invokeTauri('execute_repair_plan', {
      confirmedPlan: repairPlan,
    })
    expect(repairExecution.result.failed).to.equal(0)
    expect(repairExecution.result.verified).to.equal(1)

    await invokeTauri('list_appx_candidates')
    let rejected = false
    try {
      await invokeTauri('create_appx_plan', {
        action: { kind: 'mark', family_names: ['Microsoft.WindowsStore_8wekyb3d8bbwe'] },
      })
    } catch {
      rejected = true
    }
    expect(rejected).to.equal(true)
  })
})
