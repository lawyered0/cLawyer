use std::sync::LazyLock;

use chrono::{DateTime, Datelike, Duration, Utc, Weekday};
use serde::Deserialize;
use uuid::Uuid;

use crate::db::{CreateMatterDeadlineParams, MatterDeadlineType};

#[derive(Debug, Clone)]
pub struct CourtRule {
    pub id: String,
    pub citation: String,
    pub deadline_type: MatterDeadlineType,
    pub offset_days: i64,
    pub court_days: bool,
}

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

fn is_weekend(date: DateTime<Utc>) -> bool {
    matches!(date.weekday(), Weekday::Sat | Weekday::Sun)
}

pub fn apply_rule(rule: &CourtRule, trigger_date: DateTime<Utc>) -> DateTime<Utc> {
    if !rule.court_days {
        return trigger_date + Duration::days(rule.offset_days);
    }

    if rule.offset_days == 0 {
        return trigger_date;
    }

    let step = if rule.offset_days > 0 { 1 } else { -1 };
    let mut remaining = rule.offset_days.unsigned_abs();
    let mut cursor = trigger_date;

    while remaining > 0 {
        cursor += Duration::days(step);
        if !is_weekend(cursor) {
            remaining -= 1;
        }
    }

    cursor
}

pub fn deadline_from_rule(
    title: &str,
    rule: &CourtRule,
    trigger_date: DateTime<Utc>,
    reminder_days: Vec<i32>,
    computed_from: Option<Uuid>,
    task_id: Option<Uuid>,
) -> CreateMatterDeadlineParams {
    CreateMatterDeadlineParams {
        title: title.trim().to_string(),
        deadline_type: rule.deadline_type,
        due_at: apply_rule(rule, trigger_date),
        completed_at: None,
        reminder_days,
        rule_ref: Some(rule.citation.clone()),
        computed_from,
        task_id,
    }
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use crate::db::MatterDeadlineType;

    use super::{CourtRule, all_court_rules, apply_rule, get_court_rule};

    #[test]
    fn bundled_rules_include_required_entries() {
        let rules = all_court_rules().expect("rules should parse");
        assert!(rules.iter().any(|rule| rule.id == "frcp_12_a_1"));
        assert!(rules.iter().any(|rule| rule.id == "frcp_26_a_1"));
        assert!(rules.iter().any(|rule| rule.id == "frcp_56_c_1"));
        assert!(rules.iter().any(|rule| rule.id == "ca_ccp_412_20"));
    }

    #[test]
    fn apply_rule_uses_calendar_days_by_default() {
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
    fn apply_rule_skips_weekends_for_court_days() {
        let rule = CourtRule {
            id: "test".to_string(),
            citation: "Test Rule".to_string(),
            deadline_type: MatterDeadlineType::Internal,
            offset_days: 3,
            court_days: true,
        };
        let trigger = chrono::Utc
            .with_ymd_and_hms(2026, 3, 6, 9, 0, 0)
            .single()
            .expect("valid trigger");
        let due = apply_rule(&rule, trigger);
        assert_eq!(due.date_naive().to_string(), "2026-03-11");
    }
}
