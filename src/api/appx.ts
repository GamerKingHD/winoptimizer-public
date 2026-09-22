import { invoke } from '@tauri-apps/api/core'
import type { OperationPlan, OperationResult } from './operation'

export type AppxAction =
  | { kind: 'mark'; family_names: string[] }
  | { kind: 'unmark'; family_name: string }
  | { kind: 'disable_policy' }

export interface AppxExecution {
  result: OperationResult
  message: string
}

export const createAppxPlan = (action: AppxAction) =>
  invoke<OperationPlan>('create_appx_plan', { action })

export const executeAppxPlan = (confirmedPlan: OperationPlan) =>
  invoke<AppxExecution>('execute_appx_plan', { confirmedPlan })
