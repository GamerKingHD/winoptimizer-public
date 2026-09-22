<script setup lang="ts">
import { onMounted, ref } from 'vue'
import AppHeader from '../components/AppHeader.vue'
import BaseButton from '../components/BaseButton.vue'
import { exportSupportBundle, type SupportBundleOptions } from '../api/operation'
import { getSettings, updateSettings, type AppSettings } from '../api/settings'
import { checkForUpdates, installUpdate, type UpdateCheckResult, type UpdateDistribution, type UpdateInstallResult } from '../api/update'

const settings = [
  { to: '/system-info', title: 'System Information', description: 'View Windows, hardware, storage, and installed update details.', icon: 'info' },
  { to: '/history', title: 'History & Undo', description: 'Review recorded changes and undo eligible actions.', icon: 'history' },
  { to: '/privacy', title: 'Privacy & Updates', description: 'See local data storage, disabled collection, and opt-in update behavior.', icon: 'privacy' },
]

const supportOptions = ref<SupportBundleOptions>({
  redact_user_profile_path: true,
  redact_network_addresses: true,
  redact_hostnames: true,
  include_exact_file_paths: false,
  include_command_output: false,
})
const exportingSupportBundle = ref(false)
const supportBundlePath = ref('')
const supportBundleError = ref('')
const appSettings = ref<AppSettings | null>(null)
const settingsLoading = ref(true)
const settingsSaving = ref(false)
const settingsError = ref('')
const settingsSaved = ref(false)
const updateDistribution = ref<UpdateDistribution>('portable')
const updateChecking = ref(false)
const updateInstalling = ref(false)
const updateResult = ref<UpdateCheckResult | null>(null)
const updateInstallResult = ref<UpdateInstallResult | null>(null)
const updateError = ref('')

async function loadSettings() {
  settingsLoading.value = true
  settingsError.value = ''
  settingsSaved.value = false
  try {
    appSettings.value = await getSettings()
    if (appSettings.value.update_preferences.check_automatically) {
      void runUpdateCheck(false)
    }
  } catch (error) {
    appSettings.value = null
    settingsError.value = String(error)
  } finally {
    settingsLoading.value = false
  }
}

async function runUpdateCheck(manual: boolean) {
  updateChecking.value = true
  updateError.value = ''
  updateResult.value = null
  updateInstallResult.value = null
  try {
    updateResult.value = await checkForUpdates(manual, 'release', updateDistribution.value)
  } catch (error) {
    updateError.value = String(error)
  } finally {
    updateChecking.value = false
  }
}

async function installVerifiedUpdate() {
  updateInstalling.value = true
  updateError.value = ''
  updateInstallResult.value = null
  try {
    updateInstallResult.value = await installUpdate(updateDistribution.value)
  } catch (error) {
    updateError.value = String(error)
  } finally {
    updateInstalling.value = false
  }
}

async function saveSettings() {
  if (!appSettings.value) return
  settingsSaving.value = true
  settingsError.value = ''
  settingsSaved.value = false
  try {
    appSettings.value = await updateSettings(appSettings.value)
    settingsSaved.value = true
  } catch (error) {
    settingsError.value = String(error)
  } finally {
    settingsSaving.value = false
  }
}

onMounted(loadSettings)

async function createSupportBundle() {
  exportingSupportBundle.value = true
  supportBundlePath.value = ''
  supportBundleError.value = ''
  try {
    supportBundlePath.value = await exportSupportBundle(supportOptions.value)
  } catch (error) {
    supportBundleError.value = String(error)
  } finally {
    exportingSupportBundle.value = false
  }
}
</script>

