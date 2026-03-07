use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

use async_trait::async_trait;
use chrono::Utc;
use regex::Regex;
use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::db::{CitationVerificationStatus, CreateCitationVerificationResultParams};

static REPORTER_CITATION_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?xi)
        \b(
            \d{1,4}\s+
            (?:
                U\.S\.|
                S\.?\s*Ct\.|
                L\.?\s*Ed\.?\s*2d|
                F\.?\s*(?:Supp\.?\s*\d*d?|App'?x|2d|3d)|
                Cal\.?\s*(?:App\.?\s*\d+(?:th|d)?|Rptr\.?\s*\d*d?)|
                N\.?\s*[EW]\.?\s*\d*d?|
                S\.?\s*[EW]\.?\s*\d*d?|
                P\.?\s*\d*d?|
                A\.?\s*\d*d?
            )
            \s+\d{1,5}
            (?:\s*\([^)]+\))?
        )",
    )
    .unwrap()
});

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractedCitation {
    pub citation_text: String,
    pub normalized_citation: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CitationWaiver {
    pub citation_text: String,
    pub waived_by: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CitationVerificationOutcome {
    pub status: CitationVerificationStatus,
    pub provider_reference: Option<String>,
    pub provider_title: Option<String>,
    pub detail: Option<String>,
}

#[async_trait]
pub trait CitationVerificationProvider: Send + Sync {
    fn provider_name(&self) -> &'static str;
    async fn verify(&self, citation: &ExtractedCitation) -> CitationVerificationOutcome;
}

#[derive(Debug, Clone)]
pub struct CourtListenerCitationProvider {
    client: reqwest::Client,
    api_token: Option<String>,
    network_allowed: bool,
}

impl CourtListenerCitationProvider {
    pub fn new(api_token: Option<String>, network_allowed: bool) -> Self {
        Self {
            client: reqwest::Client::new(),
            api_token,
            network_allowed,
        }
    }

    pub fn from_env(network_allowed: bool) -> Self {
        let api_token = std::env::var("COURTLISTENER_API_TOKEN")
            .ok()
            .or_else(|| std::env::var("COURTLISTENER_TOKEN").ok())
            .filter(|value| !value.trim().is_empty());
        Self::new(api_token, network_allowed)
    }
}

#[derive(Debug, Deserialize)]
struct CourtListenerSearchResponse {
    #[serde(default)]
    count: usize,
    #[serde(default)]
    results: Vec<CourtListenerSearchResult>,
}

#[derive(Debug, Deserialize)]
struct CourtListenerSearchResult {
    #[serde(default)]
    absolute_url: Option<String>,
    #[serde(default, rename = "caseName")]
    case_name_camel: Option<String>,
    #[serde(default)]
    case_name: Option<String>,
    #[serde(default)]
    cluster_id: Option<i64>,
}

#[async_trait]
impl CitationVerificationProvider for CourtListenerCitationProvider {
    fn provider_name(&self) -> &'static str {
        "courtlistener"
    }

    async fn verify(&self, citation: &ExtractedCitation) -> CitationVerificationOutcome {
        if !self.network_allowed {
            return CitationVerificationOutcome {
                status: CitationVerificationStatus::Unverified,
                provider_reference: None,
                provider_title: None,
                detail: Some(
                    "CourtListener is not allowlisted by legal network policy".to_string(),
                ),
            };
        }

        let Some(token) = self.api_token.as_deref() else {
            return CitationVerificationOutcome {
                status: CitationVerificationStatus::Unverified,
                provider_reference: None,
                provider_title: None,
                detail: Some("CourtListener API token is not configured".to_string()),
            };
        };

        let response = match self
            .client
            .get("https://www.courtlistener.com/api/rest/v4/search/")
            .query(&[("citation", citation.citation_text.as_str())])
            .header("Authorization", format!("Token {token}"))
            .send()
            .await
        {
            Ok(response) => response,
            Err(err) => {
                return CitationVerificationOutcome {
                    status: CitationVerificationStatus::Unverified,
                    provider_reference: None,
                    provider_title: None,
                    detail: Some(format!("CourtListener request failed: {err}")),
                };
            }
        };

        if !response.status().is_success() {
            return CitationVerificationOutcome {
                status: CitationVerificationStatus::Unverified,
                provider_reference: None,
                provider_title: None,
                detail: Some(format!(
                    "CourtListener returned HTTP {}",
                    response.status().as_u16()
                )),
            };
        }

        let payload = match response.json::<CourtListenerSearchResponse>().await {
            Ok(payload) => payload,
            Err(err) => {
                return CitationVerificationOutcome {
                    status: CitationVerificationStatus::Unverified,
                    provider_reference: None,
                    provider_title: None,
                    detail: Some(format!("CourtListener response parse failed: {err}")),
                };
            }
        };

        match payload.count {
            0 => CitationVerificationOutcome {
                status: CitationVerificationStatus::Unverified,
                provider_reference: None,
                provider_title: None,
                detail: Some("No matching authority found in CourtListener".to_string()),
            },
            1 => {
                let result = payload.results.into_iter().next();
                CitationVerificationOutcome {
                    status: CitationVerificationStatus::Verified,
                    provider_reference: result
                        .as_ref()
                        .and_then(|value| value.absolute_url.clone())
                        .or_else(|| {
                            result
                                .as_ref()
                                .and_then(|value| value.cluster_id.map(|id| id.to_string()))
                        }),
                    provider_title: result.and_then(|value| {
                        value
                            .case_name_camel
                            .or(value.case_name)
                            .filter(|name| !name.trim().is_empty())
                    }),
                    detail: None,
                }
            }
            count => CitationVerificationOutcome {
                status: CitationVerificationStatus::Ambiguous,
                provider_reference: None,
                provider_title: None,
                detail: Some(format!(
                    "CourtListener returned {count} possible matches for this citation"
                )),
            },
        }
    }
}

pub fn normalize_citation(value: &str) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase()
}

