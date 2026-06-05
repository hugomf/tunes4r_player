//! JavaScript-based signature and throttle deciphering for YouTube
//!
//! Uses QuickJS (via rquickjs) to execute the actual player JavaScript
//! functions for deciphering signatures and n-parameter throttle transforms.
//! This is the same approach used by youtube_explode_dart.

#[cfg(not(target_os = "android"))]
use rquickjs::{Context, Runtime};

/// Decipher a signature using the player JavaScript code.
///
/// If `js_code` is provided, executes the decipher function from the player JS.
/// Falls back to a basic heuristic if no JS code is available.
pub fn decipher_signature(js_code: &str, signature: &str) -> String {
    if signature.is_empty() {
        return String::new();
    }

    if js_code.is_empty() {
        return basic_signature_transform(signature);
    }

    // Try to execute the decipher function via QuickJS
    match execute_decipher_js(js_code, signature) {
        Ok(deciphered) => {
            if !deciphered.is_empty() && deciphered != signature {
                return deciphered;
            }
        }
        Err(e) => {
            eprintln!("[youtube] JS decipher execution failed: {}", e);
        }
    }

    // Fallback
    basic_signature_transform(signature)
}

/// Decipher the n-parameter (throttle) using the player JavaScript.
///
/// YouTube throttles download speeds when the `n` URL parameter is not properly
/// transformed. This function executes the throttle function from the player JS.
pub fn decipher_throttle(js_code: &str, n_value: &str) -> String {
    if n_value.is_empty() {
        return String::new();
    }

    if js_code.is_empty() {
        return n_value.to_string();
    }

    match execute_throttle_js(js_code, n_value) {
        Ok(deciphered) => {
            if !deciphered.is_empty() && deciphered != n_value {
                return deciphered;
            }
        }
        Err(e) => {
            eprintln!("[youtube] JS throttle execution failed: {}", e);
        }
    }

    n_value.to_string()
}

/// Execute the decipher function from the player JavaScript via QuickJS.
#[cfg(not(target_os = "android"))]
fn execute_decipher_js(js_code: &str, signature: &str) -> Result<String, String> {
    let rt = Runtime::new().map_err(|e| format!("Failed to create JS runtime: {}", e))?;
    let ctx = Context::full(&rt).map_err(|e| format!("Failed to create JS context: {}", e))?;

    // Find the decipher function name before entering the context
    let func_name =
        find_decipher_function_name(js_code).ok_or("Could not find decipher function name")?;

    ctx.with(|ctx| {
        // Evaluate the player JS code to define all functions
        ctx.eval::<(), _>(js_code.as_bytes())
            .map_err(|e| format!("Failed to evaluate player JS: {}", e))?;

        // Build the call expression: (<func_name>)("<signature>")
        let call_code = format!("({})(\"{}\")", func_name, escape_js_string(signature));

        // Execute the decipher function
        let result: String = ctx
            .eval::<String, _>(call_code.as_bytes())
            .map_err(|e| format!("Failed to call decipher function: {}", e))?;

        Ok(result)
    })
}

/// Execute the decipher function from the player JavaScript via QuickJS.
#[cfg(target_os = "android")]
fn execute_decipher_js(_js_code: &str, signature: &str) -> Result<String, String> {
    Err(format!(
        "JS decipher not available on Android: {}",
        signature
    ))
}

/// Execute the n-parameter throttle function from the player JavaScript via QuickJS.
#[cfg(not(target_os = "android"))]
fn execute_throttle_js(js_code: &str, n_value: &str) -> Result<String, String> {
    let rt = Runtime::new().map_err(|e| format!("Failed to create JS runtime: {}", e))?;
    let ctx = Context::full(&rt).map_err(|e| format!("Failed to create JS context: {}", e))?;

    // Find the throttle function name before entering the context
    let func_name =
        find_throttle_function_name(js_code).ok_or("Could not find throttle function name")?;

    ctx.with(|ctx| {
        // Evaluate the player JS code to define all functions
        ctx.eval::<(), _>(js_code.as_bytes())
            .map_err(|e| format!("Failed to evaluate player JS: {}", e))?;

        // Build the call expression: (<func_name>)("<n_value>")
        let call_code = format!("({})(\"{}\")", func_name, escape_js_string(n_value));

        // Execute the throttle function
        let result: String = ctx
            .eval::<String, _>(call_code.as_bytes())
            .map_err(|e| format!("Failed to call throttle function: {}", e))?;

        Ok(result)
    })
}

