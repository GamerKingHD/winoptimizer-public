<script setup lang="ts">
import AppHeader from '../components/AppHeader.vue'

const localStorageItems = [
  {
    term: 'Operation journal',
    detail: 'Records operation history, lifecycle events, target states, and recovery relationships needed by History & Undo.',
  },
  {
    term: 'Application settings',
    detail: 'Stores your experience, Labs, cleanup, history, online, telemetry, crash-reporting, update, and recommendation preferences.',
  },
  {
    term: 'Scan snapshots',
    detail: 'Stores Disk Analyzer snapshots and scan metadata when snapshot persistence is enabled.',
  },
]
</script>

<template>
  <div class="privacy-page">
    <AppHeader />

    <section class="intro" aria-labelledby="privacy-heading">
      <p class="eyebrow">Transparency</p>
      <h2 id="privacy-heading">Privacy &amp; Updates</h2>
      <p>WinOptimizer keeps its core workflows local. This page describes what the app stores, what it does not collect, and how optional updates are handled.</p>
    </section>

    <section class="privacy-summary" aria-labelledby="offline-heading">
      <span class="status" role="status">Core workflows offline</span>
      <div>
        <h3 id="offline-heading">Local work does not depend on a connection</h3>
        <p>
          Scanning, cleanup review and execution, and history can operate without Internet access.
          This page does not start a network request.
        </p>
      </div>
    </section>

    <section class="privacy-section" aria-labelledby="local-storage-heading">
      <div class="section-heading">
        <p class="eyebrow">Local storage</p>
        <h2 id="local-storage-heading">What WinOptimizer stores locally</h2>
        <p>
          Most app-owned data is kept in the local application data directory, normally <code>%LOCALAPPDATA%\WinOptimizer</code>.
        </p>
      </div>

      <dl class="storage-grid">
        <div v-for="item in localStorageItems" :key="item.term" class="storage-item">
          <dt>{{ item.term }}</dt>
          <dd>{{ item.detail }}</dd>
        </div>
      </dl>

      <p class="section-note">Backups, undo records, logs, and support bundles are also local app-owned artifacts. A support bundle is created only when you explicitly export one. Local records can contain file paths or operation details needed to identify and recover your work; they are not uploaded by core workflows.</p>
    </section>

    <div class="privacy-grid">
      <section class="privacy-section" aria-labelledby="internet-heading">
        <div class="section-heading">
          <p class="eyebrow">Optional connectivity</p>
          <h2 id="internet-heading">Optional Internet behavior</h2>
        </div>
        <p>There are no always-on online features in the core workflows. Automatic release checks and guidance updates have separate opt-in settings and never run unless the matching preference is enabled. An explicit action by you, such as pressing &ldquo;Check for updates&rdquo;, is itself consent for that one request and does not require the automatic preference. Choosing either one must not make local scanning, cleanup, history, or recovery require the Internet.</p>
        <p class="section-note">
          The optional transport uses HTTPS only and does not send cookies, credentials, telemetry, or crash reports. A failed request is reported locally and leaves bundled guidance active.
        </p>
      </section>

      <section class="privacy-section" aria-labelledby="collection-heading">
        <div class="section-heading">
          <p class="eyebrow">Collection status</p>
          <h2 id="collection-heading">Telemetry and crash reports</h2>
        </div>
        <div class="status-list">
          <div class="status-row">
            <span>Telemetry</span>
            <strong>Off by default</strong>
          </div>
          <p>Telemetry is off by default, independently consented, and versioned under schema 1/privacy notice 1. If enabled, only fixed coarse operational categories can be recorded locally; this build has no telemetry upload transport.</p>
          <div class="status-row">
            <span>Crash reporting</span>
            <strong>Separate opt-in</strong>
          </div>
          <p>Crash reporting is off by default and is never implied by telemetry consent. No crash-report upload transport is enabled; explicit local diagnostics remain under your control through the support-bundle export in Settings.</p>
        </div>
      </section>
    </div>

    <section class="update-disclosure" aria-labelledby="updates-heading">
      <span class="disclosure-icon" aria-hidden="true">
        <svg viewBox="0 0 24 24"><path d="M12 3 19 6v5c0 4.5-3 8-7 10-4-2-7-5.5-7-10V6l7-3Z" /><path d="m9 12 2 2 4-4" /></svg>
      </span>
      <div>
        <p class="eyebrow">Opt-in update disclosure</p>
        <h2 id="updates-heading">Updates stay explicit and authenticated</h2>
        <p>Automatic release checks and guidance updates are opt-in and off by default. An explicit action such as &ldquo;Check for updates&rdquo; is consent for that one request and may download and stage a signed package; installing it is always a separate, explicit action. Any download must be cryptographically authenticated through the existing signature and payload-digest checks before it can be accepted. Network failure keeps the bundled guidance available and cannot affect local operations; enabling one update preference does not silently enable another or the core offline workflows.</p>
      </div>
    </section>
  </div>
