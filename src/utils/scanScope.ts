/**
 * True when `path` addresses a drive root such as `C:\` or `D:/`.
 *
 * Mirrors the backend's `volume_root_drive` rule: whole-volume (MFT) scans are
 * only selected for an exact drive root, so the UI must not claim "Entire
 * volume" for a subtree the backend scans with the Win32 walker.
 */
export function isVolumeRootPath(path: string): boolean {
  return /^[a-zA-Z]:[\\/]$/.test(path.trim())
}

/** Human scope label describing what a disk-usage scan actually covers. */
export function describeScanScope(path: string): 'Entire volume' | 'Selected folder' {
  return isVolumeRootPath(path) ? 'Entire volume' : 'Selected folder'
}
