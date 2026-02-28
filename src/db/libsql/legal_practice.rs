use chrono::{DateTime, Utc};
use libsql::params;
use uuid::Uuid;

use crate::db::{
    ClientRecord, ClientStore, ClientType, CreateClientParams, CreateMatterNoteParams,
    CreateMatterTaskParams, MatterNoteRecord, MatterNoteStore, MatterRecord, MatterStatus,
    MatterStore, MatterTaskRecord, MatterTaskStatus, MatterTaskStore, UpdateClientParams,
    UpdateMatterNoteParams, UpdateMatterParams, UpdateMatterTaskParams, UpsertMatterParams,
    normalize_party_name,
};
use crate::error::DatabaseError;

use super::{
    LibSqlBackend, fmt_ts, get_i64, get_opt_text, get_text, opt_text, opt_text_owned,
    parse_timestamp,
};

fn parse_uuid(raw: &str, field: &str) -> Result<Uuid, DatabaseError> {
    Uuid::parse_str(raw)
        .map_err(|e| DatabaseError::Serialization(format!("invalid {} uuid: {}", field, e)))
}

fn parse_dt_opt(raw: Option<String>) -> Result<Option<DateTime<Utc>>, DatabaseError> {
    match raw {
        Some(value) => parse_timestamp(&value)
            .map(Some)
            .map_err(|e| DatabaseError::Serialization(e.to_string())),
        None => Ok(None),
    }
}

fn parse_client_type(raw: &str) -> Result<ClientType, DatabaseError> {
    ClientType::from_db_value(raw)
        .ok_or_else(|| DatabaseError::Serialization(format!("invalid client_type '{}'", raw)))
}

fn parse_matter_status(raw: &str) -> Result<MatterStatus, DatabaseError> {
    MatterStatus::from_db_value(raw)
        .ok_or_else(|| DatabaseError::Serialization(format!("invalid matter status '{}'", raw)))
}

fn parse_matter_task_status(raw: &str) -> Result<MatterTaskStatus, DatabaseError> {
    MatterTaskStatus::from_db_value(raw).ok_or_else(|| {
        DatabaseError::Serialization(format!("invalid matter task status '{}'", raw))
    })
}

fn parse_json_array_strings(raw: &str) -> Result<Vec<String>, DatabaseError> {
    if raw.trim().is_empty() {
        return Ok(Vec::new());
    }
    let parsed: serde_json::Value =
        serde_json::from_str(raw).map_err(|e| DatabaseError::Serialization(e.to_string()))?;
    Ok(parsed
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|entry| entry.as_str().map(str::to_string))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default())
}

fn parse_json_array_uuids(raw: &str) -> Result<Vec<Uuid>, DatabaseError> {
    let values = parse_json_array_strings(raw)?;
    values
        .into_iter()
        .map(|value| parse_uuid(&value, "blocked_by"))
        .collect()
}

fn parse_json_object(raw: &str) -> Result<serde_json::Value, DatabaseError> {
    if raw.trim().is_empty() {
        return Ok(serde_json::json!({}));
    }
    let value: serde_json::Value =
        serde_json::from_str(raw).map_err(|e| DatabaseError::Serialization(e.to_string()))?;
    if value.is_object() {
        Ok(value)
    } else {
        Ok(serde_json::json!({}))
    }
}

fn row_to_client_record(row: &libsql::Row) -> Result<ClientRecord, DatabaseError> {
    let client_type_raw = get_text(row, 4);
    Ok(ClientRecord {
        id: parse_uuid(&get_text(row, 0), "client.id")?,
        user_id: get_text(row, 1),
        name: get_text(row, 2),
        name_normalized: get_text(row, 3),
        client_type: parse_client_type(&client_type_raw)?,
        email: get_opt_text(row, 5),
        phone: get_opt_text(row, 6),
        address: get_opt_text(row, 7),
        notes: get_opt_text(row, 8),
        created_at: parse_timestamp(&get_text(row, 9))
            .map_err(|e| DatabaseError::Serialization(e.to_string()))?,
        updated_at: parse_timestamp(&get_text(row, 10))
            .map_err(|e| DatabaseError::Serialization(e.to_string()))?,
    })
}

