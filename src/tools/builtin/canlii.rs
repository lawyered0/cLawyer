//! CanLII search tool.

use std::collections::HashSet;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use reqwest::Client;

use crate::config::LegalConfig;
use crate::context::JobContext;
use crate::legal::policy::is_network_domain_allowed;
use crate::tools::tool::{Tool, ToolError, ToolOutput, ToolRateLimitConfig, require_str};

pub struct CanLiiSearchTool {
    client: Client,
    legal: Option<LegalConfig>,
    api_key_override: Option<String>,
    base_url: String,
    read_env_api_key: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CanLiiDatabase {
    id: String,
    year: Option<i32>,
}

impl CanLiiSearchTool {
    pub fn new() -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .unwrap_or_else(|_| Client::new());
        Self {
            client,
            legal: None,
            api_key_override: None,
            base_url: "https://api.canlii.org/v1".to_string(),
            read_env_api_key: true,
        }
    }

    pub fn with_legal_policy(mut self, legal: LegalConfig) -> Self {
        self.legal = Some(legal);
        self
    }

    pub fn with_api_key(mut self, api_key: impl Into<String>) -> Self {
        self.api_key_override = Some(api_key.into());
        self
    }

    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }

    pub fn without_env_api_key(mut self) -> Self {
        self.read_env_api_key = false;
        self
    }

    fn api_key(&self) -> Option<String> {
        self.api_key_override.clone().or_else(|| {
            if !self.read_env_api_key {
                return None;
            }
            std::env::var("CANLII_API_KEY")
                .ok()
                .filter(|value| !value.trim().is_empty())
        })
    }
}

impl Default for CanLiiSearchTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for CanLiiSearchTool {
    fn name(&self) -> &str {
        "canlii_search"
    }

