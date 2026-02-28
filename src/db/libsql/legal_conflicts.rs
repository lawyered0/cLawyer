use std::collections::HashSet;

use chrono::{DateTime, NaiveDate, Utc};
use libsql::params;
use uuid::Uuid;

use crate::db::{
    ConflictClearanceRecord, ConflictHit, LegalConflictStore, PartyRole, conflict_terms_from_text,
    normalize_party_name, trigram_similarity,
};
use crate::error::DatabaseError;

use super::{LibSqlBackend, get_text, opt_text};

fn match_priority(matched_via: &str) -> u8 {
    if matched_via == "direct" {
        3
    } else if matched_via.starts_with("alias:") {
        2
    } else {
        1
    }
}

fn normalize_input_terms(input_names: &[String]) -> Vec<String> {
    input_names
        .iter()
        .map(|name| normalize_party_name(name))
        .filter(|name| !name.is_empty())
        .collect()
}

fn parse_opened_at_text(raw: Option<&str>) -> Result<Option<String>, DatabaseError> {
    let Some(raw) = raw.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(None);
    };

    if let Ok(date) = NaiveDate::parse_from_str(raw, "%Y-%m-%d") {
        let dt = date
            .and_hms_opt(0, 0, 0)
            .ok_or_else(|| DatabaseError::Serialization("invalid opened_at date".to_string()))?;
        return Ok(Some(
            dt.and_utc()
                .to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        ));
    }

    if let Ok(dt) = DateTime::parse_from_rfc3339(raw) {
        return Ok(Some(
            dt.with_timezone(&Utc)
                .to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        ));
    }

    Err(DatabaseError::Serialization(format!(
        "invalid opened_at timestamp '{}'",
        raw
    )))
}

fn dedupe_hits(
    rows: Vec<(String, String, String, String, String, f64)>,
    limit: usize,
) -> Vec<ConflictHit> {
    let mut best: std::collections::HashMap<(String, String, String), (u8, f64, ConflictHit)> =
        std::collections::HashMap::new();

    for (party, role_raw, matter_id, matter_status, matched_via, score) in rows {
        let Some(role) = PartyRole::from_db_value(&role_raw) else {
            continue;
        };

        let key = (party.clone(), role_raw, matter_id.clone());
        let hit = ConflictHit {
            party,
            role,
            matter_id,
            matter_status,
            matched_via: matched_via.clone(),
        };
        let priority = match_priority(&matched_via);

        match best.get(&key) {
            Some((existing_priority, existing_score, _))
                if *existing_priority > priority
                    || (*existing_priority == priority && *existing_score >= score) => {}
            _ => {
                best.insert(key, (priority, score, hit));
            }
        }
    }

    let mut hits: Vec<ConflictHit> = best.into_values().map(|(_, _, hit)| hit).collect();
    hits.sort_by(|a, b| {
        a.party
            .cmp(&b.party)
            .then_with(|| a.matter_id.cmp(&b.matter_id))
            .then_with(|| a.matched_via.cmp(&b.matched_via))
    });
    if hits.len() > limit {
        hits.truncate(limit);
    }
    hits
}

async fn upsert_party_libsql(
    backend: &LibSqlBackend,
    name: &str,
) -> Result<Option<String>, DatabaseError> {
    let display_name = name.trim();
    if display_name.is_empty() {
        return Ok(None);
    }
    let normalized = normalize_party_name(display_name);
    if normalized.is_empty() {
        return Ok(None);
    }

    let conn = backend.connect().await?;
    conn.execute(
        "INSERT INTO parties (id, name, name_normalized, party_type, created_at, updated_at) \
         VALUES (?1, ?2, ?3, 'entity', datetime('now'), datetime('now')) \
         ON CONFLICT(name_normalized) DO UPDATE SET \
           name = excluded.name, \
           updated_at = datetime('now')",
        params![
            Uuid::new_v4().to_string(),
            display_name,
            normalized.as_str()
        ],
    )
    .await?;

    let row = conn
        .query(
            "SELECT id FROM parties WHERE name_normalized = ?1 LIMIT 1",
            params![normalized.as_str()],
        )
        .await?
        .next()
        .await?
        .ok_or_else(|| DatabaseError::Query("failed to resolve upserted party".to_string()))?;

    Ok(Some(get_text(&row, 0)))
}

