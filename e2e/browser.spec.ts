import { test, expect } from '@playwright/test'

const routes = [
  { path: '/', title: 'Home' },
  { path: '/recommendations', title: 'Recommendations' },
  { path: '/disk-cleaner', title: 'Clean Up' },
  { path: '/performance', title: 'Performance' },
  { path: '/tools', title: 'Choose a tool' },
  { path: '/settings', title: 'System details and history' },
  { path: '/privacy', title: 'Privacy & Updates' },
  { path: '/repair-center', title: 'Repair Center' },
  { path: '/startup-manager', title: 'Startup Manager' },
  { path: '/system-tweaks', title: 'System Tweaks' },
  { path: '/hosts-editor', title: 'Hosts Editor' },
  { path: '/env-vars', title: 'Environment Variables' },
  { path: '/appx-manager', title: 'Preinstalled Apps Policy' },
  { path: '/network', title: 'Network Monitor' },
  { path: '/disk-analyzer', title: 'Disk Analyzer' },
  { path: '/process-manager', title: 'Process Manager' },
  { path: '/system-info', title: 'System Info' },
  { path: '/history', title: 'History' },
]

test.describe('WinOptimizer browser shell', () => {
  test('loads without native IPC and renders the app shell', async ({ page }) => {
    await page.goto('/')
    await expect(page.locator('#app')).toBeVisible()
    await expect(page.locator('aside')).toBeVisible()
    await expect(page.locator('main')).toBeVisible()
  })

  test('keeps the services table and actions inside the viewport', async ({ page }) => {
    await page.addInitScript(() => {
      ;(window as any).__TAURI_INTERNALS__ = {
        invoke: async (command: string) => {
          if (command === 'list_startup_programs') return []
          if (command === 'list_services') {
            return [{
              name: 'VeryLongServiceNameThatMustRemainReadable',
              display_name: 'A very long display name that should be truncated instead of widening the table',
              status: 'Stopped',
              start_type: 'auto',
              description: null,
              guidance: {
                group: 'set_manual',
                action: 'set_manual',
                rationale: 'This fixture is eligible for a reviewed Manual recommendation.',
                conditions: [],
                source_type: 'microsoft',
                source_title: 'Microsoft guidance',
                source_url: 'https://learn.microsoft.com/',
                os_scope: 'Windows 10/11 desktop',
                catalog_version: '1.0.0',
                protected: false,
                matched_by: 'exact',
              },
            }]
          }
          if (command === 'check_elevated') return true
          throw new Error('Unexpected browser command: ' + command)
        },
      }
    })

    for (const width of [800, 1100, 1366]) {
      await page.setViewportSize({ width, height: 720 })
      await page.goto('/startup-manager')
      await page.getByRole('tab', { name: 'Services', exact: true }).click()
      await expect(page.locator('select.start-type-select')).toBeVisible()
      await expect(page.locator('.svc-display-name')).toBeVisible()

      const horizontalOverflow = await page.locator('.services-table .table-wrapper').evaluate((element) => (
        element.scrollWidth > element.clientWidth + 1
      ))
      expect(horizontalOverflow).toBe(false)
      await expect(page.locator('.services-table .svc-actions')).toBeVisible()
    }
  })

  test('renders every route and responsive shell states', async ({ page }) => {
    await page.setViewportSize({ width: 1365, height: 900 })
    for (const route of routes) {
      await page.goto(route.path)
      await expect(page.locator('body')).toContainText(route.title)
      await expect(page.locator('main')).toBeVisible()
      await expect(page.getByRole('heading').first()).toBeVisible()
    }

    await page.setViewportSize({ width: 390, height: 844 })
    await page.goto('/')
    await expect(page.locator('aside')).toBeVisible()
    await expect(page.locator('main')).toBeVisible()
  })

  test('sidebar navigation updates the active route', async ({ page }) => {
    await page.goto('/')
    await expect(page.getByRole('navigation', { name: 'Main navigation' }).getByRole('button')).toHaveCount(9)
    await page.getByRole('navigation', { name: 'Main navigation' }).getByRole('button', { name: 'Clean Up', exact: true }).click()
    await expect(page).toHaveURL(/disk-cleaner/)
    await expect(page.locator('body')).toContainText('Clean Up')
    await page.getByRole('navigation', { name: 'Main navigation' }).getByRole('button', { name: 'History & Undo', exact: true }).click()
    await expect(page).toHaveURL(/history/)
    await expect(page.locator('body')).toContainText('History & Undo')
  })

  test('Home stays usable at required sizes, keyboard-only, and 200% zoom', async ({ page }) => {
    for (const { width, height } of [{ width: 800, height: 600 }, { width: 1100, height: 750 }, { width: 1366, height: 768 }]) {
      await page.setViewportSize({ width, height })
      await page.goto('/')
      await expect(page.getByRole('heading', { name: /No findings yet|Recommendations are reviewed separately/ }).first()).toBeVisible()
      await expect(page.getByRole('button', { name: /Review scan|Review recommendations/ }).first()).toBeVisible()
      await expect(page.getByRole('button', { name: 'This PC' })).toBeVisible()
    }

    await page.keyboard.press('Tab')
    await expect(page.locator(':focus')).toBeVisible()

    await page.evaluate(() => { document.documentElement.style.zoom = '200%' })
    await expect(page.getByRole('heading', { name: /No findings yet|Recommendations are reviewed separately/ }).first()).toBeVisible()
    await expect(page.getByRole('button', { name: /Review scan|Review recommendations/ }).first()).toBeVisible()
    await expect(page.getByRole('button', { name: 'This PC' })).toBeVisible()
  })

  test('captures every route at desktop target sizes', async ({ page }) => {
    for (const { width, height } of [{ width: 1280, height: 720 }, { width: 1366, height: 768 }, { width: 1440, height: 900 }, { width: 1920, height: 1080 }]) {
      await page.setViewportSize({ width, height })
      for (const route of routes) {
        await page.goto(route.path)
        await expect(page.locator('body')).toContainText(route.title)
        await page.waitForTimeout(250)
        await page.screenshot({ path: test.info().outputPath(`${route.path === '/' ? 'home' : route.path.slice(1)}-${width}x${height}.png`), fullPage: true })
      }
    }
  })

  test('Clean Up reference slice is keyboard usable at required viewport sizes and 200% zoom', async ({ page }) => {
    await page.addInitScript(() => {
      const target = {
        target_id: 'c:\\temp\\safe.tmp',
        category: 'temp',
        resource_id: 'C:\\Temp\\safe.tmp',
        description: 'Safe temporary file',
        size_bytes: 4096,
        last_modified: '1',
        confidence: 'high',
        risk: 'low',
        recommended: true,
        recommendation_reason: 'Temporary files are the only clearly safe default.',
        accessibility: 'accessible',
        reversibility: 'not_reversible',
        affected_application: null,
        group_name: 'User temporary files',
        cleanup_type: 'Root',
        requires_elevation: false,
      }
      const targets = [
        target,
        {
          ...target,
          target_id: 'c:\\browser\\cache.bin',
          category: 'browser_cache',
          resource_id: 'C:\\Browser\\cache.bin',
          description: 'Browser cache file',
          size_bytes: 8192,
          risk: 'medium',
          recommended: false,
          recommendation_reason: 'May affect signed-in applications; select explicitly.',
          affected_application: 'Example Browser',
          group_name: 'Example Browser',
          cleanup_type: 'Cache',
        },
        {
          ...target,
          target_id: 'c:\\windows\\softwaredistribution\\download\\update.bin',
          category: 'update_cache',
          resource_id: 'C:\\Windows\\SoftwareDistribution\\Download\\update.bin',
          description: 'Downloaded update file',
          size_bytes: 16384,
          risk: 'medium',
          recommended: false,
          recommendation_reason: 'Windows Update data requires explicit selection and elevation.',
          requires_elevation: true,
        },
        {
          ...target,
          target_id: 'c:\\windows\\logs\\system.log',
          category: 'logs',
          resource_id: 'C:\\Windows\\Logs\\system.log',
          description: 'System log file',
          size_bytes: 2048,
          risk: 'medium',
          recommended: false,
          recommendation_reason: 'Logs may be needed for diagnosis; select explicitly.',
          requires_elevation: true,
        },
      ]
      ;(window as any).__TAURI_INTERNALS__ = {
        invoke: async (command: string) => {
          if (command === 'scan_cleanup_targets') {
            return {
              scan_id: 'browser-scan',
              scanned_at: '2026-07-27T12:00:00Z',
              targets,
              estimated_bytes: targets.reduce((total, item) => total + item.size_bytes, 0),
              free_space_before: 100000,
              failures: [],
            }
          }
          if (command === 'create_cleanup_plan') {
            return {
              id: 'browser-plan',
              descriptor: {
                id: 'cleanup.reference_slice',
                title: 'Clean Up selected targets',
                category: 'cleanup',
                risk: 'low',
                privilege: 'standard_user',
                reversibility: 'not_reversible',
                reboot_policy: 'not_required',
                experimental: false,
              },
              targets: [{ target_id: target.target_id, requested_action: '{}', description: target.description }],
              warnings: [],
              expected_effects: [],
              requires_elevation: false,
              requires_restart: false,
              reversibility: 'not_reversible',
            }
          }
          throw new Error(`Unexpected browser command: ${command}`)
        },
      }
    })

    for (const { width, height } of [
      { width: 800, height: 600 },
      { width: 1100, height: 750 },
      { width: 1366, height: 768 },
    ]) {
      await page.setViewportSize({ width, height })
      await page.goto('/disk-cleaner')
      await expect(page.getByText('Scan to review cleanable files')).toBeVisible()
      await page.getByRole('button', { name: 'Scan', exact: true }).first().click()
      await expect(page.getByText('Choose what to clean')).toBeVisible()
      await expect(page.locator('.category-card')).toHaveCount(4)
      await page.screenshot({
        path: test.info().outputPath(`cleanup-${width}x${height}.png`),
        fullPage: true,
      })
    }

    await page.locator('.category-card').filter({
      has: page.getByRole('heading', { name: 'Temporary files', exact: true }),
    })
      .getByRole('button', { name: 'Review files' }).focus()
    await page.keyboard.press('Enter')
    await page.getByRole('checkbox', { name: /Safe temporary file/ }).focus()
    await page.keyboard.press('Space')
    await page.keyboard.press('Space')
    await page.getByRole('button', { name: /Review 1 selected/ }).focus()
    await page.keyboard.press('Enter')
    await expect(page.getByRole('dialog')).toBeVisible()
    await page.keyboard.press('Escape')
    await expect(page.getByRole('dialog')).toBeHidden()
    await expect(page.getByRole('button', { name: /Review 1 selected/ })).toBeFocused()

    await page.evaluate(() => { document.documentElement.style.zoom = '200%' })
    await expect(page.getByText('Choose what to clean')).toBeVisible()
    await page.screenshot({
      path: test.info().outputPath('cleanup-800x600-zoom-200.png'),
      fullPage: true,
    })
  })
})
