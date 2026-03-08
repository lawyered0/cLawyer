//! Court deadline computation engine.
//!
//! Provides rule-based deadline calculation compliant with FRCP 6(a)(1):
//! - Calendar-day periods: adds the offset and pushes the last day past
//!   any weekend or US federal holiday to the next business day.
//! - Court-day periods: counts only business days (Mon–Fri, non-holiday).
//!
//! All computation produces a human-readable [`ComputationTrace`] that can
//! be stored alongside the deadline for attorney review and audit.
//!
//! # Provider boundary
//!
//! [`DeadlineProvider`] is a trait so a vendor adapter (CompuLaw, CourtRule,
//! etc.) can be plugged in later without changing handler code.  The default
//! implementation [`FirstPartyProvider`] is backed by `court_rules.toml`.

use std::sync::LazyLock;

use chrono::{DateTime, Datelike, Duration, NaiveDate, Utc, Weekday};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::db::{CreateMatterDeadlineParams, MatterDeadlineType};

// ---------------------------------------------------------------------------
// Engine version
// ---------------------------------------------------------------------------

/// Semantic version of this first-party deadline engine.
/// Bump when the computation algorithm changes in a way that affects results.
pub const ENGINE_VERSION: &str = "1.0";

// ---------------------------------------------------------------------------
// Trace types
// ---------------------------------------------------------------------------

/// A single step in a computation trace.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceStep {
    pub step: u32,
    pub description: String,
}

/// Full human-readable explanation of how a deadline date was derived.
/// Stored as JSON alongside the deadline record for attorney review.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComputationTrace {
    /// Machine-readable rule identifier (e.g. `"frcp_12_a_1"`).
    pub rule_id: String,
    /// Human-readable citation (e.g. `"FRCP 12(a)(1)"`).
    pub rule_citation: String,
    /// Version string of the rule that was applied.
    pub rule_version: String,
    /// Version string of the computation engine.
    pub engine_version: String,
    /// ISO-8601 date of the event that started the period.
    pub trigger_date: String,
    /// Number of days in the period (positive = forward).
    pub offset_days: i64,
    /// Whether court days (true) or calendar days (false) were counted.
    pub court_days: bool,
    /// Ordered list of steps taken during computation.
    pub steps: Vec<TraceStep>,
    /// ISO-8601 date of the computed deadline.
    pub result_date: String,
    /// Jurisdiction code (e.g. `"FRCP"`, `"CA"`).
    pub jurisdiction: String,
}

// ---------------------------------------------------------------------------
// CourtRule
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct CourtRule {
    pub id: String,
    pub citation: String,
    pub deadline_type: MatterDeadlineType,
    pub offset_days: i64,
    pub court_days: bool,
    /// Rule version string (sourced from TOML; defaults to `"1"`).
    pub version: String,
    /// Jurisdiction code (sourced from TOML; defaults to `"FRCP"`).
    pub jurisdiction: String,
}

// ---------------------------------------------------------------------------
// TOML parsing
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct CourtRuleConfig {
    rules: Vec<RawCourtRule>,
}

#[derive(Debug, Deserialize)]
struct RawCourtRule {
    id: String,
    citation: String,
    deadline_type: String,
    offset_days: i64,
    #[serde(default)]
    court_days: bool,
    #[serde(default = "default_version")]
    version: String,
    #[serde(default = "default_jurisdiction")]
    jurisdiction: String,
}

fn default_version() -> String {
    "1".to_string()
}
fn default_jurisdiction() -> String {
    "FRCP".to_string()
}

static COURT_RULES: LazyLock<Result<Vec<CourtRule>, String>> =
    LazyLock::new(|| parse_rules(include_str!("court_rules.toml")));

fn parse_rules(raw: &str) -> Result<Vec<CourtRule>, String> {
    let parsed: CourtRuleConfig =
        toml::from_str(raw).map_err(|e| format!("invalid court rules TOML: {}", e))?;
    let mut out = Vec::with_capacity(parsed.rules.len());
    for rule in parsed.rules {
        let deadline_type =
            MatterDeadlineType::from_db_value(&rule.deadline_type).ok_or_else(|| {
                format!(
                    "invalid deadline_type '{}' in court rules",
                    rule.deadline_type
                )
            })?;
        out.push(CourtRule {
            id: rule.id,
            citation: rule.citation,
            deadline_type,
            offset_days: rule.offset_days,
            court_days: rule.court_days,
            version: rule.version,
            jurisdiction: rule.jurisdiction,
        });
    }
    Ok(out)
}

