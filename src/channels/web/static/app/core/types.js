// Shared frontend DTO typedefs for JS type-checking.
// This file is compile-time-only metadata for tsc/checkJs usage.

/**
 * @typedef {{
 *   id: string,
 *   title: string,
 *   created_at?: string,
 *   updated_at?: string,
 *   matter_id?: (string|null)
 * }} ThreadInfo
 */

/**
 * @typedef {{
 *   id: string,
 *   client: string,
 *   confidentiality?: string,
 *   retention?: string,
 *   jurisdiction?: string,
 *   practice_area?: string,
 *   opened_date?: (string|null)
 * }} MatterInfo
 */

/**
 * @typedef {{
 *   matter_id: string,
 *   metadata?: Record<string, unknown>,
 *   dashboard?: Record<string, unknown>,
 *   documents?: Array<Record<string, unknown>>,
 *   templates?: Array<Record<string, unknown>>,
 *   deadlines?: Array<Record<string, unknown>>,
 *   conversations?: Array<Record<string, unknown>>
 * }} MatterDetailResponse
 */

/**
 * @typedef {{
 *   id: string,
 *   status?: string,
 *   created_at?: string,
 *   updated_at?: string,
 *   tool_name?: string
 * }} JobInfo
 */

/**
 * @typedef {{
 *   id: string,
 *   name: string,
 *   trigger_type?: string,
 *   action_type?: string,
 *   enabled?: boolean
 * }} RoutineInfo
 */

/**
 * @typedef {{
 *   status: string,
 *   checks: Array<Record<string, unknown>>
 * }} ComplianceFunctionStatus
 */

/**
 * @typedef {{
 *   overall: string,
 *   govern: ComplianceFunctionStatus,
 *   map: ComplianceFunctionStatus,
 *   measure: ComplianceFunctionStatus,
 *   manage: ComplianceFunctionStatus,
 *   metrics?: Record<string, unknown>,
 *   data_gaps?: string[]
 * }} ComplianceStatusResponse
 */

/**
 * @typedef {{
 *   id: string,
 *   path: string,
 *   size_bytes?: number,
 *   created_at?: string
 * }} BackupArtifactInfo
 */

/**
 * @typedef {{
 *   artifact: BackupArtifactInfo,
 *   warnings?: string[]
 * }} BackupCreateResponse
 */

/**
 * @typedef {{
 *   valid: boolean,
 *   warnings?: string[]
 * }} BackupVerifyResponse
 */

/**
 * @typedef {{
 *   applied: boolean,
 *   restored_settings: number,
 *   restored_workspace_files: number,
 *   skipped_workspace_files: number,
 *   warnings?: string[]
 * }} BackupRestoreResponse
 */

/**
 * @typedef {{
 *   error?: string,
 *   message?: string,
 *   code?: string,
 *   [k: string]: unknown
 * }} ApiErrorPayload
 */

export {};
