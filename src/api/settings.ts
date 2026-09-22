import { invoke } from '@tauri-apps/api/core'

export type ExperienceLevel = 'standard' | 'advanced' | 'expert'
export interface DiskAnalyzerPreferences {
  persist_snapshots: boolean
  max_cached_snapshots: number
}

export interface CleanupPreferences {
  preselect_safe_candidates: boolean
}

export interface HistoryPreferences {
  retention_days: number
  show_internal_identifiers: boolean
}

export interface OnlinePreferences {
  allow_metered_connections: boolean
}

export interface TelemetryPreferences {
  enabled: boolean
}

export interface CrashReportingPreferences {
  enabled: boolean
}

export interface UpdatePreferences {
  check_automatically: boolean
}

export interface RecommendationPreferences {
  update_guidance: boolean
  preselect_safe_recommendations: boolean
}

export interface AppSettings {
  schema_version: number
  experience_level: ExperienceLevel
  labs_enabled: boolean
  disk_analyzer_preferences: DiskAnalyzerPreferences
  cleanup_preferences: CleanupPreferences
  history_preferences: HistoryPreferences
  online_preferences: OnlinePreferences
  telemetry_preferences: TelemetryPreferences
  crash_reporting_preferences: CrashReportingPreferences
  update_preferences: UpdatePreferences
  recommendation_preferences: RecommendationPreferences
}

export const getSettings = () => invoke<AppSettings>('get_settings')

export const updateSettings = (settings: AppSettings) =>
  invoke<AppSettings>('update_settings', { settings })
