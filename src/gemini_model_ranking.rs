// MIT License
// Copyright (c) 2026 Cedric Gegout

//! # Gemini Model Ranking Strategy
//!
//! This module is the single source of truth for all Gemini model selection decisions.
//!
//! ## Why this module exists
//!
//! Hardcoded fallback model names are fragile. Model names in the Gemini API change over time:
//! models are deprecated, new families are introduced, and naming conventions shift. Any code
//! that relies on a hardcoded string like `"gemini-1.5-flash"` will silently break the moment
//! Google retires that model or renames it in the API.
//!
//! Instead, this module fetches the live list of available models from the Gemini API on demand
//! and applies a deterministic ranking algorithm to pick the best available model for our use case
//! (text summarization using `generateContent`).
//!
//! ## Why ranking is centralized here
//!
//! Both `run` mode (batch digest generation) and `bot` mode (interactive Telegram bot) need to
//! call Gemini. If each call site had its own model selection logic, any inconsistency would lead
//! to bugs that are hard to track down (e.g., `bot` mode tries a different set of fallback models
//! than `run` mode). Centralizing here guarantees consistent behavior everywhere.
//!
//! ## Selection algorithm summary
//!
//! 1. Fetch all models from the Gemini API.
//! 2. Keep only models that support `generateContent` (the method we call).
//! 3. Exclude non-text-purpose models (TTS, image, audio, live, embeddings).
//! 4. Score remaining models: prefer newer/more capable families, penalize preview/experimental.
//! 5. Respect the configured preferred model (bump its score if present).
//! 6. Exclude models already tried in the current request.
//! 7. Return the highest-scored remaining model.

use regex::Regex;
use reqwest::Client;
use serde::{Deserialize, Serialize};

use crate::config::Config;
use crate::error::AppError;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// Information about a single Gemini model, as returned by the models listing API.
///
/// # Serde robustness
/// Many fields are optional in the actual API response. The API does not guarantee
/// that all metadata fields are present for every model. Using `Option<T>` and
/// `#[serde(default)]` prevents serde from failing to parse the *entire* list if
/// one model entry is missing a non-critical field.
#[derive(Debug, Deserialize, Serialize, Clone, PartialEq)]
pub struct GeminiModelInfo {
    /// Full resource name returned by the API, e.g. `"models/gemini-2.0-flash"`.
    /// This is what must be used in the `generateContent` URL path.
    pub name: String,

    /// Human-readable display name. May be absent for some models.
    #[serde(rename = "displayName", default)]
    pub display_name: String,

    /// Short description of the model. Often absent — must be `Option` to avoid
    /// causing serde to fail the entire models list parse.
    #[serde(default)]
    pub description: Option<String>,

    /// The set of API generation methods this model supports, e.g. `["generateContent", "countTokens"]`.
    /// This is the critical field: we must verify `generateContent` is listed before using a model
    /// for text summarization, because some models (e.g. embedding models) do not support it.
    #[serde(rename = "supportedGenerationMethods", default)]
    pub supported_generation_methods: Vec<String>,
}

impl GeminiModelInfo {
    /// Returns the short model name (without the `models/` prefix).
    /// E.g. `"models/gemini-2.0-flash"` → `"gemini-2.0-flash"`.
    pub fn short_name(&self) -> &str {
        self.name.split('/').last().unwrap_or(&self.name)
    }