/// Execute the n-parameter throttle function from the player JavaScript via QuickJS.
#[cfg(target_os = "android")]
fn execute_throttle_js(_js_code: &str, n_value: &str) -> Result<String, String> {
    Err(format!("JS throttle not available on Android: {}", n_value))
}

/// Find the signature decipher function name in the player JS.
///
/// The decipher function typically has this pattern:
///   var <name>=function(a){a=a.split("");...
#[cfg(not(target_os = "android"))]
fn find_decipher_function_name(js_code: &str) -> Option<String> {
    // Pattern 1: var <name>=function(a){var b=a.split(""),...
    let re1 = regex::Regex::new(
        r#"(?:var\s+|let\s+|const\s+)?([a-zA-Z_$][a-zA-Z0-9_$]*)\s*=\s*function\s*\(\s*a\s*\)\s*\{\s*var\s+b\s*=\s*a\.split\s*\(\s*""\s*\)"#
    ).ok()?;

    if let Some(caps) = re1.captures(js_code) {
        return Some(caps.get(1)?.as_str().to_string());
    }

    // Pattern 2: var <name>=function(a){a=a.split("");...
    let re2 = regex::Regex::new(
        r#"(?:var\s+|let\s+|const\s+)?([a-zA-Z_$][a-zA-Z0-9_$]*)\s*=\s*function\s*\(\s*a\s*\)\s*\{\s*a\s*=\s*a\.split\s*\(\s*""\s*\)"#
    ).ok()?;

    if let Some(caps) = re2.captures(js_code) {
        return Some(caps.get(1)?.as_str().to_string());
    }

    None
}

/// Find the n-parameter throttle function name in the player JS.
///
/// The throttle function typically has this pattern:
///   var <name>=function(a){a=a.split("");...switch...return a.join("")}
#[cfg(not(target_os = "android"))]
fn find_throttle_function_name(js_code: &str) -> Option<String> {
    // The throttle function is different from the decipher function.
    // It typically contains a switch statement and is more complex.
    // Pattern: var <name>=function(a){a=a.split("");var b=...switch...
    let re = regex::Regex::new(
        r#"(?:var\s+|let\s+|const\s+)?([a-zA-Z_$][a-zA-Z0-9_$]*)\s*=\s*function\s*\(\s*a\s*\)\s*\{\s*a\s*=\s*a\.split\s*\(\s*""\s*\)\s*;[^}]*switch"#
    ).ok()?;

    if let Some(caps) = re.captures(js_code) {
        return Some(caps.get(1)?.as_str().to_string());
    }

    None
}

/// Escape a string for use in JavaScript string literal.
#[cfg(not(target_os = "android"))]
fn escape_js_string(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t")
        .replace('\'', "\\'")
}

/// Basic signature transform as a fallback when JS execution is not available.
///
/// This is a heuristic based on common YouTube signature patterns.
/// It is NOT as accurate as the full JS execution approach.
fn basic_signature_transform(signature: &str) -> String {
    let mut chars: Vec<char> = signature.chars().collect();
    let sig_len = chars.len();

    if sig_len < 4 {
        return signature.to_string();
    }

    let first = sig_len % 3;
    let second = sig_len % 5;

    match first {
        0 => {
            if chars.len() > 2 {
                chars.swap(0, 2);
            }
        }
        1 => {
            chars.reverse();
        }
        2 => {
            if chars.len() > 1 {
                chars.swap(0, 1);
            }
        }
        _ => {}
    }

    if second == 1 {
        chars.reverse();
    } else if second == 2 {
        let len = chars.len();
        if len > 1 {
            chars.swap(len - 1, len - 2);
        }
    }

    chars.into_iter().collect()
}
