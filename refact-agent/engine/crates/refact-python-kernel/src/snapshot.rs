use hmac::{Hmac, Mac, KeyInit};
use sha2::Sha256;
fn digest(s: &str) -> Option<Vec<u8>> {
    if s.len() != 64 {
        return None;
    }
    (0..64)
        .step_by(2)
        .map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok())
        .collect()
}
pub fn sign(key: &[u8], sha256_hex: &str, chat_id: &str) -> String {
    sign_with_taint(key, sha256_hex, chat_id, &[])
}
pub fn sign_with_taint(key: &[u8], sha256_hex: &str, chat_id: &str, taint_digest: &[u8]) -> String {
    let Some(d) = digest(sha256_hex) else {
        return String::new();
    };
    let mut mac = Hmac::<Sha256>::new_from_slice(key).expect("HMAC key");
    mac.update(&d);
    mac.update(chat_id.as_bytes());
    mac.update(taint_digest);
    mac.finalize()
        .into_bytes()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
pub fn verify(key: &[u8], sha256_hex: &str, chat_id: &str, signature: &str) -> bool {
    verify_with_taint(key, sha256_hex, chat_id, &[], signature)
}
pub fn verify_with_taint(
    key: &[u8],
    sha256_hex: &str,
    chat_id: &str,
    taint_digest: &[u8],
    signature: &str,
) -> bool {
    let Some(d) = digest(sha256_hex) else {
        return false;
    };
    let Some(sig) = digest(signature) else {
        return false;
    };
    let mut mac = Hmac::<Sha256>::new_from_slice(key).expect("HMAC key");
    mac.update(&d);
    mac.update(chat_id.as_bytes());
    mac.update(taint_digest);
    mac.verify_slice(&sig).is_ok()
}
