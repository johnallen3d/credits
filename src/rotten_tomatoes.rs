//! Rotten Tomatoes Tomatometer scores for titles OMDb has no score for:
//! resolves a title's Rotten Tomatoes path from Wikidata's `P1258` claim
//! (using TMDB's `wikidata_id`) with OMDb's `tomatoURL` as fallback, then
//! reads the Tomatometer from the page's embedded JSON. Everything is cached
//! in SQLite and failures never break a page.

use std::time::Duration;

use jiff::Timestamp;
use reqwest::StatusCode;
use serde::Deserialize;

use crate::cache::ProviderCache;
use crate::model::{Rating, RatingSource};

const WIKIDATA_PROVIDER: &str = "wikidata";
const RT_PROVIDER: &str = "rotten_tomatoes";

#[derive(Clone)]
pub struct RottenTomatoes {
    http: reqwest::Client,
    wikidata_base: String,
    rt_base: String,
    cache: ProviderCache,
    ttl: Duration,
}

impl RottenTomatoes {
    pub fn new(wikidata_base: &str, rt_base: &str, cache: ProviderCache, ttl: Duration) -> Self {
        let http = reqwest::Client::builder()
            // Ratings are optional; don't hold the page hostage to a slow provider.
            .timeout(Duration::from_secs(4))
            .user_agent(concat!("credits/", env!("CARGO_PKG_VERSION")))
            .build()
            .expect("reqwest client builds with static settings");
        Self {
            http,
            wikidata_base: wikidata_base.trim_end_matches('/').to_owned(),
            rt_base: rt_base.trim_end_matches('/').to_owned(),
            cache,
            ttl,
        }
    }

    /// Looks up a title's Tomatometer score using its Wikidata item (`P1258`)
    /// first, then `fallback_path` (e.g. from OMDb's `tomatoURL`).
    pub async fn score(
        &self,
        wikidata_id: Option<&str>,
        fallback_path: Option<&str>,
    ) -> Option<Rating> {
        let mut paths = Vec::new();
        if let Some(qid) = wikidata_id.filter(|id| is_wikidata_id(id))
            && let Some(path) = self.wikidata_rt_path(qid).await
        {
            paths.push(path);
        }
        if let Some(path) = fallback_path.and_then(normalize_rt_path)
            && !paths.contains(&path)
        {
            paths.push(path);
        }
        for path in paths {
            if let Some(rating) = self.tomatometer(&path).await {
                return Some(rating);
            }
        }
        None
    }

    /// Cached lookup of a Wikidata entity's Rotten Tomatoes ID (`P1258`).
    async fn wikidata_rt_path(&self, qid: &str) -> Option<String> {
        let key = format!("{qid}/P1258");
        let cached = self
            .cache
            .get::<Option<String>>(WIKIDATA_PROVIDER, &key)
            .await
            .unwrap_or_else(|e| {
                tracing::warn!(error = %e, key, "wikidata cache read failed");
                None
            });
        let now = Timestamp::now();
        if let Some(c) = &cached
            && c.is_fresh(now)
        {
            return c.value.clone();
        }

        match self.fetch_wikidata_rt_path(qid).await {
            Ok(path) => {
                if let Err(e) = self
                    .cache
                    .put(WIKIDATA_PROVIDER, &key, &path, now, self.ttl)
                    .await
                {
                    tracing::warn!(error = %e, key, "wikidata cache write failed");
                }
                path
            }
            Err(e) => {
                tracing::warn!(error = %e, qid, "Wikidata P1258 lookup failed");
                cached.and_then(|c| c.value)
            }
        }
    }

    async fn fetch_wikidata_rt_path(&self, qid: &str) -> Result<Option<String>, String> {
        let response = self
            .http
            .get(format!("{}/w/api.php", self.wikidata_base))
            .query(&[
                ("action", "wbgetclaims"),
                ("entity", qid),
                ("property", "P1258"),
                ("format", "json"),
            ])
            .send()
            .await
            .map_err(|e| format!("request to Wikidata failed: {e}"))?;
        let status = response.status();
        if !status.is_success() {
            return Err(format!("Wikidata returned {status}"));
        }
        let body: ClaimsResponse = response
            .json()
            .await
            .map_err(|e| format!("unexpected Wikidata response: {e}"))?;
        Ok(pick_p1258_path(&body.claims.p1258))
    }

