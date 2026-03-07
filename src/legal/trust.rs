use chrono::NaiveDate;
use rust_decimal::Decimal;

use crate::db::{
    TrustAccountRecord, TrustLedgerEntryRecord, TrustReconciliationRecord,
    TrustStatementImportRecord, TrustStatementLineRecord,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedTrustStatementLine {
    pub entry_date: NaiveDate,
    pub description: String,
    pub debit: Decimal,
    pub credit: Decimal,
    pub running_balance: Decimal,
    pub reference_number: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedTrustStatement {
    pub statement_date: NaiveDate,
    pub starting_balance: Decimal,
    pub ending_balance: Decimal,
    pub lines: Vec<ParsedTrustStatementLine>,
}

fn parse_decimal(raw: &str, field: &str) -> Result<Decimal, String> {
    let normalized = raw.trim().replace(',', "");
    if normalized.is_empty() {
        return Ok(Decimal::ZERO);
    }
    normalized
        .parse::<Decimal>()
        .map_err(|_| format!("invalid {field} amount '{raw}'"))
}

fn parse_date(raw: &str) -> Result<NaiveDate, String> {
    NaiveDate::parse_from_str(raw.trim(), "%Y-%m-%d")
        .or_else(|_| NaiveDate::parse_from_str(raw.trim(), "%m/%d/%Y"))
        .map_err(|_| format!("invalid statement date '{raw}'"))
}

pub fn parse_statement_csv(
    csv_body: &str,
    statement_date_override: Option<NaiveDate>,
) -> Result<ParsedTrustStatement, String> {
    let mut reader = csv::ReaderBuilder::new()
        .trim(csv::Trim::All)
        .from_reader(csv_body.as_bytes());
    let headers = reader
        .headers()
        .map_err(|err| format!("invalid CSV header row: {err}"))?
        .clone();
    let required = ["date", "description", "debit", "credit", "balance"];
    for field in required {
        if !headers
            .iter()
            .any(|header| header.eq_ignore_ascii_case(field))
        {
            return Err(format!("missing required CSV column '{field}'"));
        }
    }

    let mut lines = Vec::new();
    for (index, record) in reader.records().enumerate() {
        let record = record.map_err(|err| format!("invalid CSV row {}: {err}", index + 2))?;
        let get = |name: &str| -> &str {
            headers
                .iter()
                .position(|header| header.eq_ignore_ascii_case(name))
                .and_then(|idx| record.get(idx))
                .unwrap_or("")
        };
        let entry_date = parse_date(get("date"))?;
        let description = get("description").trim().to_string();
        if description.is_empty() {
            return Err(format!("row {} is missing description", index + 2));
        }
        let debit = parse_decimal(get("debit"), "debit")?;
        let credit = parse_decimal(get("credit"), "credit")?;
        let running_balance = parse_decimal(get("balance"), "balance")?;
        let reference_number = {
            let value = get("reference").trim().to_string();
            if value.is_empty() { None } else { Some(value) }
        };

        lines.push(ParsedTrustStatementLine {
            entry_date,
            description,
            debit,
            credit,
            running_balance,
            reference_number,
        });
    }

    if lines.is_empty() {
        return Err("statement CSV contained no transactions".to_string());
    }

    let ending_balance = lines
        .last()
        .map(|line| line.running_balance)
        .unwrap_or_default();
    let net_change = lines
        .iter()
        .fold(Decimal::ZERO, |acc, line| acc + line.credit - line.debit);
    let starting_balance = (ending_balance - net_change).round_dp(2);
    let statement_date = statement_date_override.unwrap_or_else(|| {
        lines
            .last()
            .map(|line| line.entry_date)
            .unwrap_or_else(|| NaiveDate::from_ymd_opt(1970, 1, 1).unwrap())
    });

    Ok(ParsedTrustStatement {
        statement_date,
        starting_balance,
        ending_balance,
        lines,
    })
}

pub fn render_examiner_report(
    account: &TrustAccountRecord,
    statement: &TrustStatementImportRecord,
    statement_lines: &[TrustStatementLineRecord],
    reconciliation: &TrustReconciliationRecord,
    matter_ledger: &[TrustLedgerEntryRecord],
) -> String {
    let mut report = String::new();
    report.push_str("# Trust Reconciliation Report\n\n");
    report.push_str(&format!("Account: {}\n", account.name));
    if let Some(bank_name) = account.bank_name.as_deref() {
        report.push_str(&format!("Bank: {}\n", bank_name));
    }
    report.push_str(&format!("Statement Date: {}\n", statement.statement_date));
    report.push_str(&format!(
        "Statement Ending Balance: {}\nBook Balance: {}\nClient Ledger Total: {}\nDifference: {}\nStatus: {}\n",
        statement.ending_balance,
        reconciliation.book_balance,
        reconciliation.client_balance_total,
        reconciliation.difference,
        reconciliation.status.as_str(),
    ));
    if let Some(signed_off_by) = reconciliation.signed_off_by.as_deref() {
        report.push_str(&format!("Signed Off By: {}\n", signed_off_by));
    }
    report.push_str("\n## Statement Transactions\n");
    for line in statement_lines {
        report.push_str(&format!(
            "- {} | {} | debit {} | credit {} | balance {}\n",
            line.entry_date, line.description, line.debit, line.credit, line.running_balance
        ));
    }
    report.push_str("\n## Matter Ledger Entries\n");
    for entry in matter_ledger {
        report.push_str(&format!(
            "- {} | {} | {} | delta {} | balance {}\n",
            entry.created_at.date_naive(),
            entry.matter_id,
            entry.description,
            entry.delta,
            entry.balance_after
        ));
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    #[test]
    fn parses_canonical_statement_csv() {
        let parsed = parse_statement_csv(
            "date,description,debit,credit,balance,reference\n2026-03-01,Deposit,0,500.00,500.00,DEP-1\n2026-03-02,Check,100.00,0,400.00,CHK-1\n",
            None,
        )
        .expect("statement parsed");

        assert_eq!(parsed.lines.len(), 2);
        assert_eq!(parsed.starting_balance, Decimal::ZERO);
        assert_eq!(parsed.ending_balance, dec!(400.00));
        assert_eq!(parsed.lines[1].reference_number.as_deref(), Some("CHK-1"));
    }
}