fn row_to_matter_record(row: &libsql::Row) -> Result<MatterRecord, DatabaseError> {
    let status_raw = get_text(row, 3);
    let opened_at = parse_dt_opt(get_opt_text(row, 7))?;
    let closed_at = parse_dt_opt(get_opt_text(row, 8))?;

    Ok(MatterRecord {
        user_id: get_text(row, 0),
        matter_id: get_text(row, 1),
        client_id: parse_uuid(&get_text(row, 2), "matters.client_id")?,
        status: parse_matter_status(&status_raw)?,
        stage: get_opt_text(row, 4),
        practice_area: get_opt_text(row, 5),
        jurisdiction: get_opt_text(row, 6),
        opened_at,
        closed_at,
        assigned_to: parse_json_array_strings(&get_text(row, 9))?,
        custom_fields: parse_json_object(&get_text(row, 10))?,
        created_at: parse_timestamp(&get_text(row, 11))
            .map_err(|e| DatabaseError::Serialization(e.to_string()))?,
        updated_at: parse_timestamp(&get_text(row, 12))
            .map_err(|e| DatabaseError::Serialization(e.to_string()))?,
    })
}

fn row_to_matter_task_record(row: &libsql::Row) -> Result<MatterTaskRecord, DatabaseError> {
    let status_raw = get_text(row, 5);
    let due_at = parse_dt_opt(get_opt_text(row, 7))?;
    Ok(MatterTaskRecord {
        id: parse_uuid(&get_text(row, 0), "matter_tasks.id")?,
        user_id: get_text(row, 1),
        matter_id: get_text(row, 2),
        title: get_text(row, 3),
        description: get_opt_text(row, 4),
        status: parse_matter_task_status(&status_raw)?,
        assignee: get_opt_text(row, 6),
        due_at,
        blocked_by: parse_json_array_uuids(&get_text(row, 8))?,
        created_at: parse_timestamp(&get_text(row, 9))
            .map_err(|e| DatabaseError::Serialization(e.to_string()))?,
        updated_at: parse_timestamp(&get_text(row, 10))
            .map_err(|e| DatabaseError::Serialization(e.to_string()))?,
    })
}

fn row_to_matter_note_record(row: &libsql::Row) -> Result<MatterNoteRecord, DatabaseError> {
    Ok(MatterNoteRecord {
        id: parse_uuid(&get_text(row, 0), "matter_notes.id")?,
        user_id: get_text(row, 1),
        matter_id: get_text(row, 2),
        author: get_text(row, 3),
        body: get_text(row, 4),
        pinned: get_i64(row, 5) != 0,
        created_at: parse_timestamp(&get_text(row, 6))
            .map_err(|e| DatabaseError::Serialization(e.to_string()))?,
        updated_at: parse_timestamp(&get_text(row, 7))
            .map_err(|e| DatabaseError::Serialization(e.to_string()))?,
    })
}

#[async_trait::async_trait]
impl ClientStore for LibSqlBackend {
    async fn create_client(
        &self,
        user_id: &str,
        input: &CreateClientParams,
    ) -> Result<ClientRecord, DatabaseError> {
        let normalized_name = normalize_party_name(&input.name);
        if normalized_name.is_empty() {
            return Err(DatabaseError::Serialization(
                "client name cannot be empty".to_string(),
            ));
        }

        let conn = self.connect().await?;
        let id = Uuid::new_v4().to_string();
        conn.execute(
            "INSERT INTO clients (id, user_id, name, name_normalized, client_type, email, phone, address, notes) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                id,
                user_id,
                input.name.trim(),
                normalized_name.as_str(),
                input.client_type.as_str(),
                opt_text(input.email.as_deref()),
                opt_text(input.phone.as_deref()),
                opt_text(input.address.as_deref()),
                opt_text(input.notes.as_deref()),
            ],
        )
        .await?;

        let row = conn
            .query(
                "SELECT id, user_id, name, name_normalized, client_type, email, phone, address, notes, created_at, updated_at \
                 FROM clients WHERE user_id = ?1 AND name_normalized = ?2 LIMIT 1",
                params![user_id, normalized_name.as_str()],
            )
            .await?
            .next()
            .await?
            .ok_or_else(|| DatabaseError::Query("failed to load created client".to_string()))?;