    /// Returns true if this model supports `generateContent`.
    pub fn supports_generate_content(&self) -> bool {
        self.supported_generation_methods
            .iter()
            .any(|m| m == "generateContent")
    }
}

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Model name substrings that indicate the model is not suitable for text summarization.
/// These are excluded from the candidate list regardless of what the API returns.
///
/// Why: Some Gemini models are specialized for TTS, image generation, audio, or live
/// streaming. Sending a text summarization prompt to such a model would either fail or
/// return garbage. We exclude them proactively.
const EXCLUDED_NAME_PATTERNS: &[&str] = &[
    "tts",
    "image",
    "live",
    "audio",
    "vision",
    "embed",
    "aqa",
];

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Fetches all available Gemini models from the live API.
///
/// Uses `pageSize=200` to get all models in a single request and avoid pagination.
/// The default page size is 50, which may miss newer models that appear later in the list.
///
/// # Arguments
/// * `api_key` — the Gemini API key (not logged)
pub async fn list_available_models(api_key: &str) -> Result<Vec<GeminiModelInfo>, AppError> {
    tracing::info!("Fetching list of available Gemini models from API...");

    // pageSize=200 to retrieve all models without needing pagination.
    // The Gemini API default is 50 which can miss fallback candidates.
    let url = format!(
        "https://generativelanguage.googleapis.com/v1beta/models?pageSize=200&key={}",
        api_key
    );

    let client = Client::new();
    let response = client.get(&url).send().await.map_err(|e| {
        AppError::Io(std::io::Error::new(
            std::io::ErrorKind::Other,
            format!("Failed to reach Gemini models endpoint: {}", e),
        ))
    })?;

    let status = response.status();
    // Read body as raw bytes so we can log it on parse failure without consuming the response twice.
    let body_bytes = response.bytes().await.map_err(|e| {
        AppError::Io(std::io::Error::new(
            std::io::ErrorKind::Other,
            format!("Failed to read Gemini models response body: {}", e),
        ))
    })?;

    if !status.is_success() {
        let body_text = String::from_utf8_lossy(&body_bytes);
        tracing::error!(
            "Gemini models list API returned HTTP {}: {}",
            status,
            body_text
        );
        return Err(AppError::Io(std::io::Error::new(
            std::io::ErrorKind::Other,
            format!("Gemini models list error: HTTP {}", status),
        )));
    }

    tracing::debug!(
        "Gemini models list raw response: {} bytes received",
        body_bytes.len()
    );

    // Local struct for deserialization only — not exposed publicly.
    #[derive(Deserialize)]
    struct ModelsResponse {
        #[serde(default)]
        models: Vec<GeminiModelInfo>,
    }

    let data: ModelsResponse = serde_json::from_slice(&body_bytes).map_err(|e| {
        // Log a snippet of the body to help diagnose the error without leaking huge payloads.
        let snippet = String::from_utf8_lossy(&body_bytes[..body_bytes.len().min(500)]);
        tracing::error!(
            "Failed to parse Gemini models JSON: {}. Response snippet: {}",
            e,
            snippet
        );
        AppError::Io(std::io::Error::new(
            std::io::ErrorKind::Other,
            format!("Failed to parse Gemini models JSON: {}", e),
        ))
    })?;

    tracing::info!(
        "Fetched {} Gemini models from API",
        data.models.len()
    );
    Ok(data.models)
}

/// Filters the model list to only those that support `generateContent`.
///
/// # Why this filter is necessary
/// The Gemini API lists all models, including those that only support embedding or
/// other methods. Sending a `generateContent` request to an embedding-only model will
/// return a 404 or "unsupported method" error. We must verify capability before selection.
pub fn filter_generate_content_models(models: &[GeminiModelInfo]) -> Vec<GeminiModelInfo> {
    let before = models.len();
    let filtered: Vec<GeminiModelInfo> = models
        .iter()
        .filter(|m| {
            let ok = m.supports_generate_content();
            if !ok {
                tracing::debug!(
                    "Excluded model '{}': does not support generateContent (methods: {:?})",
                    m.short_name(),
                    m.supported_generation_methods
                );
            }
            ok
        })
        .cloned()
        .collect();

    tracing::info!(
        "Filtered down to {} models supporting generateContent (from {})",
        filtered.len(),
        before
    );
    filtered
}

