//! Canadian jurisdiction profiles and Ontario holiday calendars.

use chrono::{Datelike, Duration, NaiveDate, Weekday};

/// Citation style preferences for jurisdiction-specific drafting helpers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CitationStyle {
    McGill,
    Neutral,
}

/// Supported Canadian jurisdictions for first-party legal helpers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CanadianJurisdiction {
    Ontario,
    BritishColumbia,
    Alberta,
    Federal,
}

impl CanadianJurisdiction {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ontario => "ON",
            Self::BritishColumbia => "BC",
            Self::Alberta => "AB",
            Self::Federal => "CA",
        }
    }
}

/// Jurisdiction-level metadata used by legal tools and citation helpers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JurisdictionProfile {
    pub jurisdiction: CanadianJurisdiction,
    pub citation_style: CitationStyle,
    pub court_prefix: &'static str,
}

impl JurisdictionProfile {
    pub fn ontario() -> Self {
        Self {
            jurisdiction: CanadianJurisdiction::Ontario,
            citation_style: CitationStyle::McGill,
            court_prefix: "Ont",
        }
    }
}

/// Compute Easter Sunday using the Anonymous Gregorian algorithm.
pub fn easter_sunday(year: i32) -> Option<NaiveDate> {
    if year < 1582 {
        return None;
    }

    let a = year % 19;
    let b = year / 100;
    let c = year % 100;
    let d = b / 4;
    let e = b % 4;
    let f = (b + 8) / 25;
    let g = (b - f + 1) / 3;
    let h = (19 * a + b - d - g + 15) % 30;
    let i = c / 4;
    let k = c % 4;
    let l = (32 + 2 * e + 2 * i - h - k) % 7;
    let m = (a + 11 * h + 22 * l) / 451;
    let month = (h + l - 7 * m + 114) / 31;
    let day = ((h + l - 7 * m + 114) % 31) + 1;
    NaiveDate::from_ymd_opt(year, month as u32, day as u32)
}

/// Ontario statutory holidays.
pub fn ontario_statutory_holidays(year: i32) -> Vec<NaiveDate> {
    let mut days = ontario_shared_holidays(year);

    if let Some(remembrance_day) = NaiveDate::from_ymd_opt(year, 11, 11) {
        days.push(observed_monday(remembrance_day));
    }

    days.sort_unstable();
    days.dedup();
    days
}

/// Ontario court holidays.
///
/// This intentionally excludes Remembrance Day and includes Easter Monday.
pub fn ontario_court_holidays(year: i32) -> Vec<NaiveDate> {
    let mut days = ontario_shared_holidays(year);

    if let Some(easter_monday) =
        easter_sunday(year).and_then(|day| day.checked_add_signed(Duration::days(1)))
    {
        days.push(easter_monday);
    }

    days.sort_unstable();
    days.dedup();
    days
}

fn ontario_shared_holidays(year: i32) -> Vec<NaiveDate> {
    let mut days = Vec::new();

    if let Some(new_years_day) = NaiveDate::from_ymd_opt(year, 1, 1) {
        days.push(observed_monday(new_years_day));
    }

    if year >= 2008
        && let Some(family_day) = nth_weekday_of_month(year, 2, Weekday::Mon, 3)
    {
        days.push(family_day);
    }

    if let Some(good_friday) =
        easter_sunday(year).and_then(|day| day.checked_sub_signed(Duration::days(2)))
    {
        days.push(good_friday);
    }

    if let Some(victoria_day) = monday_before(year, 5, 25) {
        days.push(victoria_day);
    }

    if let Some(canada_day) = NaiveDate::from_ymd_opt(year, 7, 1) {
        days.push(observed_monday(canada_day));
    }

    if let Some(civic_holiday) = nth_weekday_of_month(year, 8, Weekday::Mon, 1) {
        days.push(civic_holiday);
    }

    if let Some(labour_day) = nth_weekday_of_month(year, 9, Weekday::Mon, 1) {
        days.push(labour_day);
    }

    if let Some(thanksgiving) = nth_weekday_of_month(year, 10, Weekday::Mon, 2) {
        days.push(thanksgiving);
    }

    if let Some((christmas, boxing_day)) = observed_christmas_pair(year) {
        days.push(christmas);
        days.push(boxing_day);
    }

    days
}

