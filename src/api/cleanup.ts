import { invoke } from '@tauri-apps/api/core'
import type { OperationPlan, OperationResult } from './operation'

export type CleanupDeletionMode = 'recycle_bin' | 'permanent'

export interface CleanupTarget {
  target_id: string
  category: string
  resource_id: string
  description: string
  size_bytes: number
  last_modified: string | null
  confidence: string
  risk: 'low' | 'medium' | 'high' | 'critical'
  recommended: boolean
  recommendation_reason: string
  accessibility: string
  reversibility: 'fully_reversible' | 'best_effort' | 'not_reversible'
  affected_application: string | null
  group_name: string
  cleanup_type: string
  requires_elevation: boolean
}

export interface CleanupScan {
  scan_id: string
  scanned_at: string
  targets: CleanupTarget[]
  estimated_bytes: number
  free_space_before: number | null
  failures: string[]
}

export interface CleanupExecution {
  result: OperationResult
  estimated_bytes: number
  measured_disk_space_delta: number | null
  journal_id: string
}

export const scanCleanupTargets = () =>
  invoke<CleanupScan>('scan_cleanup_targets')

export const createCleanupPlan = (
  scan: CleanupScan,
  selectedTargetIds: string[],
  deletionMode: CleanupDeletionMode = 'recycle_bin',
) => invoke<OperationPlan>('create_cleanup_plan', { scan, selectedTargetIds, deletionMode })

export const executeCleanupPlan = (confirmedPlan: OperationPlan) =>
  invoke<CleanupExecution>('execute_cleanup_plan', { confirmedPlan })

export const cancelCleanup = (operationId: string) =>
  invoke<void>('cancel_cleanup', { operationId })