#[async_trait::async_trait]
impl LegalConflictStore for LibSqlBackend {
    async fn find_conflict_hits_for_names(
        &self,
        input_names: &[String],
        limit: usize,
    ) -> Result<Vec<ConflictHit>, DatabaseError> {
        let terms = normalize_input_terms(input_names);
        if terms.is_empty() || limit == 0 {
            return Ok(Vec::new());
        }

        let limit = limit.min(200);
        let conn = self.connect().await?;
        let mut rows: Vec<(String, String, String, String, String, f64)> = Vec::new();

        for term in &terms {
            let mut direct_rows = conn
                .query(
                    "SELECT p.name, mp.role, mp.matter_id, \
                            CASE WHEN mp.closed_at IS NULL THEN 'Open' ELSE 'Closed' END AS matter_status, \
                            'direct' AS matched_via \
                     FROM parties p \
                     JOIN matter_parties mp ON mp.party_id = p.id \
                     WHERE p.name_normalized = ?1 \
                     LIMIT ?2",
                    params![term.as_str(), limit as i64],
                )
                .await?;
            while let Some(row) = direct_rows.next().await? {
                rows.push((
                    get_text(&row, 0),
                    get_text(&row, 1),
                    get_text(&row, 2),
                    get_text(&row, 3),
                    get_text(&row, 4),
                    1.0,
                ));
            }

            let mut alias_rows = conn
                .query(
                    "SELECT p.name, mp.role, mp.matter_id, \
                            CASE WHEN mp.closed_at IS NULL THEN 'Open' ELSE 'Closed' END AS matter_status, \
                            ('alias:' || pa.alias) AS matched_via \
                     FROM party_aliases pa \
                     JOIN parties p ON p.id = pa.party_id \
                     JOIN matter_parties mp ON mp.party_id = p.id \
                     WHERE pa.alias_normalized = ?1 \
                     LIMIT ?2",
                    params![term.as_str(), limit as i64],
                )
                .await?;
            while let Some(row) = alias_rows.next().await? {
                rows.push((
                    get_text(&row, 0),
                    get_text(&row, 1),
                    get_text(&row, 2),
                    get_text(&row, 3),
                    get_text(&row, 4),
                    0.9,
                ));
            }

            // Fuzzy fallback: narrow via token LIKE candidates, then score via trigram in Rust.
            let mut fuzzy_tokens: Vec<String> = term
                .split_whitespace()
                .filter(|token| token.len() >= 3)
                .map(str::to_string)
                .collect();
            if fuzzy_tokens.is_empty() {
                fuzzy_tokens.push(term.clone());
            }
            fuzzy_tokens.sort();
            fuzzy_tokens.dedup();

            for token in &fuzzy_tokens {
                let like_term = format!("%{}%", token);
                let mut fuzzy_party_rows = conn
                    .query(
                        "SELECT p.name, p.name_normalized, mp.role, mp.matter_id, \
                                CASE WHEN mp.closed_at IS NULL THEN 'Open' ELSE 'Closed' END AS matter_status \
                         FROM parties p \
                         JOIN matter_parties mp ON mp.party_id = p.id \
                         WHERE p.name_normalized LIKE ?1 \
                         LIMIT ?2",
                        params![like_term, limit as i64],
                    )
                    .await?;
                while let Some(row) = fuzzy_party_rows.next().await? {
                    let normalized = get_text(&row, 1);
                    let score = trigram_similarity(&normalized, term);
                    if score >= 0.45 {
                        rows.push((
                            get_text(&row, 0),
                            get_text(&row, 2),
                            get_text(&row, 3),
                            get_text(&row, 4),
                            format!("fuzzy:{term}"),
                            score,
                        ));
                    }
                }

                let mut fuzzy_alias_rows = conn
                    .query(
                        "SELECT p.name, pa.alias_normalized, mp.role, mp.matter_id, \
                                CASE WHEN mp.closed_at IS NULL THEN 'Open' ELSE 'Closed' END AS matter_status \
                         FROM party_aliases pa \
                         JOIN parties p ON p.id = pa.party_id \
                         JOIN matter_parties mp ON mp.party_id = p.id \
                         WHERE pa.alias_normalized LIKE ?1 \
                         LIMIT ?2",
                        params![format!("%{}%", token), limit as i64],
                    )
                    .await?;
                while let Some(row) = fuzzy_alias_rows.next().await? {
                    let normalized = get_text(&row, 1);
                    let score = trigram_similarity(&normalized, term);
                    if score >= 0.45 {
                        rows.push((
                            get_text(&row, 0),
                            get_text(&row, 2),
                            get_text(&row, 3),
                            get_text(&row, 4),
                            format!("fuzzy:{term}"),
                            score,
                        ));
                    }
                }
            }
        }

        Ok(dedupe_hits(rows, limit))
    }

