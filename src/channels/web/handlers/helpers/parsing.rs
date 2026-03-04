//! Input parsing and validation helpers for web handlers.

use axum::http::StatusCode;
use chrono::{DateTime, NaiveDate, Utc};
use rust_decimal::Decimal;
use uuid::Uuid;

use crate::db::{
    ClientType, ExpenseCategory, MatterDeadlineType, MatterDocumentCategory, MatterStatus,
    MatterTaskStatus,
};

use super::legal::{
    MAX_DEADLINE_REMINDER_DAYS, MAX_DEADLINE_REMINDERS, MAX_INTAKE_CONFLICT_PARTIES,
};

const MAX_INTAKE_CONFLICT_PARTY_CHARS: usize = 160;

pub(crate) fn parse_required_matter_field(
    field_name: &str,
    value: &str,
) -> Result<String, (StatusCode, String)> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            format!("'{}' is required", field_name),
        ));
    }
    Ok(trimmed.to_string())
}

pub(crate) fn parse_optional_matter_field(value: Option<String>) -> Option<String> {
    value.and_then(|raw| {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

pub(crate) fn parse_optional_matter_field_patch(
    value: Option<Option<String>>,
) -> Option<Option<String>> {
    match value {
        None => None,
        Some(None) => Some(None),
        Some(Some(raw)) => Some(parse_optional_matter_field(Some(raw))),
    }
}

const OPTIONAL_MATTER_FIELD_MAX_CHARS: usize = 256;

pub(crate) fn validate_optional_matter_field_length(
    field_name: &str,
    value: &Option<String>,
) -> Result<(), (StatusCode, String)> {
    if let Some(text) = value
        && text.chars().count() > OPTIONAL_MATTER_FIELD_MAX_CHARS
    {
        return Err((
            StatusCode::BAD_REQUEST,
            format!(
                "'{}' must be at most {} characters",
                field_name, OPTIONAL_MATTER_FIELD_MAX_CHARS
            ),
        ));
    }
    Ok(())
}

pub(crate) fn validate_opened_date(value: &str) -> Result<(), (StatusCode, String)> {
    match NaiveDate::parse_from_str(value, "%Y-%m-%d") {
        Ok(parsed) if parsed.format("%Y-%m-%d").to_string() == value => Ok(()),
        _ => Err((
            StatusCode::BAD_REQUEST,
            "'opened_date' must be in YYYY-MM-DD format".to_string(),
        )),
    }
}

pub(crate) fn parse_matter_list(values: Vec<String>) -> Vec<String> {
    values
        .into_iter()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .collect()
}

pub(crate) fn parse_client_type(value: &str) -> Result<ClientType, (StatusCode, String)> {
    match value.trim().to_ascii_lowercase().as_str() {
        "individual" => Ok(ClientType::Individual),
        "entity" => Ok(ClientType::Entity),
        _ => Err((
            StatusCode::BAD_REQUEST,
            "'client_type' must be 'individual' or 'entity'".to_string(),
        )),
    }
}

pub(crate) fn parse_matter_status(value: &str) -> Result<MatterStatus, (StatusCode, String)> {
    match value.trim().to_ascii_lowercase().as_str() {
        "intake" => Ok(MatterStatus::Intake),
        "active" => Ok(MatterStatus::Active),
        "pending" => Ok(MatterStatus::Pending),
        "closed" => Ok(MatterStatus::Closed),
        "archived" => Ok(MatterStatus::Archived),
        _ => Err((
            StatusCode::BAD_REQUEST,
            "'status' must be one of: intake, active, pending, closed, archived".to_string(),
        )),
    }
}

pub(crate) fn parse_matter_task_status(
    value: &str,
) -> Result<MatterTaskStatus, (StatusCode, String)> {
    match value.trim().to_ascii_lowercase().as_str() {
        "todo" => Ok(MatterTaskStatus::Todo),
        "in_progress" => Ok(MatterTaskStatus::InProgress),
        "done" => Ok(MatterTaskStatus::Done),
        "blocked" => Ok(MatterTaskStatus::Blocked),
        "cancelled" => Ok(MatterTaskStatus::Cancelled),
        _ => Err((
            StatusCode::BAD_REQUEST,
            "'status' must be one of: todo, in_progress, done, blocked, cancelled".to_string(),
        )),
    }
}

pub(crate) fn parse_matter_deadline_type(
    value: &str,
) -> Result<MatterDeadlineType, (StatusCode, String)> {
    match value.trim().to_ascii_lowercase().as_str() {
        "court_date" => Ok(MatterDeadlineType::CourtDate),
        "filing" => Ok(MatterDeadlineType::Filing),
        "statute_of_limitations" => Ok(MatterDeadlineType::StatuteOfLimitations),
        "response_due" => Ok(MatterDeadlineType::ResponseDue),
        "discovery_cutoff" => Ok(MatterDeadlineType::DiscoveryCutoff),
        "internal" => Ok(MatterDeadlineType::Internal),
        _ => Err((
            StatusCode::BAD_REQUEST,
            "'deadline_type' must be one of: court_date, filing, statute_of_limitations, response_due, discovery_cutoff, internal".to_string(),
        )),
    }
}

pub(crate) fn parse_expense_category(value: &str) -> Result<ExpenseCategory, (StatusCode, String)> {
    match value.trim().to_ascii_lowercase().as_str() {
        "filing_fee" => Ok(ExpenseCategory::FilingFee),
        "travel" => Ok(ExpenseCategory::Travel),
        "postage" => Ok(ExpenseCategory::Postage),
        "expert" => Ok(ExpenseCategory::Expert),
        "copying" => Ok(ExpenseCategory::Copying),
        "court_reporter" => Ok(ExpenseCategory::CourtReporter),
        "other" => Ok(ExpenseCategory::Other),
        _ => Err((
            StatusCode::BAD_REQUEST,
            "'category' must be one of: filing_fee, travel, postage, expert, copying, court_reporter, other".to_string(),
        )),
    }
}

pub(crate) fn parse_date_only(
    field_name: &str,
    raw: &str,
) -> Result<NaiveDate, (StatusCode, String)> {
    let value = raw.trim();
    if value.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            format!("'{}' is required", field_name),
        ));
    }
    let parsed = NaiveDate::parse_from_str(value, "%Y-%m-%d").map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            format!("'{}' must be in YYYY-MM-DD format", field_name),
        )
    })?;
    if parsed.format("%Y-%m-%d").to_string() != value {
        return Err((
            StatusCode::BAD_REQUEST,
            format!("'{}' must be in YYYY-MM-DD format", field_name),
        ));
    }
    Ok(parsed)
}

