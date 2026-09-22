// Tauri invoke wrappers: typed bindings for frontend-consumed backend commands.

import { invoke } from '@tauri-apps/api/core'
import type { OperationPlan } from './operation'

export const OPERATION_CANCELLED_ERROR_CODE = 'operation_cancelled' as const

export interface TauriErrorPayload {
  code?: string
  message?: string
}

export function getTauriErrorCode(error: unknown): string | undefined {
  if (!error || typeof error !== 'object') return undefined
  const code = (error as TauriErrorPayload).code
  return typeof code === 'string' ? code : undefined
}

export function isTauriErrorCode(error: unknown, code: string): boolean {
  return getTauriErrorCode(error) === code
}

// ---- Types mirroring the Rust structs ----

export interface CleanItem {
  path: string
  size_bytes: number
  last_accessed: string | null
  category: string
  description: string
  group_key: string
  sub_group: string
}

export interface CleanResult {
  category: string
  items_cleaned: number
  bytes_freed: number
  failures: string[]
  cleaned_paths: string[]
}

export interface BrowserInfo {
  name: string
  cache_path: string | null
  is_running: boolean
}

export interface StartupProgram {
  id: string
  name: string
  command: string
  location: string
  value_type: 'REG_SZ' | 'REG_EXPAND_SZ' | null
  enabled: boolean
  protected: boolean
  recovery_action_id?: string | null
}

export interface MutationResult {
  status: 'completed' | 'partial_failure' | 'failed'
  action_id: string
  message: string
  recovery_available: boolean
  verified: boolean
}

export type GuidanceAction = 'keep_enabled' | 'set_manual' | 'disable_if_unused' | 'review_first'
export type GuidanceSourceType = 'microsoft' | 'vendor' | 'community' | 'policy' | 'unknown'
export type GuidanceMatch = 'protected' | 'exact' | 'pattern' | 'out_of_scope' | 'conflict' | 'none'

export interface GuidanceInfo {
  group: GuidanceAction
  action: GuidanceAction
  rationale: string
  conditions: string[]
  source_type: GuidanceSourceType
  source_title: string
  source_url: string
  os_scope: string
  catalog_version: string
  protected: boolean
  matched_by: GuidanceMatch
}

export interface ServiceInfo {
  name: string
  display_name: string
  status: string
  start_type: string
  description: string | null
  guidance: GuidanceInfo
}

export interface TaskDefinitionSnapshot {
  task_path: string
  enabled: boolean
  current_state: string
  next_run_time: string | null
  definition_xml: string
  triggers_xml: string
  actions_xml: string
  principals_xml: string
  run_level: string
  security_descriptor: string | null
}

export interface ScheduledTask {
  name: string
  path: string
  status: string
  trigger: string
  classification: 'recommended_disable' | 'review' | 'protected'
  can_modify: boolean
  recommendation: string
  guidance: GuidanceInfo
  definition: TaskDefinitionSnapshot
}

export interface PrivacyTweak {
  id: string
  name: string
  description: string
  risk: string
  is_applied: boolean
}

export interface PerfTweak {
  id: string
  name: string
  description: string
  risk: string
  is_applied: boolean
}

export interface ContextMenuEntry {
  name: string
  command: string
  location: string
  enabled: boolean
}

export interface DiskNode {
  id: number
  parent_id: number | null
  name: string
  is_directory: boolean
  has_children: boolean
  logical_size_bytes: number
  allocated_size_bytes: number
  unique_allocated_size_bytes: number
  file_count: number
  directory_count: number
  flags: number
  shared_alias: boolean
}

export type ScanStatus = 'complete' | 'partial' | 'cancelled' | 'failed'
export type DiskScanSource = 'mft' | 'win32' | 'snapshot' | 'usn_delta'
export type DiskCacheSource = 'fresh' | 'snapshot' | 'usn_delta' | 'unavailable'
export type DiskReconciliation = 'volume_bitmap' | 'unavailable'

export interface DiskSummary {
  logical_size_bytes: number
  allocated_size_bytes: number
  unique_allocated_size_bytes: number
  metadata_size_bytes: number
  unaccounted_size_bytes: number | null
  file_count: number
  directory_count: number
  stream_count: number
  hard_link_count: number
}

export interface DiskTimingDiagnostics {
  validation_ms: number
  scan_ms: number
  result_finalization_ms: number
  index_finalization_ms: number
}