    async fn find_conflict_hits_for_text(
        &self,
        text: &str,
        active_matter: Option<&str>,
        limit: usize,
    ) -> Result<Vec<ConflictHit>, DatabaseError> {
        let terms = conflict_terms_from_text(text, active_matter);
        self.find_conflict_hits_for_names(&terms, limit).await
    }

    async fn seed_matter_parties(
        &self,
        matter_id: &str,
        client: &str,
        adversaries: &[String],
        opened_at: Option<&str>,
    ) -> Result<(), DatabaseError> {
        let matter_id = matter_id.trim();
        if matter_id.is_empty() {
            return Err(DatabaseError::Serialization(
                "matter_id cannot be empty".to_string(),
            ));
        }

        let opened_at = parse_opened_at_text(opened_at)?;

        if let Some(client_party_id) = upsert_party_libsql(self, client).await? {
            let conn = self.connect().await?;
            conn.execute(
                "INSERT INTO matter_parties \
                 (id, matter_id, party_id, role, opened_at, closed_at, created_at, updated_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, datetime('now'), datetime('now')) \
                 ON CONFLICT(matter_id, party_id, role) DO UPDATE SET \
                    opened_at = COALESCE(matter_parties.opened_at, excluded.opened_at), \
                    updated_at = datetime('now')",
                params![
                    Uuid::new_v4().to_string(),
                    matter_id,
                    client_party_id,
                    PartyRole::Client.as_str(),
                    opt_text(opened_at.as_deref()),
                    libsql::Value::Null,
                ],
            )
            .await?;
        }

        for name in adversaries {
            let Some(adverse_party_id) = upsert_party_libsql(self, name).await? else {
                continue;
            };
            let conn = self.connect().await?;
            conn.execute(
                "INSERT INTO matter_parties \
                 (id, matter_id, party_id, role, opened_at, closed_at, created_at, updated_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, datetime('now'), datetime('now')) \
                 ON CONFLICT(matter_id, party_id, role) DO UPDATE SET \
                    opened_at = COALESCE(matter_parties.opened_at, excluded.opened_at), \
                    updated_at = datetime('now')",
                params![
                    Uuid::new_v4().to_string(),
                    matter_id,
                    adverse_party_id,
                    PartyRole::Adverse.as_str(),
                    opt_text(opened_at.as_deref()),
                    libsql::Value::Null,
                ],
            )
            .await?;
        }

        Ok(())
    }

    async fn reset_conflict_graph(&self) -> Result<(), DatabaseError> {
        let conn = self.connect().await?;
        conn.execute("DELETE FROM matter_parties", ()).await?;
        conn.execute("DELETE FROM party_aliases", ()).await?;
        conn.execute("DELETE FROM party_relationships", ()).await?;
        conn.execute("DELETE FROM parties", ()).await?;
        Ok(())
    }

    async fn upsert_party_aliases(
        &self,
        canonical_name: &str,
        aliases: &[String],
    ) -> Result<(), DatabaseError> {
        if aliases.is_empty() {
            return Ok(());
        }

        let Some(party_id) = upsert_party_libsql(self, canonical_name).await? else {
            return Ok(());
        };

        let conn = self.connect().await?;
        let mut seen: HashSet<String> = HashSet::new();
        for alias in aliases {
            let display_alias = alias.trim();
            if display_alias.is_empty() {
                continue;
            }
            let normalized_alias = normalize_party_name(display_alias);
            if normalized_alias.is_empty() || !seen.insert(normalized_alias.clone()) {
                continue;
            }
            conn.execute(
                "INSERT INTO party_aliases \
                 (id, party_id, alias, alias_normalized, created_at, updated_at) \
                 VALUES (?1, ?2, ?3, ?4, datetime('now'), datetime('now')) \
                 ON CONFLICT(party_id, alias_normalized) DO UPDATE SET \
                    alias = excluded.alias, \
                    updated_at = datetime('now')",
                params![
                    Uuid::new_v4().to_string(),
                    party_id.as_str(),
                    display_alias,
                    normalized_alias,
                ],
            )
            .await?;
        }
        Ok(())
    }