    fn description(&self) -> &str {
        "Search Canadian case law and legislation on CanLII. Requires CANLII_API_KEY. Searches within one specified jurisdiction per query because CanLII does not support a single cross-jurisdiction search call."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Full-text search keywords"
                },
                "jurisdiction": {
                    "type": "string",
                    "description": "CanLII jurisdiction code, e.g. 'onca', 'onsc', 'scc', 'ca'"
                },
                "language": {
                    "type": "string",
                    "enum": ["en", "fr"],
                    "description": "Language for results (default: en)"
                },
                "max_results": {
                    "type": "integer",
                    "description": "Maximum results to return (default: 10, max: 20)"
                }
            },
            "required": ["query", "jurisdiction"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        _ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = Instant::now();
        if let Some(legal) = self.legal.as_ref()
            && !is_network_domain_allowed(legal, "api.canlii.org")
        {
            return Err(ToolError::NotAuthorized(
                "api.canlii.org is blocked by legal network policy".to_string(),
            ));
        }

        let api_key = self.api_key().ok_or_else(|| {
            ToolError::ExecutionFailed("CANLII_API_KEY is not configured".to_string())
        })?;
        let query = require_str(&params, "query")?.trim();
        let jurisdiction = require_str(&params, "jurisdiction")?
            .trim()
            .to_ascii_lowercase();
        let language = params
            .get("language")
            .and_then(|value| value.as_str())
            .unwrap_or("en");
        if !matches!(language, "en" | "fr") {
            return Err(ToolError::InvalidParameters(
                "language must be 'en' or 'fr'".to_string(),
            ));
        }
        let max_results = params
            .get("max_results")
            .and_then(|value| value.as_u64())
            .map(|value| value.clamp(1, 20) as usize)
            .unwrap_or(10);

        let browse_response = self
            .client
            .get(format!(
                "{}/caseBrowse/{}/{}/",
                self.base_url.trim_end_matches('/'),
                language,
                jurisdiction
            ))
            .query(&[("api_key", api_key.as_str())])
            .send()
            .await
            .map_err(|err| {
                ToolError::ExternalService(format!("CanLII browse request failed: {err}"))
            })?;

        if !browse_response.status().is_success() {
            return Err(ToolError::ExternalService(format!(
                "CanLII browse returned HTTP {}",
                browse_response.status().as_u16()
            )));
        }

        let browse_json = browse_response
            .json::<serde_json::Value>()
            .await
            .map_err(|err| {
                ToolError::ExternalService(format!("CanLII browse parse failed: {err}"))
            })?;
        let mut databases = extract_databases(&browse_json);
        databases.sort_by(|left, right| {
            right
                .year
                .cmp(&left.year)
                .then_with(|| left.id.cmp(&right.id))
        });

        if databases.is_empty() {
            return Ok(ToolOutput::text(
                format!(
                    "No CanLII databases were returned for jurisdiction '{}'. CanLII searches one jurisdiction at a time; verify the code and try again.",
                    jurisdiction
                ),
                start.elapsed(),
            ));
        }

        let mut results = Vec::new();
        let mut seen = HashSet::new();
        for database in databases {
            let search_response = self
                .client
                .get(format!(
                    "{}/caseSearch/{}/{}/",
                    self.base_url.trim_end_matches('/'),
                    language,
                    database.id
                ))
                .query(&[("keyword", query), ("api_key", api_key.as_str())])
                .send()
                .await
                .map_err(|err| {
                    ToolError::ExternalService(format!(
                        "CanLII search request failed for {}: {err}",
                        database.id
                    ))
                })?;
            if !search_response.status().is_success() {
                return Err(ToolError::ExternalService(format!(
                    "CanLII search returned HTTP {} for {}",
                    search_response.status().as_u16(),
                    database.id
                )));
            }

            let search_json = search_response
                .json::<serde_json::Value>()
                .await
                .map_err(|err| {
                    ToolError::ExternalService(format!(
                        "CanLII search parse failed for {}: {err}",
                        database.id
                    ))
                })?;
            for result in extract_search_results(&search_json, &database.id) {
                let unique_key = format!(
                    "{}|{}",
                    result
                        .get("case_id")
                        .and_then(|value| value.as_str())
                        .unwrap_or_default(),
                    result
                        .get("url")
                        .and_then(|value| value.as_str())
                        .unwrap_or_default()
                );
                if seen.insert(unique_key) {
                    results.push(result);
                    if results.len() >= max_results {
                        break;
                    }
                }
            }
            if results.len() >= max_results {
                break;
            }
        }

        Ok(ToolOutput::success(
            serde_json::json!({
                "note": "CanLII search is jurisdiction-scoped; run separate queries for other courts or jurisdictions.",
                "jurisdiction": jurisdiction,
                "results": results,
            }),
            start.elapsed(),
        ))
    }

    fn rate_limit_config(&self) -> Option<ToolRateLimitConfig> {
        Some(ToolRateLimitConfig::new(20, 200))
    }
}

fn extract_databases(payload: &serde_json::Value) -> Vec<CanLiiDatabase> {
    let Some(items) = payload
        .get("caseDatabases")
        .or_else(|| payload.get("databases"))
        .or_else(|| payload.get("results"))
        .and_then(|value| value.as_array())
    else {
        return Vec::new();
    };

    items
        .iter()
        .filter_map(|item| {
            let id = item
                .get("databaseId")
                .or_else(|| item.get("id"))
                .or_else(|| item.get("database"))
                .and_then(|value| value.as_str())?
                .trim()
                .to_string();
            if id.is_empty() {
                return None;
            }
            let year = item
                .get("year")
                .and_then(|value| value.as_i64())
                .map(|value| value as i32)
                .or_else(|| parse_year_from_identifier(&id));
            Some(CanLiiDatabase { id, year })
        })
        .collect()
}