        row_to_client_record(&row)
    }

    async fn upsert_client_by_normalized_name(
        &self,
        user_id: &str,
        input: &CreateClientParams,
    ) -> Result<ClientRecord, DatabaseError> {
        let normalized_name = normalize_party_name(&input.name);
        if normalized_name.is_empty() {
            return Err(DatabaseError::Serialization(
                "client name cannot be empty".to_string(),
            ));
        }

        let conn = self.connect().await?;
        conn.execute(
            "INSERT INTO clients (id, user_id, name, name_normalized, client_type, email, phone, address, notes, created_at, updated_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, datetime('now'), datetime('now')) \
             ON CONFLICT (user_id, name_normalized) DO UPDATE SET \
               name = excluded.name, \
               client_type = excluded.client_type, \
               email = COALESCE(excluded.email, clients.email), \
               phone = COALESCE(excluded.phone, clients.phone), \
               address = COALESCE(excluded.address, clients.address), \
               notes = COALESCE(excluded.notes, clients.notes), \
               updated_at = datetime('now')",
            params![
                Uuid::new_v4().to_string(),
                user_id,
                input.name.trim(),
                normalized_name.as_str(),
                input.client_type.as_str(),
                opt_text(input.email.as_deref()),
                opt_text(input.phone.as_deref()),
                opt_text(input.address.as_deref()),
                opt_text(input.notes.as_deref()),
            ],
        )
        .await?;

        let row = conn
            .query(
                "SELECT id, user_id, name, name_normalized, client_type, email, phone, address, notes, created_at, updated_at \
                 FROM clients WHERE user_id = ?1 AND name_normalized = ?2 LIMIT 1",
                params![user_id, normalized_name.as_str()],
            )
            .await?
            .next()
            .await?
            .ok_or_else(|| DatabaseError::Query("failed to resolve upserted client".to_string()))?;

        row_to_client_record(&row)
    }

    async fn list_clients(
        &self,
        user_id: &str,
        query: Option<&str>,
    ) -> Result<Vec<ClientRecord>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = if let Some(search_raw) = query {
            let search = normalize_party_name(search_raw);
            if search.is_empty() {
                conn.query(
                    "SELECT id, user_id, name, name_normalized, client_type, email, phone, address, notes, created_at, updated_at \
                     FROM clients WHERE user_id = ?1 ORDER BY name ASC",
                    params![user_id],
                )
                .await?
            } else {
                let like = format!("%{}%", search);
                conn.query(
                    "SELECT id, user_id, name, name_normalized, client_type, email, phone, address, notes, created_at, updated_at \
                     FROM clients WHERE user_id = ?1 AND name_normalized LIKE ?2 ORDER BY name ASC",
                    params![user_id, like],
                )
                .await?
            }
        } else {
            conn.query(
                "SELECT id, user_id, name, name_normalized, client_type, email, phone, address, notes, created_at, updated_at \
                 FROM clients WHERE user_id = ?1 ORDER BY name ASC",
                params![user_id],
            )
            .await?
        };

        let mut out = Vec::new();
        while let Some(row) = rows.next().await? {
            out.push(row_to_client_record(&row)?);
        }
        Ok(out)
    }

    async fn get_client(
        &self,
        user_id: &str,
        client_id: Uuid,
    ) -> Result<Option<ClientRecord>, DatabaseError> {
        let conn = self.connect().await?;
        let row = conn
            .query(
                "SELECT id, user_id, name, name_normalized, client_type, email, phone, address, notes, created_at, updated_at \
                 FROM clients WHERE user_id = ?1 AND id = ?2 LIMIT 1",
                params![user_id, client_id.to_string()],
            )
            .await?
            .next()
            .await?;

        row.map(|row| row_to_client_record(&row)).transpose()
    }

    async fn update_client(
        &self,
        user_id: &str,
        client_id: Uuid,
        input: &UpdateClientParams,
    ) -> Result<Option<ClientRecord>, DatabaseError> {
        let Some(existing) = self.get_client(user_id, client_id).await? else {
            return Ok(None);
        };

        let merged_name = input
            .name
            .as_deref()
            .unwrap_or(existing.name.as_str())
            .trim();
        let normalized_name = normalize_party_name(merged_name);
        if normalized_name.is_empty() {
            return Err(DatabaseError::Serialization(
                "client name cannot be empty".to_string(),
            ));
        }
        let merged_client_type = input.client_type.unwrap_or(existing.client_type);
        let merged_email = input.email.clone().unwrap_or(existing.email);
        let merged_phone = input.phone.clone().unwrap_or(existing.phone);
        let merged_address = input.address.clone().unwrap_or(existing.address);
        let merged_notes = input.notes.clone().unwrap_or(existing.notes);

        let conn = self.connect().await?;
        conn.execute(
            "UPDATE clients SET \
               name = ?3, \
               name_normalized = ?4, \
               client_type = ?5, \
               email = ?6, \
               phone = ?7, \
               address = ?8, \
               notes = ?9, \
               updated_at = datetime('now') \
             WHERE user_id = ?1 AND id = ?2",
            params![
                user_id,
                client_id.to_string(),
                merged_name,
                normalized_name.as_str(),
                merged_client_type.as_str(),
                opt_text(merged_email.as_deref()),
                opt_text(merged_phone.as_deref()),
                opt_text(merged_address.as_deref()),
                opt_text(merged_notes.as_deref()),
            ],
        )
        .await?;

        self.get_client(user_id, client_id).await
    }

    async fn delete_client(&self, user_id: &str, client_id: Uuid) -> Result<bool, DatabaseError> {
        let conn = self.connect().await?;
        let deleted = conn
            .execute(
                "DELETE FROM clients WHERE user_id = ?1 AND id = ?2",
                params![user_id, client_id.to_string()],
            )
            .await?;
        Ok(deleted > 0)
    }
}