    async fn record_conflict_clearance(
        &self,
        row: &ConflictClearanceRecord,
    ) -> Result<(), DatabaseError> {
        let conn = self.connect().await?;
        let hits_json = serde_json::to_string(&row.hits_json)
            .map_err(|e| DatabaseError::Serialization(e.to_string()))?;
        conn.execute(
            "INSERT INTO conflict_clearances \
             (id, matter_id, checked_by, cleared_by, decision, note, hits_json, hit_count, created_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, datetime('now'))",
            params![
                Uuid::new_v4().to_string(),
                row.matter_id.as_str(),
                row.checked_by.as_str(),
                opt_text(row.cleared_by.as_deref()),
                row.decision.as_str(),
                opt_text(row.note.as_deref()),
                hits_json,
                row.hit_count,
            ],
        )
        .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;

    struct TestBackend {
        backend: LibSqlBackend,
        _tmpdir: tempfile::TempDir,
    }

    async fn setup_backend() -> TestBackend {
        // Use a temp-file database so all connections share schema/state.
        let tmpdir = tempfile::tempdir().expect("tempdir");
        let db_path = tmpdir.path().join("legal_conflicts_test.db");
        let backend = LibSqlBackend::new_local(&db_path)
            .await
            .expect("local backend should initialize");
        backend
            .run_migrations()
            .await
            .expect("migrations should succeed");
        TestBackend {
            backend,
            _tmpdir: tmpdir,
        }
    }

    #[tokio::test]
    async fn schema_contains_legal_conflict_tables() {
        let fixture = setup_backend().await;
        let conn = fixture.backend.connect().await.expect("connect");

        for table in [
            "parties",
            "party_aliases",
            "party_relationships",
            "matter_parties",
            "conflict_clearances",
        ] {
            let row = conn
                .query(
                    "SELECT name FROM sqlite_master WHERE type='table' AND name=?1",
                    params![table],
                )
                .await
                .expect("query sqlite_master")
                .next()
                .await
                .expect("row read");
            assert!(row.is_some(), "missing table {table}");
        }
    }

    #[tokio::test]
    async fn find_conflict_hits_for_names_exact_match() {
        let fixture = setup_backend().await;
        fixture
            .backend
            .seed_matter_parties("matter-a", "Acme Corp", &[], Some("2026-01-01"))
            .await
            .expect("seed matter parties");

        let hits = fixture
            .backend
            .find_conflict_hits_for_names(&["Acme Corp".to_string()], 20)
            .await
            .expect("query hits");

        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].party, "Acme Corp");
        assert_eq!(hits[0].role, PartyRole::Client);
        assert_eq!(hits[0].matter_id, "matter-a");
        assert_eq!(hits[0].matched_via, "direct");
    }

    #[tokio::test]
    async fn alias_hit_maps_to_canonical_party() {
        let fixture = setup_backend().await;
        fixture
            .backend
            .seed_matter_parties("matter-a", "Acme Corporation", &[], None)
            .await
            .expect("seed parties");

        let conn = fixture.backend.connect().await.expect("connect");
        let party_id = conn
            .query(
                "SELECT id FROM parties WHERE name_normalized = ?1 LIMIT 1",
                params!["acme corporation"],
            )
            .await
            .expect("party query")
            .next()
            .await
            .expect("party row")
            .expect("party exists")
            .get::<String>(0)
            .expect("party id text");

        conn.execute(
            "INSERT INTO party_aliases (id, party_id, alias, alias_normalized, created_at, updated_at) \
             VALUES (?1, ?2, ?3, ?4, datetime('now'), datetime('now'))",
            params![
                Uuid::new_v4().to_string(),
                party_id,
                "Acme",
                "acme",
            ],
        )
        .await
        .expect("insert alias");

        let hits = fixture
            .backend
            .find_conflict_hits_for_names(&["Acme".to_string()], 20)
            .await
            .expect("query hits");

        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].party, "Acme Corporation");
        assert!(hits[0].matched_via.starts_with("alias:"));
    }

    #[tokio::test]
    async fn fuzzy_typo_hit_matches_without_phonetics() {
        let fixture = setup_backend().await;
        fixture
            .backend
            .seed_matter_parties("matter-a", "Smith Holdings", &[], None)
            .await
            .expect("seed parties");

        let hits = fixture
            .backend
            .find_conflict_hits_for_names(&["Smit Holdings".to_string()], 20)
            .await
            .expect("query hits");

        assert!(
            hits.iter().any(|hit| {
                hit.party == "Smith Holdings"
                    && (hit.matched_via == "direct" || hit.matched_via.starts_with("fuzzy:"))
            }),
            "expected fuzzy match for Smith/Smyth"
        );
    }

    #[tokio::test]
    async fn results_are_deduped_by_party_role_matter() {
        let fixture = setup_backend().await;
        fixture
            .backend
            .seed_matter_parties("matter-a", "Acme Corp", &[], None)
            .await
            .expect("seed parties");

        let conn = fixture.backend.connect().await.expect("connect");
        let party_id = conn
            .query(
                "SELECT id FROM parties WHERE name_normalized = ?1 LIMIT 1",
                params!["acme corp"],
            )
            .await
            .expect("party query")
            .next()
            .await
            .expect("party row")
            .expect("party exists")
            .get::<String>(0)
            .expect("party id text");

        conn.execute(
            "INSERT INTO party_aliases (id, party_id, alias, alias_normalized, created_at, updated_at) \
             VALUES (?1, ?2, ?3, ?4, datetime('now'), datetime('now'))",
            params![Uuid::new_v4().to_string(), party_id, "Acme", "acme"],
        )
        .await
        .expect("insert alias");

        let hits = fixture
            .backend
            .find_conflict_hits_for_names(&["Acme Corp".to_string(), "Acme".to_string()], 20)
            .await
            .expect("query hits");

        assert_eq!(hits.len(), 1, "expected deduped hit list");
    }

    #[tokio::test]
    async fn seed_matter_parties_is_idempotent() {
        let fixture = setup_backend().await;
        let adversaries = vec!["Foo LLC".to_string()];

        fixture
            .backend
            .seed_matter_parties("matter-a", "Acme Corp", &adversaries, Some("2026-02-20"))
            .await
            .expect("first seed");
        fixture
            .backend
            .seed_matter_parties("matter-a", "Acme Corp", &adversaries, Some("2026-02-20"))
            .await
            .expect("second seed");

        let conn = fixture.backend.connect().await.expect("connect");
        let parties_count = conn
            .query("SELECT COUNT(*) FROM parties", ())
            .await
            .expect("count parties")
            .next()
            .await
            .expect("row read")
            .expect("count row")
            .get::<i64>(0)
            .expect("count int");
        assert_eq!(parties_count, 2);

        let matter_parties_count = conn
            .query(
                "SELECT COUNT(*) FROM matter_parties WHERE matter_id = ?1",
                params!["matter-a"],
            )
            .await
            .expect("count matter parties")
            .next()
            .await
            .expect("row read")
            .expect("count row")
            .get::<i64>(0)
            .expect("count int");
        assert_eq!(matter_parties_count, 2);
    }

    #[tokio::test]
    async fn upsert_party_aliases_is_idempotent() {
        let fixture = setup_backend().await;
        fixture
            .backend
            .upsert_party_aliases(
                "Acme Corp",
                &["Acme".to_string(), "Acme Corporation".to_string()],
            )
            .await
            .expect("initial alias upsert");
        fixture
            .backend
            .upsert_party_aliases(
                "Acme Corp",
                &["Acme".to_string(), "Acme Corporation".to_string()],
            )
            .await
            .expect("repeated alias upsert");

        let conn = fixture.backend.connect().await.expect("connect");
        let alias_count = conn
            .query("SELECT COUNT(*) FROM party_aliases", ())
            .await
            .expect("count aliases")
            .next()
            .await
            .expect("row read")
            .expect("row exists")
            .get::<i64>(0)
            .expect("count int");
        assert_eq!(alias_count, 2);
    }

    #[tokio::test]
    async fn reset_conflict_graph_clears_party_graph_tables() {
        let fixture = setup_backend().await;
        fixture
            .backend
            .seed_matter_parties("matter-a", "Acme Corp", &["Foo LLC".to_string()], None)
            .await
            .expect("seed parties");
        fixture
            .backend
            .upsert_party_aliases("Acme Corp", &["Acme".to_string()])
            .await
            .expect("seed aliases");

        fixture
            .backend
            .reset_conflict_graph()
            .await
            .expect("reset graph");

        let conn = fixture.backend.connect().await.expect("connect");
        for (table, expected) in [
            ("matter_parties", 0i64),
            ("party_aliases", 0i64),
            ("party_relationships", 0i64),
            ("parties", 0i64),
        ] {
            let count = conn
                .query(&format!("SELECT COUNT(*) FROM {table}"), ())
                .await
                .expect("count query")
                .next()
                .await
                .expect("row read")
                .expect("row exists")
                .get::<i64>(0)
                .expect("count int");
            assert_eq!(count, expected, "unexpected row count for {table}");
        }
    }
}