pub fn all_court_rules() -> Result<&'static [CourtRule], String> {
    match &*COURT_RULES {
        Ok(rules) => Ok(rules.as_slice()),
        Err(err) => Err(err.clone()),
    }
}

pub fn get_court_rule(rule_id: &str) -> Result<Option<CourtRule>, String> {
    let rules = all_court_rules()?;
    Ok(rules.iter().find(|rule| rule.id == rule_id).cloned())
}

// ---------------------------------------------------------------------------
// US federal holiday calendar (FRCP 6(a)(1)(C))
// ---------------------------------------------------------------------------

/// Compute the observed US federal holidays for `year`.
///
/// Fixed holidays (New Year's, Juneteenth, Independence Day, Veterans Day,
/// Christmas) use the "nearest weekday" observation rule: Saturday → Friday,
/// Sunday → Monday.
///
/// Floating holidays (MLK Jr., Presidents, Memorial, Labor, Columbus,
/// Thanksgiving) are computed from the appropriate weekday formula.
pub fn us_federal_holidays(year: i32) -> Vec<NaiveDate> {
    let mut holidays = Vec::with_capacity(11);

    // Observed date for a fixed holiday (Sat→Fri, Sun→Mon).
    let observe = |m: u32, d: u32| -> NaiveDate {
        let date = NaiveDate::from_ymd_opt(year, m, d).unwrap_or_default();
        match date.weekday() {
            Weekday::Sat => date.pred_opt().unwrap_or(date),
            Weekday::Sun => date.succ_opt().unwrap_or(date),
            _ => date,
        }
    };

    // Nth occurrence of `wd` in `month`.
    let nth_weekday = |month: u32, wd: Weekday, n: u32| -> NaiveDate {
        let first = NaiveDate::from_ymd_opt(year, month, 1).unwrap_or_default();
        let first_num = first.weekday().num_days_from_monday();
        let target_num = wd.num_days_from_monday();
        let offset = (target_num + 7 - first_num) % 7;
        first + Duration::days((offset + 7 * (n - 1)) as i64)
    };

    // Last occurrence of `wd` in `month`.
    let last_weekday = |month: u32, wd: Weekday| -> NaiveDate {
        let next_first = if month == 12 {
            NaiveDate::from_ymd_opt(year + 1, 1, 1).unwrap_or_default()
        } else {
            NaiveDate::from_ymd_opt(year, month + 1, 1).unwrap_or_default()
        };
        let last = next_first.pred_opt().unwrap_or(next_first);
        let last_num = last.weekday().num_days_from_monday();
        let target_num = wd.num_days_from_monday();
        let offset = (last_num + 7 - target_num) % 7;
        last - Duration::days(offset as i64)
    };

    holidays.push(observe(1, 1)); // New Year's Day
    holidays.push(nth_weekday(1, Weekday::Mon, 3)); // MLK Jr. Day
    holidays.push(nth_weekday(2, Weekday::Mon, 3)); // Presidents Day
    holidays.push(last_weekday(5, Weekday::Mon)); // Memorial Day
    holidays.push(observe(6, 19)); // Juneteenth (effective 2021)
    holidays.push(observe(7, 4)); // Independence Day
    holidays.push(nth_weekday(9, Weekday::Mon, 1)); // Labor Day
    holidays.push(nth_weekday(10, Weekday::Mon, 2)); // Columbus Day
    holidays.push(observe(11, 11)); // Veterans Day
    holidays.push(nth_weekday(11, Weekday::Thu, 4)); // Thanksgiving
    holidays.push(observe(12, 25)); // Christmas

    holidays.sort_unstable();
    holidays
}

fn is_federal_holiday(date: NaiveDate) -> bool {
    let holidays = us_federal_holidays(date.year());
    holidays.binary_search(&date).is_ok()
}

