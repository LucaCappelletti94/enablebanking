//! Signing the JSON Web Tokens that authenticate an application to the API.

use std::sync::{Arc, Mutex};

use jiff::{SignedDuration, Timestamp};
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use serde::Serialize;

/// How long a token stays valid. The API accepts at most 24 hours.
const LIFETIME: SignedDuration = SignedDuration::from_hours(1);

/// A cached token is replaced once less than this much of its lifetime is left.
const RENEWAL_MARGIN: SignedDuration = SignedDuration::from_mins(5);

/// Signs API requests as one registered application, reusing each token until shortly before it expires.
///
/// Clones share the key and the cached token.
#[derive(Clone)]
pub struct Signer(Arc<Inner>);

/// The state a [`Signer`] and its clones share.
struct Inner {
    /// Application id, sent as the token's key id.
    application_id: String,
    /// The application's RSA private key.
    key: EncodingKey,
    /// The last token and when it expires.
    cached: Mutex<Option<(String, Timestamp)>>,
}

/// Why a [`Signer`] could not be built or could not sign.
#[derive(Debug, thiserror::Error)]
pub enum SignError {
    /// The private key is not an RSA key in PEM format.
    #[error("the private key is not a PEM-encoded RSA key: {0}")]
    Key(#[source] jsonwebtoken::errors::Error),
    /// Signing the token failed.
    #[error("signing the token failed: {0}")]
    Signing(#[source] jsonwebtoken::errors::Error),
}

/// The claims Enable Banking requires in every token.
#[derive(Serialize)]
struct Claims<'a> {
    /// Issuer, always `enablebanking.com`.
    iss: &'a str,
    /// Audience, always `api.enablebanking.com`.
    aud: &'a str,
    /// Issue time, in seconds since the epoch.
    iat: i64,
    /// Expiry time, in seconds since the epoch.
    exp: i64,
}

impl Signer {
    /// A signer for the application with id `application_id`, using the RSA private key whose certificate was uploaded when registering it.
    ///
    /// # Errors
    ///
    /// Returns [`SignError::Key`] when `private_key_pem` is not a PEM-encoded RSA private key.
    pub fn new(
        application_id: impl Into<String>,
        private_key_pem: &[u8],
    ) -> Result<Self, SignError> {
        Ok(Self(Arc::new(Inner {
            application_id: application_id.into(),
            key: EncodingKey::from_rsa_pem(private_key_pem).map_err(SignError::Key)?,
            cached: Mutex::new(None),
        })))
    }

    /// A token valid at least for the next few minutes, signed now or reused from an earlier call.
    ///
    /// # Errors
    ///
    /// Returns [`SignError::Signing`] when the key cannot sign.
    pub fn token(&self) -> Result<String, SignError> {
        self.token_at(Timestamp::now())
    }

    /// [`Signer::token`] as of `now`.
    fn token_at(&self, now: Timestamp) -> Result<String, SignError> {
        if let Some(token) = self.cached_until(now + RENEWAL_MARGIN) {
            return Ok(token);
        }
        let expires = now + LIFETIME;
        let mut header = Header::new(Algorithm::RS256);
        header.kid = Some(self.0.application_id.clone());
        let claims = Claims {
            iss: "enablebanking.com",
            aud: "api.enablebanking.com",
            iat: now.as_second(),
            exp: expires.as_second(),
        };
        let token =
            jsonwebtoken::encode(&header, &claims, &self.0.key).map_err(SignError::Signing)?;
        *self.cache() = Some((token.clone(), expires));
        Ok(token)
    }

    /// The cached token, when it is still valid at `instant`.
    fn cached_until(&self, instant: Timestamp) -> Option<String> {
        self.cache()
            .as_ref()
            .filter(|(_, expires)| instant < *expires)
            .map(|(token, _)| token.clone())
    }

    /// The cache, usable even after a panic poisoned its lock.
    fn cache(&self) -> std::sync::MutexGuard<'_, Option<(String, Timestamp)>> {
        self.0
            .cached
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

impl std::fmt::Debug for Signer {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Signer")
            .field("application_id", &self.0.application_id)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
#[expect(clippy::unwrap_used, reason = "a test panics on any unexpected value")]
mod tests {
    use jiff::{SignedDuration, Timestamp};

    use super::{LIFETIME, RENEWAL_MARGIN, Signer};

    const KEY: &[u8] = include_bytes!("../tests/fixtures/test-key.pem");

    #[test]
    fn a_token_is_reused_until_its_renewal_margin_then_replaced() {
        let signer = Signer::new("app", KEY).unwrap();
        let start = Timestamp::from_second(1_800_000_000).unwrap();
        let first = signer.token_at(start).unwrap();
        let last_reuse = start + LIFETIME - RENEWAL_MARGIN - SignedDuration::from_secs(1);
        assert_eq!(signer.token_at(last_reuse).unwrap(), first);
        let renewal = start + LIFETIME - RENEWAL_MARGIN;
        assert_ne!(signer.token_at(renewal).unwrap(), first);
    }
}
