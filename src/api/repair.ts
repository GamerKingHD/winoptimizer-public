import { invoke } from '@tauri-apps/api/core'
import type { OperationPlan, OperationResult } from './operation'
import type { RepairAction, RepairExecutionLog } from './tauri'

export type RepairVerificationScope =
  | 'execution_acknowledgement'
  | 'state_postcondition'
  | 'health_postcondition'
  | 'user_symptom'

export interface RepairTarget extends RepairAction {
  experimental: boolean
  execution_available: boolean
  verification_method: string
  verification_scope: RepairVerificationScope
}

export interface RepairGuide {
  id: string
  title: string
  symptom: string
  description: string
  evidence_action_ids: string[]
  related_action_ids: string[]
  limitations: string
}

export interface RepairExecution {
  result: OperationResult
  log: RepairExecutionLog
  verification_message: string
}

export const listRepairTargets = () =>
  invoke<RepairTarget[]>('list_repair_targets')

export const listRepairGuides = () =>
  invoke<RepairGuide[]>('list_repair_guides')

export const createRepairPlan = (actionId: string) =>
  invoke<OperationPlan>('create_repair_plan', { actionId })

export const executeRepairPlan = (confirmedPlan: OperationPlan) =>
  invoke<RepairExecution>('execute_repair_plan', { confirmedPlan })