fn extract_search_results(
    payload: &serde_json::Value,
    database_id: &str,
) -> Vec<serde_json::Value> {
    let Some(items) = payload
        .get("cases")
        .or_else(|| payload.get("results"))
        .or_else(|| payload.get("documents"))
        .and_then(|value| value.as_array())
    else {
        return Vec::new();
    };

    items
        .iter()
        .map(|item| {
            let case_id = string_field(item, &["caseId", "id", "documentId"]).unwrap_or_default();
            let title = string_field(item, &["title", "caseTitle", "name"]).unwrap_or_default();
            let citation = string_field(item, &["citation", "neutralCitation"]);
            let date = string_field(item, &["date", "decisionDate", "judgmentDate"]);
            let url = string_field(item, &["url", "absoluteUrl"])
                .or_else(|| {
                    string_field(item, &["path"]).map(|path| {
                        if path.starts_with("http://") || path.starts_with("https://") {
                            path
                        } else {
                            format!("https://www.canlii.org{}", path)
                        }
                    })
                })
                .unwrap_or_default();
            serde_json::json!({
                "case_id": case_id,
                "title": title,
                "citation": citation,
                "date": date,
                "database_id": database_id,
                "url": url,
            })
        })
        .collect()
}

fn string_field(value: &serde_json::Value, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        value
            .get(*key)
            .and_then(|entry| entry.as_str())
            .map(str::trim)
            .filter(|entry| !entry.is_empty())
            .map(|entry| entry.to_string())
    })
}