</template>

<style scoped>
.privacy-page { width: 100%; min-width: 0; }
.intro { margin-bottom: var(--space-5); }
.intro h2 { margin: var(--space-1) 0 var(--space-2); color: var(--text-primary); font-size: var(--text-xl); }
.intro p:not(.eyebrow) { max-width: 760px; margin: 0; color: var(--text-secondary); line-height: var(--leading-normal); }
.eyebrow { margin: 0; color: var(--accent); font-size: var(--text-xs); font-weight: var(--weight-semibold); letter-spacing: var(--tracking-wide); text-transform: uppercase; }
.privacy-summary, .privacy-section, .update-disclosure { border: 1px solid var(--border-default); border-radius: var(--radius-xl); background: var(--bg-elevated); }
.privacy-summary { display: grid; grid-template-columns: auto 1fr; gap: var(--space-4); align-items: start; margin-bottom: var(--space-5); padding: var(--space-4); }
.privacy-summary h3 { margin: 0 0 var(--space-1); color: var(--text-primary); font-size: var(--text-base); }
.privacy-summary p { margin: 0; color: var(--text-secondary); font-size: var(--text-sm); line-height: var(--leading-relaxed); }
.status { padding: var(--space-1) var(--space-2); border: 1px solid var(--border-default); border-radius: var(--radius-full); color: var(--accent); background: var(--bg-hover); font-size: var(--text-xs); font-weight: var(--weight-semibold); white-space: nowrap; }
.privacy-section { padding: var(--space-4); }
.section-heading { margin-bottom: var(--space-4); }
.section-heading h2 { margin: var(--space-1) 0 var(--space-2); color: var(--text-primary); font-size: var(--text-md); }
.section-heading p:not(.eyebrow) { margin: 0; color: var(--text-secondary); font-size: var(--text-sm); line-height: var(--leading-relaxed); }
code { padding: 0 var(--space-1); color: var(--text-primary); background: var(--bg-hover); border-radius: var(--radius-sm); font-size: .9em; }
.storage-grid { display: grid; grid-template-columns: repeat(auto-fit, minmax(220px, 1fr)); gap: var(--space-3); margin: 0; }
.storage-item { padding: var(--space-3); border: 1px solid var(--border-subtle); border-radius: var(--radius-lg); }
.storage-item dt { color: var(--text-primary); font-size: var(--text-sm); font-weight: var(--weight-semibold); }
.storage-item dd { margin: var(--space-1) 0 0; color: var(--text-secondary); font-size: var(--text-xs); line-height: var(--leading-relaxed); }
.section-note { margin: var(--space-4) 0 0; color: var(--text-muted); font-size: var(--text-xs); line-height: var(--leading-relaxed); }
.privacy-grid { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: var(--space-4); margin-top: var(--space-4); }
.privacy-section > p { margin: 0; color: var(--text-secondary); font-size: var(--text-sm); line-height: var(--leading-relaxed); }
.status-list { display: grid; gap: var(--space-2); }
.status-row { display: flex; align-items: center; justify-content: space-between; gap: var(--space-3); padding-bottom: var(--space-2); border-bottom: 1px solid var(--border-subtle); color: var(--text-primary); font-size: var(--text-sm); }
.status-row strong { color: var(--text-muted); font-size: var(--text-xs); font-weight: var(--weight-semibold); text-transform: uppercase; }
.status-list p { margin: 0 0 var(--space-2); color: var(--text-secondary); font-size: var(--text-xs); line-height: var(--leading-relaxed); }
.update-disclosure { display: grid; grid-template-columns: auto 1fr; gap: var(--space-3); align-items: start; margin-top: var(--space-4); padding: var(--space-4); border-color: var(--accent); }
.disclosure-icon { display: grid; place-items: center; width: 36px; height: 36px; color: var(--accent); background: var(--bg-hover); border-radius: 50%; }
.disclosure-icon svg { width: 19px; height: 19px; fill: none; stroke: currentColor; stroke-width: 1.8; stroke-linecap: round; stroke-linejoin: round; }
.update-disclosure h2 { margin: var(--space-1) 0 var(--space-2); color: var(--text-primary); font-size: var(--text-md); }
.update-disclosure p:last-child { margin: 0; color: var(--text-secondary); font-size: var(--text-sm); line-height: var(--leading-relaxed); }
@media (max-width: 700px) {
  .privacy-grid { grid-template-columns: 1fr; }
}
@media (max-width: 560px) {
  .privacy-summary, .update-disclosure { grid-template-columns: 1fr; }
  .status { justify-self: start; }
}
</style>