pub(crate) fn parse_decimal_field(
    field_name: &str,
    raw: &str,
) -> Result<Decimal, (StatusCode, String)> {
    let value = raw.trim();
    if value.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            format!("'{}' is required", field_name),
        ));
    }
    let decimal = value.parse::<Decimal>().map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            format!("'{}' must be a valid decimal number", field_name),
        )
    })?;
    if decimal <= Decimal::ZERO {
        return Err((
            StatusCode::BAD_REQUEST,
            format!("'{}' must be greater than 0", field_name),
        ));
    }
    Ok(decimal)
}

pub(crate) fn parse_optional_decimal_field(
    field_name: &str,
    raw: Option<String>,
) -> Result<Option<Decimal>, (StatusCode, String)> {
    match parse_optional_matter_field(raw) {
        Some(value) => parse_decimal_field(field_name, &value).map(Some),
        None => Ok(None),
    }
}

pub(crate) fn parse_matter_document_category(
    value: Option<&str>,
) -> Result<MatterDocumentCategory, (StatusCode, String)> {
    let raw = value.unwrap_or("internal").trim().to_ascii_lowercase();
    match raw.as_str() {
        "pleading" => Ok(MatterDocumentCategory::Pleading),
        "correspondence" => Ok(MatterDocumentCategory::Correspondence),
        "contract" => Ok(MatterDocumentCategory::Contract),
        "filing" => Ok(MatterDocumentCategory::Filing),
        "evidence" => Ok(MatterDocumentCategory::Evidence),
        "internal" | "" => Ok(MatterDocumentCategory::Internal),
        _ => Err((
            StatusCode::BAD_REQUEST,
            "'category' must be one of: pleading, correspondence, contract, filing, evidence, internal".to_string(),
        )),
    }
}

pub(crate) fn infer_matter_document_category(path: &str) -> MatterDocumentCategory {
    let lower = path.to_ascii_lowercase();
    if lower.contains("/filing") || lower.contains("/pleading") {
        MatterDocumentCategory::Filing
    } else if lower.contains("/evidence") {
        MatterDocumentCategory::Evidence
    } else if lower.contains("/contract") || lower.contains("/agreement") {
        MatterDocumentCategory::Contract
    } else if lower.contains("/correspondence") || lower.contains("/communication") {
        MatterDocumentCategory::Correspondence
    } else {
        MatterDocumentCategory::Internal
    }
}

