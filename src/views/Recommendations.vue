<script setup lang="ts">
import { onMounted, ref } from 'vue'
import type { TweakPreset } from '../api/systemTweaks'
import { listTweakPresets } from '../api/systemTweaks'
import AppHeader from '../components/AppHeader.vue'

type EvidenceConfidence = 'high' | 'medium' | 'low' | 'unknown'

interface RecommendationPreset extends TweakPreset {
  finding: string
  mechanism: string
  tradeoffs: string[]
  confidence: EvidenceConfidence
  provenance: {
    source_type: string
    source_name: string
    source_version: string | null
    scope: string
    matching_rule: string
    confidence: EvidenceConfidence
    rationale: string
    tradeoffs: string[]
    conflict: 'none' | 'conflicting' | 'unknown'
    last_bundled_update: string | null
  }
}

const recommendations = ref<RecommendationPreset[]>([])
const loading = ref(true)
const loadError = ref(false)

function isRecommendationPreset(preset: TweakPreset): preset is RecommendationPreset {
  const candidate = preset as unknown as Record<string, unknown>
  const provenance = candidate.provenance as Record<string, unknown> | undefined
  return Boolean(
      typeof candidate.finding === 'string'
      && typeof candidate.mechanism === 'string'
      && Array.isArray(candidate.target_ids)
      && candidate.target_ids.length > 0
      && Array.isArray(candidate.tradeoffs)
      && (candidate.confidence === 'high'
        || candidate.confidence === 'medium'
        || candidate.confidence === 'low'
        || candidate.confidence === 'unknown')
      && provenance
      && typeof provenance.source_name === 'string'
      && typeof provenance.scope === 'string'
      && typeof provenance.rationale === 'string'
      && provenance.source_type !== 'unknown'
      && provenance.matching_rule !== 'unknown'
      && provenance.confidence === candidate.confidence
      && provenance.conflict === 'none',
  )
}

function confidenceLabel(confidence: EvidenceConfidence) {
  return confidence === 'unknown'
    ? 'Review manually — no machine-specific benefit measured'
    : `${confidence[0].toUpperCase()}${confidence.slice(1)} evidence`
}

async function loadRecommendations() {
  try {
    const presets = await listTweakPresets()
    recommendations.value = presets.filter(isRecommendationPreset)
  } catch {
    recommendations.value = []
    loadError.value = true
  } finally {
    loading.value = false
  }
}

onMounted(() => { void loadRecommendations() })

const reviewWorkflows = [
  {
    to: '/disk-cleaner',
    title: 'Storage cleanup',
    description: 'Scan temporary files, caches, and logs, then review exact targets before any cleanup.',
    action: 'Review storage',
  },
  {
    to: '/startup-manager',
    title: 'Startup entries',
    description: 'Inspect programs, services, and scheduled tasks that were detected on this PC.',
    action: 'Review startup entries',
  },
  {
    to: '/system-tweaks',
    title: 'System settings',
    description: 'Inspect current settings, evidence, risks, and reversibility in the normal tweaks workflow.',
    action: 'Review system settings',
  },
]
</script>

<template>
  <div class="recommendations-page" :aria-busy="loading">
    <AppHeader />

    <section class="intro" aria-labelledby="recommendations-heading">
      <p class="eyebrow">Read-only guidance</p>
      <h2 id="recommendations-heading">Recommendations</h2>
      <p>This page does not make system changes. Any action opens the normal workflow for review, planning, and confirmation.</p>
    </section>

    <section v-if="loading" class="manual-review" role="status" aria-live="polite">
      <span class="status">Checking machine state</span>
      <div>
        <p>Reading current tweak state before showing any optional guidance.</p>
      </div>
    </section>

    <section v-else-if="recommendations.length === 0" class="manual-review" role="status" aria-live="polite" aria-labelledby="manual-review-title">
      <span class="status">Review manually</span>
      <div>
        <h3 id="manual-review-title">No machine-specific recommendation is available</h3>
        <p v-if="loadError">Current tweak state could not be verified, so optional recommendations remain hidden.</p>
        <p v-else>WinOptimizer has not established enough evidence here to label an optimization as recommended. Choose a workflow below only if you want to inspect its current findings.</p>
      </div>
    </section>

    <section v-else class="recommendation-list" aria-labelledby="recommendation-list-title">
      <div class="section-heading">
        <span class="status">Review manually</span>
        <div>
          <h3 id="recommendation-list-title">Machine-aware review candidates</h3>
          <p>These settings are not applied and passed the stable-scope checks. They are suggestions for review, not measured performance claims.</p>
        </div>
      </div>
      <article v-for="recommendation in recommendations" :key="recommendation.id" class="recommendation-card">
        <h4>{{ recommendation.name }}</h4>
        <p class="finding">{{ recommendation.finding }}</p>
        <dl class="evidence-list">
          <div>
            <dt>Mechanism</dt>
            <dd>{{ recommendation.mechanism }}</dd>
          </div>
          <div>
            <dt>Confidence</dt>
            <dd>{{ confidenceLabel(recommendation.confidence) }}</dd>
          </div>
          <div>
            <dt>Provenance</dt>
            <dd>{{ recommendation.provenance.source_name }} · {{ recommendation.provenance.scope }}</dd>
          </div>
          <div>
            <dt>Evidence basis</dt>
            <dd>{{ recommendation.provenance.rationale }}</dd>
          </div>
          <div>
            <dt>Trade-offs</dt>
            <dd>{{ recommendation.tradeoffs.join(' ') }}</dd>
          </div>
        </dl>
        <RouterLink to="/system-tweaks" class="review-link">Review exact targets →</RouterLink>
      </article>
    </section>

    <nav class="workflow-grid" aria-label="Review workflows">
      <RouterLink v-for="workflow in reviewWorkflows" :key="workflow.to" :to="workflow.to" class="workflow-card">
        <span>
          <strong>{{ workflow.title }}</strong>
          <small>{{ workflow.description }}</small>
        </span>
        <span class="workflow-action">{{ workflow.action }} →</span>
      </RouterLink>
    </nav>
  </div>