/// Computes a capability/priority score for a model based on its name.
///
/// Higher score = more capable / preferred for text summarization.
/// Lower score = cheaper / less capable / less preferred.
///
/// # Scoring rationale
/// - We prefer newer generation families (higher version = higher score).
/// - Within a generation, we prefer pro > flash > flash-lite.
/// - We penalise preview and experimental models (prefer stable GA models).
/// - We exclude models whose names suggest they are not for text summarization.
///
/// This is a heuristic. It does not rely on hardcoded model names; it relies on naming
/// *patterns* that Google has consistently used across generations.
fn score_model(short_name: &str) -> Option<i32> {
    let lower = short_name.to_lowercase();

    // Exclude non-text-purpose models entirely.
    // Why: sending a text prompt to a TTS or image model wastes API quota and fails.
    for pattern in EXCLUDED_NAME_PATTERNS {
        if lower.contains(pattern) {
            tracing::debug!(
                "Excluded model '{}': name matches excluded pattern '{}'",
                short_name,
                pattern
            );
            return None; // None signals "exclude this model"
        }
    }

    let mut score: i32 = 0;

    // Generation family score — dynamically extracted from the model name.
    // We look for a pattern like "X.Y" (e.g. "2.5", "3.0", "4.1") and compute
    // score = major * 100 + minor * 10. This makes the scoring future-proof:
    // any new generation will automatically score higher than older ones.
    let version_re = Regex::new(r"(\d+)\.(\d+)").expect("version regex must compile");
    if let Some(caps) = version_re.captures(&lower) {
        let major: i32 = caps[1].parse().unwrap_or(0);
        let minor: i32 = caps[2].parse().unwrap_or(0);
        score += major * 100 + minor * 10;
        tracing::debug!("Model '{}' version detected: {}.{}", short_name, major, minor);
    }
    // Models without a version number get 0 (catch-all, deprioritised)

    // Capability tier within a generation — pro > flash > flash-lite
    if lower.contains("pro") {
        score += 30;
    } else if lower.contains("flash") && !lower.contains("flash-lite") && !lower.contains("flash-8b") {
        score += 20;
    } else if lower.contains("flash-lite") || lower.contains("flash-8b") {
        score += 10;
    }

    // Stability bonus — prefer non-preview, non-experimental models
    // Why: preview models may be removed or changed without notice.
    if lower.contains("preview") || lower.contains("experimental") || lower.contains("exp") {
        score -= 15;
    }

    tracing::debug!("Model '{}' scored {}", short_name, score);
    Some(score)
}

/// Ranks a list of candidate models from most capable (index 0) to least capable.
///
/// Models with no valid score (i.e. excluded patterns) are dropped from the result.
///
/// The ranking is stable: models with equal score preserve their original API order,
/// which means we get deterministic, reproducible selections.
pub fn rank_models(models: &[GeminiModelInfo], preferred_models: &[String]) -> Vec<GeminiModelInfo> {
    let mut scored: Vec<(Option<usize>, i32, &GeminiModelInfo)> = models
        .iter()
        .filter_map(|m| {
            score_model(m.short_name()).map(|s| {
                let lower_short = m.short_name().to_lowercase();
                let lower_full = m.name.to_lowercase();
                let pref_idx = preferred_models.iter().position(|p| {
                    let p_lower = p.to_lowercase();
                    p_lower == lower_short || p_lower == lower_full
                });
                (pref_idx, s, m)
            })
        })
        .collect();

    // Sort:
    // 1. If a has Some(idx) and b has None: a comes first (a < b)
    // 2. If b has Some(idx) and a has None: b comes first (b < a)
    // 3. If both have Some(idx): the one with smaller index comes first
    // 4. If both have None: the one with higher score comes first
    scored.sort_by(|a, b| {
        match (a.0, b.0) {
            (Some(idx_a), Some(idx_b)) => idx_a.cmp(&idx_b),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => b.1.cmp(&a.1),
        }
    });

    let ranked: Vec<GeminiModelInfo> = scored.into_iter().map(|(_, _, m)| m.clone()).collect();

    if ranked.is_empty() {
        tracing::warn!("No models remain after ranking — all were excluded");
    } else {
        let names: Vec<&str> = ranked.iter().map(|m| m.short_name()).collect();
        tracing::info!("Ranked candidate order: {}", names.join(", "));
    }

    ranked
}