pub(crate) fn normalize_reminder_days(values: &[i32]) -> Result<Vec<i32>, (StatusCode, String)> {
    use std::collections::BTreeSet;

    if values.len() > MAX_DEADLINE_REMINDERS {
        return Err((
            StatusCode::BAD_REQUEST,
            format!(
                "'reminder_days' supports at most {} values",
                MAX_DEADLINE_REMINDERS
            ),
        ));
    }

    let mut unique = BTreeSet::new();
    for day in values {
        if *day < 0 {
            return Err((
                StatusCode::BAD_REQUEST,
                "'reminder_days' values must be >= 0".to_string(),
            ));
        }
        if *day > MAX_DEADLINE_REMINDER_DAYS {
            return Err((
                StatusCode::BAD_REQUEST,
                format!(
                    "'reminder_days' values must be <= {}",
                    MAX_DEADLINE_REMINDER_DAYS
                ),
            ));
        }
        unique.insert(*day);
    }

    Ok(unique.into_iter().collect())
}

pub(crate) fn parse_datetime_value(
    field: &str,
    raw: &str,
) -> Result<DateTime<Utc>, (StatusCode, String)> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            format!("'{}' cannot be empty", field),
        ));
    }
    if let Ok(date) = NaiveDate::parse_from_str(trimmed, "%Y-%m-%d")
        && let Some(dt) = date.and_hms_opt(0, 0, 0)
    {
        return Ok(dt.and_utc());
    }
    if let Ok(dt) = DateTime::parse_from_rfc3339(trimmed) {
        return Ok(dt.with_timezone(&Utc));
    }
    Err((
        StatusCode::BAD_REQUEST,
        format!("'{}' must be YYYY-MM-DD or RFC3339 datetime", field),
    ))
}

pub(crate) fn parse_optional_datetime(
    field: &str,
    raw: Option<String>,
) -> Result<Option<DateTime<Utc>>, (StatusCode, String)> {
    let Some(raw) = raw else {
        return Ok(None);
    };
    if raw.trim().is_empty() {
        return Ok(None);
    }
    parse_datetime_value(field, &raw).map(Some)
}

pub(crate) fn parse_optional_datetime_patch(
    field: &str,
    raw: Option<Option<String>>,
) -> Result<Option<Option<DateTime<Utc>>>, (StatusCode, String)> {
    let Some(raw) = raw else {
        return Ok(None);
    };
    let Some(raw) = raw else {
        return Ok(Some(None));
    };
    if raw.trim().is_empty() {
        return Ok(Some(None));
    }
    Ok(Some(Some(parse_datetime_value(field, &raw)?)))
}

pub(crate) fn parse_uuid(value: &str, field: &str) -> Result<Uuid, (StatusCode, String)> {
    Uuid::parse_str(value).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            format!("'{}' must be a valid UUID", field),
        )
    })
}

pub(crate) fn parse_optional_uuid_field(
    value: Option<String>,
    field: &str,
) -> Result<Option<Uuid>, (StatusCode, String)> {
    let Some(raw) = value else {
        return Ok(None);
    };
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    parse_uuid(trimmed, field).map(Some)
}

pub(crate) fn parse_optional_uuid_patch_field(
    value: Option<Option<String>>,
    field: &str,
) -> Result<Option<Option<Uuid>>, (StatusCode, String)> {
    let Some(raw) = value else {
        return Ok(None);
    };
    let Some(raw) = raw else {
        return Ok(Some(None));
    };
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(Some(None));
    }
    parse_uuid(trimmed, field).map(|uuid| Some(Some(uuid)))
}
pub(crate) fn parse_uuid_list(
    values: &[String],
    field: &str,
) -> Result<Vec<Uuid>, (StatusCode, String)> {
    values
        .iter()
        .map(|value| parse_uuid(value, field))
        .collect()
}
fn validate_intake_party_name(field_name: &str, value: &str) -> Result<(), (StatusCode, String)> {
    if value.chars().count() > MAX_INTAKE_CONFLICT_PARTY_CHARS {
        return Err((
            StatusCode::BAD_REQUEST,
            format!(
                "'{}' entries must be at most {} characters",
                field_name, MAX_INTAKE_CONFLICT_PARTY_CHARS
            ),
        ));
    }
    Ok(())
}

pub(crate) fn validate_intake_party_list(
    field_name: &str,
    values: &[String],
) -> Result<(), (StatusCode, String)> {
    if values.len() > MAX_INTAKE_CONFLICT_PARTIES {
        return Err((
            StatusCode::BAD_REQUEST,
            format!(
                "'{}' may include at most {} names",
                field_name, MAX_INTAKE_CONFLICT_PARTIES
            ),
        ));
    }
    for value in values {
        validate_intake_party_name(field_name, value)?;
    }
    Ok(())
}