#[async_trait::async_trait]
impl MatterStore for LibSqlBackend {
    async fn upsert_matter(
        &self,
        user_id: &str,
        input: &UpsertMatterParams,
    ) -> Result<MatterRecord, DatabaseError> {
        let assigned_to = serde_json::to_string(&input.assigned_to)
            .map_err(|e| DatabaseError::Serialization(e.to_string()))?;
        let custom_fields = if input.custom_fields.is_object() {
            serde_json::to_string(&input.custom_fields)
                .map_err(|e| DatabaseError::Serialization(e.to_string()))?
        } else {
            "{}".to_string()
        };

        let conn = self.connect().await?;
        conn.execute(
            "INSERT INTO matters \
             (user_id, matter_id, client_id, status, stage, practice_area, jurisdiction, opened_at, closed_at, assigned_to, custom_fields, created_at, updated_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, datetime('now'), datetime('now')) \
             ON CONFLICT (user_id, matter_id) DO UPDATE SET \
                client_id = excluded.client_id, \
                status = excluded.status, \
                stage = excluded.stage, \
                practice_area = excluded.practice_area, \
                jurisdiction = excluded.jurisdiction, \
                opened_at = excluded.opened_at, \
                closed_at = excluded.closed_at, \
                assigned_to = excluded.assigned_to, \
                custom_fields = excluded.custom_fields, \
                updated_at = datetime('now')",
            params![
                user_id,
                input.matter_id.as_str(),
                input.client_id.to_string(),
                input.status.as_str(),
                opt_text(input.stage.as_deref()),
                opt_text(input.practice_area.as_deref()),
                opt_text(input.jurisdiction.as_deref()),
                opt_text_owned(input.opened_at.as_ref().map(fmt_ts)),
                opt_text_owned(input.closed_at.as_ref().map(fmt_ts)),
                assigned_to,
                custom_fields,
            ],
        )
        .await?;

        self.get_matter_db(user_id, &input.matter_id)
            .await?
            .ok_or_else(|| DatabaseError::Query("failed to resolve upserted matter".to_string()))
    }