<template>
  <div class="settings-page">
    <AppHeader />

    <section class="intro" aria-labelledby="settings-heading">
      <h2 id="settings-heading">System details and history</h2>
      <p>Review system details, recorded changes, and privacy behavior.</p>
    </section>

    <section class="preferences-card" aria-labelledby="preferences-heading">
      <div>
        <p class="cache-note-label">Application preferences</p>
        <h2 id="preferences-heading">Progressive disclosure</h2>
        <p class="support-copy">These settings are stored locally. If they cannot be loaded, no changes can be submitted.</p>
      </div>
      <div v-if="settingsLoading" class="support-copy">Loading application settings…</div>
      <form v-else-if="appSettings" class="preferences-form" @submit.prevent="saveSettings">
        <label>
          <span>Experience level</span>
          <select v-model="appSettings.experience_level">
            <option value="standard">Standard</option>
            <option value="advanced">Advanced</option>
            <option value="expert">Expert</option>
          </select>
        </label>
        <label class="checkbox-row">
          <input v-model="appSettings.labs_enabled" type="checkbox">
          <span>Enable Labs</span>
        </label>
        <div class="preference-option">
          <label class="checkbox-row">
            <input
              v-model="appSettings.update_preferences.check_automatically"
              aria-label="Allow optional update checks"
              type="checkbox"
            >
            <span>Allow optional update checks</span>
          </label>
          <small>Off by default. Enabling this preference does not make local workflows depend on the Internet.</small>
        </div>
        <div class="preference-option">
          <label class="checkbox-row">
            <input
              v-model="appSettings.telemetry_preferences.enabled"
              aria-label="Allow privacy-minimized telemetry"
              type="checkbox"
            >
            <span>Allow privacy-minimized telemetry</span>
          </label>
          <small>Off by default. If enabled, only the versioned, coarse, allow-listed local event schema is eligible for future transport; paths, host data, command output, identifiers, and user text are excluded.</small>
        </div>
        <div class="preference-option">
          <label class="checkbox-row">
            <input
              v-model="appSettings.crash_reporting_preferences.enabled"
              aria-label="Allow crash reporting"
              type="checkbox"
            >
            <span>Allow crash reporting</span>
          </label>
          <small>Separate opt-in. Crash reporting is disabled by default and is not implied by telemetry consent.</small>
        </div>
        <div class="preference-option">
          <label class="checkbox-row">
            <input
              v-model="appSettings.recommendation_preferences.update_guidance"
              aria-label="Allow optional guidance updates"
              type="checkbox"
            >
            <span>Allow optional guidance updates</span>
          </label>
          <small>Separate opt-in for refreshed service and task guidance. Bundled guidance remains available if the network is unavailable.</small>
        </div>
        <div class="preferences-actions">
          <BaseButton type="submit" variant="primary" size="sm" :loading="settingsSaving">Save settings</BaseButton>
          <span v-if="settingsSaved" class="support-result" role="status">Settings saved.</span>
        </div>
      </form>
      <div v-else class="preferences-error" role="alert">
        <p>Application settings are unavailable and remain unchanged.</p>
        <p class="support-error">{{ settingsError }}</p>
        <BaseButton variant="secondary" size="sm" :loading="settingsLoading" @click="loadSettings">Retry</BaseButton>
      </div>
    </section>

    <section class="preferences-card update-card" aria-labelledby="updates-heading">
      <div>
        <p class="cache-note-label">Authenticated updates</p>
        <h2 id="updates-heading">Check and install explicitly</h2>
        <p class="support-copy">Automatic checks use only a signed manifest after opt-in. A manual check may download and stage a signed package; installation always remains a separate action.</p>
      </div>
      <div class="update-actions">
        <label>
          <span>Installation form</span>
          <select v-model="updateDistribution" :disabled="updateChecking || updateInstalling">
            <option value="portable">Portable executable</option>
            <option value="msi">MSI installer</option>
          </select>
        </label>
        <BaseButton type="button" variant="secondary" size="sm" :loading="updateChecking" @click="runUpdateCheck(true)">Check for updates</BaseButton>
        <BaseButton
          v-if="updateResult?.staged_artifact"
          type="button"
          variant="primary"
          size="sm"
          :loading="updateInstalling"
          @click="installVerifiedUpdate"
        >Install verified update</BaseButton>
      </div>
      <p v-if="updateResult" class="support-result" role="status">
        {{ updateResult.availability === 'available' ? `Signed update ${updateResult.version} is available.` : `You are up to date at ${updateResult.current_version}.` }}
        <span v-if="updateResult.staged_artifact"> Package staged for {{ updateResult.distribution }}.</span>
      </p>
      <p v-if="updateInstallResult" class="support-result" role="status">
        {{ updateInstallResult.status === 'portable_replacement_scheduled' ? 'Portable replacement scheduled; close and restart the app to complete it.' : 'MSI installer launched for the explicit upgrade action.' }}
      </p>
      <p v-if="updateError" class="support-error" role="alert">{{ updateError }}</p>
    </section>

    <nav class="settings-list" aria-label="System, history, privacy, and updates">
      <RouterLink v-for="setting in settings" :key="setting.to" :to="setting.to" class="setting-row">
        <span class="setting-icon" aria-hidden="true">
          <svg v-if="setting.icon === 'info'" viewBox="0 0 24 24"><circle cx="12" cy="12" r="9" /><path d="M12 11v5M12 8h.01" /></svg>
          <svg v-else-if="setting.icon === 'privacy'" viewBox="0 0 24 24"><path d="M12 3 19 6v5c0 4.5-3 8-7 10-4-2-7-5.5-7-10V6l7-3Z" /><path d="m9 12 2 2 4-4" /></svg>
          <svg v-else viewBox="0 0 24 24"><circle cx="12" cy="12" r="9" /><path d="M12 7v5l3 2" /></svg>
        </span>
        <span class="setting-copy"><strong>{{ setting.title }}</strong><span>{{ setting.description }}</span></span>
        <span class="setting-arrow" aria-hidden="true">→</span>
      </RouterLink>
    </nav>

    <section class="support-card" aria-labelledby="support-heading">
      <div>
        <p class="cache-note-label">Support</p>
        <h2 id="support-heading">Export a support bundle</h2>
        <p class="support-copy">Creates a local diagnostic package with operation history and conservative redaction enabled by default.</p>
      </div>
      <div class="support-options">
        <label><input v-model="supportOptions.redact_user_profile_path" type="checkbox"> Redact user profile paths</label>
        <label><input v-model="supportOptions.redact_network_addresses" type="checkbox"> Redact network addresses</label>
        <label><input v-model="supportOptions.redact_hostnames" type="checkbox"> Redact hostnames</label>
        <label><input v-model="supportOptions.include_exact_file_paths" type="checkbox"> Include exact file paths</label>
        <label><input v-model="supportOptions.include_command_output" type="checkbox"> Include command output</label>
      </div>
      <p class="support-warning">Only enable exact paths or command output when support staff specifically request them.</p>
      <BaseButton variant="primary" size="sm" :loading="exportingSupportBundle" @click="createSupportBundle">
        Export Support Bundle
      </BaseButton>
      <p v-if="supportBundlePath" class="support-result">Saved locally: {{ supportBundlePath }}</p>
      <p v-if="supportBundleError" class="support-error">Could not export bundle: {{ supportBundleError }}</p>
    </section>

    <aside class="cache-note" aria-labelledby="cache-compression-heading">
      <span class="cache-note-icon" aria-hidden="true">
        <svg viewBox="0 0 24 24"><circle cx="12" cy="12" r="9" /><path d="M12 11v5M12 8h.01" /></svg>
      </span>
      <div class="cache-note-copy">
        <p class="cache-note-label">Disk Analyzer cache</p>
        <h3 id="cache-compression-heading">Windows compression is automatic</h3>
        <p>Completed scan snapshots use Windows Overlay Filter (WOF) compression. Windows opens them transparently, so the cache stays reusable while taking less space. Files on the scanned drive are not changed.</p>
        <ul class="cache-policy">
          <li><strong>Up to 5 logical processors</strong><span>XPRESS4K — lightest background work.</span></li>
          <li><strong>6–11 logical processors</strong><span>XPRESS8K — balanced space and CPU use.</span></li>
          <li><strong>12–15 logical processors</strong><span>XPRESS16K — more compact storage.</span></li>
          <li><strong>16 or more logical processors</strong><span>LZX — smallest storage, most compression work.</span></li>
        </ul>
        <p class="cache-note-footnote">Compression is queued lazily at low priority. If Windows cannot provide WOF on the cache drive, the snapshot remains valid and simply stays uncompressed.</p>
      </div>
    </aside>
  </div>