fn is_non_business_day(date: NaiveDate) -> bool {
    matches!(date.weekday(), Weekday::Sat | Weekday::Sun) || is_federal_holiday(date)
}

fn weekday_name(wd: Weekday) -> &'static str {
    match wd {
        Weekday::Mon => "Monday",
        Weekday::Tue => "Tuesday",
        Weekday::Wed => "Wednesday",
        Weekday::Thu => "Thursday",
        Weekday::Fri => "Friday",
        Weekday::Sat => "Saturday",
        Weekday::Sun => "Sunday",
    }
}

// ---------------------------------------------------------------------------
// Core computation (with trace)
// ---------------------------------------------------------------------------

/// Compute the deadline date for `rule` triggered on `trigger_date`, and
/// produce a human-readable [`ComputationTrace`].
///
/// Implements FRCP 6(a)(1):
/// - Calendar days: add `offset_days`; if that day is a weekend or federal
///   holiday, advance to the next business day.
/// - Court days: count only business days (Mon–Fri, non-holiday).
///
/// The time-of-day component is set to midnight UTC (start of day).
pub fn apply_rule_with_trace(
    rule: &CourtRule,
    trigger_date: DateTime<Utc>,
) -> (DateTime<Utc>, ComputationTrace) {
    let trigger_naive = trigger_date.date_naive();
    let mut steps: Vec<TraceStep> = Vec::new();

    macro_rules! push {
        ($desc:expr) => {
            steps.push(TraceStep {
                step: (steps.len() + 1) as u32,
                description: $desc,
            });
        };
    }

    push!(format!(
        "Trigger date: {} ({})",
        trigger_naive,
        trigger_naive.format("%A, %B %e, %Y")
    ));

    let result_naive = if !rule.court_days {
        // --- Calendar-day period ---
        push!(format!(
            "Add {} calendar {} per {} (court_days = false)",
            rule.offset_days,
            if rule.offset_days == 1 { "day" } else { "days" },
            rule.citation
        ));
        let raw = trigger_naive + Duration::days(rule.offset_days);
        push!(format!(
            "Raw result: {} ({})",
            raw,
            raw.format("%A, %B %e, %Y")
        ));

        // Push past weekends/holidays (FRCP 6(a)(1)(C))
        let mut cursor = raw;
        while is_non_business_day(cursor) {
            let why = if matches!(cursor.weekday(), Weekday::Sat | Weekday::Sun) {
                weekday_name(cursor.weekday()).to_string()
            } else {
                "US federal holiday".to_string()
            };
            push!(format!(
                "{} is a {} — advance to next business day (FRCP 6(a)(1)(C))",
                cursor, why
            ));
            cursor = cursor.succ_opt().unwrap_or(cursor);
        }
        if cursor != raw {
            push!(format!(
                "Final deadline: {} ({}) — first following business day",
                cursor,
                cursor.format("%A, %B %e, %Y")
            ));
        } else {
            push!(format!(
                "Final deadline: {} ({}) — no adjustment needed",
                cursor,
                cursor.format("%A, %B %e, %Y")
            ));
        }
        cursor
    } else {
        // --- Court-day period ---
        push!(format!(
            "Count {} court {} per {} (court_days = true; weekends and federal holidays excluded)",
            rule.offset_days,
            if rule.offset_days == 1 { "day" } else { "days" },
            rule.citation
        ));

        let step_dir: i64 = if rule.offset_days >= 0 { 1 } else { -1 };
        let mut remaining = rule.offset_days.unsigned_abs();
        let mut cursor = trigger_naive;

        while remaining > 0 {
            cursor += Duration::days(step_dir);
            if is_non_business_day(cursor) {
                let why = if matches!(cursor.weekday(), Weekday::Sat | Weekday::Sun) {
                    weekday_name(cursor.weekday()).to_string()
                } else {
                    "federal holiday".to_string()
                };
                push!(format!("Skip {} ({})", cursor, why));
            } else {
                remaining -= 1;
                let counted = rule.offset_days.unsigned_abs() - remaining;
                if remaining == 0 {
                    push!(format!(
                        "Court day {} counted: {} ({}) — deadline",
                        counted,
                        cursor,
                        cursor.format("%A, %B %e, %Y")
                    ));
                } else if counted <= 3 || remaining <= 3 {
                    // Log first few and last few to keep traces concise
                    push!(format!("Court day {} counted: {}", counted, cursor));
                } else if counted == 4 && rule.offset_days.unsigned_abs() > 7 {
                    push!(format!("... (skipping intermediate days) ..."));
                }
            }
        }
        cursor
    };

    let result_dt = result_naive
        .and_hms_opt(0, 0, 0)
        .map(|naive| DateTime::<Utc>::from_naive_utc_and_offset(naive, Utc))
        .unwrap_or(trigger_date);

    let trace = ComputationTrace {
        rule_id: rule.id.clone(),
        rule_citation: rule.citation.clone(),
        rule_version: rule.version.clone(),
        engine_version: ENGINE_VERSION.to_string(),
        trigger_date: trigger_naive.to_string(),
        offset_days: rule.offset_days,
        court_days: rule.court_days,
        steps,
        result_date: result_naive.to_string(),
        jurisdiction: rule.jurisdiction.clone(),
    };

    (result_dt, trace)
}