    /// Cached lookup of a Rotten Tomatoes page's Tomatometer score.
    async fn tomatometer(&self, path: &str) -> Option<Rating> {
        let cached = self
            .cache
            .get::<Option<f64>>(RT_PROVIDER, path)
            .await
            .unwrap_or_else(|e| {
                tracing::warn!(error = %e, path, "Rotten Tomatoes cache read failed");
                None
            });
        let now = Timestamp::now();
        if let Some(c) = &cached
            && c.is_fresh(now)
        {
            return c.value.map(rt_rating);
        }

        match self.fetch_tomatometer(path).await {
            Ok(score) => {
                if let Err(e) = self
                    .cache
                    .put(RT_PROVIDER, path, &score, now, self.ttl)
                    .await
                {
                    tracing::warn!(error = %e, path, "Rotten Tomatoes cache write failed");
                }
                score.map(rt_rating)
            }
            Err(e) => {
                tracing::warn!(error = %e, path, "Rotten Tomatoes lookup failed");
                cached.and_then(|c| c.value).map(rt_rating)
            }
        }
    }

    async fn fetch_tomatometer(&self, path: &str) -> Result<Option<f64>, String> {
        let response = self
            .http
            .get(format!("{}/{path}", self.rt_base))
            .send()
            .await
            .map_err(|e| format!("request to Rotten Tomatoes failed: {e}"))?;
        let status = response.status();
        if matches!(status, StatusCode::NOT_FOUND | StatusCode::GONE) {
            return Ok(None);
        }
        if !status.is_success() {
            return Err(format!("Rotten Tomatoes returned {status}"));
        }
        let html = response
            .text()
            .await
            .map_err(|e| format!("reading Rotten Tomatoes page failed: {e}"))?;
        Ok(parse_tomatometer(&html))
    }
}

fn rt_rating(score: f64) -> Rating {
    Rating {
        source: RatingSource::RottenTomatoes,
        score,
        scale: 100,
        votes: None,
    }
}

/// Wikidata item identifiers look like `Q1747211`.
pub fn is_wikidata_id(id: &str) -> bool {
    id.strip_prefix('Q')
        .is_some_and(|digits| !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()))
}

/// Extracts a normalized `m/...` or `tv/...` path from a Rotten Tomatoes URL
/// such as OMDb's `tomatoURL`.
pub fn normalize_rt_url(url: &str) -> Option<String> {
    let trimmed = url.trim();
    let without_scheme = trimmed
        .strip_prefix("https://")
        .or_else(|| trimmed.strip_prefix("http://"))?;
    let (host, rest) = without_scheme.split_once('/')?;
    if !host.eq_ignore_ascii_case("www.rottentomatoes.com")
        && !host.eq_ignore_ascii_case("rottentomatoes.com")
    {
        return None;
    }
    let path = rest.split(['?', '#']).next()?;
    normalize_rt_path(path)
}

/// Validates and normalizes a Rotten Tomatoes title path (`m/...` or `tv/...`).
pub fn normalize_rt_path(raw: &str) -> Option<String> {
    let path = raw.trim().trim_matches('/');
    let rest = path
        .strip_prefix("m/")
        .or_else(|| path.strip_prefix("tv/"))?;
    if rest.is_empty()
        || path.contains("..")
        || path.contains("//")
        || !path
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'/'))
    {
        return None;
    }
    Some(path.to_owned())
}

#[derive(Deserialize, Default)]
struct ClaimsResponse {
    #[serde(default)]
    claims: Claims,
}

#[derive(Deserialize, Default)]
struct Claims {
    #[serde(rename = "P1258", default)]
    p1258: Vec<Claim>,
}