pub fn document_hash(content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    format!("{:x}", hasher.finalize())
}

pub fn extract_citations(content: &str) -> Vec<ExtractedCitation> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for capture in REPORTER_CITATION_RE.captures_iter(content) {
        let Some(matched) = capture.get(1) else {
            continue;
        };
        let citation_text = matched.as_str().trim().to_string();
        if citation_text.is_empty() {
            continue;
        }
        let normalized_citation = normalize_citation(&citation_text);
        if seen.insert(normalized_citation.clone()) {
            out.push(ExtractedCitation {
                citation_text,
                normalized_citation,
            });
        }
    }
    out
}

pub async fn verify_document_with_provider<P: CitationVerificationProvider>(
    provider: &P,
    content: &str,
    waivers: &[CitationWaiver],
) -> Vec<CreateCitationVerificationResultParams> {
    let waiver_map: HashMap<String, &CitationWaiver> = waivers
        .iter()
        .map(|waiver| (normalize_citation(&waiver.citation_text), waiver))
        .collect();

    let mut results = Vec::new();
    for citation in extract_citations(content) {
        let mut outcome = provider.verify(&citation).await;
        if !matches!(outcome.status, CitationVerificationStatus::Verified)
            && let Some(waiver) = waiver_map.get(&citation.normalized_citation)
        {
            outcome.status = CitationVerificationStatus::Waived;
            outcome.detail = outcome
                .detail
                .or_else(|| Some("Attorney waiver applied".to_string()));
            results.push(CreateCitationVerificationResultParams {
                citation_text: citation.citation_text.clone(),
                normalized_citation: citation.normalized_citation.clone(),
                status: CitationVerificationStatus::Waived,
                provider_reference: outcome.provider_reference,
                provider_title: outcome.provider_title,
                detail: outcome.detail,
                waived_by: Some(waiver.waived_by.clone()),
                waiver_reason: Some(waiver.reason.clone()),
                waived_at: Some(Utc::now()),
            });
            continue;
        }

        results.push(CreateCitationVerificationResultParams {
            citation_text: citation.citation_text,
            normalized_citation: citation.normalized_citation,
            status: outcome.status,
            provider_reference: outcome.provider_reference,
            provider_title: outcome.provider_title,
            detail: outcome.detail,
            waived_by: None,
            waiver_reason: None,
            waived_at: None,
        });
    }
    results
}

#[cfg(test)]
mod tests {
    use super::*;

    struct MockProvider;

    #[async_trait]
    impl CitationVerificationProvider for MockProvider {
        fn provider_name(&self) -> &'static str {
            "mock"
        }

        async fn verify(&self, citation: &ExtractedCitation) -> CitationVerificationOutcome {
            if citation.normalized_citation.contains("410 u.s. 113") {
                CitationVerificationOutcome {
                    status: CitationVerificationStatus::Verified,
                    provider_reference: Some("https://example.test/roe".to_string()),
                    provider_title: Some("Roe v. Wade".to_string()),
                    detail: None,
                }
            } else {
                CitationVerificationOutcome {
                    status: CitationVerificationStatus::Unverified,
                    provider_reference: None,
                    provider_title: None,
                    detail: Some("missing".to_string()),
                }
            }
        }
    }

    #[test]
    fn extracts_reporter_style_citations() {
        let citations = extract_citations(
            "The memo cites Roe v. Wade, 410 U.S. 113 (1973) and Brown, 347 U.S. 483.",
        );
        assert_eq!(citations.len(), 2);
        assert_eq!(citations[0].normalized_citation, "410 u.s. 113 (1973)");
        assert_eq!(citations[1].normalized_citation, "347 u.s. 483");
    }

    #[tokio::test]
    async fn verification_applies_attorney_waivers() {
        let results = verify_document_with_provider(
            &MockProvider,
            "Roe, 410 U.S. 113 (1973). Fake cite 999 U.S. 999.",
            &[CitationWaiver {
                citation_text: "999 U.S. 999".to_string(),
                waived_by: "Attorney".to_string(),
                reason: "Pending manual verification".to_string(),
            }],
        )
        .await;

        assert_eq!(results.len(), 2);
        assert!(matches!(
            results[0].status,
            CitationVerificationStatus::Verified
        ));
        assert!(matches!(
            results[1].status,
            CitationVerificationStatus::Waived
        ));
        assert_eq!(results[1].waived_by.as_deref(), Some("Attorney"));
    }
}
