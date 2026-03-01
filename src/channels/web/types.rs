//! Request and response DTOs for the web gateway API.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

// --- Chat ---

#[derive(Debug, Deserialize)]
pub struct SendMessageRequest {
    pub content: String,
    pub thread_id: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct SendMessageResponse {
    pub message_id: Uuid,
    pub status: &'static str,
}

#[derive(Debug, Serialize)]
pub struct ThreadInfo {
    pub id: Uuid,
    pub state: String,
    pub turn_count: usize,
    pub created_at: String,
    pub updated_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thread_type: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ThreadListResponse {
    /// The pinned assistant thread (always present after first load).
    pub assistant_thread: Option<ThreadInfo>,
    /// Regular conversation threads.
    pub threads: Vec<ThreadInfo>,
    pub active_thread: Option<Uuid>,
}

#[derive(Debug, Serialize)]
pub struct TurnInfo {
    pub turn_number: usize,
    pub user_input: String,
    pub response: Option<String>,
    pub state: String,
    pub started_at: String,
    pub completed_at: Option<String>,
    pub tool_calls: Vec<ToolCallInfo>,
}

#[derive(Debug, Serialize)]
pub struct ToolCallInfo {
    pub name: String,
    pub has_result: bool,
    pub has_error: bool,
}

#[derive(Debug, Serialize)]
pub struct HistoryResponse {
    pub thread_id: Uuid,
    pub turns: Vec<TurnInfo>,
    /// Whether there are older messages available.
    #[serde(default)]
    pub has_more: bool,
    /// Cursor for the next page (ISO8601 timestamp of the oldest message returned).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub oldest_timestamp: Option<String>,
}

// --- Approval ---

#[derive(Debug, Deserialize)]
pub struct ApprovalRequest {
    pub request_id: String,
    /// "approve", "always", or "deny"
    pub action: String,
    /// Thread that owns the pending approval (so the agent loop finds the right session).
    pub thread_id: Option<String>,
}

// --- SSE Event Types ---

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type")]
pub enum SseEvent {
    #[serde(rename = "response")]
    Response { content: String, thread_id: String },
    #[serde(rename = "thinking")]
    Thinking {
        message: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        thread_id: Option<String>,
    },
    #[serde(rename = "tool_started")]
    ToolStarted {
        name: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        thread_id: Option<String>,
    },
    #[serde(rename = "tool_completed")]
    ToolCompleted {
        name: String,
        success: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        thread_id: Option<String>,
    },
    #[serde(rename = "tool_result")]
    ToolResult {
        name: String,
        preview: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        thread_id: Option<String>,
    },
    #[serde(rename = "stream_chunk")]
    StreamChunk {
        content: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        thread_id: Option<String>,
    },
    #[serde(rename = "status")]
    Status {
        message: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        thread_id: Option<String>,
    },
    #[serde(rename = "job_started")]
    JobStarted {
        job_id: String,
        title: String,
        browse_url: String,
    },
    #[serde(rename = "approval_needed")]
    ApprovalNeeded {
        request_id: String,
        tool_name: String,
        description: String,
        parameters: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        thread_id: Option<String>,
    },
    #[serde(rename = "auth_required")]
    AuthRequired {
        extension_name: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        instructions: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        auth_url: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        setup_url: Option<String>,
    },
    #[serde(rename = "auth_completed")]
    AuthCompleted {
        extension_name: String,
        success: bool,
        message: String,
    },
    #[serde(rename = "error")]
    Error {
        message: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        thread_id: Option<String>,
    },
    #[serde(rename = "heartbeat")]
    Heartbeat,