    async fn list_matters_db(&self, user_id: &str) -> Result<Vec<MatterRecord>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                "SELECT user_id, matter_id, client_id, status, stage, practice_area, jurisdiction, opened_at, closed_at, assigned_to, custom_fields, created_at, updated_at \
                 FROM matters WHERE user_id = ?1 ORDER BY matter_id ASC",
                params![user_id],
            )
            .await?;

        let mut out = Vec::new();
        while let Some(row) = rows.next().await? {
            out.push(row_to_matter_record(&row)?);
        }
        Ok(out)
    }

    async fn get_matter_db(
        &self,
        user_id: &str,
        matter_id: &str,
    ) -> Result<Option<MatterRecord>, DatabaseError> {
        let conn = self.connect().await?;
        let row = conn
            .query(
                "SELECT user_id, matter_id, client_id, status, stage, practice_area, jurisdiction, opened_at, closed_at, assigned_to, custom_fields, created_at, updated_at \
                 FROM matters WHERE user_id = ?1 AND matter_id = ?2 LIMIT 1",
                params![user_id, matter_id],
            )
            .await?
            .next()
            .await?;
        row.map(|row| row_to_matter_record(&row)).transpose()
    }

    async fn update_matter(
        &self,
        user_id: &str,
        matter_id: &str,
        input: &UpdateMatterParams,
    ) -> Result<Option<MatterRecord>, DatabaseError> {
        let Some(existing) = self.get_matter_db(user_id, matter_id).await? else {
            return Ok(None);
        };

        let merged = UpsertMatterParams {
            matter_id: existing.matter_id.clone(),
            client_id: input.client_id.unwrap_or(existing.client_id),
            status: input.status.unwrap_or(existing.status),
            stage: input.stage.clone().unwrap_or(existing.stage),
            practice_area: input
                .practice_area
                .clone()
                .unwrap_or(existing.practice_area),
            jurisdiction: input.jurisdiction.clone().unwrap_or(existing.jurisdiction),
            opened_at: input.opened_at.unwrap_or(existing.opened_at),
            closed_at: input.closed_at.unwrap_or(existing.closed_at),
            assigned_to: input.assigned_to.clone().unwrap_or(existing.assigned_to),
            custom_fields: input
                .custom_fields
                .clone()
                .unwrap_or(existing.custom_fields),
        };

        let updated = self.upsert_matter(user_id, &merged).await?;
        Ok(Some(updated))
    }

    async fn delete_matter(&self, user_id: &str, matter_id: &str) -> Result<bool, DatabaseError> {
        let conn = self.connect().await?;
        let deleted = conn
            .execute(
                "DELETE FROM matters WHERE user_id = ?1 AND matter_id = ?2",
                params![user_id, matter_id],
            )
            .await?;
        Ok(deleted > 0)
    }
}