/// Selects the best available model from a pre-ranked list.
///
/// # Arguments
/// * `ranked_models` — models already ranked from most to least capable (output of `rank_models`)
/// * `failed_model` — the model that just failed (will be excluded from selection)
/// * `already_tried` — all models tried in this request (also excluded)
/// * `preferred_model` — if `Some`, this model is bumped to first position if present in the list
///
/// # Returns
/// `Some(GeminiModelInfo)` for the chosen model, or `None` if all candidates are exhausted.
///
/// # Why `failed_model` is a parameter
/// The caller knows which model just failed. Passing it here keeps the selection function
/// stateless and testable: given the same inputs, you always get the same output.
pub fn select_best_model(
    ranked_models: &[GeminiModelInfo],
    failed_model: Option<&str>,
    already_tried: &[String],
    preferred_model: Option<&str>,
) -> Option<GeminiModelInfo> {
    if let Some(fm) = failed_model {
        tracing::info!("Model selection: failed model received: '{}'", fm);
    } else {
        tracing::info!("Model selection: no failed model provided, selecting highest-capability model");
    }

    // Build the set of model names to exclude (normalised to short name and full name)
    let excluded: Vec<String> = already_tried
        .iter()
        .map(|s| {
            let short = s.split('/').last().unwrap_or(s);
            short.to_lowercase()
        })
        .chain(
            failed_model
                .iter()
                .map(|s| s.split('/').last().unwrap_or(s).to_lowercase()),
        )
        .collect();

    if !excluded.is_empty() {
        tracing::debug!("Excluding models from selection: {:?}", excluded);
    }

    // If a preferred model is configured and present in the ranked list (and not excluded),
    // promote it to the front of our candidate search. This respects user preference while
    // still allowing automatic fallback when the preferred model is unavailable.
    let mut candidates: Vec<&GeminiModelInfo> = ranked_models.iter().collect();
    if let Some(pref) = preferred_model {
        let pref_short = pref.split('/').last().unwrap_or(pref).to_lowercase();
        // Move preferred model to front if it exists and is not excluded
        if let Some(pos) = candidates
            .iter()
            .position(|m| m.short_name().to_lowercase() == pref_short)
        {
            if !excluded.contains(&pref_short) {
                let preferred = candidates.remove(pos);
                candidates.insert(0, preferred);
                tracing::debug!(
                    "Promoted preferred model '{}' to front of candidate list",
                    pref_short
                );
            }
        }
    }

    // Select the first model not in the excluded set
    for model in &candidates {
        let short = model.short_name().to_lowercase();
        if excluded.contains(&short) {
            tracing::debug!(
                "Skipping already-tried or failed model: '{}'",
                model.short_name()
            );
            continue;
        }
        tracing::info!(
            "Selected model: '{}' (full name: '{}')",
            model.short_name(),
            model.name
        );
        return Some((*model).clone());
    }

    tracing::warn!(
        "No eligible Gemini model remains after exclusions {:?}",
        excluded
    );
    None
}

