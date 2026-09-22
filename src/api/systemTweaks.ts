import { invoke } from '@tauri-apps/api/core'
import type { OperationPlan, OperationResult } from './operation'

export type TweakKind = 'privacy' | 'performance' | 'customization'

export interface SystemTweakTarget {
  kind: TweakKind
  id: string
  name: string
  description: string
  risk: 'low' | 'medium' | 'high' | 'critical'
  is_applied: boolean
  experimental: boolean
  requires_elevation: boolean
  requires_restart: boolean
  evidence_status: string
  exposure: string
  reversibility: string
}

export interface SystemTweakExecution {
  result: OperationResult
  message: string
}

export interface TweakPreset {
  id: string
  name: string
  description: string
  target_ids: string[]
}

export const listSystemTweakTargets = () =>
  invoke<SystemTweakTarget[]>('list_system_tweak_targets')

export const createSystemTweakPlan = (kind: TweakKind, tweakId: string) =>
  invoke<OperationPlan>('create_system_tweak_plan', { kind, tweakId })

export const executeSystemTweakPlan = (confirmedPlan: OperationPlan) =>
  invoke<SystemTweakExecution>('execute_system_tweak_plan', { confirmedPlan })

export const listTweakPresets = () =>
  invoke<TweakPreset[]>('list_tweak_presets')

export const createTweakPresetPlan = (presetId: string, selectedTargetIds?: string[]) =>
  invoke<OperationPlan>('create_tweak_preset_plan', { presetId, selectedTargetIds })

export const executeTweakPresetPlan = (confirmedPlan: OperationPlan) =>
  invoke<SystemTweakExecution>('execute_tweak_preset_plan', { confirmedPlan })
