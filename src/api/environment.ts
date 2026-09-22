import { invoke } from '@tauri-apps/api/core'
import type { EnvScope, EnvVar, EnvVarType } from './tauri'
import type { OperationPlan, OperationResult } from './operation'

export type EnvAction =
  | { kind: 'set'; variable: EnvVar }
  | { kind: 'delete'; name: string; scope: EnvScope }

export type { EnvScope, EnvVarType }

export const createEnvPlan = (action: EnvAction) =>
  invoke<OperationPlan>('create_env_plan', { action })

export const executeEnvPlan = (confirmedPlan: OperationPlan) =>
  invoke<{ result: OperationResult; message: string }>('execute_env_plan', { confirmedPlan })
