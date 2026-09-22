const units = ['B', 'KiB', 'MiB', 'GiB', 'TiB'] as const

export function formatBytes(bytes: number | null | undefined): string {
  if (!bytes || bytes === 0) return '0 B'
  const i = Math.min(units.length - 1, Math.floor(Math.log(bytes) / Math.log(1024)))
  return (bytes / Math.pow(1024, i)).toFixed(1) + ' ' + units[i]
}

export function formatBytesOrDash(bytes: number | null | undefined): string {
  if (!bytes || bytes === 0) return '-'
  return formatBytes(bytes)
}

export function formatPercent(value: number): string {
  return `${Math.round(value)}%`
}
