//! Spring Security remember-me token issuance and validation.

use std::time::{SystemTime, UNIX_EPOCH};

use base64::Engine;
use base64::engine::general_purpose::{STANDARD, STANDARD_NO_PAD};
use komga_types::KomgaUser;
use sha2::{Digest, Sha256};

use super::SessionRegistry;
use crate::identity::{IdentityError, IdentityService};
use crate::settings::RememberMeSettings;

const REMEMBER_ME_SIGNATURE_ALGORITHM: &str = "SHA256";
const SPRING_DEFAULT_REMEMBER_ME_SECONDS: u64 = 14 * 24 * 60 * 60;

impl SessionRegistry {
    /// Issue the URL-encoded, unpadded Base64 token used by Spring Security.
    pub fn issue_remember_me_token(
        &self,
        user: &KomgaUser,
        settings: &RememberMeSettings,
    ) -> String {
        let now_millis = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time must be after the Unix epoch")
            .as_millis() as u64;
        let token_lifetime_seconds = if settings.validity_seconds() < 0 {
            SPRING_DEFAULT_REMEMBER_ME_SECONDS
        } else {
            settings.validity_seconds() as u64
        };
        let expiry_millis = now_millis + (token_lifetime_seconds * 1000);
        let signature = Self::remember_me_signature(
            &user.email,
            expiry_millis,
            &user.password_hash,
            settings.key(),
        );

        STANDARD_NO_PAD.encode(format!(
            "{}:{}:{}:{}",
            encode_form_component(&user.email),
            expiry_millis,
            REMEMBER_ME_SIGNATURE_ALGORITHM,
            signature
        ))
    }

    /// Validate a remember-me token against user's current password hash.
    pub fn validate_remember_me_token(
        &self,
        token: &str,
        user: &KomgaUser,
        settings: &RememberMeSettings,
    ) -> bool {
        let Some(decoded_str) = decode_remember_me_token(token) else {
            return false;
        };

        let mut parts = decoded_str.split(':');
        let Some(encoded_email) = parts.next() else {
            return false;
        };
        let Some(expiry_text) = parts.next() else {
            return false;
        };
        let Some(algorithm) = parts.next() else {
            return false;
        };
        let Some(signature) = parts.next() else {
            return false;
        };
        if parts.next().is_some() {
            return false;
        }
        let Some(email) = decode_form_component(encoded_email) else {
            return false;
        };
        let Ok(expiry_millis) = expiry_text.parse::<u64>() else {
            return false;
        };

        if algorithm != REMEMBER_ME_SIGNATURE_ALGORITHM {
            return false;
        }

        let now_millis = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time must be after the Unix epoch")
            .as_millis() as u64;

        if expiry_millis < now_millis {
            return false;
        }

        if !email.eq_ignore_ascii_case(&user.email) {
            return false;
        }

        let expected_signature = Self::remember_me_signature(
            &user.email,
            expiry_millis,
            &user.password_hash,
            settings.key(),
        );
        constant_time_eq(&expected_signature, signature)
    }

    fn remember_me_signature(
        email: &str,
        expiry_millis: u64,
        password_hash: &str,
        key: &str,
    ) -> String {
        let payload = format!("{email}:{expiry_millis}:{password_hash}:{key}");
        let mut hasher = Sha256::new();
        hasher.update(payload.as_bytes());
        hex::encode(hasher.finalize())
    }
}

impl IdentityService {
    /// Authenticate a remember-me token using the current persisted key and user password.
    pub async fn authenticate_remember_me(
        &self,
        token: &str,
        settings: &RememberMeSettings,
    ) -> Result<Option<KomgaUser>, IdentityError> {
        let Some(email) = remember_me_email(token) else {
            return Ok(None);
        };
        let Some(user) = self.find_by_email(&email).await? else {
            return Ok(None);
        };
        if self
            .sessions
            .validate_remember_me_token(token, &user, settings)
        {
            Ok(Some(user))
        } else {
            Ok(None)
        }
    }
}

fn remember_me_email(token: &str) -> Option<String> {
    let decoded = decode_remember_me_token(token)?;
    decode_form_component(decoded.split(':').next()?)
}

fn decode_remember_me_token(token: &str) -> Option<String> {
    let token = token.trim();
    let decoded = STANDARD_NO_PAD
        .decode(token)
        .or_else(|_| STANDARD.decode(token))
        .ok()?;
    String::from_utf8(decoded).ok()
}

fn encode_form_component(value: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'.' | b'-' | b'*' | b'_' => {
                encoded.push(char::from(byte));
            }
            b' ' => encoded.push('+'),
            _ => {
                encoded.push('%');
                encoded.push(char::from(HEX[(byte >> 4) as usize]));
                encoded.push(char::from(HEX[(byte & 0x0f) as usize]));
            }
        }
    }
    encoded
}

fn decode_form_component(value: &str) -> Option<String> {
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'+' => {
                decoded.push(b' ');
                index += 1;
            }
            b'%' => {
                let high = *bytes.get(index + 1)?;
                let low = *bytes.get(index + 2)?;
                decoded.push((hex_digit(high)? << 4) | hex_digit(low)?);
                index += 3;
            }
            byte => {
                decoded.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8(decoded).ok()
}

fn hex_digit(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn constant_time_eq(left: &str, right: &str) -> bool {
    let mut difference = left.len() ^ right.len();
    for (left, right) in left.bytes().zip(right.bytes()) {
        difference |= usize::from(left ^ right);
    }
    difference == 0
}