#[derive(Deserialize)]
struct Claim {
    #[serde(default)]
    rank: String,
    mainsnak: Option<Snak>,
}

#[derive(Deserialize)]
struct Snak {
    datavalue: Option<DataValue>,
}

#[derive(Deserialize)]
struct DataValue {
    value: serde_json::Value,
}

fn pick_p1258_path(claims: &[Claim]) -> Option<String> {
    let valid = |claim: &&Claim| {
        if claim.rank == "deprecated" {
            return None;
        }
        claim
            .mainsnak
            .as_ref()?
            .datavalue
            .as_ref()?
            .value
            .as_str()
            .and_then(normalize_rt_path)
    };
    claims
        .iter()
        .filter(|c| c.rank == "preferred")
        .find_map(|c| valid(&c))
        .or_else(|| claims.iter().find_map(|c| valid(&c)))
}

#[derive(Deserialize)]
struct ScorecardPayload {
    #[serde(rename = "criticsScore")]
    critics_score: Option<CriticsScore>,
}

#[derive(Deserialize)]
struct CriticsScore {
    #[serde(rename = "scorePercent")]
    score_percent: Option<String>,
}

/// Reads the Tomatometer score (0–100) from Rotten Tomatoes' embedded
/// `media-scorecard-json` script, falling back to Schema.org JSON-LD.
pub fn parse_tomatometer(html: &str) -> Option<f64> {
    for (attrs, body) in script_blocks(html) {
        if has_attr_value(attrs, "id", "media-scorecard-json")
            && let Ok(payload) = serde_json::from_str::<ScorecardPayload>(body.trim())
            && let Some(pct) = payload
                .critics_score
                .and_then(|c| c.score_percent)
                .and_then(|s| parse_percent(&s))
        {
            return Some(pct);
        }
    }
    for (attrs, body) in script_blocks(html) {
        if has_attr_value(attrs, "type", "application/ld+json")
            && let Ok(value) = serde_json::from_str::<serde_json::Value>(body.trim())
            && let Some(score) = json_ld_tomatometer(&value)
        {
            return Some(score);
        }
    }
    None
}

fn json_ld_tomatometer(value: &serde_json::Value) -> Option<f64> {
    let agg = value.get("aggregateRating")?;
    if let Some(name) = agg.get("name").and_then(|v| v.as_str())
        && !name.eq_ignore_ascii_case("Tomatometer")
    {
        return None;
    }
    let raw = agg.get("ratingValue")?;
    match raw {
        serde_json::Value::String(s) => parse_score(s),
        serde_json::Value::Number(n) => n
            .as_f64()
            .filter(|v| v.is_finite() && (0.0..=100.0).contains(v)),
        _ => None,
    }
}

fn parse_percent(value: &str) -> Option<f64> {
    parse_score(value.trim().strip_suffix('%')?)
}

fn parse_score(value: &str) -> Option<f64> {
    value
        .trim()
        .parse::<f64>()
        .ok()
        .filter(|v| v.is_finite() && (0.0..=100.0).contains(v))
}

/// Yields `(opening_tag_attributes, inner_body)` for each `<script ...>...</script>`
/// block in `html`.
fn script_blocks(html: &str) -> Vec<(&str, &str)> {
    let lower = html.to_ascii_lowercase();
    let mut blocks = Vec::new();
    let mut cursor = 0;
    while let Some(rel) = lower[cursor..].find("<script") {
        let start = cursor + rel;
        let after_name = start + "<script".len();
        if let Some(next) = lower.as_bytes().get(after_name)
            && !(next.is_ascii_whitespace() || *next == b'>')
        {
            cursor = after_name;
            continue;
        }
        let Some(tag_end_rel) = lower[after_name..].find('>') else {
            break;
        };
        let tag_end = after_name + tag_end_rel;
        let body_start = tag_end + 1;
        let Some(close_rel) = lower[body_start..].find("</script") else {
            break;
        };
        let body_end = body_start + close_rel;
        blocks.push((&html[after_name..tag_end], &html[body_start..body_end]));
        cursor = body_end + "</script".len();
    }
    blocks
}

