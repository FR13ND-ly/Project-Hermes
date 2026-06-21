//! Centralised access to the platform's mandatory cryptographic secrets.
//!
//! Historically `JWT_SECRET` and `HERMES_ENCRYPTION_KEY` had hard-coded fallback
//! values scattered across several modules. That was dangerous on two fronts:
//!   * the AES key fallback was a publicly-known constant, so any deploy that
//!     forgot to set it shipped trivially-decryptable secrets, and
//!   * the JWT fallback differed between the signing and verifying call sites,
//!     so an unset `JWT_SECRET` silently broke all authentication.
//!
//! Secrets are now read through this single module and validated once at startup
//! via [`validate`]. There are no insecure fallbacks: a misconfigured deploy
//! fails fast and loudly instead of running in an unsafe state.

use std::env;

/// HS256 signing/verification secret for session JWTs.
///
/// Panics only if called before [`validate`] succeeded; in normal operation the
/// startup check guarantees the variable is present.
pub fn jwt_secret() -> String {
    env::var("JWT_SECRET").expect("JWT_SECRET must be set (validated at startup)")
}

/// 32-byte symmetric key used for AES-256-GCM encryption of stored env values.
pub fn encryption_key() -> Vec<u8> {
    env::var("HERMES_ENCRYPTION_KEY")
        .expect("HERMES_ENCRYPTION_KEY must be set (validated at startup)")
        .into_bytes()
}

/// Fail-fast validation of every mandatory secret. Call exactly once during boot,
/// before the server starts accepting traffic.
pub fn validate() -> Result<(), anyhow::Error> {
    let jwt = env::var("JWT_SECRET").map_err(|_| {
        anyhow::anyhow!("CRITICAL CONFIG ERROR: JWT_SECRET environment variable is not set.")
    })?;
    if jwt.len() < 32 {
        anyhow::bail!(
            "CRITICAL CONFIG ERROR: JWT_SECRET must be at least 32 characters (got {}).",
            jwt.len()
        );
    }

    let key = env::var("HERMES_ENCRYPTION_KEY").map_err(|_| {
        anyhow::anyhow!("CRITICAL CONFIG ERROR: HERMES_ENCRYPTION_KEY environment variable is not set.")
    })?;
    if key.as_bytes().len() != 32 {
        anyhow::bail!(
            "CRITICAL CONFIG ERROR: HERMES_ENCRYPTION_KEY must be exactly 32 bytes for AES-256 (got {}).",
            key.as_bytes().len()
        );
    }

    Ok(())
}