</template>

<style scoped>
.recommendations-page { width: 100%; min-width: 0; }
.intro { margin-bottom: var(--space-5); }
.intro h2 { margin: var(--space-1) 0 var(--space-2); color: var(--text-primary); font-size: var(--text-xl); }
.intro p { max-width: 720px; margin: 0; color: var(--text-secondary); line-height: var(--leading-normal); }
.eyebrow { color: var(--accent) !important; font-size: var(--text-xs); font-weight: var(--weight-semibold); letter-spacing: var(--tracking-wide); text-transform: uppercase; }
.manual-review { display: grid; grid-template-columns: auto 1fr; gap: var(--space-4); align-items: start; margin-bottom: var(--space-5); padding: var(--space-4); border: 1px solid var(--border-default); border-radius: var(--radius-lg); background: var(--bg-elevated); }
.manual-review h3 { margin: 0 0 var(--space-1); color: var(--text-primary); font-size: var(--text-base); }
.manual-review p { margin: 0; color: var(--text-secondary); font-size: var(--text-sm); line-height: var(--leading-relaxed); }
.status { padding: var(--space-1) var(--space-2); border: 1px solid var(--border-default); border-radius: var(--radius-full); color: var(--text-secondary); font-size: var(--text-xs); font-weight: var(--weight-semibold); white-space: nowrap; }
.recommendation-list { display: grid; gap: var(--space-3); margin-bottom: var(--space-5); }
.section-heading { display: grid; grid-template-columns: auto 1fr; gap: var(--space-4); align-items: start; padding: var(--space-4); border: 1px solid var(--border-default); border-radius: var(--radius-lg); background: var(--bg-elevated); }
.section-heading h3 { margin: 0 0 var(--space-1); color: var(--text-primary); font-size: var(--text-base); }
.section-heading p { margin: 0; color: var(--text-secondary); font-size: var(--text-sm); line-height: var(--leading-relaxed); }
.recommendation-card { display: grid; gap: var(--space-3); padding: var(--space-4); border: 1px solid var(--border-default); border-radius: var(--radius-lg); background: var(--bg-elevated); }
.recommendation-card h4 { margin: 0; color: var(--text-primary); font-size: var(--text-base); }
.finding { margin: 0; color: var(--text-secondary); font-size: var(--text-sm); line-height: var(--leading-relaxed); }
.evidence-list { display: grid; gap: var(--space-2); margin: 0; }
.evidence-list > div { display: grid; grid-template-columns: 110px 1fr; gap: var(--space-3); }
.evidence-list dt { color: var(--text-secondary); font-size: var(--text-xs); font-weight: var(--weight-semibold); text-transform: uppercase; }
.evidence-list dd { margin: 0; color: var(--text-secondary); font-size: var(--text-sm); line-height: var(--leading-relaxed); overflow-wrap: anywhere; }
.review-link { color: var(--accent); font-size: var(--text-sm); font-weight: var(--weight-semibold); text-decoration: none; }
.review-link:hover { text-decoration: underline; }
.workflow-grid { display: grid; grid-template-columns: repeat(auto-fit, minmax(240px, 1fr)); gap: var(--space-3); }
.workflow-card { display: flex; min-height: 150px; flex-direction: column; justify-content: space-between; gap: var(--space-4); padding: var(--space-4); border: 1px solid var(--border-default); border-radius: var(--radius-xl); background: var(--bg-elevated); color: inherit; text-decoration: none; transition: border-color var(--duration-fast), box-shadow var(--duration-fast), transform var(--duration-fast); }
.workflow-card:hover { border-color: var(--accent); box-shadow: var(--shadow-sm); transform: translateY(-1px); }
.workflow-card > span:first-child { display: grid; gap: var(--space-2); }
.workflow-card strong { color: var(--text-primary); font-size: var(--text-base); }
.workflow-card small { color: var(--text-secondary); font-size: var(--text-sm); line-height: var(--leading-relaxed); }
.workflow-action { color: var(--accent); font-size: var(--text-sm); font-weight: var(--weight-semibold); }
@media (max-width: 560px) {
  .manual-review, .section-heading { grid-template-columns: 1fr; }
  .status { justify-self: start; }
  .evidence-list > div { grid-template-columns: 1fr; gap: var(--space-1); }
  .workflow-grid { grid-template-columns: 1fr; }
}
</style>
