//! API keys at rest: protected with Windows DPAPI for the current user.
//! See docs/AI-REWORK-DESIGN.md.
use windows::core::PCWSTR;
use windows::Win32::Foundation::{LocalFree, HLOCAL};
use windows::Win32::Security::Cryptography::{
    CryptProtectData, CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
};

/// Marks a settings value as DPAPI ciphertext rather than plain text (never written on this
/// platform, but `unprotect` must still refuse a plain value it might find in a hand-edited file).
const PREFIX: &str = "dpapi:";

fn to_hex(bytes: &[u8]) -> String {
    bytes.iter().fold(String::with_capacity(bytes.len() * 2), |mut out, b| {
        out.push_str(&format!("{b:02x}"));
        out
    })
}

fn from_hex(text: &str) -> Option<Vec<u8>> {
    // Byte-indexed, never `str`-sliced: a hand-edited settings file can put any text after
    // `dpapi:`, and slicing by byte offset into non-ASCII text panics on a split multi-byte
    // character even when the byte length is even (e.g. "€a" is 4 bytes: 3 + 1).
    fn nibble(b: u8) -> Option<u8> {
        match b {
            b'0'..=b'9' => Some(b - b'0'),
            b'a'..=b'f' => Some(b - b'a' + 10),
            b'A'..=b'F' => Some(b - b'A' + 10),
            _ => None,
        }
    }
    let bytes = text.as_bytes();
    if bytes.len() % 2 != 0 {
        return None;
    }
    bytes.chunks_exact(2).map(|pair| Some((nibble(pair[0])? << 4) | nibble(pair[1])?)).collect()
}

/// The stored form of `secret`. Never the plain text on Windows.
pub fn protect(secret: &str) -> Result<String, String> {
    let mut plain = secret.as_bytes().to_vec();
    let input = CRYPT_INTEGER_BLOB {
        cbData: u32::try_from(plain.len()).map_err(|_| "The API key is too long to store.".to_string())?,
        pbData: plain.as_mut_ptr(),
    };
    let mut output = CRYPT_INTEGER_BLOB::default();
    let protected = unsafe {
        CryptProtectData(&input, PCWSTR::null(), None, None, None, CRYPTPROTECT_UI_FORBIDDEN, &mut output).is_ok()
    };
    plain.fill(0);
    if !protected {
        return Err(format!(
            "Windows could not protect the API key: {}",
            std::io::Error::last_os_error()
        ));
    }
    let bytes = unsafe { std::slice::from_raw_parts(output.pbData, output.cbData as usize) };
    let stored = format!("{PREFIX}{}", to_hex(bytes));
    let _ = unsafe { LocalFree(HLOCAL(output.pbData.cast())) };
    Ok(stored)
}

/// The plain secret, or `None` when `stored` cannot be read by this user on this machine.
pub fn unprotect(stored: &str) -> Option<String> {
    let mut ciphertext = from_hex(stored.strip_prefix(PREFIX)?)?;
    let input = CRYPT_INTEGER_BLOB {
        cbData: u32::try_from(ciphertext.len()).ok()?,
        pbData: ciphertext.as_mut_ptr(),
    };
    let mut output = CRYPT_INTEGER_BLOB::default();
    let unprotected = unsafe {
        CryptUnprotectData(&input, None, None, None, None, CRYPTPROTECT_UI_FORBIDDEN, &mut output).is_ok()
    };
    ciphertext.fill(0);
    if !unprotected {
        return None;
    }
    let bytes = unsafe { std::slice::from_raw_parts(output.pbData, output.cbData as usize) };
    let text = String::from_utf8(bytes.to_vec()).ok();
    let _ = unsafe { LocalFree(HLOCAL(output.pbData.cast())) };
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_protected_secret_round_trips_through_this_users_dpapi() {
        for secret in ["sk-test-1234567890", "", "žltý kôň — s divným textom 🦄"] {
            let stored = protect(secret).unwrap();
            assert!(stored.starts_with(PREFIX));
            assert_ne!(stored, secret);
            if !secret.is_empty() {
                assert!(!stored.contains(secret), "the ciphertext must not carry the plain key");
            }
            assert_eq!(unprotect(&stored).as_deref(), Some(secret));
        }
    }

    #[test]
    fn a_tampered_value_does_not_unprotect() {
        let mut stored = protect("sk-tamper-test").unwrap();
        stored.push('0'); // still valid hex, but no longer the ciphertext DPAPI produced
        assert_eq!(unprotect(&stored), None);
        assert_eq!(unprotect("dpapi:not-hex-at-all"), None);
        assert_eq!(unprotect("dpapi:"), None);
    }

    #[test]
    fn a_value_without_the_dpapi_prefix_is_never_treated_as_ciphertext() {
        assert_eq!(unprotect("plain text key"), None);
        assert_eq!(unprotect("sk-1234567890"), None);
        assert_eq!(unprotect(""), None);
    }

    /// A hand-edited settings file can hold anything after `dpapi:`; non-ASCII text of even
    /// byte length used to panic (byte-index slicing landing inside a multi-byte character)
    /// instead of the "counts as none" the contract requires.
    #[test]
    fn non_ascii_text_after_the_prefix_is_rejected_without_panicking() {
        assert_eq!(unprotect("dpapi:€a"), None); // 3-byte + 1-byte char = 4 bytes, even
        assert_eq!(unprotect("dpapi:žžžž"), None); // every char 2 bytes
        assert_eq!(unprotect("dpapi:🦄"), None); // 4-byte char, odd-length guard still applies
    }
}