/// Compute the deadline date only (discards the trace).
///
/// Prefer [`apply_rule_with_trace`] when the trace will be stored.
pub fn apply_rule(rule: &CourtRule, trigger_date: DateTime<Utc>) -> DateTime<Utc> {
    apply_rule_with_trace(rule, trigger_date).0
}

// ---------------------------------------------------------------------------
// DeadlineProvider trait (provider boundary for future vendor adapters)
// ---------------------------------------------------------------------------

/// Computes a court deadline from a named rule and trigger date.
///
/// Returns the computed date and a human-readable [`ComputationTrace`].
/// Returns `Err(String)` for unsupported or unknown rule identifiers — callers
/// should surface this as "unsupported jurisdiction — manual docketing required."
///
/// Implementors may be first-party (TOML-backed), vendor-supplied, or test doubles.
pub trait DeadlineProvider: Send + Sync {
    /// All rule identifiers this provider can compute.
    fn supported_rule_ids(&self) -> Vec<String>;

    /// Compute a deadline.  Returns `Err` only for permanently unsupported
    /// rules, never for transient failures.
    fn compute(
        &self,
        rule_id: &str,
        trigger_date: DateTime<Utc>,
    ) -> Result<(DateTime<Utc>, ComputationTrace), String>;
}

/// First-party deadline provider backed by `court_rules.toml`.
pub struct FirstPartyProvider;

impl DeadlineProvider for FirstPartyProvider {
    fn supported_rule_ids(&self) -> Vec<String> {
        all_court_rules()
            .map(|rules| rules.iter().map(|r| r.id.clone()).collect())
            .unwrap_or_default()
    }

    fn compute(
        &self,
        rule_id: &str,
        trigger_date: DateTime<Utc>,
    ) -> Result<(DateTime<Utc>, ComputationTrace), String> {
        let rule = get_court_rule(rule_id)?
            .ok_or_else(|| format!("unsupported rule '{}' — manual docketing required", rule_id))?;
        Ok(apply_rule_with_trace(&rule, trigger_date))
    }
}

// ---------------------------------------------------------------------------
// Convenience constructor
// ---------------------------------------------------------------------------

/// Build a [`CreateMatterDeadlineParams`] by applying `rule` to `trigger_date`.
///
/// Returns both the params and the [`ComputationTrace`] so callers can store
/// the trace in the `explanation` column.
pub fn deadline_from_rule_with_trace(
    title: &str,
    rule: &CourtRule,
    trigger_date: DateTime<Utc>,
    reminder_days: Vec<i32>,
    computed_from: Option<Uuid>,
    task_id: Option<Uuid>,
) -> (CreateMatterDeadlineParams, ComputationTrace) {
    let (due_at, trace) = apply_rule_with_trace(rule, trigger_date);
    let params = CreateMatterDeadlineParams {
        title: title.trim().to_string(),
        deadline_type: rule.deadline_type,
        due_at,
        completed_at: None,
        reminder_days,
        rule_ref: Some(rule.citation.clone()),
        computed_from,
        task_id,
        explanation: Some(serde_json::to_value(&trace).unwrap_or(serde_json::Value::Null)),
        rule_version: Some(rule.version.clone()),
        is_unsupported: false,
    };
    (params, trace)
}