export interface DiskDiagnostics {
  source: DiskScanSource
  cache_source: DiskCacheSource
  reconciliation: DiskReconciliation
  elapsed_ms: number
  timings?: DiskTimingDiagnostics
  partial_scan: boolean
  access_denied: boolean
  skipped_records: number
  reparse_points: number
  messages: string[]
}

export interface DiskScanResult {
  snapshot_id: string
  source: DiskScanSource
  root: DiskNode
  status: ScanStatus
  engine: 'ntfs_mft' | 'win32_fallback'
  summary: DiskSummary
  diagnostics: DiskDiagnostics
}

export interface DiskStreamDetails {
  name: string
  logical_size_bytes: number
  allocated_size_bytes: number
  valid_data_length_bytes: number
  resident: boolean
  sparse: boolean
  compressed: boolean
  run_count: number
}

export interface DiskEntryDetails {
  snapshot_id: string
  node_id: number
  path: string
  logical_size_bytes: number
  allocated_size_bytes: number
  unique_allocated_size_bytes: number
  mft_record_number: number | null
  sequence: number | null
  file_reference: number | null
  parent_reference: number | null
  flags: number
  reparse_tag: number | null
  link_count: number
  streams: DiskStreamDetails[]
}

export interface LargestDiskFile {
  node_id: number
  name: string
  path: string
  logical_size_bytes: number
  allocated_size_bytes: number
  unique_allocated_size_bytes: number
  shared_alias: boolean
}
export interface ProcessInfo {
  pid: number
  start_time: number
  name: string
  cpu_usage: number
  memory_bytes: number
  disk_read_bytes: number
  disk_write_bytes: number
  status: string
  path: string | null
}

export interface HardwareInfo {
  cpu: CpuInfo
  memory: MemoryInfo
  disks: DiskInfo[]
  gpu: GpuInfo | null
}

export interface CpuInfo {
  name: string
  cores: number
  logical_processors: number
  architecture: string
  usage_percent: number
}

export interface MemoryInfo {
  total_bytes: number
  available_bytes: number
  in_use_percent: number
}

export interface DiskInfo {
  name: string
  mount_point: string
  total_bytes: number
  available_bytes: number
  file_system: string
  is_ssd: boolean
  disk_type: string
}

export interface GpuInfo {
  name: string
  adapter_ram: number
  driver_version: string
}

export interface OsInfo {
  version: string
  edition: string
  build: string
  arch: string
  release_id: string | null
  install_date: string | null
  last_boot: string | null
  user_name: string
  domain: string | null
}

export interface UpdateInfo {
  kb_number: string | null
  title: string
  installed_date: string | null
}

export interface UndoEntry {
  id: string
  session_id: string
  kind: string
  target: string
  reversal_command: string
  original_size_bytes: number | null
  created_at: string
  rolled_back_at: string | null
}

// ---- Command wrappers ----

// Disk Cleaner
export const scanTempFiles = () => invoke<CleanItem[]>('scan_temp_files')

export const detectBrowsers = () => invoke<BrowserInfo[]>('detect_browsers')
export const scanBrowserCache = () => invoke<CleanItem[]>('scan_browser_cache')

export const scanWindowsUpdateCache = () => invoke<CleanItem[]>('scan_windows_update_cache')

export const scanSystemLogs = () => invoke<CleanItem[]>('scan_system_logs')

// Startup Manager
export const listStartupPrograms = () => invoke<StartupProgram[]>('list_startup_programs')
export const listServices = () => invoke<ServiceInfo[]>('list_services')
export const listStartupTasks = () => invoke<ScheduledTask[]>('list_startup_tasks')

// Tweaks
export const listContextMenuEntries = () => invoke<ContextMenuEntry[]>('list_context_menu_entries')

// AppX / Modern App Manager
export interface AppxPackage {
  family_name: string
  display_name: string
  is_protected: boolean
  is_marked_for_removal: boolean
  installed_package_full_name?: string | null
  provisioned_package_name?: string | null
  risk: 'safe' | 'caution' | 'advanced'
}

export interface AppxDeProvisionResult {
  packages_marked: string[]
  packages_skipped: string[]
  enabled: boolean
}

export const listAppxCandidates = () => invoke<AppxPackage[]>('list_appx_candidates')
export const getAppxPolicyStatus = () => invoke<boolean>('get_appx_policy_status')

// Analyzer
export const scanDiskUsage = (path: string, maxDepth: number = 64) =>
  invoke<DiskScanResult>('scan_disk_usage', { path, maxDepth })

