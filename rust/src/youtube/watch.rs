//! YouTube watch page fetching and parsing
//!
//! Fetches the watch page to extract:
//! - Visitor data (from ytcfg)
//! - Cookies (from Set-Cookie headers)
//! - Player JS URL (from ytcfg.PLAYER_JS_URL)
//! - Player base JS URL (from ytcfg)
//! - Decipher/throttle function names (from player JS)
//! - Signature transform sequence (from player JS)

use regex::Regex;
use std::collections::HashMap;
use std::sync::Mutex;

/// Cached player JavaScript code keyed by base JS path.
static PLAYER_JS_CACHE: Mutex<Option<(String, String)>> = Mutex::new(None);

/// Data extracted from the YouTube watch page.
#[derive(Debug, Clone, Default)]
pub struct WatchData {
    pub cookies: Vec<String>,
    pub visitor_data: Option<String>,
    /// URL of the main player JavaScript file (e.g., "/s/player/xxx/player_ias.vflset/en_US/base.js").
    pub player_js_url: Option<String>,
}

/// Fetch the YouTube watch page and extract all necessary data.
pub fn fetch_watch_page(
    http_client: &reqwest::blocking::Client,
    video_id: &str,
) -> Result<WatchData, String> {
    let url = format!(
        "https://www.youtube.com/watch?v={}&bpctr=9999999999&has_verified=1&hl=en",
        video_id
    );

    let response = http_client
        .get(&url)
        .header(
            "User-Agent",
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
             (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36",
        )
        .header("Accept", "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8")
        .header("Accept-Language", "en-US,en;q=0.9")
        .header("Sec-Fetch-Dest", "document")
        .header("Sec-Fetch-Mode", "navigate")
        .header("Sec-Fetch-Site", "none")
        .header("Sec-Ch-Ua", "\"Chromium\";v=\"131\", \"Not_A Brand\";v=\"24\"")
        .header("Sec-Ch-Ua-Mobile", "?0")
        .header("Sec-Ch-Ua-Platform", "\"Windows\"")
        .send()
        .map_err(|e| format!("Watch page request failed: {}", e))?;

    // Extract cookies from Set-Cookie headers
    let cookies: Vec<String> = response
        .headers()
        .get_all("set-cookie")
        .iter()
        .filter_map(|c| c.to_str().ok())
        .filter_map(|c| {
            c.split(';').next().and_then(|part| {
                let kv: Vec<&str> = part.splitn(2, '=').collect();
                if kv.len() == 2 {
                    Some(format!("{}={}", kv[0].trim(), kv[1].trim()))
                } else {
                    None
                }
            })
        })
        .collect();

    let body = response.text().map_err(|e| e.to_string())?;

    // Extract ytcfg
    let ytcfg = extract_ytcfg(&body).unwrap_or_default();

    // Extract visitor data from ytcfg.INNERTUBE_CONTEXT.client.visitorData
    let visitor_data = ytcfg
        .get("VISITOR_DATA")
        .cloned()
        .or_else(|| {
            // Fallback: try to extract from INNERTUBE_CONTEXT
            extract_visitor_from_context(&body)
        });

    // Extract player JS URL
    let player_js_url = ytcfg
        .get("PLAYER_JS_URL")
        .cloned()
        .or_else(|| extract_player_js_url_from_config(&body));

    Ok(WatchData {
        cookies,
        visitor_data,
        player_js_url,
    })
}

/// Fetch the player JavaScript code (with caching).
pub fn fetch_player_js(
    http_client: &reqwest::blocking::Client,
    player_js_url: &str,
) -> Result<String, String> {
    // Check cache
    {
        let cache = PLAYER_JS_CACHE.lock().map_err(|e| e.to_string())?;
        if let Some((url, code)) = cache.as_ref() {
            if url == player_js_url {
                return Ok(code.clone());
            }
        }
    }

    let full_url = if player_js_url.starts_with("http") {
        player_js_url.to_string()
    } else if player_js_url.starts_with("//") {
        format!("https:{}", player_js_url)
    } else {
        format!("https://www.youtube.com{}", player_js_url)
    };

    eprintln!("[youtube] Fetching player JS from: {}", full_url);

    let response = http_client
        .get(&full_url)
        .header(
            "User-Agent",
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
             (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36",
        )
        .header("Accept", "*/*")
        .header("Referer", "https://www.youtube.com/")
        .send()
        .map_err(|e| format!("Player JS request failed: {}", e))?;

    if !response.status().is_success() {
        return Err(format!("Player JS HTTP {}", response.status()));
    }

    let js_code = response.text().map_err(|e| e.to_string())?;

    // Cache it
    {
        let mut cache = PLAYER_JS_CACHE.lock().map_err(|e| e.to_string())?;
        *cache = Some((player_js_url.to_string(), js_code.clone()));
    }

    Ok(js_code)
}

/// Apply signature transforms to a signature string.
/// transforms is a list like ["reverse", "splice:5", "swap:3"]
pub fn apply_signature_transforms(signature: &str, transforms: &[String]) -> String {
    let mut chars: Vec<char> = signature.chars().collect();

    for op in transforms {
        if op == "reverse" {
            chars.reverse();
        } else if let Some(n) = op.strip_prefix("splice:") {
            if let Ok(n) = n.parse::<usize>() {
                if n <= chars.len() {
                    chars.drain(..n);
                }
            }
        } else if let Some(n) = op.strip_prefix("swap:") {
            if let Ok(n) = n.parse::<usize>() {
                if chars.len() > 1 {
                    let idx = n % chars.len();
                    chars.swap(0, idx);
                }
            }
        }
    }

    chars.into_iter().collect()
}

// ── Internal helpers ──

/// Extract all ytcfg key-value pairs from the watch page HTML.
fn extract_ytcfg(body: &str) -> Option<HashMap<String, String>> {
    // Match: ytcfg.set({...});
    let re = Regex::new(r"ytcfg\.set\s*\(\s*(\{.+?\})\s*\)\s*;").ok()?;
    let json_str = re.captures(body)?.get(1)?.as_str();

    let raw: serde_json::Value = serde_json::from_str(json_str).ok()?;

    let mut map = HashMap::new();

    // Flatten the ytcfg object into key-value pairs.
    // Most keys are top-level strings.
    if let Some(obj) = raw.as_object() {
        for (k, v) in obj {
            if let Some(s) = v.as_str() {
                map.insert(k.clone(), s.to_string());
            }
        }
    }

    Some(map)
}

/// Extract visitor data from the INNERTUBE_CONTEXT embedded in the page.
fn extract_visitor_from_context(body: &str) -> Option<String> {
    let re = Regex::new(r#""visitorData"\s*:\s*"([^"]+)""#).ok()?;
    let caps = re.captures(body)?;
    let encoded = caps.get(1)?.as_str();
    Some(decode_escaped_unicode(encoded))
}

/// Extract player JS URL from the ytcfg or embedded config.
fn extract_player_js_url_from_config(body: &str) -> Option<String> {
    // Try PLAYER_JS_URL in ytcfg
    let re = Regex::new(r#""PLAYER_JS_URL"\s*:\s*"(""([^"]+)")""#).ok()?;
    if let Some(caps) = re.captures(body) {
        return Some(caps.get(2)?.as_str().to_string());
    }

    // Try /s/player/ pattern
    let re2 = Regex::new(r#""(/s/player/[^"]+/player_ias\.vflset/[^"]+/base\.js)""#).ok()?;
    if let Some(caps) = re2.captures(body) {
        return Some(caps.get(1)?.as_str().to_string());
    }

    None
}

/// Decode escaped unicode sequences like \u0026 -> &
fn decode_escaped_unicode(s: &str) -> String {
    let re = Regex::new(r"\\u([0-9a-fA-F]{4})").unwrap();
    re.replace_all(s, |caps: &regex::Captures| {
        let hex = caps.get(1).unwrap().as_str();
        if let Ok(codepoint) = u32::from_str_radix(hex, 16) {
            if let Some(c) = char::from_u32(codepoint) {
                return c.to_string();
            }
        }
        caps.get(0).unwrap().as_str().to_string()
    })
    .to_string()
}

/// Extract signature transforms from the player JavaScript.
///
/// YouTube's player JS contains a decipher function that transforms the signature
/// using a sequence of array operations. We parse this sequence statically.
///
/// The pattern in the player JS looks like:
/// ```javascript
/// var sig=function(a){a=a.split("");xx.reverse();yy.splice(0,1);zz.swap(0,2);return a.join("")};
/// ```
/// Where xx, yy, zz are helper functions defined elsewhere:
/// - reverse: `a.reverse()`
/// - splice: `a.splice(0,n)`
/// - swap: `var b=a[0];a[0]=a[n%a.length];a[n%a.length]=b`
pub fn extract_signature_transforms(js_code: &str) -> Vec<String> {
    let mut transforms = Vec::new();

    // Step 1: Find the decipher function name.
    // Pattern: `var <name>=function(a){var b=a.split(""),...`
    // or: `<name>=function(a){...`
    let func_name_re = Regex::new(
        r#"(?:var\s+|let\s+|const\s+)?([a-zA-Z_$][a-zA-Z0-9_$]*)\s*=\s*function\s*\(\s*a\s*\)\s*\{\s*var\s+b\s*=\s*a\.split\s*\(\s*""\s*\)"#
    ).unwrap();

    let func_name = match func_name_re.captures(js_code) {
        Some(caps) => caps.get(1).unwrap().as_str().to_string(),
        None => {
            eprintln!("[youtube] Could not find signature decipher function name");
            return transforms;
        }
    };

    eprintln!("[youtube] Found decipher function: {}", func_name);

    // Step 2: Extract the function body.
    let body = match extract_function_body(js_code, &func_name) {
        Some(b) => b,
        None => {
            eprintln!("[youtube] Could not extract decipher function body");
            return transforms;
        }
    };

    // Step 3: Find the array variable name (the one that's split and joined).
    let array_var_re = Regex::new(r#"var\s+([a-zA-Z_$][a-zA-Z0-9_$]*)\s*=\s*a\.split\s*\(\s*""\s*\)"#).unwrap();
    let array_var = array_var_re
        .captures(&body)
        .map(|caps| caps.get(1).unwrap().as_str().to_string())
        .unwrap_or_else(|| "b".to_string());

    // Step 4: Find the helper function calls on the array variable.
    // Pattern: <array_var>.<op>()  or  <array_var>.<op>(<arg>)
    let call_re = Regex::new(&format!(
        r"{}\s*\.\s*([a-zA-Z_$][a-zA-Z0-9_$]*)\s*\(\s*([0-9]*)\s*\)",
        regex::escape(&array_var)
    ))
    .unwrap();

    // We need to map the helper function names to their actual operations.
    // First, find all helper function calls in the function body.
    let mut helper_calls: Vec<(String, String)> = Vec::new();
    for caps in call_re.captures_iter(&body) {
        let helper_name = caps.get(1).unwrap().as_str().to_string();
        let arg = caps.get(2).unwrap().as_str().to_string();
        helper_calls.push((helper_name, arg));
    }

    if helper_calls.is_empty() {
        eprintln!("[youtube] No helper function calls found in decipher body");
        return transforms;
    }

    // Step 5: For each unique helper function, determine what it does
    // by examining its definition in the JS code.
    let mut helper_types: HashMap<String, String> = HashMap::new();

    for (helper_name, _) in &helper_calls {
        if helper_types.contains_key(helper_name) {
            continue;
        }

        let op_type = classify_helper_function(js_code, helper_name);
        helper_types.insert(helper_name.clone(), op_type);
    }

    // Step 6: Build the transform sequence.
    for (helper_name, arg) in &helper_calls {
        let op_type = &helper_types[helper_name.as_str()];
        match op_type.as_str() {
            "reverse" => transforms.push("reverse".to_string()),
            "splice" => transforms.push(format!("splice:{}", arg)),
            "swap" => transforms.push(format!("swap:{}", arg)),
            _ => {
                eprintln!("[youtube] Unknown helper operation: {} for {}", op_type, helper_name);
            }
        }
    }

    eprintln!(
        "[youtube] Signature transforms: {:?}",
        transforms
    );

    transforms
}

/// Classify a helper function as "reverse", "splice", or "swap"
/// by examining its source code in the player JS.
///
/// - reverse: body contains `a.reverse()`
/// - splice:  body contains `a.splice(0, ...)`
/// - swap:    body contains pattern like `var b=a[0];a[0]=a[n%a.length];...`
fn classify_helper_function(js_code: &str, func_name: &str) -> String {
    if let Some(body) = extract_function_body(js_code, func_name) {
        // Check for reverse
        if body.contains(".reverse()") {
            return "reverse".to_string();
        }
        // Check for splice
        if body.contains(".splice(") || body.contains(".splice (") {
            return "splice".to_string();
        }
        // Check for swap pattern: a[0]=a[...]; a[...]=b
        if body.contains("[0]=") && body.contains("%") && body.contains("[") {
            return "swap".to_string();
        }
    }

    // Fallback: try to detect by name pattern (not reliable but helps)
    "unknown".to_string()
}

/// Extract the body of a named function from JavaScript source code.
/// Handles nested braces correctly.
fn extract_function_body(js_code: &str, func_name: &str) -> Option<String> {
    // Find: function <name>(  or  var <name>=function(  or  <name>=function(
    let patterns = [
        format!("function {}(", regex::escape(func_name)),
        format!("=function {}(", regex::escape(func_name)),
        format!("=function({}", regex::escape(func_name)),
    ];

    for pattern in &patterns {
        if let Some(pos) = js_code.find(pattern) {
            let start = pos + pattern.len();
            // Find the opening brace
            let brace_start = js_code[start..].find('{')? + start + 1;
            let body = extract_balanced_braces(&js_code[brace_start - 1..])?;
            return Some(body);
        }
    }

    None
}

/// Extract content between matching braces, starting from the first '{'.
fn extract_balanced_braces(code: &str) -> Option<String> {
    let mut depth = 0;
    let mut in_string = false;
    let mut string_char = '\0';
    let mut escaped = false;
    let mut end = 0;

    for (i, c) in code.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if c == '\\' && in_string {
            escaped = true;
            continue;
        }
        if in_string {
            if c == string_char {
                in_string = false;
            }
            continue;
        }
        if c == '"' || c == '\'' || c == '`' {
            in_string = true;
            string_char = c;
            continue;
        }
        if c == '{' {
            depth += 1;
        } else if c == '}' {
            depth -= 1;
            if depth == 0 {
                end = i;
                break;
            }
        }
    }

    if end > 0 {
        // Return everything between the first { and the matching }
        Some(code[1..end].to_string())
    } else {
        None
    }
}

