import { invoke } from '@tauri-apps/api/core'

export type UpdateKind = 'release' | 'guidance'
export type UpdateDistribution = 'portable' | 'msi'
export type UpdateAvailability = 'up_to_date' | 'available'

export interface UpdateCheckResult {
  kind: UpdateKind
  channel: string
  current_version: string
  version: string
  availability: UpdateAvailability
  distribution: UpdateDistribution
  package_downloaded: boolean
  staged_artifact: string | null
}

export type UpdateInstallStatus = 'portable_replacement_scheduled' | 'msi_installer_launched'

export interface UpdateInstallResult {
  status: UpdateInstallStatus
  distribution: UpdateDistribution
  version: string
  artifact_path: string
  requires_restart: boolean
}

export const checkForUpdates = (
  manual: boolean,
  kind: UpdateKind = 'release',
  distribution: UpdateDistribution = 'portable',
) => invoke<UpdateCheckResult>('check_for_updates', { manual, kind, distribution })

export const installUpdate = (distribution: UpdateDistribution) =>
  invoke<UpdateInstallResult>('install_update', { distribution })