fn observed_monday(date: NaiveDate) -> NaiveDate {
    match date.weekday() {
        Weekday::Sat => date + Duration::days(2),
        Weekday::Sun => date + Duration::days(1),
        _ => date,
    }
}

fn observed_christmas_pair(year: i32) -> Option<(NaiveDate, NaiveDate)> {
    let christmas = NaiveDate::from_ymd_opt(year, 12, 25)?;
    let boxing_day = NaiveDate::from_ymd_opt(year, 12, 26)?;
    let observed = match christmas.weekday() {
        Weekday::Sat => (
            christmas + Duration::days(2),
            boxing_day + Duration::days(2),
        ),
        Weekday::Sun => (christmas + Duration::days(2), boxing_day),
        Weekday::Fri => (christmas, boxing_day + Duration::days(2)),
        _ => (christmas, boxing_day),
    };
    Some(observed)
}

fn monday_before(year: i32, month: u32, day_of_month: u32) -> Option<NaiveDate> {
    let anchor = NaiveDate::from_ymd_opt(year, month, day_of_month)?;
    let days_since_monday = anchor.weekday().num_days_from_monday();
    let offset = if days_since_monday == 0 {
        7
    } else {
        days_since_monday as i64
    };
    anchor.checked_sub_signed(Duration::days(offset))
}

fn nth_weekday_of_month(year: i32, month: u32, weekday: Weekday, n: u32) -> Option<NaiveDate> {
    if n == 0 {
        return None;
    }
    let first = NaiveDate::from_ymd_opt(year, month, 1)?;
    let offset = (weekday.num_days_from_monday() + 7 - first.weekday().num_days_from_monday()) % 7;
    let first_occurrence = first + Duration::days(offset as i64);
    first_occurrence.checked_add_signed(Duration::weeks((n - 1) as i64))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn easter_2026_matches_known_date() {
        let easter = easter_sunday(2026).expect("easter");
        assert_eq!(
            easter,
            NaiveDate::from_ymd_opt(2026, 4, 5).expect("valid date")
        );
        let good_friday = easter
            .checked_sub_signed(Duration::days(2))
            .expect("good friday");
        assert_eq!(
            good_friday,
            NaiveDate::from_ymd_opt(2026, 4, 3).expect("valid date")
        );
    }

    #[test]
    fn easter_2025_matches_known_date() {
        let easter = easter_sunday(2025).expect("easter");
        assert_eq!(
            easter,
            NaiveDate::from_ymd_opt(2025, 4, 20).expect("valid date")
        );
    }

    #[test]
    fn ontario_statutory_holidays_include_family_day() {
        let holidays = ontario_statutory_holidays(2026);
        assert!(
            holidays.contains(&NaiveDate::from_ymd_opt(2026, 2, 16).expect("valid family day"))
        );
        assert!(holidays.len() >= 11);
    }

    #[test]
    fn ontario_court_holidays_include_easter_monday_but_not_remembrance_day() {
        let holidays = ontario_court_holidays(2026);
        assert!(
            holidays.contains(&NaiveDate::from_ymd_opt(2026, 4, 6).expect("valid easter monday"))
        );
        assert!(
            !holidays
                .contains(&NaiveDate::from_ymd_opt(2026, 11, 11).expect("valid remembrance day"))
        );
    }

    #[test]
    fn christmas_and_boxing_day_observation_split_when_weekend() {
        let holidays = ontario_court_holidays(2021);
        assert!(holidays.contains(&NaiveDate::from_ymd_opt(2021, 12, 27).expect("valid date")));
        assert!(holidays.contains(&NaiveDate::from_ymd_opt(2021, 12, 28).expect("valid date")));
    }
}