fn has_attr_value(attrs: &str, name: &str, expected: &str) -> bool {
    let lower = attrs.to_ascii_lowercase();
    let double = format!("{name}=\"{expected}\"");
    let single = format!("{name}='{expected}'");
    lower.contains(&double) || lower.contains(&single)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_wikidata_ids_and_rt_paths() {
        assert!(is_wikidata_id("Q1747211"));
        assert!(!is_wikidata_id("Q"));
        assert!(!is_wikidata_id("P1258"));
        assert!(!is_wikidata_id("tt0110771"));

        assert_eq!(normalize_rt_path("m/paper").as_deref(), Some("m/paper"));
        assert_eq!(
            normalize_rt_path("/tv/the_paper_2025/").as_deref(),
            Some("tv/the_paper_2025")
        );
        assert_eq!(normalize_rt_path("celebrity/ron_howard"), None);
        assert_eq!(normalize_rt_path("m/"), None);
        assert_eq!(normalize_rt_path("m/../etc/passwd"), None);

        assert_eq!(
            normalize_rt_url("https://www.rottentomatoes.com/m/paper/").as_deref(),
            Some("m/paper")
        );
        assert_eq!(
            normalize_rt_url("http://rottentomatoes.com/tv/severance?cmp=1").as_deref(),
            Some("tv/severance")
        );
        assert_eq!(normalize_rt_url("N/A"), None);
        assert_eq!(normalize_rt_url("https://example.com/m/paper"), None);
    }

    #[test]
    fn picks_preferred_non_deprecated_wikidata_claim() {
        let body: ClaimsResponse = serde_json::from_str(
            r#"{
              "claims": {
                "P1258": [
                  { "rank": "deprecated", "mainsnak": { "datavalue": { "value": "m/old" } } },
                  { "rank": "normal", "mainsnak": { "datavalue": { "value": "m/normal" } } },
                  { "rank": "preferred", "mainsnak": { "datavalue": { "value": "m/preferred" } } }
                ]
              }
            }"#,
        )
        .unwrap();
        assert_eq!(
            pick_p1258_path(&body.claims.p1258).as_deref(),
            Some("m/preferred")
        );

        let celeb: ClaimsResponse = serde_json::from_str(
            r#"{"claims":{"P1258":[{"rank":"normal","mainsnak":{"datavalue":{"value":"celebrity/douglas_adams"}}}]}}"#,
        )
        .unwrap();
        assert_eq!(pick_p1258_path(&celeb.claims.p1258), None);
    }

    #[test]
    fn parses_scorecard_json_and_json_ld_fallback() {
        let scorecard = r#"<html><body>
            <script id="media-scorecard-json" type="application/json">
                {"criticsScore":{"scorePercent":"86%","sentiment":"POSITIVE","title":"Tomatometer"}}
            </script>
        </body></html>"#;
        assert_eq!(parse_tomatometer(scorecard), Some(86.0));

        let unrated = r#"<html><body>
            <script id="media-scorecard-json" type="application/json">
                {"criticsScore":{"likedCount":0,"notLikedCount":0,"title":"Tomatometer"}}
            </script>
        </body></html>"#;
        assert_eq!(parse_tomatometer(unrated), None);

        let json_ld = r#"<html><head>
            <script type="application/ld+json">
                {"@type":"Movie","aggregateRating":{"@type":"AggregateRating","name":"Tomatometer","ratingValue":"49"}}
            </script>
        </head></html>"#;
        assert_eq!(parse_tomatometer(json_ld), Some(49.0));

        let other_rating = r#"<html><head>
            <script type="application/ld+json">
                {"@type":"Movie","aggregateRating":{"@type":"AggregateRating","name":"Popcornmeter","ratingValue":"91"}}
            </script>
        </head></html>"#;
        assert_eq!(parse_tomatometer(other_rating), None);
    }
}