    // Sandbox job streaming events (worker + Claude Code bridge)
    #[serde(rename = "job_message")]
    JobMessage {
        job_id: String,
        role: String,
        content: String,
    },
    #[serde(rename = "job_tool_use")]
    JobToolUse {
        job_id: String,
        tool_name: String,
        input: serde_json::Value,
    },
    #[serde(rename = "job_tool_result")]
    JobToolResult {
        job_id: String,
        tool_name: String,
        output: String,
    },
    #[serde(rename = "job_status")]
    JobStatus { job_id: String, message: String },
    #[serde(rename = "job_result")]
    JobResult {
        job_id: String,
        status: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        session_id: Option<String>,
    },
}

// --- Memory ---

#[derive(Debug, Serialize)]
pub struct MemoryTreeResponse {
    pub entries: Vec<TreeEntry>,
}

#[derive(Debug, Serialize)]
pub struct TreeEntry {
    pub path: String,
    pub is_dir: bool,
}

#[derive(Debug, Serialize)]
pub struct MemoryListResponse {
    pub path: String,
    pub entries: Vec<ListEntry>,
}

#[derive(Debug, Serialize)]
pub struct ListEntry {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    pub updated_at: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct MemoryReadResponse {
    pub path: String,
    pub content: String,
    pub updated_at: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct MemoryWriteRequest {
    pub path: String,
    pub content: String,
}

#[derive(Debug, Serialize)]
pub struct MemoryWriteResponse {
    pub path: String,
    pub status: &'static str,
}

#[derive(Debug, Deserialize)]
pub struct MemorySearchRequest {
    pub query: String,
    pub limit: Option<usize>,
}

#[derive(Debug, Serialize)]
pub struct MemorySearchResponse {
    pub results: Vec<SearchHit>,
}

#[derive(Debug, Serialize)]
pub struct SearchHit {
    pub path: String,
    pub content: String,
    pub score: f64,
}

// --- Matters ---

/// Single matter returned by `GET /api/matters`.
#[derive(Debug, Serialize)]
pub struct MatterInfo {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_id: Option<String>,
    pub client: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stage: Option<String>,
    pub confidentiality: Option<String>,
    pub team: Vec<String>,
    pub adversaries: Vec<String>,
    pub retention: Option<String>,
    pub jurisdiction: Option<String>,
    pub practice_area: Option<String>,
    pub opened_at: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct MattersListResponse {
    pub matters: Vec<MatterInfo>,
}

#[derive(Debug, Serialize)]
pub struct ClientInfo {
    pub id: String,
    pub name: String,
    pub client_type: String,
    pub email: Option<String>,
    pub phone: Option<String>,
    pub address: Option<String>,
    pub notes: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Serialize)]
pub struct ClientsListResponse {
    pub clients: Vec<ClientInfo>,
}

#[derive(Debug, Deserialize)]
pub struct CreateClientRequest {
    pub name: String,
    pub client_type: String,
    #[serde(default)]
    pub email: Option<String>,
    #[serde(default)]
    pub phone: Option<String>,
    #[serde(default)]
    pub address: Option<String>,
    #[serde(default)]
    pub notes: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateClientRequest {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub client_type: Option<String>,
    #[serde(default)]
    pub email: Option<Option<String>>,
    #[serde(default)]
    pub phone: Option<Option<String>>,
    #[serde(default)]
    pub address: Option<Option<String>>,
    #[serde(default)]
    pub notes: Option<Option<String>>,
}

#[derive(Debug, Serialize)]
pub struct MatterTaskInfo {
    pub id: String,
    pub title: String,
    pub description: Option<String>,
    pub status: String,
    pub assignee: Option<String>,
    pub due_at: Option<String>,
    pub blocked_by: Vec<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Serialize)]
pub struct MatterTasksListResponse {
    pub tasks: Vec<MatterTaskInfo>,
}

#[derive(Debug, Deserialize)]
pub struct CreateMatterTaskRequest {
    pub title: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub assignee: Option<String>,
    #[serde(default)]
    pub due_at: Option<String>,
    #[serde(default)]
    pub blocked_by: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateMatterTaskRequest {
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub description: Option<Option<String>>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub assignee: Option<Option<String>>,
    #[serde(default)]
    pub due_at: Option<Option<String>>,
    #[serde(default)]
    pub blocked_by: Option<Vec<String>>,
}

#[derive(Debug, Serialize)]
pub struct MatterNoteInfo {
    pub id: String,
    pub author: String,
    pub body: String,
    pub pinned: bool,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Serialize)]
pub struct MatterNotesListResponse {
    pub notes: Vec<MatterNoteInfo>,
}

#[derive(Debug, Deserialize)]
pub struct CreateMatterNoteRequest {
    pub author: String,
    pub body: String,
    #[serde(default)]
    pub pinned: bool,
}

#[derive(Debug, Deserialize)]
pub struct UpdateMatterNoteRequest {
    #[serde(default)]
    pub author: Option<String>,
    #[serde(default)]
    pub body: Option<String>,
    #[serde(default)]
    pub pinned: Option<bool>,
}

#[derive(Debug, Serialize)]
pub struct TimeEntryInfo {
    pub id: String,
    pub timekeeper: String,
    pub description: String,
    pub hours: String,
    pub hourly_rate: Option<String>,
    pub entry_date: String,
    pub billable: bool,
    pub billed_invoice_id: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Serialize)]
pub struct MatterTimeEntriesResponse {
    pub entries: Vec<TimeEntryInfo>,
}

#[derive(Debug, Deserialize)]
pub struct CreateTimeEntryRequest {
    pub timekeeper: String,
    pub description: String,
    pub hours: String,
    #[serde(default)]
    pub hourly_rate: Option<String>,
    pub entry_date: String,
    #[serde(default)]
    pub billable: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateTimeEntryRequest {
    #[serde(default)]
    pub timekeeper: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub hours: Option<String>,
    #[serde(default)]
    pub hourly_rate: Option<Option<String>>,
    #[serde(default)]
    pub entry_date: Option<String>,
    #[serde(default)]
    pub billable: Option<bool>,
}

#[derive(Debug, Serialize)]
pub struct ExpenseEntryInfo {
    pub id: String,
    pub submitted_by: String,
    pub description: String,
    pub amount: String,
    pub category: String,
    pub entry_date: String,
    pub receipt_path: Option<String>,
    pub billable: bool,
    pub billed_invoice_id: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Serialize)]
pub struct MatterExpenseEntriesResponse {
    pub entries: Vec<ExpenseEntryInfo>,
}

#[derive(Debug, Deserialize)]
pub struct CreateExpenseEntryRequest {
    pub submitted_by: String,
    pub description: String,
    pub amount: String,
    pub category: String,
    pub entry_date: String,
    #[serde(default)]
    pub receipt_path: Option<String>,
    #[serde(default)]
    pub billable: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateExpenseEntryRequest {
    #[serde(default)]
    pub submitted_by: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub amount: Option<String>,
    #[serde(default)]
    pub category: Option<String>,
    #[serde(default)]
    pub entry_date: Option<String>,
    #[serde(default)]
    pub receipt_path: Option<Option<String>>,
    #[serde(default)]
    pub billable: Option<bool>,
}

#[derive(Debug, Serialize)]
pub struct MatterTimeSummaryResponse {
    pub total_hours: String,
    pub billable_hours: String,
    pub unbilled_hours: String,
    pub total_expenses: String,
    pub billable_expenses: String,
    pub unbilled_expenses: String,
}

#[derive(Debug, Serialize)]
pub struct InvoiceLineItemInfo {
    pub id: String,
    pub description: String,
    pub quantity: String,
    pub unit_price: String,
    pub amount: String,
    pub time_entry_id: Option<String>,
    pub expense_entry_id: Option<String>,
    pub sort_order: i32,
}

#[derive(Debug, Serialize)]
pub struct InvoiceInfo {
    pub id: String,
    pub matter_id: String,
    pub invoice_number: String,
    pub status: String,
    pub issued_date: Option<String>,
    pub due_date: Option<String>,
    pub subtotal: String,
    pub tax: String,
    pub total: String,
    pub paid_amount: String,
    pub notes: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Serialize)]
pub struct InvoiceDetailResponse {
    pub invoice: InvoiceInfo,
    pub line_items: Vec<InvoiceLineItemInfo>,
}

#[derive(Debug, Deserialize)]
pub struct DraftInvoiceRequest {
    pub matter_id: String,
    pub invoice_number: String,
    #[serde(default)]
    pub due_date: Option<String>,
    #[serde(default)]
    pub notes: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct InvoiceDraftResponse {
    pub invoice: InvoiceDraftInfo,
    pub line_items: Vec<InvoiceLineItemInfo>,
}

#[derive(Debug, Serialize)]
pub struct InvoiceDraftInfo {
    pub matter_id: String,
    pub invoice_number: String,
    pub status: String,
    pub due_date: Option<String>,
    pub subtotal: String,
    pub tax: String,
    pub total: String,
    pub notes: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct RecordInvoicePaymentRequest {
    pub amount: String,
    pub recorded_by: String,
    #[serde(default)]
    pub draw_from_trust: bool,
    #[serde(default)]
    pub description: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct RecordInvoicePaymentResponse {
    pub invoice: InvoiceInfo,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trust_entry: Option<TrustLedgerEntryInfo>,
}

#[derive(Debug, Deserialize)]
pub struct TrustDepositRequest {
    pub amount: String,
    pub recorded_by: String,
    #[serde(default)]
    pub description: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct TrustLedgerEntryInfo {
    pub id: String,
    pub matter_id: String,
    pub entry_type: String,
    pub amount: String,
    pub balance_after: String,
    pub description: String,
    pub invoice_id: Option<String>,
    pub recorded_by: String,
    pub created_at: String,
}

#[derive(Debug, Serialize)]
pub struct TrustLedgerResponse {
    pub matter_id: String,
    pub balance: String,
    pub entries: Vec<TrustLedgerEntryInfo>,
}

/// Response for `GET /api/matters/active`.
#[derive(Debug, Serialize)]
pub struct ActiveMatterResponse {
    pub matter_id: Option<String>,
}

/// Request body for `POST /api/matters/active`.
#[derive(Debug, Deserialize)]
pub struct SetActiveMatterRequest {
    /// Pass `null` or omit to clear the active matter.
    pub matter_id: Option<String>,
}

/// Request body for `POST /api/matters`.
#[derive(Debug, Deserialize)]
pub struct CreateMatterRequest {
    pub matter_id: String,
    pub client: String,
    pub confidentiality: String,
    pub retention: String,
    #[serde(default)]
    pub jurisdiction: Option<String>,
    #[serde(default)]
    pub practice_area: Option<String>,
    #[serde(default)]
    pub opened_at: Option<String>,
    #[serde(default)]
    pub team: Vec<String>,
    #[serde(default)]
    pub adversaries: Vec<String>,
    #[serde(default)]
    pub conflict_decision: Option<crate::db::ConflictDecision>,
    #[serde(default)]
    pub conflict_note: Option<String>,
}

/// Response body for `POST /api/matters`.
#[derive(Debug, Serialize)]
pub struct CreateMatterResponse {
    pub matter: MatterInfo,
    pub active_matter_id: String,
}

#[derive(Debug, Deserialize)]
pub struct UpdateMatterRequest {
    #[serde(default)]
    pub client_id: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub stage: Option<Option<String>>,
    #[serde(default)]
    pub practice_area: Option<Option<String>>,
    #[serde(default)]
    pub jurisdiction: Option<Option<String>>,
    #[serde(default)]
    pub opened_at: Option<Option<String>>,
    #[serde(default)]
    pub closed_at: Option<Option<String>>,
    #[serde(default)]
    pub assigned_to: Option<Vec<String>>,
    #[serde(default)]
    pub custom_fields: Option<serde_json::Value>,
}

/// Request body for `POST /api/matters/conflicts/check`.
#[derive(Debug, Deserialize)]
pub struct MatterConflictCheckRequest {
    pub text: String,
    pub matter_id: Option<String>,
}

/// Response body for `POST /api/matters/conflicts/check`.
#[derive(Debug, Serialize)]
pub struct MatterConflictCheckResponse {
    pub matched: bool,
    pub conflict: Option<String>,
    pub matter_id: Option<String>,
    pub hits: Vec<crate::db::ConflictHit>,
}

/// Request body for `POST /api/matters/conflict-check`.
#[derive(Debug, Deserialize)]
pub struct MatterIntakeConflictCheckRequest {
    pub matter_id: String,
    pub client_names: Vec<String>,
    #[serde(default)]
    pub adversary_names: Vec<String>,
}

/// Response body for `POST /api/matters/conflict-check`.
#[derive(Debug, Serialize)]
pub struct MatterIntakeConflictCheckResponse {
    pub matched: bool,
    pub hits: Vec<crate::db::ConflictHit>,
    pub matter_id: String,
    pub checked_parties: Vec<String>,
}

/// Response body for `POST /api/matters/conflicts/reindex`.
#[derive(Debug, Serialize)]
pub struct MatterConflictGraphReindexResponse {
    pub status: &'static str,
    pub report: crate::legal::matter::ConflictGraphReindexReport,
}

// --- Legal audit ---

#[derive(Debug, Serialize)]
pub struct LegalAuditEventInfo {
    pub id: String,
    pub ts: String,
    pub event_type: String,
    pub actor: String,
    pub matter_id: Option<String>,
    pub severity: String,
    pub details: serde_json::Value,
}

#[derive(Debug, Serialize)]
pub struct LegalAuditListResponse {
    pub events: Vec<LegalAuditEventInfo>,
    pub total: usize,
    pub next_offset: Option<usize>,
}

#[derive(Debug, Serialize)]
pub struct MatterDocumentInfo {
    pub id: Option<String>,
    pub memory_document_id: Option<String>,
    pub name: String,
    pub display_name: Option<String>,
    pub path: String,
    pub is_dir: bool,
    pub category: Option<String>,
    pub updated_at: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct MatterDocumentsResponse {
    pub matter_id: String,
    pub documents: Vec<MatterDocumentInfo>,
}

#[derive(Debug, Serialize)]
pub struct MatterTemplateInfo {
    pub id: Option<String>,
    pub matter_id: Option<String>,
    pub name: String,
    pub path: String,
    pub variables_json: Option<serde_json::Value>,
    pub updated_at: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct MatterTemplatesResponse {
    pub matter_id: String,
    pub templates: Vec<MatterTemplateInfo>,
}

#[derive(Debug, Deserialize)]
pub struct MatterTemplateApplyRequest {
    pub template_name: String,
}

#[derive(Debug, Serialize)]
pub struct MatterTemplateApplyResponse {
    pub path: String,
    pub status: &'static str,
}

#[derive(Debug, Deserialize)]
pub struct GenerateDocumentRequest {
    pub template_id: String,
    pub matter_id: String,
    #[serde(default)]
    pub extra: serde_json::Value,
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub category: Option<String>,
    #[serde(default)]
    pub label: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct GenerateDocumentResponse {
    pub matter_document_id: String,
    pub memory_document_id: String,
    pub path: String,
    pub display_name: String,
    pub category: String,
    pub version_number: i32,
    pub label: String,
}

#[derive(Debug, Serialize)]
pub struct MatterDashboardResponse {
    pub matter_id: String,
    pub document_count: usize,
    pub template_count: usize,
    pub draft_count: usize,
    pub checklist_completed: usize,
    pub checklist_total: usize,
    pub overdue_deadlines: usize,
    pub upcoming_deadlines_14d: usize,
    pub next_deadline: Option<MatterDeadlineInfo>,
}

#[derive(Debug, Serialize, Clone)]
pub struct MatterDeadlineInfo {
    pub date: String,
    pub title: String,
    pub owner: Option<String>,
    pub status: Option<String>,
    pub source: Option<String>,
    pub is_overdue: bool,
}

#[derive(Debug, Serialize)]
pub struct MatterDeadlinesResponse {
    pub matter_id: String,
    pub deadlines: Vec<MatterDeadlineInfo>,
}

#[derive(Debug, Serialize)]
pub struct MatterDeadlineRecordInfo {
    pub id: String,
    pub title: String,
    pub deadline_type: String,
    pub due_at: String,
    pub completed_at: Option<String>,
    pub reminder_days: Vec<i32>,
    pub rule_ref: Option<String>,
    pub computed_from: Option<String>,
    pub task_id: Option<String>,
    pub is_overdue: bool,
    pub days_until_due: i64,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Deserialize)]
pub struct CreateMatterDeadlineRequest {
    pub title: String,
    pub deadline_type: String,
    pub due_at: String,
    #[serde(default)]
    pub completed_at: Option<String>,
    #[serde(default)]
    pub reminder_days: Vec<i32>,
    #[serde(default)]
    pub rule_ref: Option<String>,
    #[serde(default)]
    pub computed_from: Option<String>,
    #[serde(default)]
    pub task_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateMatterDeadlineRequest {
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub deadline_type: Option<String>,
    #[serde(default)]
    pub due_at: Option<String>,
    #[serde(default)]
    pub completed_at: Option<Option<String>>,
    #[serde(default)]
    pub reminder_days: Option<Vec<i32>>,
    #[serde(default)]
    pub rule_ref: Option<Option<String>>,
    #[serde(default)]
    pub computed_from: Option<Option<String>>,
    #[serde(default)]
    pub task_id: Option<Option<String>>,
}

#[derive(Debug, Deserialize)]
pub struct MatterDeadlineComputeRequest {
    pub rule_id: String,
    pub trigger_date: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub reminder_days: Vec<i32>,
    #[serde(default)]
    pub computed_from: Option<String>,
    #[serde(default)]
    pub task_id: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct MatterDeadlineComputeResponse {
    pub matter_id: String,
    pub rule: CourtRuleInfo,
    pub deadline: MatterDeadlineComputePreview,
}

#[derive(Debug, Serialize)]
pub struct MatterDeadlineComputePreview {
    pub title: String,
    pub deadline_type: String,
    pub due_at: String,
    pub reminder_days: Vec<i32>,
    pub rule_ref: Option<String>,
    pub computed_from: Option<String>,
    pub task_id: Option<String>,
    pub is_overdue: bool,
    pub days_until_due: i64,
}

#[derive(Debug, Serialize)]
pub struct CourtRuleInfo {
    pub id: String,
    pub citation: String,
    pub deadline_type: String,
    pub offset_days: i64,
    pub court_days: bool,
}

#[derive(Debug, Serialize)]
pub struct CourtRulesResponse {
    pub rules: Vec<CourtRuleInfo>,
}

#[derive(Debug, Serialize)]
pub struct MatterFilingPackageResponse {
    pub matter_id: String,
    pub path: String,
    pub generated_at: String,
    pub status: &'static str,
}

// --- Memory upload ---

/// One successfully uploaded file entry in the upload response.
#[derive(Debug, Serialize)]
pub struct UploadedFile {
    pub path: String,
    pub bytes: usize,
    pub status: &'static str,
}

/// Response body returned by `POST /api/memory/upload`.
#[derive(Debug, Serialize)]
pub struct MemoryUploadResponse {
    pub files: Vec<UploadedFile>,
}

// --- Jobs ---

#[derive(Debug, Serialize)]
pub struct JobInfo {
    pub id: Uuid,
    pub title: String,
    pub state: String,
    pub user_id: String,
    pub created_at: String,
    pub started_at: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct JobListResponse {
    pub jobs: Vec<JobInfo>,
}

#[derive(Debug, Serialize)]
pub struct JobSummaryResponse {
    pub total: usize,
    pub pending: usize,
    pub in_progress: usize,
    pub completed: usize,
    pub failed: usize,
    pub stuck: usize,
}

#[derive(Debug, Serialize)]
pub struct JobDetailResponse {
    pub id: Uuid,
    pub title: String,
    pub description: String,
    pub state: String,
    pub user_id: String,
    pub created_at: String,
    pub started_at: Option<String>,
    pub completed_at: Option<String>,
    pub elapsed_secs: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_dir: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub browse_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub job_mode: Option<String>,
    pub transitions: Vec<TransitionInfo>,
}

// --- Project Files ---

#[derive(Debug, Serialize)]
pub struct ProjectFileEntry {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
}

#[derive(Debug, Serialize)]
pub struct ProjectFilesResponse {
    pub entries: Vec<ProjectFileEntry>,
}

#[derive(Debug, Serialize)]
pub struct ProjectFileReadResponse {
    pub path: String,
    pub content: String,
}

#[derive(Debug, Serialize)]
pub struct TransitionInfo {
    pub from: String,
    pub to: String,
    pub timestamp: String,
    pub reason: Option<String>,
}

// --- Extensions ---

#[derive(Debug, Serialize)]
pub struct ExtensionInfo {
    pub name: String,
    pub kind: String,
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    pub authenticated: bool,
    pub active: bool,
    pub tools: Vec<String>,
    /// Whether this extension has configurable secrets (setup schema).
    #[serde(default)]
    pub needs_setup: bool,
}

#[derive(Debug, Serialize)]
pub struct ExtensionListResponse {
    pub extensions: Vec<ExtensionInfo>,
}

#[derive(Debug, Serialize)]
pub struct ToolInfo {
    pub name: String,
    pub description: String,
}

#[derive(Debug, Serialize)]
pub struct ToolListResponse {
    pub tools: Vec<ToolInfo>,
}

#[derive(Debug, Deserialize)]
pub struct InstallExtensionRequest {
    pub name: String,
    pub url: Option<String>,
    pub kind: Option<String>,
}

// --- Extension Setup ---

#[derive(Debug, Serialize)]
pub struct ExtensionSetupResponse {
    pub name: String,
    pub kind: String,
    pub secrets: Vec<SecretFieldInfo>,
}

#[derive(Debug, Serialize)]
pub struct SecretFieldInfo {
    pub name: String,
    pub prompt: String,
    pub optional: bool,
    /// Whether this secret is already stored.
    pub provided: bool,
    /// Whether the secret will be auto-generated if left empty.
    pub auto_generate: bool,
}

#[derive(Debug, Deserialize)]
pub struct ExtensionSetupRequest {
    pub secrets: std::collections::HashMap<String, String>,
}

#[derive(Debug, Serialize)]
pub struct ActionResponse {
    pub success: bool,
    pub message: String,
    /// Auth URL to open (when activation requires OAuth).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth_url: Option<String>,
    /// Whether the extension is waiting for a manual token.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub awaiting_token: Option<bool>,
    /// Instructions for manual token entry.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,
}

impl ActionResponse {
    pub fn ok(message: impl Into<String>) -> Self {
        Self {
            success: true,
            message: message.into(),
            auth_url: None,
            awaiting_token: None,
            instructions: None,
        }
    }

    pub fn fail(message: impl Into<String>) -> Self {
        Self {
            success: false,
            message: message.into(),
            auth_url: None,
            awaiting_token: None,
            instructions: None,
        }
    }
}

// --- Registry ---

#[derive(Debug, Serialize)]
pub struct RegistryEntryInfo {
    pub name: String,
    pub display_name: String,
    pub kind: String,
    pub description: String,
    pub keywords: Vec<String>,
    pub installed: bool,
}

#[derive(Debug, Serialize)]
pub struct RegistrySearchResponse {
    pub entries: Vec<RegistryEntryInfo>,
}

#[derive(Debug, Deserialize)]
pub struct RegistrySearchQuery {
    pub query: Option<String>,
}

// --- Pairing ---

#[derive(Debug, Serialize)]
pub struct PairingListResponse {
    pub channel: String,
    pub requests: Vec<PairingRequestInfo>,
}

#[derive(Debug, Serialize)]
pub struct PairingRequestInfo {
    pub code: String,
    pub sender_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<serde_json::Value>,
    pub created_at: String,
}

#[derive(Debug, Deserialize)]
pub struct PairingApproveRequest {
    pub code: String,
}

// --- Skills ---

#[derive(Debug, Serialize)]
pub struct SkillInfo {
    pub name: String,
    pub description: String,
    pub version: String,
    pub trust: String,
    pub source: String,
    pub keywords: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct SkillListResponse {
    pub skills: Vec<SkillInfo>,
    pub count: usize,
}

#[derive(Debug, Deserialize)]
pub struct SkillSearchRequest {
    pub query: String,
}

#[derive(Debug, Serialize)]
pub struct SkillSearchResponse {
    pub catalog: Vec<serde_json::Value>,
    pub installed: Vec<SkillInfo>,
    pub registry_url: String,
    /// If the catalog registry was unreachable or errored, a human-readable message.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub catalog_error: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct SkillInstallRequest {
    pub name: String,
    pub url: Option<String>,
    pub content: Option<String>,
}

// --- Auth Token ---

/// Request to submit an auth token for an extension (dedicated endpoint).
#[derive(Debug, Deserialize)]
pub struct AuthTokenRequest {
    pub extension_name: String,
    pub token: String,
}

/// Request to cancel an in-progress auth flow.
#[derive(Debug, Deserialize)]
pub struct AuthCancelRequest {
    pub extension_name: String,
}

// --- WebSocket ---

/// Message sent by a WebSocket client to the server.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type")]
pub enum WsClientMessage {
    /// Send a chat message to the agent.
    #[serde(rename = "message")]
    Message {
        content: String,
        thread_id: Option<String>,
    },
    /// Approve or deny a pending tool execution.
    #[serde(rename = "approval")]
    Approval {
        request_id: String,
        /// "approve", "always", or "deny"
        action: String,
        /// Thread that owns the pending approval.
        thread_id: Option<String>,
    },
    /// Submit an auth token for an extension (bypasses message pipeline).
    #[serde(rename = "auth_token")]
    AuthToken {
        extension_name: String,
        token: String,
    },
    /// Cancel an in-progress auth flow.
    #[serde(rename = "auth_cancel")]
    AuthCancel { extension_name: String },
    /// Client heartbeat ping.
    #[serde(rename = "ping")]
    Ping,
}

/// Message sent by the server to a WebSocket client.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type")]
pub enum WsServerMessage {
    /// An SSE-style event forwarded over WebSocket.
    #[serde(rename = "event")]
    Event {
        /// The event sub-type (response, thinking, tool_started, etc.)
        event_type: String,
        /// The event payload as a JSON value.
        data: serde_json::Value,
    },
    /// Server heartbeat pong.
    #[serde(rename = "pong")]
    Pong,
    /// Error message.
    #[serde(rename = "error")]
    Error { message: String },
}

impl WsServerMessage {
    /// Create a WsServerMessage from an SseEvent.
    pub fn from_sse_event(event: &SseEvent) -> Self {
        let event_type = match event {
            SseEvent::Response { .. } => "response",
            SseEvent::Thinking { .. } => "thinking",
            SseEvent::ToolStarted { .. } => "tool_started",
            SseEvent::ToolCompleted { .. } => "tool_completed",
            SseEvent::ToolResult { .. } => "tool_result",
            SseEvent::StreamChunk { .. } => "stream_chunk",
            SseEvent::Status { .. } => "status",
            SseEvent::JobStarted { .. } => "job_started",
            SseEvent::ApprovalNeeded { .. } => "approval_needed",
            SseEvent::AuthRequired { .. } => "auth_required",
            SseEvent::AuthCompleted { .. } => "auth_completed",
            SseEvent::Error { .. } => "error",
            SseEvent::Heartbeat => "heartbeat",
            SseEvent::JobMessage { .. } => "job_message",
            SseEvent::JobToolUse { .. } => "job_tool_use",
            SseEvent::JobToolResult { .. } => "job_tool_result",
            SseEvent::JobStatus { .. } => "job_status",
            SseEvent::JobResult { .. } => "job_result",
        };
        let data = serde_json::to_value(event).unwrap_or(serde_json::Value::Null);
        WsServerMessage::Event {
            event_type: event_type.to_string(),
            data,
        }
    }
}

// --- Routines ---

#[derive(Debug, Serialize)]
pub struct RoutineInfo {
    pub id: Uuid,
    pub name: String,
    pub description: String,
    pub enabled: bool,
    pub trigger_type: String,
    pub trigger_summary: String,
    pub action_type: String,
    pub last_run_at: Option<String>,
    pub next_fire_at: Option<String>,
    pub run_count: u64,
    pub consecutive_failures: u32,
    pub status: String,
}

#[derive(Debug, Serialize)]
pub struct RoutineListResponse {
    pub routines: Vec<RoutineInfo>,
}

#[derive(Debug, Serialize)]
pub struct RoutineSummaryResponse {
    pub total: u64,
    pub enabled: u64,
    pub disabled: u64,
    pub failing: u64,
    pub runs_today: u64,
}

#[derive(Debug, Serialize)]
pub struct RoutineDetailResponse {
    pub id: Uuid,
    pub name: String,
    pub description: String,
    pub enabled: bool,
    pub trigger: serde_json::Value,
    pub action: serde_json::Value,
    pub guardrails: serde_json::Value,
    pub notify: serde_json::Value,
    pub last_run_at: Option<String>,
    pub next_fire_at: Option<String>,
    pub run_count: u64,
    pub consecutive_failures: u32,
    pub created_at: String,
    pub recent_runs: Vec<RoutineRunInfo>,
}

#[derive(Debug, Serialize)]
pub struct RoutineRunInfo {
    pub id: Uuid,
    pub trigger_type: String,
    pub started_at: String,
    pub completed_at: Option<String>,
    pub status: String,
    pub result_summary: Option<String>,
    pub tokens_used: Option<i32>,
}

#[derive(Debug, Deserialize)]
pub struct RoutineCreateRequest {
    pub name: String,
    pub description: Option<String>,
    // Trigger
    pub trigger_type: String,
    pub schedule: Option<String>,
    pub event_pattern: Option<String>,
    pub event_channel: Option<String>,
    // Action
    pub action_type: Option<String>,
    pub prompt: String,
    pub context_paths: Option<Vec<String>>,
    // Guardrails
    pub cooldown_secs: Option<u64>,
}

// --- Settings ---

#[derive(Debug, Serialize)]
pub struct SettingResponse {
    pub key: String,
    pub value: serde_json::Value,
    pub updated_at: String,
}

#[derive(Debug, Serialize)]
pub struct SettingsListResponse {
    pub settings: Vec<SettingResponse>,
}

#[derive(Debug, Deserialize)]
pub struct SettingWriteRequest {
    pub value: serde_json::Value,
}

#[derive(Debug, Deserialize)]
pub struct SettingsImportRequest {
    pub settings: std::collections::HashMap<String, serde_json::Value>,
}

#[derive(Debug, Serialize)]
pub struct SettingsExportResponse {
    pub settings: std::collections::HashMap<String, serde_json::Value>,
}

// --- Health ---

#[derive(Debug, Serialize)]
pub struct HealthResponse {
    pub status: &'static str,
    pub channel: &'static str,
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- WsClientMessage deserialization tests ----

    #[test]
    fn test_ws_client_message_parse() {
        let json = r#"{"type":"message","content":"hello","thread_id":"t1"}"#;
        let msg: WsClientMessage = serde_json::from_str(json).unwrap();
        match msg {
            WsClientMessage::Message { content, thread_id } => {
                assert_eq!(content, "hello");
                assert_eq!(thread_id.as_deref(), Some("t1"));
            }
            _ => panic!("Expected Message variant"),
        }
    }

    #[test]
    fn test_ws_client_message_no_thread() {
        let json = r#"{"type":"message","content":"hi"}"#;
        let msg: WsClientMessage = serde_json::from_str(json).unwrap();
        match msg {
            WsClientMessage::Message { content, thread_id } => {
                assert_eq!(content, "hi");
                assert!(thread_id.is_none());
            }
            _ => panic!("Expected Message variant"),
        }
    }

    #[test]
    fn test_ws_client_approval_parse() {
        let json =
            r#"{"type":"approval","request_id":"abc-123","action":"approve","thread_id":"t1"}"#;
        let msg: WsClientMessage = serde_json::from_str(json).unwrap();
        match msg {
            WsClientMessage::Approval {
                request_id,
                action,
                thread_id,
            } => {
                assert_eq!(request_id, "abc-123");
                assert_eq!(action, "approve");
                assert_eq!(thread_id.as_deref(), Some("t1"));
            }
            _ => panic!("Expected Approval variant"),
        }
    }

    #[test]
    fn test_ws_client_approval_parse_no_thread() {
        let json = r#"{"type":"approval","request_id":"abc-123","action":"deny"}"#;
        let msg: WsClientMessage = serde_json::from_str(json).unwrap();
        match msg {
            WsClientMessage::Approval {
                request_id,
                action,
                thread_id,
            } => {
                assert_eq!(request_id, "abc-123");
                assert_eq!(action, "deny");
                assert!(thread_id.is_none());
            }
            _ => panic!("Expected Approval variant"),
        }
    }

    #[test]
    fn test_ws_client_ping_parse() {
        let json = r#"{"type":"ping"}"#;
        let msg: WsClientMessage = serde_json::from_str(json).unwrap();
        assert!(matches!(msg, WsClientMessage::Ping));
    }

    #[test]
    fn test_ws_client_unknown_type_fails() {
        let json = r#"{"type":"unknown"}"#;
        let result: Result<WsClientMessage, _> = serde_json::from_str(json);
        assert!(result.is_err());
    }

    // ---- WsServerMessage serialization tests ----

    #[test]
    fn test_ws_server_pong_serialize() {
        let msg = WsServerMessage::Pong;
        let json = serde_json::to_string(&msg).unwrap();
        assert_eq!(json, r#"{"type":"pong"}"#);
    }

    #[test]
    fn test_ws_server_error_serialize() {
        let msg = WsServerMessage::Error {
            message: "bad request".to_string(),
        };
        let json = serde_json::to_string(&msg).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["type"], "error");
        assert_eq!(parsed["message"], "bad request");
    }

    #[test]
    fn test_ws_server_from_sse_response() {
        let sse = SseEvent::Response {
            content: "hello".to_string(),
            thread_id: "t1".to_string(),
        };
        let ws = WsServerMessage::from_sse_event(&sse);
        match ws {
            WsServerMessage::Event { event_type, data } => {
                assert_eq!(event_type, "response");
                assert_eq!(data["content"], "hello");
                assert_eq!(data["thread_id"], "t1");
            }
            _ => panic!("Expected Event variant"),
        }
    }

    #[test]
    fn test_ws_server_from_sse_thinking() {
        let sse = SseEvent::Thinking {
            message: "reasoning...".to_string(),
            thread_id: None,
        };
        let ws = WsServerMessage::from_sse_event(&sse);
        match ws {
            WsServerMessage::Event { event_type, data } => {
                assert_eq!(event_type, "thinking");
                assert_eq!(data["message"], "reasoning...");
            }
            _ => panic!("Expected Event variant"),
        }
    }

    #[test]
    fn test_ws_server_from_sse_approval_needed() {
        let sse = SseEvent::ApprovalNeeded {
            request_id: "r1".to_string(),
            tool_name: "shell".to_string(),
            description: "Run ls".to_string(),
            parameters: "{}".to_string(),
            thread_id: Some("t1".to_string()),
        };
        let ws = WsServerMessage::from_sse_event(&sse);
        match ws {
            WsServerMessage::Event { event_type, data } => {
                assert_eq!(event_type, "approval_needed");
                assert_eq!(data["tool_name"], "shell");
                assert_eq!(data["thread_id"], "t1");
            }
            _ => panic!("Expected Event variant"),
        }
    }

    #[test]
    fn test_ws_server_from_sse_heartbeat() {
        let sse = SseEvent::Heartbeat;
        let ws = WsServerMessage::from_sse_event(&sse);
        match ws {
            WsServerMessage::Event { event_type, .. } => {
                assert_eq!(event_type, "heartbeat");
            }
            _ => panic!("Expected Event variant"),
        }
    }

    // ---- Auth type tests ----

    #[test]
    fn test_ws_client_auth_token_parse() {
        let json = r#"{"type":"auth_token","extension_name":"notion","token":"sk-123"}"#;
        let msg: WsClientMessage = serde_json::from_str(json).unwrap();
        match msg {
            WsClientMessage::AuthToken {
                extension_name,
                token,
            } => {
                assert_eq!(extension_name, "notion");
                assert_eq!(token, "sk-123");
            }
            _ => panic!("Expected AuthToken variant"),
        }
    }

    #[test]
    fn test_ws_client_auth_cancel_parse() {
        let json = r#"{"type":"auth_cancel","extension_name":"notion"}"#;
        let msg: WsClientMessage = serde_json::from_str(json).unwrap();
        match msg {
            WsClientMessage::AuthCancel { extension_name } => {
                assert_eq!(extension_name, "notion");
            }
            _ => panic!("Expected AuthCancel variant"),
        }
    }

    #[test]
    fn test_sse_auth_required_serialize() {
        let event = SseEvent::AuthRequired {
            extension_name: "notion".to_string(),
            instructions: Some("Get your token from...".to_string()),
            auth_url: None,
            setup_url: Some("https://notion.so/integrations".to_string()),
        };
        let json = serde_json::to_string(&event).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["type"], "auth_required");
        assert_eq!(parsed["extension_name"], "notion");
        assert_eq!(parsed["instructions"], "Get your token from...");
        assert!(parsed.get("auth_url").is_none());
        assert_eq!(parsed["setup_url"], "https://notion.so/integrations");
    }

    #[test]
    fn test_sse_auth_completed_serialize() {
        let event = SseEvent::AuthCompleted {
            extension_name: "notion".to_string(),
            success: true,
            message: "notion authenticated (3 tools loaded)".to_string(),
        };
        let json = serde_json::to_string(&event).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["type"], "auth_completed");
        assert_eq!(parsed["extension_name"], "notion");
        assert_eq!(parsed["success"], true);
    }

    #[test]
    fn test_ws_server_from_sse_auth_required() {
        let sse = SseEvent::AuthRequired {
            extension_name: "openai".to_string(),
            instructions: Some("Enter API key".to_string()),
            auth_url: None,
            setup_url: None,
        };
        let ws = WsServerMessage::from_sse_event(&sse);
        match ws {
            WsServerMessage::Event { event_type, data } => {
                assert_eq!(event_type, "auth_required");
                assert_eq!(data["extension_name"], "openai");
            }
            _ => panic!("Expected Event variant"),
        }
    }

    #[test]
    fn test_ws_server_from_sse_auth_completed() {
        let sse = SseEvent::AuthCompleted {
            extension_name: "slack".to_string(),
            success: false,
            message: "Invalid token".to_string(),
        };
        let ws = WsServerMessage::from_sse_event(&sse);
        match ws {
            WsServerMessage::Event { event_type, data } => {
                assert_eq!(event_type, "auth_completed");
                assert_eq!(data["success"], false);
            }
            _ => panic!("Expected Event variant"),
        }
    }

    #[test]
    fn test_auth_token_request_deserialize() {
        let json = r#"{"extension_name":"telegram","token":"bot12345"}"#;
        let req: AuthTokenRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.extension_name, "telegram");
        assert_eq!(req.token, "bot12345");
    }

    #[test]
    fn test_auth_cancel_request_deserialize() {
        let json = r#"{"extension_name":"telegram"}"#;
        let req: AuthCancelRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.extension_name, "telegram");
    }
}