#[async_trait::async_trait]
impl MatterTaskStore for LibSqlBackend {
    async fn list_matter_tasks(
        &self,
        user_id: &str,
        matter_id: &str,
    ) -> Result<Vec<MatterTaskRecord>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                "SELECT id, user_id, matter_id, title, description, status, assignee, due_at, blocked_by, created_at, updated_at \
                 FROM matter_tasks WHERE user_id = ?1 AND matter_id = ?2 ORDER BY created_at DESC",
                params![user_id, matter_id],
            )
            .await?;

        let mut out = Vec::new();
        while let Some(row) = rows.next().await? {
            out.push(row_to_matter_task_record(&row)?);
        }
        Ok(out)
    }

    async fn create_matter_task(
        &self,
        user_id: &str,
        matter_id: &str,
        input: &CreateMatterTaskParams,
    ) -> Result<MatterTaskRecord, DatabaseError> {
        let blocked_by = serde_json::to_string(
            &input
                .blocked_by
                .iter()
                .map(Uuid::to_string)
                .collect::<Vec<_>>(),
        )
        .map_err(|e| DatabaseError::Serialization(e.to_string()))?;

        let conn = self.connect().await?;
        let task_id = Uuid::new_v4();
        conn.execute(
            "INSERT INTO matter_tasks \
             (id, user_id, matter_id, title, description, status, assignee, due_at, blocked_by, created_at, updated_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, datetime('now'), datetime('now'))",
            params![
                task_id.to_string(),
                user_id,
                matter_id,
                input.title.as_str(),
                opt_text(input.description.as_deref()),
                input.status.as_str(),
                opt_text(input.assignee.as_deref()),
                opt_text_owned(input.due_at.as_ref().map(fmt_ts)),
                blocked_by,
            ],
        )
        .await?;

        let row = conn
            .query(
                "SELECT id, user_id, matter_id, title, description, status, assignee, due_at, blocked_by, created_at, updated_at \
                 FROM matter_tasks WHERE user_id = ?1 AND matter_id = ?2 AND id = ?3 LIMIT 1",
                params![user_id, matter_id, task_id.to_string()],
            )
            .await?
            .next()
            .await?
            .ok_or_else(|| DatabaseError::Query("failed to load created matter task".to_string()))?;

        row_to_matter_task_record(&row)
    }

    async fn update_matter_task(
        &self,
        user_id: &str,
        matter_id: &str,
        task_id: Uuid,
        input: &UpdateMatterTaskParams,
    ) -> Result<Option<MatterTaskRecord>, DatabaseError> {
        let conn = self.connect().await?;
        let existing_row = conn
            .query(
                "SELECT id, user_id, matter_id, title, description, status, assignee, due_at, blocked_by, created_at, updated_at \
                 FROM matter_tasks WHERE user_id = ?1 AND matter_id = ?2 AND id = ?3 LIMIT 1",
                params![user_id, matter_id, task_id.to_string()],
            )
            .await?
            .next()
            .await?;
        let Some(existing_row) = existing_row else {
            return Ok(None);
        };
        let existing = row_to_matter_task_record(&existing_row)?;

        let merged_title = input.title.clone().unwrap_or(existing.title);
        let merged_description = input.description.clone().unwrap_or(existing.description);
        let merged_status = input.status.unwrap_or(existing.status);
        let merged_assignee = input.assignee.clone().unwrap_or(existing.assignee);
        let merged_due_at = input.due_at.unwrap_or(existing.due_at);
        let merged_blocked_by = input.blocked_by.clone().unwrap_or(existing.blocked_by);
        let blocked_by = serde_json::to_string(
            &merged_blocked_by
                .into_iter()
                .map(|id| id.to_string())
                .collect::<Vec<_>>(),
        )
        .map_err(|e| DatabaseError::Serialization(e.to_string()))?;

        conn.execute(
            "UPDATE matter_tasks SET \
                title = ?4, \
                description = ?5, \
                status = ?6, \
                assignee = ?7, \
                due_at = ?8, \
                blocked_by = ?9, \
                updated_at = datetime('now') \
             WHERE user_id = ?1 AND matter_id = ?2 AND id = ?3",
            params![
                user_id,
                matter_id,
                task_id.to_string(),
                merged_title,
                opt_text(merged_description.as_deref()),
                merged_status.as_str(),
                opt_text(merged_assignee.as_deref()),
                opt_text_owned(merged_due_at.as_ref().map(fmt_ts)),
                blocked_by,
            ],
        )
        .await?;

        let updated = conn
            .query(
                "SELECT id, user_id, matter_id, title, description, status, assignee, due_at, blocked_by, created_at, updated_at \
                 FROM matter_tasks WHERE user_id = ?1 AND matter_id = ?2 AND id = ?3 LIMIT 1",
                params![user_id, matter_id, task_id.to_string()],
            )
            .await?
            .next()
            .await?;
        updated
            .map(|row| row_to_matter_task_record(&row))
            .transpose()
    }

    async fn delete_matter_task(
        &self,
        user_id: &str,
        matter_id: &str,
        task_id: Uuid,
    ) -> Result<bool, DatabaseError> {
        let conn = self.connect().await?;
        let deleted = conn
            .execute(
                "DELETE FROM matter_tasks WHERE user_id = ?1 AND matter_id = ?2 AND id = ?3",
                params![user_id, matter_id, task_id.to_string()],
            )
            .await?;
        Ok(deleted > 0)
    }
}

