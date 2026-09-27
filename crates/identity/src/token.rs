//! Random bearer tokens (invitations, sessions). Only hashes are stored.

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use sha2::{Digest, Sha256};

/// 256 random bits, base64url without padding.
pub fn new_token() -> String {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).expect("OS random source unavailable");
    URL_SAFE_NO_PAD.encode(bytes)
}

/// Lowercase hex SHA-256 of the token.
pub fn hash_token(token: &str) -> String {
    Sha256::digest(token.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
