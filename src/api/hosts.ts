import { invoke } from '@tauri-apps/api/core'
import type { HostEntry } from './tauri'
import type { OperationPlan, OperationResult } from './operation'

export interface HostsExecution {
  result: OperationResult
  message: string
}

export const createHostsPlan = (entries: HostEntry[]) =>
  invoke<OperationPlan>('create_hosts_plan', { entries })

export const executeHostsPlan = (confirmedPlan: OperationPlan) =>
  invoke<HostsExecution>('execute_hosts_plan', { confirmedPlan })