#[async_trait::async_trait]
impl MatterNoteStore for LibSqlBackend {
    async fn list_matter_notes(
        &self,
        user_id: &str,
        matter_id: &str,
    ) -> Result<Vec<MatterNoteRecord>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                "SELECT id, user_id, matter_id, author, body, pinned, created_at, updated_at \
                 FROM matter_notes WHERE user_id = ?1 AND matter_id = ?2 \
                 ORDER BY pinned DESC, created_at DESC",
                params![user_id, matter_id],
            )
            .await?;

        let mut out = Vec::new();
        while let Some(row) = rows.next().await? {
            out.push(row_to_matter_note_record(&row)?);
        }
        Ok(out)
    }

    async fn create_matter_note(
        &self,
        user_id: &str,
        matter_id: &str,
        input: &CreateMatterNoteParams,
    ) -> Result<MatterNoteRecord, DatabaseError> {
        let conn = self.connect().await?;
        let note_id = Uuid::new_v4();
        conn.execute(
            "INSERT INTO matter_notes (id, user_id, matter_id, author, body, pinned, created_at, updated_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, datetime('now'), datetime('now'))",
            params![
                note_id.to_string(),
                user_id,
                matter_id,
                input.author.as_str(),
                input.body.as_str(),
                if input.pinned { 1 } else { 0 },
            ],
        )
        .await?;

        let row = conn
            .query(
                "SELECT id, user_id, matter_id, author, body, pinned, created_at, updated_at \
                 FROM matter_notes WHERE user_id = ?1 AND matter_id = ?2 AND id = ?3 LIMIT 1",
                params![user_id, matter_id, note_id.to_string()],
            )
            .await?
            .next()
            .await?
            .ok_or_else(|| {
                DatabaseError::Query("failed to load created matter note".to_string())
            })?;
        row_to_matter_note_record(&row)
    }

    async fn update_matter_note(
        &self,
        user_id: &str,
        matter_id: &str,
        note_id: Uuid,
        input: &UpdateMatterNoteParams,
    ) -> Result<Option<MatterNoteRecord>, DatabaseError> {
        let conn = self.connect().await?;
        let existing_row = conn
            .query(
                "SELECT id, user_id, matter_id, author, body, pinned, created_at, updated_at \
                 FROM matter_notes WHERE user_id = ?1 AND matter_id = ?2 AND id = ?3 LIMIT 1",
                params![user_id, matter_id, note_id.to_string()],
            )
            .await?
            .next()
            .await?;
        let Some(existing_row) = existing_row else {
            return Ok(None);
        };
        let existing = row_to_matter_note_record(&existing_row)?;

        let merged_author = input.author.clone().unwrap_or(existing.author);
        let merged_body = input.body.clone().unwrap_or(existing.body);
        let merged_pinned = input.pinned.unwrap_or(existing.pinned);

        conn.execute(
            "UPDATE matter_notes SET \
                author = ?4, \
                body = ?5, \
                pinned = ?6, \
                updated_at = datetime('now') \
             WHERE user_id = ?1 AND matter_id = ?2 AND id = ?3",
            params![
                user_id,
                matter_id,
                note_id.to_string(),
                merged_author,
                merged_body,
                if merged_pinned { 1 } else { 0 },
            ],
        )
        .await?;

        let updated = conn
            .query(
                "SELECT id, user_id, matter_id, author, body, pinned, created_at, updated_at \
                 FROM matter_notes WHERE user_id = ?1 AND matter_id = ?2 AND id = ?3 LIMIT 1",
                params![user_id, matter_id, note_id.to_string()],
            )
            .await?
            .next()
            .await?;
        updated
            .map(|row| row_to_matter_note_record(&row))
            .transpose()
    }

    async fn delete_matter_note(
        &self,
        user_id: &str,
        matter_id: &str,
        note_id: Uuid,
    ) -> Result<bool, DatabaseError> {
        let conn = self.connect().await?;
        let deleted = conn
            .execute(
                "DELETE FROM matter_notes WHERE user_id = ?1 AND matter_id = ?2 AND id = ?3",
                params![user_id, matter_id, note_id.to_string()],
            )
            .await?;
        Ok(deleted > 0)
    }
}
