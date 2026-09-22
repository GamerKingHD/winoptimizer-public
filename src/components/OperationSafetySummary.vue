<script setup lang="ts">
import { computed, useId } from 'vue'
import type { OperationPlan } from '../api/operation'
import BaseCard from './BaseCard.vue'
import EvidenceList from './EvidenceList.vue'
import PrivilegeBadge from './PrivilegeBadge.vue'
import ProofBadge from './ProofBadge.vue'
import RecoveryBadge from './RecoveryBadge.vue'
import RestartBadge from './RestartBadge.vue'
import RiskBadge from './RiskBadge.vue'

const props = withDefaults(defineProps<{
  plan: OperationPlan
  title?: string
  changes?: string[]
  warnings?: OperationPlan['warnings']
  targetCount?: number
  compact?: boolean
}>(), {
  title: 'Operation safety summary',
  compact: false,
})

const instanceId = useId()
const headingId = `operation-safety-${instanceId}`
const warningsId = `${headingId}-warnings`
const MAX_SUMMARY_ITEMS = 24

const descriptor = computed(() => props.plan.descriptor)
const targetCount = computed(() => props.targetCount ?? props.plan.targets?.length ?? 0)
const changeItems = computed(() => {
  const explicitChanges = props.changes?.filter(Boolean) ?? []
  if (explicitChanges.length > 0) return explicitChanges

  const expectedEffects = props.plan.expected_effects?.map(effect => effect.description).filter(Boolean) ?? []
  if (expectedEffects.length > 0) return expectedEffects

  const targetDescriptions = props.plan.targets?.map(target => target.description).filter(Boolean) ?? []
  return targetDescriptions.length > 0 ? targetDescriptions : ['No target changes were declared.']
})
const warningItems = computed(() => props.warnings ?? props.plan.warnings ?? [])
function boundedItems<T>(items: T[], remainderLabel: (count: number) => T): T[] {
  if (items.length <= MAX_SUMMARY_ITEMS) return items
  return [...items.slice(0, MAX_SUMMARY_ITEMS - 1), remainderLabel(items.length - MAX_SUMMARY_ITEMS + 1)]
}

const changeEvidenceItems = computed(() => boundedItems(
  changeItems.value.map(label => ({ label })),
  count => ({ label: `... and ${count} more declared changes` }),
))
const warningEvidenceItems = computed(() => boundedItems(
  warningItems.value.map(warning => ({
    label: warning.code,
    detail: warning.message,
    status: 'warning' as const,
  })),
  count => ({
    label: `... and ${count} more warnings`,
    status: 'warning' as const,
  }),
))

const risk = computed(() => descriptor.value?.risk ?? 'none')
const requiresElevation = computed(() =>
  props.plan.requires_elevation || descriptor.value?.privilege === 'administrator',
)

const verificationLabel = computed(() => {
  const requiresRevalidation = descriptor.value?.requires_revalidation
  const requiresVerification = descriptor.value?.requires_verification
  if (requiresRevalidation && requiresVerification) return 'Required before and after execution'
  if (requiresRevalidation) return 'Required before execution'
  if (requiresVerification) return 'Required after execution'
  return 'Not required'
})

</script>

<template>
  <BaseCard
    as="section"
    class="operation-safety-summary"
    :class="{ 'is-compact': compact }"
    :variant="risk === 'high' || risk === 'critical' ? 'warning' : 'default'"
    :aria-labelledby="headingId"
  >
    <div class="summary-header">
      <div>
        <p class="summary-kicker">Before you continue</p>
        <h2 :id="headingId">{{ title }}</h2>
      </div>
      <RiskBadge :risk="risk" />
    </div>

    <div class="summary-layout">
      <EvidenceList :items="changeEvidenceItems" :heading="`What changes`" />

      <dl class="safety-facts">
        <div>
          <dt>Target count</dt>
          <dd>{{ targetCount }}</dd>
        </div>
        <div>
          <dt>Risk</dt>
          <dd><RiskBadge :risk="risk" /></dd>
        </div>
        <div>
          <dt>Privilege</dt>
          <dd>
            <PrivilegeBadge
              :privilege="descriptor?.privilege"
              :requires-elevation="requiresElevation"
            />
          </dd>
        </div>
        <div>
          <dt>Restart</dt>
          <dd>
            <RestartBadge
              :policy="descriptor?.reboot_policy"
              :requires-restart="plan.requires_restart"
            />
          </dd>
        </div>
        <div>
          <dt>Recovery</dt>
          <dd>
            <RecoveryBadge
              :recovery="descriptor?.recovery"
              :reversibility="plan.reversibility ?? descriptor?.reversibility"
            />
          </dd>
        </div>
        <div>
          <dt>Evidence</dt>
          <dd><ProofBadge :proof="descriptor?.proof_status" :experimental="descriptor?.experimental" /></dd>
        </div>
        <div>
          <dt>Verification</dt>
          <dd>{{ verificationLabel }}</dd>
        </div>
        <div>
          <dt>Warnings</dt>
          <dd>{{ warningItems.length ? `${warningItems.length} listed` : 'None' }}</dd>
        </div>
      </dl>
    </div>

    <section v-if="warningItems.length > 0" class="warning-section" :aria-labelledby="warningsId">
      <h3 :id="warningsId">Warnings</h3>
      <EvidenceList :items="warningEvidenceItems" />
    </section>
  </BaseCard>
</template>

<style scoped>
.operation-safety-summary { display: grid; gap: var(--space-4); }
.operation-safety-summary.is-compact { gap: var(--space-3); padding: var(--space-4); }
.summary-header { display: flex; align-items: flex-start; justify-content: space-between; gap: var(--space-4); }
.summary-kicker { margin: 0 0 var(--space-1); color: var(--text-muted); font-size: var(--text-xs); font-weight: var(--weight-semibold); letter-spacing: var(--tracking-wide); text-transform: uppercase; }
h2, h3 { margin: 0; }
h2 { font-size: var(--text-lg); }
h3 { margin-bottom: var(--space-2); font-size: var(--text-sm); }
.summary-layout { display: grid; grid-template-columns: minmax(0, 1.2fr) minmax(220px, .8fr); gap: var(--space-5); }
.safety-facts { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: var(--space-2); margin: 0; }
.safety-facts div { min-width: 0; padding: var(--space-2) var(--space-3); border: 1px solid var(--border-subtle); border-radius: var(--radius-md); background: var(--bg-surface); }
.safety-facts dt { color: var(--text-muted); font-size: var(--text-xs); }
.safety-facts dd { margin: var(--space-1) 0 0; color: var(--text-primary); font-size: var(--text-sm); font-weight: var(--weight-semibold); overflow-wrap: anywhere; }
.warning-section { padding-top: var(--space-3); border-top: 1px solid var(--border-subtle); }
@media (max-width: 720px) {
  .summary-layout { grid-template-columns: 1fr; }
}
</style>