fn parse_year_from_identifier(value: &str) -> Option<i32> {
    let digits = value
        .chars()
        .rev()
        .take_while(|ch| ch.is_ascii_digit())
        .collect::<String>()
        .chars()
        .rev()
        .collect::<String>();
    if digits.len() == 4 {
        digits.parse::<i32>().ok()
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use axum::{Json, Router, extract::Path, routing::get};
    use serde_json::json;

    use super::*;

    #[test]
    fn schema_valid() {
        let tool = CanLiiSearchTool::new();
        let schema = tool.parameters_schema();
        assert!(schema["properties"]["query"].is_object());
        assert!(schema["properties"]["jurisdiction"].is_object());
        assert!(
            schema["required"]
                .as_array()
                .expect("required array")
                .contains(&serde_json::Value::String("query".to_string()))
        );
    }

    #[tokio::test]
    async fn missing_api_key_fails() {
        let tool = CanLiiSearchTool::new().with_base_url("http://127.0.0.1:9/v1");
        let tool = tool.without_env_api_key();
        let err = tool
            .execute(
                json!({
                    "query": "contract",
                    "jurisdiction": "onca"
                }),
                &JobContext::default(),
            )
            .await
            .expect_err("missing key should fail");
        assert!(matches!(err, ToolError::ExecutionFailed(_)));
    }

    #[tokio::test]
    async fn blocked_by_legal_policy() {
        let mut legal = crate::config::LegalConfig::resolve(&crate::settings::Settings::default())
            .expect("legal config");
        legal.network.allowed_domains.clear();
        let tool = CanLiiSearchTool::new()
            .with_legal_policy(legal)
            .with_api_key("test-key");
        let err = tool
            .execute(
                json!({
                    "query": "contract",
                    "jurisdiction": "onca"
                }),
                &JobContext::default(),
            )
            .await
            .expect_err("policy should block request");
        assert!(matches!(err, ToolError::NotAuthorized(_)));
    }

    #[tokio::test]
    async fn happy_path_returns_results() {
        async fn browse(
            Path((_lang, _jurisdiction)): Path<(String, String)>,
        ) -> Json<serde_json::Value> {
            Json(json!({
                "caseDatabases": [
                    { "databaseId": "onca2024" },
                    { "databaseId": "onca2023" }
                ]
            }))
        }

        async fn search(
            Path((_lang, database_id)): Path<(String, String)>,
        ) -> Json<serde_json::Value> {
            Json(json!({
                "results": [
                    {
                        "caseId": format!("{}-1", database_id),
                        "title": format!("{} result", database_id),
                        "citation": "2024 ONCA 123",
                        "date": "2024-03-01",
                        "url": "https://www.canlii.org/en/on/onca/doc/2024/2024onca123/2024onca123.html"
                    }
                ]
            }))
        }

        let app = Router::new()
            .route("/v1/caseBrowse/{lang}/{jurisdiction}/", get(browse))
            .route("/v1/caseSearch/{lang}/{database_id}/", get(search));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind listener");
        let addr = listener.local_addr().expect("local addr");
        tokio::spawn(async move {
            axum::serve(listener, app).await.expect("server");
        });

        let tool = CanLiiSearchTool::new()
            .with_base_url(format!("http://{addr}/v1"))
            .with_api_key("test-key");
        let output = tool
            .execute(
                json!({
                    "query": "contract",
                    "jurisdiction": "onca",
                    "max_results": 2
                }),
                &JobContext::default(),
            )
            .await
            .expect("search should succeed");

        assert_eq!(output.result["jurisdiction"], "onca");
        assert_eq!(
            output.result["results"].as_array().expect("results").len(),
            2
        );
    }

    #[tokio::test]
    async fn empty_browse_results_return_advisory_text() {
        async fn browse(
            Path((_lang, _jurisdiction)): Path<(String, String)>,
        ) -> Json<serde_json::Value> {
            Json(json!({
                "caseDatabases": []
            }))
        }

        let app = Router::new().route("/v1/caseBrowse/{lang}/{jurisdiction}/", get(browse));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind listener");
        let addr = listener.local_addr().expect("local addr");
        tokio::spawn(async move {
            axum::serve(listener, app).await.expect("server");
        });

        let tool = CanLiiSearchTool::new()
            .with_base_url(format!("http://{addr}/v1"))
            .with_api_key("test-key");
        let output = tool
            .execute(
                json!({
                    "query": "contract",
                    "jurisdiction": "onca"
                }),
                &JobContext::default(),
            )
            .await
            .expect("empty browse should succeed with advisory text");

        assert!(output.result.is_string());
        assert!(
            output
                .result
                .as_str()
                .expect("text result")
                .contains("No CanLII databases were returned")
        );
    }

    #[tokio::test]
    async fn searches_older_databases_when_newer_ones_are_empty() {
        async fn browse(
            Path((_lang, _jurisdiction)): Path<(String, String)>,
        ) -> Json<serde_json::Value> {
            Json(json!({
                "caseDatabases": [
                    { "databaseId": "onca2024" },
                    { "databaseId": "onca2023" },
                    { "databaseId": "onca2022" },
                    { "databaseId": "onca2021" }
                ]
            }))
        }

        async fn search(
            Path((_lang, database_id)): Path<(String, String)>,
        ) -> Json<serde_json::Value> {
            let results = if database_id == "onca2021" {
                vec![json!({
                    "caseId": "onca2021-1",
                    "title": "older result",
                    "citation": "2021 ONCA 321",
                    "date": "2021-06-01",
                    "url": "https://www.canlii.org/en/on/onca/doc/2021/2021onca321/2021onca321.html"
                })]
            } else {
                Vec::new()
            };
            Json(json!({ "results": results }))
        }

        let app = Router::new()
            .route("/v1/caseBrowse/{lang}/{jurisdiction}/", get(browse))
            .route("/v1/caseSearch/{lang}/{database_id}/", get(search));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind listener");
        let addr = listener.local_addr().expect("local addr");
        tokio::spawn(async move {
            axum::serve(listener, app).await.expect("server");
        });

        let tool = CanLiiSearchTool::new()
            .with_base_url(format!("http://{addr}/v1"))
            .with_api_key("test-key");
        let output = tool
            .execute(
                json!({
                    "query": "contract",
                    "jurisdiction": "onca",
                    "max_results": 1
                }),
                &JobContext::default(),
            )
            .await
            .expect("search should succeed");

        let results = output.result["results"].as_array().expect("results");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0]["database_id"], "onca2021");
    }
}
