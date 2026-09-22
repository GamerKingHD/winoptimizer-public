import type { OperationPlan } from './operation'
import { executeCleanupPlan } from './cleanup'
import { executeEnvPlan } from './environment'
import { executeHostsPlan } from './hosts'
import { executeAppxPlan } from './appx'
import { executeProcessPlan } from './processes'
import { executeRepairPlan } from './repair'
import {
  executeScheduledTaskPlan,
  executeScheduledTasksGuidancePlan,
  executeServicePlan,
  executeServicesGuidancePlan,
  executeServicesManualPlan,
  executeStartupProgramEnablePlan,
  executeStartupProgramPlan,
} from './startup'
import { executeSystemTweakPlan, executeTweakPresetPlan } from './systemTweaks'
/**
 * Dispatch a retry plan to the executor for its own operation domain.
 *
 * `create_retry_plan` rediscovers targets per domain, so the returned plan
 * carries the domain-specific immutable descriptor id. History must execute it
 * with the matching executor instead of assuming the cleanup domain.
 */
export function executeRetryPlan(confirmedPlan: OperationPlan): Promise<unknown> {
  const id = confirmedPlan.descriptor.id
  if (id === 'cleanup.reference_slice') return executeCleanupPlan(confirmedPlan)
  if (id === 'environment.change') return executeEnvPlan(confirmedPlan)
  if (id === 'hosts.save') return executeHostsPlan(confirmedPlan)
  if (id === 'appx.policy_change') return executeAppxPlan(confirmedPlan)
  if (id === 'process.terminate') return executeProcessPlan(confirmedPlan)
  if (id === 'repair.verified_diagnostic' || id === 'repair.maintenance') {
    return executeRepairPlan(confirmedPlan)
  }
  if (id === 'system_tweak.change') return executeSystemTweakPlan(confirmedPlan)
  if (id.startsWith('preset.')) return executeTweakPresetPlan(confirmedPlan)
  if (id === 'startup.disable_program') return executeStartupProgramPlan(confirmedPlan)
  if (id === 'startup.enable_program') return executeStartupProgramEnablePlan(confirmedPlan)
  if (id === 'startup.service_change') return executeServicePlan(confirmedPlan)
  if (id === 'startup.services_set_manual') return executeServicesManualPlan(confirmedPlan)
  if (id === 'startup.services_guidance') return executeServicesGuidancePlan(confirmedPlan)
  if (id === 'startup.scheduled_task_change') return executeScheduledTaskPlan(confirmedPlan)
  if (id === 'startup.scheduled_tasks_guidance') {
    return executeScheduledTasksGuidancePlan(confirmedPlan)
  }
  return Promise.reject(new Error(`Retry is not supported for operation ${id}`))
}