export const listDiskChildren = (
  snapshotId: string,
  parentId: number | null,
  sort: 'allocated' | 'logical' | 'unique' | 'name' = 'allocated',
  offset = 0,
  limit = 200,
) => invoke<DiskNode[]>('list_disk_children', {
  snapshotId,
  parentId,
  sort,
  offset,
  limit,
})

export const listLargestDiskFiles = (
  snapshotId: string,
  sort: 'allocated' | 'logical' | 'unique' = 'allocated',
  limit = 50,
) => invoke<LargestDiskFile[]>('list_largest_disk_files', { snapshotId, sort, limit })

export const getDiskEntryDetails = (snapshotId: string, nodeId: number) =>
  invoke<DiskEntryDetails>('get_disk_entry_details', { snapshotId, nodeId })

export const exportDiskSnapshotJson = (snapshotId: string) =>
  invoke<string>('export_disk_snapshot_json', { snapshotId })
export const cancelScan = (operation: string) =>
  invoke<void>('cancel_scan', { operation })
// Process Manager
export const processSnapshot = () => invoke<ProcessInfo[]>('process_snapshot')

// System Info
export const getHardwareInfo = () => invoke<HardwareInfo>('get_hardware_info')
export const getOsInfo = () => invoke<OsInfo>('get_os_info')
export const getInstalledUpdates = () => invoke<UpdateInfo[]>('get_installed_updates')

// Battery
export interface BatteryInfo {
  present: boolean
  charge_percent: number
  charger_connected: boolean
  charging: boolean
  time_remaining_minutes: number
  saving_mode: boolean
  is_laptop: boolean
}

export const getBatteryInfo = () => invoke<BatteryInfo>('get_battery_info')

// Environment Variables
export type EnvScope = 'user' | 'system'
export type EnvVarType = 'REG_SZ' | 'REG_EXPAND_SZ'

export interface EnvVar {
  name: string
  value: string
  scope: EnvScope
  var_type: EnvVarType
}

export const listEnvVars = () => invoke<EnvVar[]>('list_env_vars')

// Network
export interface NetworkAdapter {
  name: string
  description: string
  ip_addresses: string[]
  mac_address: string
  gateway: string
  dhcp_server: string
  dns_servers: string[]
  operational: boolean
  mtu: number
  link_speed_mbps: number
}

export interface TcpConnection {
  local_addr: string
  local_port: number
  remote_addr: string
  remote_port: number
  state: string
  pid: number
  process_name: string
}

export interface NetworkInfo {
  adapters: NetworkAdapter[]
  connections: TcpConnection[]
}

export const getNetworkInfo = () => invoke<NetworkInfo>('get_network_info')

// Undo
export const getUndoLog = () => invoke<UndoEntry[]>('get_undo_log')

// Elevation
export const checkElevated = () => invoke<boolean>('check_elevated')

// Repair Center
export interface RepairAction {
  id: string
  category: string
  name: string
  description: string
  command: string
  args: string[]
  estimated_minutes: number
  requires_admin: boolean
  mutates_system: boolean
  restart_required: boolean
  risk: string
  limitations: string
}

export interface RepairExecutionLog {
  id: string
  action_id: string
  command: string
  args: string[]
  started_at: string
  ended_at: string
  exit_code: number | null
  stdout: string
  stderr: string
  requires_admin: boolean
  mutates_system: boolean
  restart_required: boolean
}

export const listRepairActions = () => invoke<RepairAction[]>('list_repair_actions')

export const cancelRepair = () => invoke<void>('cancel_repair')

// Hosts File
export interface HostEntry {
  id: number
  ip: string
  hostnames: string[]
  comment: string | null
  enabled: boolean
  raw_line?: string | null
}

export const listHostsEntries = () => invoke<HostEntry[]>('list_hosts_entries')

// ---- Progress Events ----

export interface ProgressEvent {
  operation: string
  phase: string       // "started", "progress", "completed", "error"
  percent?: number
  message?: string
  bytes_processed?: number
  bytes_total?: number
}

import { listen, type UnlistenFn } from '@tauri-apps/api/event'

/** Subscribe to progress events from long-running backend operations. */
export const onProgress = (callback: (event: ProgressEvent) => void): Promise<UnlistenFn> => {
  return listen<ProgressEvent>('progress-event', (event) => {
    callback(event.payload)
  })
}
