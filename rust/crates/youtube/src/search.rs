//! YouTube search functionality
//!
//! Provides two search strategies:
//! 1. `search()` — HTML scraping of YouTube search results (primary)
//! 2. `search_videos()` / `search_via_invidious()` — Invidious API (fallback)

use crate::client::is_valid_video_id;
use regex::Regex;
use serde::{Deserialize, Serialize};

const INVIDIOUS_INSTANCES: &[&str] = &[
    "https://yewtu.be",
    "https://invidious.snopyta.org",
    "https://invidious.tiekoetter.com",
    "https://invidious.incognitus.net",
];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    pub id: String,
    pub title: String,
    pub author: String,
    pub duration: u64,
    pub thumbnail: String,
}

// ============================================================================
// HTML scraping search (primary — used by FFI, handling.rs, examples)
// ============================================================================

/// Search YouTube via HTML scraping of the search results page.
pub fn search(
    client: &reqwest::blocking::Client,
    query: &str,
    limit: usize,
) -> Result<Vec<SearchResult>, String> {
    let url = format!(
        "https://www.youtube.com/results?search_query={}&hl=en&gl=US",
        urlencoding::encode(query)
    );

    let response = client
        .get(&url)
        .header(
            "User-Agent",
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
             (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36",
        )
        .header(
            "Accept",
            "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8",
        )
        .header("Accept-Language", "en-US,en;q=0.9")
        .send()
        .map_err(|e| e.to_string())?;

    if !response.status().is_success() {
        return Err(format!("HTTP error: {}", response.status()));
    }

    let body = response.text().map_err(|e| e.to_string())?;

    if let Ok(results) = parse_yt_initial_data(&body, limit) {
        if !results.is_empty() {
            return Ok(results);
        }
    }

    parse_search_html_fallback(&body, limit)
}

fn parse_yt_initial_data(body: &str, limit: usize) -> Result<Vec<SearchResult>, String> {
    let re = Regex::new(r"var ytInitialData\s*=\s*(\{.+?\})\s*;\s*</script>").unwrap();
    let json_str = match re.captures(body) {
        Some(caps) => caps.get(1).unwrap().as_str(),
        None => {
            let re2 = Regex::new(r"ytInitialData\s*=\s*(\{.+?\})\s*;").unwrap();
            match re2.captures(body) {
                Some(caps) => caps.get(1).unwrap().as_str(),
                None => return Err("Could not find ytInitialData".to_string()),
            }
        }
    };

    let data: serde_json::Value = serde_json::from_str(json_str)
        .map_err(|e| format!("Failed to parse ytInitialData: {}", e))?;

    let mut results = Vec::new();

    if let Some(contents) = data.pointer(
        "/contents/twoColumnSearchResultsRenderer/primaryContents/sectionListRenderer/contents",
    ) {
        if let Some(sections) = contents.as_array() {
            for section in sections {
                if let Some(items) = section.pointer("/itemSectionRenderer/contents") {
                    if let Some(items_arr) = items.as_array() {
                        for item in items_arr {
                            if let Some(renderer) = item.get("videoRenderer") {
                                if let Some(result) = parse_video_renderer(renderer) {
                                    results.push(result);
                                    if results.len() >= limit {
                                        return Ok(results);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    Ok(results)
}

fn parse_video_renderer(renderer: &serde_json::Value) -> Option<SearchResult> {
    let id = renderer.get("videoId")?.as_str()?.to_string();

    let title = renderer
        .pointer("/title/runs")
        .and_then(|v| v.as_array())
        .and_then(|runs| runs.first())
        .and_then(|run| run.get("text"))
        .and_then(|t| t.as_str())
        .unwrap_or("")
        .to_string();

    let author = renderer
        .pointer("/ownerText/runs")
        .and_then(|v| v.as_array())
        .and_then(|runs| runs.first())
        .and_then(|run| run.get("text"))
        .and_then(|t| t.as_str())
        .unwrap_or("")
        .to_string();

    let thumbnail = renderer
        .pointer("/thumbnail/thumbnails")
        .and_then(|v| v.as_array())
        .and_then(|thumbs| thumbs.last())
        .and_then(|thumb| thumb.get("url"))
        .and_then(|t| t.as_str())
        .unwrap_or("")
        .to_string();

    let duration = renderer
        .pointer("/lengthText/simpleText")
        .and_then(|t| t.as_str())
        .and_then(parse_duration)
        .unwrap_or(0);

    Some(SearchResult {
        id,
        title,
        author,
        duration,
        thumbnail,
    })
}

fn parse_duration(s: &str) -> Option<u64> {
    let parts: Vec<&str> = s.split(':').collect();
    match parts.len() {
        2 => {
            let mins: u64 = parts[0].parse().ok()?;
            let secs: u64 = parts[1].parse().ok()?;
            Some(mins * 60 + secs)
        }
        3 => {
            let hours: u64 = parts[0].parse().ok()?;
            let mins: u64 = parts[1].parse().ok()?;
            let secs: u64 = parts[2].parse().ok()?;
            Some(hours * 3600 + mins * 60 + secs)
        }
        _ => None,
    }
}

fn parse_search_html_fallback(body: &str, limit: usize) -> Result<Vec<SearchResult>, String> {
    let mut results = Vec::new();
    let mut seen = std::collections::HashSet::new();

    let re = Regex::new(r"watch\?v=([a-zA-Z0-9_-]{11})").unwrap();

    for cap in re.captures_iter(body) {
        if results.len() >= limit {
            break;
        }
        if let Some(id) = cap.get(1) {
            let id_str = id.as_str().to_string();
            if seen.insert(id_str.clone()) {
                results.push(SearchResult {
                    id: id_str,
                    title: String::new(),
                    author: String::new(),
                    duration: 0,
                    thumbnail: String::new(),
                });
            }
        }
    }

    Ok(results)
}

// ============================================================================
// Invidious search (legacy fallback)
// ============================================================================

#[derive(Debug, Clone, Deserialize)]
struct InvidiousVideo {
    #[serde(rename = "videoId")]
    video_id: Option<String>,
    title: Option<String>,
    author: Option<String>,
    duration: Option<u64>,
    #[serde(alias = "thumbnail")]
    thumbnail_url: Option<String>,
}

fn parse_invidious_response(body: &str, limit: usize) -> Vec<SearchResult> {
    let videos: Vec<InvidiousVideo> = match serde_json::from_str(body) {
        Ok(v) => v,
        Err(_) => return vec![],
    };

    videos
        .into_iter()
        .filter_map(|v| {
            let video_id = v.video_id?;
            if !is_valid_video_id(&video_id) {
                return None;
            }
            Some(SearchResult {
                id: video_id,
                title: v.title.unwrap_or_default(),
                author: v.author.unwrap_or_default(),
                duration: v.duration.unwrap_or(0),
                thumbnail: v.thumbnail_url.unwrap_or_default(),
            })
        })
        .take(limit)
        .collect()
}

pub fn search_via_invidious(
    http_client: &reqwest::blocking::Client,
    query: &str,
    limit: usize,
) -> Result<Vec<SearchResult>, String> {
    for instance in INVIDIOUS_INSTANCES {
        let url = format!(
            "{}/api/v1/search?q={}&type=video",
            instance,
            urlencoding::encode(query)
        );

        match http_client
            .get(&url)
            .header("Accept", "*/*")
            .timeout(std::time::Duration::from_secs(5))
            .send()
        {
            Ok(response) if response.status().is_success() => match response.text() {
                Ok(body) => {
                    let results = parse_invidious_response(&body, limit);
                    if !results.is_empty() {
                        eprintln!(
                            "[youtube] Invidious search \"{}\": {} results from {}",
                            query,
                            results.len(),
                            instance
                        );
                        return Ok(results);
                    }
                }
                Err(e) => {
                    eprintln!("[youtube] Failed to read response from {}: {}", instance, e);
                }
            },
            Ok(response) => {
                eprintln!(
                    "[youtube] Invidious {} returned HTTP {}",
                    instance,
                    response.status()
                );
            }
            Err(e) => {
                eprintln!("[youtube] Invidious request to {} failed: {}", instance, e);
            }
        }
    }

    eprintln!(
        "[youtube] All Invidious instances failed for query: {}",
        query
    );
    Err("All Invidious instances failed".to_string())
}

pub fn search_videos(
    http_client: &reqwest::blocking::Client,
    query: &str,
    limit: usize,
) -> Result<Vec<SearchResult>, String> {
    match search_via_invidious(http_client, query, limit) {
        Ok(results) if !results.is_empty() => Ok(results),
        _ => Err("No results found".to_string()),
    }
}