</template>

<style scoped>
.settings-page { width: 100%; min-width: 0; }
.intro { margin-bottom: var(--space-5); }
.intro h2 { margin: 0 0 var(--space-1); color: var(--text-primary); font-size: var(--text-lg); }
.intro p { margin: 0; color: var(--text-secondary); font-size: var(--text-sm); }
.preferences-card, .settings-list { background: var(--bg-elevated); border: 1px solid var(--border-default); border-radius: var(--radius-xl); }
.preferences-card { display: grid; gap: var(--space-3); margin-bottom: var(--space-5); padding: var(--space-4); }
.preferences-card h2 { margin: 0 0 var(--space-1); color: var(--text-primary); font-size: var(--text-md); }
.preferences-form { display: grid; gap: var(--space-3); }
.preferences-form label:not(.checkbox-row) { display: grid; gap: var(--space-1); color: var(--text-secondary); font-size: var(--text-xs); }
.preferences-form select { max-width: 260px; padding: var(--space-2); color: var(--text-primary); background: var(--bg-base); border: 1px solid var(--border-default); border-radius: var(--radius-md); }
.preferences-form small { color: var(--text-muted); font-size: var(--text-xs); }
.preference-option { display: grid; gap: var(--space-1); }
.checkbox-row { display: flex; gap: var(--space-2); align-items: center; color: var(--text-secondary); font-size: var(--text-xs); }
.preferences-actions { display: flex; gap: var(--space-3); align-items: center; }
.update-actions { display: flex; flex-wrap: wrap; gap: var(--space-3); align-items: end; }
.update-actions label { display: grid; gap: var(--space-1); color: var(--text-secondary); font-size: var(--font-size-sm); }
.update-actions select { min-width: 220px; padding: var(--space-2); color: var(--text-primary); background: var(--bg-base); border: 1px solid var(--border-default); border-radius: var(--radius-md); }
.preferences-error { display: grid; gap: var(--space-2); }
.preferences-error p { margin: 0; }
.settings-list { overflow: hidden; }
.setting-row { display: grid; grid-template-columns: auto 1fr auto; align-items: center; gap: var(--space-3); padding: var(--space-4); color: inherit; text-decoration: none; border-bottom: 1px solid var(--border-subtle); transition: background var(--duration-fast); }
.setting-row:last-child { border-bottom: 0; }
.setting-row:hover { background: var(--bg-hover); }
.setting-icon { display: grid; place-items: center; width: 40px; height: 40px; color: var(--accent); background: var(--bg-hover); border-radius: var(--radius-lg); }
.setting-icon svg { width: 20px; height: 20px; fill: none; stroke: currentColor; stroke-width: 1.8; stroke-linecap: round; stroke-linejoin: round; }
.setting-copy { display: grid; gap: var(--space-1); }
.setting-copy strong { color: var(--text-primary); font-size: var(--text-sm); font-weight: var(--weight-semibold); }
.setting-copy span { color: var(--text-secondary); font-size: var(--text-xs); line-height: var(--leading-relaxed); }
.setting-arrow { color: var(--text-muted); font-size: var(--text-lg); }
.support-card { display: grid; gap: var(--space-3); margin-top: var(--space-5); padding: var(--space-4); background: var(--bg-elevated); border: 1px solid var(--border-default); border-radius: var(--radius-xl); }
.support-card h2 { margin: 0 0 var(--space-1); color: var(--text-primary); font-size: var(--text-md); }
.support-copy, .support-warning, .support-result, .support-error { margin: 0; color: var(--text-secondary); font-size: var(--text-xs); line-height: var(--leading-relaxed); }
.support-warning, .support-error { color: var(--text-muted); }
.support-options { display: grid; gap: var(--space-2); color: var(--text-secondary); font-size: var(--text-xs); }
.support-options label { display: flex; gap: var(--space-2); align-items: center; }
.cache-note { display: grid; grid-template-columns: auto 1fr; gap: var(--space-3); margin-top: var(--space-5); padding: var(--space-4); background: var(--bg-elevated); border: 1px solid var(--accent); border-radius: var(--radius-xl); }
.cache-note-icon { display: grid; place-items: center; width: 36px; height: 36px; color: var(--accent); background: var(--bg-hover); border-radius: 50%; }
.cache-note-icon svg { width: 19px; height: 19px; fill: none; stroke: currentColor; stroke-width: 1.8; stroke-linecap: round; stroke-linejoin: round; }
.cache-note-copy { min-width: 0; }
.cache-note-label { margin: 0 0 var(--space-1); color: var(--accent); font-size: var(--text-xs); font-weight: var(--weight-semibold); letter-spacing: .04em; text-transform: uppercase; }
.cache-note h3 { margin: 0 0 var(--space-1); color: var(--text-primary); font-size: var(--text-sm); font-weight: var(--weight-semibold); }
.cache-note-copy > p:not(.cache-note-label):not(.cache-note-footnote) { margin: 0; color: var(--text-secondary); font-size: var(--text-xs); line-height: var(--leading-relaxed); }
.cache-policy { display: grid; gap: var(--space-2); margin: var(--space-3) 0; padding: 0; color: var(--text-secondary); font-size: var(--text-xs); list-style: none; }
.cache-policy li { display: grid; grid-template-columns: minmax(170px, auto) 1fr; gap: var(--space-2); }
.cache-policy strong { color: var(--text-primary); font-weight: var(--weight-semibold); }
.cache-note-footnote { margin: 0; color: var(--text-muted); font-size: var(--text-xs); line-height: var(--leading-relaxed); }
@media (max-width: 640px) {
  .cache-policy li { grid-template-columns: 1fr; gap: 0; }
}
</style>
