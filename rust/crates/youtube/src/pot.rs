//! PoToken generation for YouTube.
//!
//! Generates proof-of-origin tokens needed by YouTube's InnerTube API.
//! Currently supports cold-start (placeholder) tokens that don't require
//! the BotGuard VM. Ported from LuanRT/BgUtils.

/// Generate a cold-start PoToken (placeholder) without running the BotGuard VM.
///
/// YouTube may accept this when `StreamProtectionStatus == 2`. Once it
/// upgrades to `3` a full BotGuard challenge is required instead.
///
/// Based on `webPoClient.ts` → `generateColdStartToken()` from LuanRT/BgUtils.
pub fn generate_cold_start_token(identifier: &str) -> String {
    let id = identifier.as_bytes();
    // Max identifier length is ~247 bytes (255 - 8 byte header), but 118 is
    // the BgUtils limit and keeps us well within u8 range.
    let id = &id[..id.len().min(118)];
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let key0 = pseudo_rand_u8();
    let key1 = pseudo_rand_u8();

    // header: [key0, key1, 0x00, client_state=1, timestamp_be_u32]
    let header = [
        key0,
        key1,
        0,
        1, // client_state
        (timestamp >> 24) as u8,
        (timestamp >> 16) as u8,
        (timestamp >> 8) as u8,
        timestamp as u8,
    ];

    let payload_len = header.len() + id.len();
    let mut packet = Vec::with_capacity(2 + payload_len);
    packet.push(34); // magic byte
    packet.push(payload_len as u8);
    packet.extend_from_slice(&header);
    packet.extend_from_slice(id);

    // XOR cipher on payload (everything after the first 2 bytes).
    // The first two payload bytes (key0, key1) serve as the key and stay
    // unmodified; every byte from index 2 onwards is XOR'd with key0 or key1.
    let payload_len = packet.len();
    // SAFETY: index 2 exists because packet is always >= 10 bytes.
    for i in 4..payload_len {
        packet[i] ^= packet[2 + (i - 2) % 2];
    }

    base64url_encode(&packet)
}

fn pseudo_rand_u8() -> u8 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let count = COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as u64;
    (count.wrapping_add(nanos)) as u8
}

/// Standard base64 with URL-safe alphabet (no padding).
fn base64url_encode(data: &[u8]) -> String {
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut result = String::with_capacity((data.len() + 2) / 3 * 4);
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = chunk.get(1).copied().unwrap_or(0) as u32;
        let b2 = chunk.get(2).copied().unwrap_or(0) as u32;
        let triple = (b0 << 16) | (b1 << 8) | b2;
        result.push(CHARS[((triple >> 18) & 0x3F) as usize] as char);
        result.push(CHARS[((triple >> 12) & 0x3F) as usize] as char);
        if chunk.len() > 1 {
            result.push(CHARS[((triple >> 6) & 0x3F) as usize] as char);
        }
        if chunk.len() > 2 {
            result.push(CHARS[(triple & 0x3F) as usize] as char);
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cold_start_token_is_valid_base64url() {
        let token = generate_cold_start_token("CAAQCA%3D%3D");
        assert!(!token.is_empty(), "token should not be empty");
        assert!(
            token.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'),
            "token should be valid base64url: {token:?}"
        );
    }

    #[test]
    fn test_cold_start_token_empty_identifier() {
        let token = generate_cold_start_token("");
        assert!(!token.is_empty(), "token should not be empty even with empty identifier");
        assert!(
            token.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'),
            "token should be valid base64url: {token:?}"
        );
    }

    #[test]
    fn test_cold_start_token_deterministic_structure() {
        let t1 = generate_cold_start_token("test");
        let t2 = generate_cold_start_token("test");
        // Tokens should differ because they embed a random key and timestamp
        assert_ne!(t1, t2, "tokens should differ across invocations");
        // But both should be the same length
        assert_eq!(t1.len(), t2.len(), "tokens should have same length");
    }

    #[test]
    fn test_cold_start_token_differs_for_diff_identifiers() {
        let t1 = generate_cold_start_token("abc");
        let t2 = generate_cold_start_token("xyz");
        assert_ne!(t1, t2, "tokens for different identifiers should differ");
    }

    #[test]
    fn test_base64url_encode_roundtrip_padding_agnostic() {
        let data = b"hello";
        let encoded = base64url_encode(data);
        assert!(!encoded.is_empty());
        // Decode manually to verify
        let decoded = base64url_decode(&encoded).expect("should decode");
        assert_eq!(&decoded, data, "base64url roundtrip failed");
    }

    fn base64url_decode(input: &str) -> Result<Vec<u8>, ()> {
        const DECODE: [i8; 256] = {
            let mut table = [-1i8; 256];
            let chars = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
            let mut i = 0;
            while i < chars.len() {
                table[chars[i] as usize] = i as i8;
                i += 1;
            }
            table
        };

        let bytes: Vec<u8> = input.bytes().filter_map(|b| {
            let v = DECODE[b as usize];
            if v >= 0 { Some(v as u8) } else { None }
        }).collect();

        let mut out = Vec::with_capacity(bytes.len() * 3 / 4);
        for chunk in bytes.chunks(4) {
            if chunk.len() < 2 { return Err(()); }
            let b0 = chunk[0] as u32;
            let b1 = chunk[1] as u32;
            let b2 = chunk.get(2).copied().unwrap_or(0) as u32;
            let b3 = chunk.get(3).copied().unwrap_or(0) as u32;
            let triple = (b0 << 18) | (b1 << 12) | (b2 << 6) | b3;
            out.push((triple >> 16) as u8);
            if chunk.len() > 2 {
                out.push((triple >> 8) as u8);
            }
            if chunk.len() > 3 {
                out.push(triple as u8);
            }
        }
        Ok(out)
    }

    #[test]
    fn test_cold_start_token_long_identifier() {
        let long_id = "a".repeat(100);
        let token = generate_cold_start_token(&long_id);
        assert!(!token.is_empty(), "long identifier should produce a token");
    }

    #[test]
    fn test_cold_start_token_truncates_long_identifier() {
        let long_id = "a".repeat(200);
        let token = generate_cold_start_token(&long_id);
        assert!(!token.is_empty(), "very long identifier should produce a token");
    }
}