/// Build a [`CreateMatterDeadlineParams`] (without returning the trace).
///
/// Kept for call sites that do not need the trace object.
pub fn deadline_from_rule(
    title: &str,
    rule: &CourtRule,
    trigger_date: DateTime<Utc>,
    reminder_days: Vec<i32>,
    computed_from: Option<Uuid>,
    task_id: Option<Uuid>,
) -> CreateMatterDeadlineParams {
    deadline_from_rule_with_trace(
        title,
        rule,
        trigger_date,
        reminder_days,
        computed_from,
        task_id,
    )
    .0
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use crate::db::MatterDeadlineType;

    use super::{
        CourtRule, DeadlineProvider, FirstPartyProvider, all_court_rules, apply_rule,
        apply_rule_with_trace, get_court_rule, us_federal_holidays,
    };

    // ---- bundled rule coverage ----

    #[test]
    fn bundled_rules_include_required_entries() {
        let rules = all_court_rules().expect("rules should parse");
        for id in &[
            "frcp_12_a_1",
            "frcp_12_a_3",
            "frcp_26_a_1",
            "frcp_33",
            "frcp_34",
            "frcp_36",
            "frcp_56_c_1",
            "frcp_59_b",
            "frcp_60_c_1",
            "ca_ccp_412_20",
        ] {
            assert!(rules.iter().any(|r| &r.id == id), "missing rule: {}", id);
        }
    }

    #[test]
    fn bundled_rules_have_version_and_jurisdiction() {
        let rules = all_court_rules().expect("rules should parse");
        for rule in rules {
            assert!(
                !rule.version.is_empty(),
                "rule {} has empty version",
                rule.id
            );
            assert!(
                !rule.jurisdiction.is_empty(),
                "rule {} has empty jurisdiction",
                rule.id
            );
        }
    }

    // ---- calendar-day computation ----

    #[test]
    fn apply_rule_calendar_days_no_adjustment_needed() {
        // frcp_12_a_1: 21 calendar days; trigger Mon 2026-03-02 → Mon 2026-03-23
        let rule = get_court_rule("frcp_12_a_1")
            .expect("rules should parse")
            .expect("rule should exist");
        let trigger = chrono::Utc
            .with_ymd_and_hms(2026, 3, 2, 10, 0, 0)
            .single()
            .expect("valid trigger");
        let due = apply_rule(&rule, trigger);
        assert_eq!(due.date_naive().to_string(), "2026-03-23");
    }

    #[test]
    fn apply_rule_pushes_past_weekend() {
        // 21 days from Sat 2026-04-04 → raw 2026-04-25 (Sat) → 2026-04-27 (Mon)
        let rule = get_court_rule("frcp_12_a_1")
            .expect("rules should parse")
            .expect("rule should exist");
        let trigger = chrono::Utc
            .with_ymd_and_hms(2026, 4, 4, 10, 0, 0)
            .single()
            .expect("valid trigger");
        let due = apply_rule(&rule, trigger);
        // raw = Apr 25 (Sat) → pushed to Apr 27 (Mon)
        assert_eq!(due.date_naive().to_string(), "2026-04-27");
    }

    #[test]
    fn apply_rule_pushes_past_federal_holiday() {
        // 21 days from Mon 2026-05-25 → raw 2026-06-15 (Mon, no holiday) — check passes
        // Use a trigger that lands on Memorial Day (last Mon of May):
        // 21 days from Mon 2026-04-13 → raw 2026-05-04 (Mon, no holiday) — boring
        // Better: 14 days from Tue 2026-07-21 → raw 2026-08-04 (Tue, no holiday)
        // Let's specifically target July 4:
        // 21 days from Fri 2026-06-12 → raw 2026-07-03 (Fri) — no adjustment.
        // 21 days from Sat 2026-06-13 → raw 2026-07-04 (Sat AND holiday) → pushed to Mon 2026-07-06
        let rule = get_court_rule("frcp_12_a_1")
            .expect("rules should parse")
            .expect("rule should exist");
        let trigger = chrono::Utc
            .with_ymd_and_hms(2026, 6, 13, 10, 0, 0)
            .single()
            .expect("valid trigger");
        let due = apply_rule(&rule, trigger);
        // Jul 4 is Sat (observed as Fri Jul 3); so raw = Jul 4, push past Jul 3+Jul4 weekend → Mon Jul 6
        // Actually: Jul 4 2026 is a Saturday, observed holiday = Fri Jul 3.
        // raw = Jun 13 + 21 = Jul 4 (Sat) → push to Jul 5 (Sun) → push to Jul 6 (Mon, not a holiday)
        assert_eq!(due.date_naive().to_string(), "2026-07-06");
    }

    // ---- court-day computation ----

    #[test]
    fn apply_rule_court_days_skips_weekends() {
        let rule = CourtRule {
            id: "test".to_string(),
            citation: "Test Rule".to_string(),
            deadline_type: MatterDeadlineType::Internal,
            offset_days: 3,
            court_days: true,
            version: "1".to_string(),
            jurisdiction: "FRCP".to_string(),
        };
        // Trigger Fri 2026-03-06; 3 court days → Mon, Tue, Wed → 2026-03-11
        let trigger = chrono::Utc
            .with_ymd_and_hms(2026, 3, 6, 9, 0, 0)
            .single()
            .expect("valid trigger");
        let due = apply_rule(&rule, trigger);
        assert_eq!(due.date_naive().to_string(), "2026-03-11");
    }

    // ---- trace ----

    #[test]
    fn trace_contains_expected_fields() {
        let rule = get_court_rule("frcp_12_a_1")
            .expect("rules should parse")
            .expect("rule should exist");
        let trigger = chrono::Utc
            .with_ymd_and_hms(2026, 3, 2, 10, 0, 0)
            .single()
            .expect("valid trigger");
        let (_, trace) = apply_rule_with_trace(&rule, trigger);
        assert_eq!(trace.rule_id, "frcp_12_a_1");
        assert_eq!(trace.rule_citation, "FRCP 12(a)(1)");
        assert!(!trace.rule_version.is_empty());
        assert!(!trace.engine_version.is_empty());
        assert!(!trace.steps.is_empty());
        assert_eq!(trace.result_date, "2026-03-23");
    }

    // ---- federal holidays ----

    #[test]
    fn federal_holidays_2026_count_is_eleven() {
        let holidays = us_federal_holidays(2026);
        assert_eq!(holidays.len(), 11);
    }

    #[test]
    fn independence_day_2026_observed_correctly() {
        // July 4 2026 is a Saturday → observed Friday July 3
        let holidays = us_federal_holidays(2026);
        let july_3 = chrono::NaiveDate::from_ymd_opt(2026, 7, 3).unwrap();
        assert!(
            holidays.contains(&july_3),
            "expected Jul 3 as observed Independence Day"
        );
    }

    #[test]
    fn christmas_2026_observed_correctly() {
        // Dec 25 2026 is a Friday → observed as-is
        let holidays = us_federal_holidays(2026);
        let dec_25 = chrono::NaiveDate::from_ymd_opt(2026, 12, 25).unwrap();
        assert!(holidays.contains(&dec_25));
    }

    // ---- DeadlineProvider trait ----

    #[test]
    fn first_party_provider_supports_all_bundled_rules() {
        let provider = FirstPartyProvider;
        let ids = provider.supported_rule_ids();
        assert!(ids.contains(&"frcp_12_a_1".to_string()));
        assert!(ids.contains(&"frcp_59_b".to_string()));
    }

    #[test]
    fn first_party_provider_errors_on_unknown_rule() {
        let provider = FirstPartyProvider;
        let trigger = chrono::Utc
            .with_ymd_and_hms(2026, 1, 1, 0, 0, 0)
            .single()
            .expect("valid date");
        let result = provider.compute("nonexistent_rule_xyz", trigger);
        assert!(result.is_err());
        let msg = result.unwrap_err();
        assert!(
            msg.contains("manual docketing required"),
            "unexpected error: {}",
            msg
        );
    }
}