/// End-to-end model selection: fetches models, filters, ranks, and selects.
///
/// This is the primary entry point called by `call_gemini_with_fallbacks` whenever
/// it needs a (possibly fallback) model. It is stateless and reusable from any context.
///
/// # Arguments
/// * `config` — application config (provides API key and preferred model name)
/// * `failed_model` — the model name that just failed (`None` for the first selection)
/// * `already_tried` — all models already tried in this request cycle
///
/// # Returns
/// `Ok(GeminiModelInfo)` for the best available model, or `Err` if discovery fails or
/// no suitable models exist.
pub async fn select_model_for_request(
    config: &Config,
    failed_model: Option<&str>,
    already_tried: &[String],
) -> Result<GeminiModelInfo, AppError> {
    tracing::info!("Starting Gemini model selection (already tried: {:?})", already_tried);

    let all_models = list_available_models(&config.gemini.api_key).await?;
    let content_models = filter_generate_content_models(&all_models);

    if content_models.is_empty() {
        return Err(AppError::Io(std::io::Error::new(
            std::io::ErrorKind::Other,
            "No Gemini models supporting generateContent are available",
        )));
    }

    let ranked = rank_models(&content_models, &config.gemini.preferred_models);

    select_best_model(
        &ranked,
        failed_model,
        already_tried,
        None,
    )
    .ok_or_else(|| {
        AppError::Io(std::io::Error::new(
            std::io::ErrorKind::Other,
            format!(
                "All Gemini models exhausted. Already tried: {:?}",
                already_tried
            ),
        ))
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn make_model(name: &str, methods: &[&str]) -> GeminiModelInfo {
        GeminiModelInfo {
            name: format!("models/{}", name),
            display_name: name.to_string(),
            description: None,
            supported_generation_methods: methods.iter().map(|s| s.to_string()).collect(),
        }
    }

    // -----------------------------------------------------------------------
    // filter_generate_content_models
    // -----------------------------------------------------------------------

    #[test]
    fn test_filter_keeps_only_generate_content_models() {
        let models = vec![
            make_model("gemini-2.0-flash", &["generateContent", "countTokens"]),
            make_model("text-embedding-004", &["embedContent"]),
            make_model("gemini-1.5-pro", &["generateContent"]),
        ];
        let filtered = filter_generate_content_models(&models);
        assert_eq!(filtered.len(), 2);
        assert!(filtered.iter().all(|m| m.supports_generate_content()));
        assert!(!filtered.iter().any(|m| m.name.contains("embedding")));
    }

    #[test]
    fn test_filter_empty_input() {
        let filtered = filter_generate_content_models(&[]);
        assert!(filtered.is_empty());
    }

    // -----------------------------------------------------------------------
    // rank_models
    // -----------------------------------------------------------------------

    #[test]
    fn test_ranking_order_capability() {
        let models = vec![
            make_model("gemini-1.5-flash", &["generateContent"]),
            make_model("gemini-2.5-pro", &["generateContent"]),
            make_model("gemini-2.0-flash", &["generateContent"]),
        ];
        let ranked = rank_models(&models, &[]);
        // 2.5-pro should come first (250+30=280), then 2.0-flash (200+20=220), then 1.5-flash (150+20=170)
        assert_eq!(ranked[0].short_name(), "gemini-2.5-pro");
        assert_eq!(ranked[1].short_name(), "gemini-2.0-flash");
        assert_eq!(ranked[2].short_name(), "gemini-1.5-flash");
    }

    #[test]
    fn test_ranking_excludes_tts_models() {
        let models = vec![
            make_model("gemini-2.0-flash", &["generateContent"]),
            make_model("gemini-2.0-tts", &["generateContent"]),
        ];
        let ranked = rank_models(&models, &[]);
        assert_eq!(ranked.len(), 1);
        assert_eq!(ranked[0].short_name(), "gemini-2.0-flash");
    }

    #[test]
    fn test_ranking_excludes_image_models() {
        let models = vec![
            make_model("gemini-2.0-flash", &["generateContent"]),
            make_model("imagen-3.0-generate-001", &["generateContent"]),
        ];
        let ranked = rank_models(&models, &[]);
        assert_eq!(ranked.len(), 1);
        assert_eq!(ranked[0].short_name(), "gemini-2.0-flash");
    }

    #[test]
    fn test_ranking_penalises_preview() {
        let models = vec![
            make_model("gemini-2.5-flash-preview", &["generateContent"]),
            make_model("gemini-2.0-flash", &["generateContent"]),
        ];
        let ranked = rank_models(&models, &[]);
        // 2.5-flash-preview: 250+20-15 = 255; 2.0-flash: 200+20 = 220 → preview should win
        // but let's just verify both are present and preview is first (255 > 220)
        assert_eq!(ranked.len(), 2);
        assert_eq!(ranked[0].short_name(), "gemini-2.5-flash-preview");
    }

    #[test]
    fn test_ranking_stable_beats_older_preview() {
        // A stable 2.0-flash vs a 1.5-pro-preview
        let models = vec![
            make_model("gemini-1.5-pro-preview", &["generateContent"]),
            make_model("gemini-2.0-flash", &["generateContent"]),
        ];
        let ranked = rank_models(&models, &[]);
        // 2.0-flash: 200+20+0 = 220; 1.5-pro-preview: 150+30-15 = 165
        assert_eq!(ranked[0].short_name(), "gemini-2.0-flash");
        assert_eq!(ranked[1].short_name(), "gemini-1.5-pro-preview");
    }

    #[test]
    fn test_ranking_with_preferred_models() {
        let models = vec![
            make_model("gemini-1.5-flash", &["generateContent"]),
            make_model("gemini-2.5-pro", &["generateContent"]),
            make_model("gemini-2.0-flash", &["generateContent"]),
            make_model("gemini-3.5-flash", &["generateContent"]),
        ];
        let preferred = vec![
            "gemini-3.5-flash".to_string(),
            "gemini-2.5-pro".to_string(),
        ];
        let ranked = rank_models(&models, &preferred);
        // gemini-3.5-flash and gemini-2.5-pro should come first in that exact order,
        // then the rest of the models ordered by capability score (2.0-flash > 1.5-flash)
        assert_eq!(ranked[0].short_name(), "gemini-3.5-flash");
        assert_eq!(ranked[1].short_name(), "gemini-2.5-pro");
        assert_eq!(ranked[2].short_name(), "gemini-2.0-flash");
        assert_eq!(ranked[3].short_name(), "gemini-1.5-flash");
    }

    // -----------------------------------------------------------------------
    // select_best_model
    // -----------------------------------------------------------------------

    #[test]
    fn test_select_initial_returns_first() {
        let ranked = vec![
            make_model("gemini-2.5-pro", &["generateContent"]),
            make_model("gemini-2.0-flash", &["generateContent"]),
        ];
        let selected = select_best_model(&ranked, None, &[], None);
        assert!(selected.is_some());
        assert_eq!(selected.unwrap().short_name(), "gemini-2.5-pro");
    }

    #[test]
    fn test_select_excludes_failed_model() {
        let ranked = vec![
            make_model("gemini-2.5-pro", &["generateContent"]),
            make_model("gemini-2.0-flash", &["generateContent"]),
        ];
        let selected = select_best_model(&ranked, Some("gemini-2.5-pro"), &[], None);
        assert!(selected.is_some());
        assert_eq!(selected.unwrap().short_name(), "gemini-2.0-flash");
    }

    #[test]
    fn test_select_excludes_already_tried() {
        let ranked = vec![
            make_model("gemini-2.5-pro", &["generateContent"]),
            make_model("gemini-2.0-flash", &["generateContent"]),
            make_model("gemini-1.5-flash", &["generateContent"]),
        ];
        let already = vec!["gemini-2.5-pro".to_string(), "gemini-2.0-flash".to_string()];
        let selected = select_best_model(&ranked, None, &already, None);
        assert!(selected.is_some());
        assert_eq!(selected.unwrap().short_name(), "gemini-1.5-flash");
    }

    #[test]
    fn test_select_returns_none_when_all_exhausted() {
        let ranked = vec![make_model("gemini-2.0-flash", &["generateContent"])];
        let already = vec!["gemini-2.0-flash".to_string()];
        let selected = select_best_model(&ranked, None, &already, None);
        assert!(selected.is_none());
    }

    #[test]
    fn test_select_promotes_preferred_model() {
        let ranked = vec![
            make_model("gemini-2.5-pro", &["generateContent"]),
            make_model("gemini-2.0-flash", &["generateContent"]),
        ];
        // User prefers gemini-2.0-flash — it should be returned first even if 2.5-pro is ranked higher
        let selected = select_best_model(&ranked, None, &[], Some("gemini-2.0-flash"));
        assert!(selected.is_some());
        assert_eq!(selected.unwrap().short_name(), "gemini-2.0-flash");
    }

    #[test]
    fn test_select_falls_through_preferred_when_excluded() {
        let ranked = vec![
            make_model("gemini-2.5-pro", &["generateContent"]),
            make_model("gemini-2.0-flash", &["generateContent"]),
        ];
        // Preferred model is already tried, so fall through to next
        let already = vec!["gemini-2.0-flash".to_string()];
        let selected = select_best_model(&ranked, None, &already, Some("gemini-2.0-flash"));
        assert!(selected.is_some());
        assert_eq!(selected.unwrap().short_name(), "gemini-2.5-pro");
    }

    #[test]
    fn test_select_handles_models_prefix() {
        // already_tried may contain full names like "models/gemini-2.0-flash"
        let ranked = vec![
            make_model("gemini-2.5-pro", &["generateContent"]),
            make_model("gemini-2.0-flash", &["generateContent"]),
        ];
        let already = vec!["models/gemini-2.5-pro".to_string()];
        let selected = select_best_model(&ranked, None, &already, None);
        assert!(selected.is_some());
        assert_eq!(selected.unwrap().short_name(), "gemini-2.0-flash");
    }
}
