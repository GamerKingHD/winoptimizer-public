import { invoke } from '@tauri-apps/api/core'
import type { ProcessInfo } from './tauri'
import type { OperationPlan, OperationResult } from './operation'

export const createProcessPlan = (process: ProcessInfo) =>
  invoke<OperationPlan>('create_process_plan', { process })

export const executeProcessPlan = (confirmedPlan: OperationPlan) =>
  invoke<{ result: OperationResult; message: string }>('execute_process_plan', { confirmedPlan })
