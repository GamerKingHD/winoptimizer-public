import { invoke } from '@tauri-apps/api/core'
import type { OperationPlan, OperationResult } from './operation'
import type { ScheduledTask } from './tauri'

export interface StartupExecution {
  result: OperationResult
  recovery_action_id: string
  message: string
}

export interface StartupEnableExecution {
  result: OperationResult
  message: string
}

export type ServiceAction =
  | { kind: 'start' }
  | { kind: 'stop' }
  | { kind: 'set_start_type'; start_type: 'auto' | 'demand' | 'disabled' }

export interface ServiceExecution {
  result: OperationResult
  message: string
}

export interface ServicesBulkExecution {
  result: OperationResult
  message: string
}

export interface ServiceGuidanceTarget {
  serviceName: string
  desiredStartType: 'auto' | 'demand' | 'disabled'
}

export interface ScheduledTaskGuidanceTarget {
  taskName: string
  enabled: boolean
}

export interface ScheduledTaskExecution {
  result: OperationResult
  message: string
}

export interface ScheduledTaskScan {
  tasks: ScheduledTask[]
  collected_at: string
  classification: 'supporting'
  mutation_available: boolean
  error: string | null
}

export const createStartupProgramPlan = (programId: string) =>
  invoke<OperationPlan>('create_startup_program_plan', { programId })

export const executeStartupProgramPlan = (confirmedPlan: OperationPlan) =>
  invoke<StartupExecution>('execute_startup_program_plan', { confirmedPlan })

export const createStartupProgramEnablePlan = (programId: string) =>
  invoke<OperationPlan>('create_startup_program_enable_plan', { programId })

export const executeStartupProgramEnablePlan = (confirmedPlan: OperationPlan) =>
  invoke<StartupEnableExecution>('execute_startup_program_enable_plan', { confirmedPlan })

export const createServicePlan = (serviceName: string, action: ServiceAction) =>
  invoke<OperationPlan>('create_service_plan', { serviceName, action })

export const executeServicePlan = (confirmedPlan: OperationPlan) =>
  invoke<ServiceExecution>('execute_service_plan', { confirmedPlan })

export const createServicesManualPlan = () =>
  invoke<OperationPlan>('create_services_manual_plan')

export const executeServicesManualPlan = (confirmedPlan: OperationPlan) =>
  invoke<ServicesBulkExecution>('execute_services_manual_plan', { confirmedPlan })

export const createServicesGuidancePlan = (targets: ServiceGuidanceTarget[]) =>
  invoke<OperationPlan>('create_services_guidance_plan', { targets })

export const executeServicesGuidancePlan = (confirmedPlan: OperationPlan) =>
  invoke<ServicesBulkExecution>('execute_services_guidance_plan', { confirmedPlan })

export const createScheduledTaskPlan = (taskName: string, enabled: boolean) =>
  invoke<OperationPlan>('create_scheduled_task_plan', { taskName, enabled })

export const executeScheduledTaskPlan = (confirmedPlan: OperationPlan) =>
  invoke<ScheduledTaskExecution>('execute_scheduled_task_plan', { confirmedPlan })

export const createScheduledTasksGuidancePlan = (targets: ScheduledTaskGuidanceTarget[]) =>
  invoke<OperationPlan>('create_scheduled_tasks_guidance_plan', { targets })

export const executeScheduledTasksGuidancePlan = (confirmedPlan: OperationPlan) =>
  invoke<ScheduledTaskExecution>('execute_scheduled_tasks_guidance_plan', { confirmedPlan })

export const scanScheduledTasks = () =>
  invoke<ScheduledTaskScan>('scan_scheduled_tasks')
