use chrono::Utc;
use tera::Context;

use crate::db::{ClientRecord, MatterRecord};

pub fn build_context(
    matter: &MatterRecord,
    client: &ClientRecord,
    extra: Option<&serde_json::Value>,
) -> serde_json::Value {
    let extra = extra.cloned().unwrap_or_else(|| serde_json::json!({}));
    serde_json::json!({
        "generated_at": Utc::now().to_rfc3339(),
        "matter": {
            "matter_id": matter.matter_id,
            "status": matter.status.as_str(),
            "stage": matter.stage,
            "practice_area": matter.practice_area,
            "jurisdiction": matter.jurisdiction,
            "opened_at": matter.opened_at.map(|dt| dt.date_naive().to_string()),
            "closed_at": matter.closed_at.map(|dt| dt.date_naive().to_string()),
            "assigned_to": matter.assigned_to,
            "custom_fields": matter.custom_fields,
        },
        "client": {
            "id": client.id.to_string(),
            "name": client.name,
            "type": client.client_type.as_str(),
            "email": client.email,
            "phone": client.phone,
            "address": client.address,
            "notes": client.notes,
        },
        "extra": extra,
    })
}

pub fn render_template(body: &str, context: &serde_json::Value) -> Result<String, String> {
    let map = context
        .as_object()
        .ok_or_else(|| "template context must be a JSON object at the root".to_string())?;
    let mut tera_context = Context::new();
    for (key, value) in map {
        tera_context.insert(key, value);
    }

    tera::Tera::one_off(body, &tera_context, false)
        .map_err(|err| format!("failed to render template: {}", err))
}

#[cfg(test)]
mod tests {
    use super::{build_context, render_template};
    use crate::db::{ClientRecord, ClientType, MatterRecord, MatterStatus};
    use chrono::Utc;
    use uuid::Uuid;

    fn sample_matter(client_id: Uuid) -> MatterRecord {
        MatterRecord {
            user_id: "u1".to_string(),
            matter_id: "demo".to_string(),
            client_id,
            status: MatterStatus::Active,
            stage: Some("pleadings".to_string()),
            practice_area: Some("litigation".to_string()),
            jurisdiction: Some("SDNY".to_string()),
            opened_at: None,
            closed_at: None,
            assigned_to: vec!["alice".to_string()],
            custom_fields: serde_json::json!({"docket": "24-cv-100"}),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    fn sample_client(client_id: Uuid) -> ClientRecord {
        ClientRecord {
            id: client_id,
            user_id: "u1".to_string(),
            name: "Acme Corp".to_string(),
            name_normalized: "acme corp".to_string(),
            client_type: ClientType::Entity,
            email: Some("legal@acme.test".to_string()),
            phone: None,
            address: None,
            notes: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn render_template_includes_matter_and_client_data() {
        let client_id = Uuid::new_v4();
        let context = build_context(
            &sample_matter(client_id),
            &sample_client(client_id),
            Some(&serde_json::json!({"request": "summary"})),
        );

        let rendered = render_template(
            "Matter {{ matter.matter_id }} for {{ client.name }} ({{ extra.request }})",
            &context,
        )
        .expect("render should succeed");
        assert!(rendered.contains("Matter demo for Acme Corp (summary)"));
    }
}
