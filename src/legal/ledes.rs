use chrono::NaiveDate;
use rust_decimal::Decimal;

const LEDES_98B_HEADER: &str = concat!(
    "INVOICE_DATE|INVOICE_NUMBER|CLIENT_ID|LAW_FIRM_MATTER_ID|INVOICE_TOTAL|",
    "BILLING_START_DATE|BILLING_END_DATE|LINE_ITEM_NUMBER|EXP/FEE/INV_ADJ_TYPE|",
    "LINE_ITEM_NUMBER_OF_UNITS|LINE_ITEM_ADJUSTMENT_AMOUNT|LINE_ITEM_TOTAL|LINE_ITEM_DATE|",
    "LINE_ITEM_TASK_CODE|LINE_ITEM_EXPENSE_CODE|LINE_ITEM_ACTIVITY_CODE|TIMEKEEPER_ID|",
    "LINE_ITEM_DESCRIPTION|LAW_FIRM_ID|LINE_ITEM_UNIT_COST|TIMEKEEPER_NAME|",
    "TIMEKEEPER_CLASSIFICATION|CLIENT_MATTER_ID|PO_NUMBER"
);

#[derive(Debug, Clone)]
pub struct Ledes98BInvoiceContext {
    pub invoice_date: NaiveDate,
    pub invoice_number: String,
    pub client_id: String,
    pub law_firm_matter_id: String,
    pub invoice_total: Decimal,
    pub billing_start_date: NaiveDate,
    pub billing_end_date: NaiveDate,
    pub law_firm_id: String,
    pub client_matter_id: String,
    pub po_number: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Ledes98BLineItem {
    pub line_item_number: usize,
    pub line_item_type: String,
    pub units: Decimal,
    pub adjustment_amount: Decimal,
    pub total: Decimal,
    pub line_item_date: NaiveDate,
    pub task_code: Option<String>,
    pub expense_code: Option<String>,
    pub activity_code: Option<String>,
    pub timekeeper_id: Option<String>,
    pub description: String,
    pub unit_cost: Decimal,
    pub timekeeper_name: Option<String>,
    pub timekeeper_classification: Option<String>,
}

fn fmt_date(date: NaiveDate) -> String {
    date.format("%Y%m%d").to_string()
}

fn fmt_decimal(value: Decimal) -> String {
    value.round_dp(2).to_string()
}

fn sanitize_field(value: &str) -> String {
    value.replace(['|', '\n', '\r'], " ").trim().to_string()
}

pub fn export_ledes98b(
    invoice: &Ledes98BInvoiceContext,
    line_items: &[Ledes98BLineItem],
) -> String {
    let mut lines = Vec::with_capacity(line_items.len() + 2);
    lines.push("LEDES1998B[]".to_string());
    lines.push(LEDES_98B_HEADER.to_string());

    for item in line_items {
        let row = [
            fmt_date(invoice.invoice_date),
            sanitize_field(&invoice.invoice_number),
            sanitize_field(&invoice.client_id),
            sanitize_field(&invoice.law_firm_matter_id),
            fmt_decimal(invoice.invoice_total),
            fmt_date(invoice.billing_start_date),
            fmt_date(invoice.billing_end_date),
            item.line_item_number.to_string(),
            sanitize_field(&item.line_item_type),
            fmt_decimal(item.units),
            fmt_decimal(item.adjustment_amount),
            fmt_decimal(item.total),
            fmt_date(item.line_item_date),
            item.task_code
                .as_deref()
                .map(sanitize_field)
                .unwrap_or_default(),
            item.expense_code
                .as_deref()
                .map(sanitize_field)
                .unwrap_or_default(),
            item.activity_code
                .as_deref()
                .map(sanitize_field)
                .unwrap_or_default(),
            item.timekeeper_id
                .as_deref()
                .map(sanitize_field)
                .unwrap_or_default(),
            sanitize_field(&item.description),
            sanitize_field(&invoice.law_firm_id),
            fmt_decimal(item.unit_cost),
            item.timekeeper_name
                .as_deref()
                .map(sanitize_field)
                .unwrap_or_default(),
            item.timekeeper_classification
                .as_deref()
                .map(sanitize_field)
                .unwrap_or_else(|| "OT".to_string()),
            sanitize_field(&invoice.client_matter_id),
            invoice
                .po_number
                .as_deref()
                .map(sanitize_field)
                .unwrap_or_default(),
        ]
        .join("|");
        lines.push(row);
    }

    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    #[test]
    fn renders_ledes_98b_header_and_row() {
        let output = export_ledes98b(
            &Ledes98BInvoiceContext {
                invoice_date: NaiveDate::from_ymd_opt(2026, 3, 7).unwrap(),
                invoice_number: "INV-100".to_string(),
                client_id: "CLIENT-1".to_string(),
                law_firm_matter_id: "matter-1".to_string(),
                invoice_total: dec!(150.00),
                billing_start_date: NaiveDate::from_ymd_opt(2026, 3, 1).unwrap(),
                billing_end_date: NaiveDate::from_ymd_opt(2026, 3, 7).unwrap(),
                law_firm_id: "firm-1".to_string(),
                client_matter_id: "client-matter-1".to_string(),
                po_number: None,
            },
            &[Ledes98BLineItem {
                line_item_number: 1,
                line_item_type: "FEE".to_string(),
                units: dec!(1.50),
                adjustment_amount: Decimal::ZERO,
                total: dec!(150.00),
                line_item_date: NaiveDate::from_ymd_opt(2026, 3, 5).unwrap(),
                task_code: Some("B110".to_string()),
                expense_code: None,
                activity_code: Some("A101".to_string()),
                timekeeper_id: Some("attorney-1".to_string()),
                description: "Draft motion".to_string(),
                unit_cost: dec!(100.00),
                timekeeper_name: Some("Attorney One".to_string()),
                timekeeper_classification: Some("OT".to_string()),
            }],
        );

        assert!(output.starts_with("LEDES1998B[]\nINVOICE_DATE|INVOICE_NUMBER|CLIENT_ID|"));
        assert!(output.contains("20260307|INV-100|CLIENT-1|matter-1|150.00|20260301|20260307|1|FEE|1.50|0|150.00|20260305|B110||A101|attorney-1|Draft motion|firm-1|100.00|Attorney One|OT|client-matter-1|"));
    }
}
