//! Authentification du futur serveur : mots de passe (Argon2id, format PHC) et jetons d'API.
//!
//! - Les mots de passe sont hachés avec Argon2id et un sel aléatoire de 16 octets ; le
//!   résultat est une chaîne PHC (`$argon2id$v=19$m=…,t=…,p=…$sel$hachage`) qui porte ses
//!   propres paramètres, ce qui permet de les renforcer plus tard ([`PasswordHasher::needs_rehash`]).
//! - Les jetons d'API sont 256 bits aléatoires encodés en base64url (préfixe `stx_`). Le
//!   serveur n'en stocke que l'empreinte SHA-256 ([`StoredToken`]) : une fuite de la base ne
//!   révèle aucun jeton utilisable. La comparaison se fait en temps constant.

use std::fmt;
use std::time::Duration;

use argon2::password_hash::phc::PasswordHash;
use argon2::{Algorithm, PasswordHasher as _, PasswordVerifier as _, Version};
use base64ct::{Base64UrlUnpadded, Encoding};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use zeroize::Zeroizing;

use crate::crypto::{CryptoError, KdfParams};

/// Longueur maximale d'un mot de passe, en octets (au-delà : refus, pour borner le travail).
pub const MAX_PASSWORD_LEN: usize = 1024;
/// Taille du sel des hachages de mots de passe, en octets (RFC 9106).
pub const PASSWORD_SALT_LEN: usize = 16;
/// Préfixe des jetons d'API, pour les reconnaître (journaux, détection de fuites).
pub const TOKEN_PREFIX: &str = "stx_";
/// Entropie d'un jeton d'API, en octets (256 bits).
pub const TOKEN_BYTES: usize = 32;

/// Erreur d'authentification.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum AuthError {
    /// Mot de passe vide.
    #[error("le mot de passe est vide")]
    EmptyPassword,
    /// Mot de passe trop long.
    #[error("le mot de passe dépasse {MAX_PASSWORD_LEN} octets")]
    PasswordTooLong,
    /// Mot de passe ou jeton refusé. Volontairement indistinct : ne dit pas si l'identifiant
    /// existe.
    #[error("identifiants invalides")]
    InvalidCredentials,
    /// Jeton reconnu mais expiré.
    #[error("jeton d'API expiré")]
    TokenExpired,
    /// Durée de validité nulle ou débordant l'horloge.
    #[error("durée de validité du jeton invalide")]
    InvalidLifetime,
    /// Hachage stocké illisible, ou algorithme autre qu'Argon2id.
    #[error("hachage de mot de passe stocké invalide : {0}")]
    InvalidStoredHash(String),
    /// Erreur cryptographique (paramètres, aléa).
    #[error(transparent)]
    Crypto(#[from] CryptoError),
}

impl From<getrandom::Error> for AuthError {
    fn from(_: getrandom::Error) -> Self {
        Self::Crypto(CryptoError::Random)
    }
}

fn check_password(password: &str) -> Result<(), AuthError> {
    if password.is_empty() {
        Err(AuthError::EmptyPassword)
    } else if password.len() > MAX_PASSWORD_LEN {
        Err(AuthError::PasswordTooLong)
    } else {
        Ok(())
    }
}

/// Hachage et vérification des mots de passe des comptes (Argon2id, format PHC).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PasswordHasher {
    params: KdfParams,
}

impl PasswordHasher {
    /// Hacheur avec les paramètres donnés pour les **nouveaux** hachages.
    #[must_use]
    pub const fn new(params: KdfParams) -> Self {
        Self { params }
    }

    /// Paramètres des nouveaux hachages.
    #[must_use]
    pub const fn params(&self) -> KdfParams {
        self.params
    }

    /// Hache un mot de passe ; renvoie la chaîne PHC à stocker.
    ///
    /// # Errors
    /// Mot de passe vide ou trop long ; aléa indisponible.
    pub fn hash(&self, password: &str) -> Result<String, AuthError> {
        check_password(password)?;
        let mut salt = [0u8; PASSWORD_SALT_LEN];
        getrandom::fill(&mut salt)?;
        let hash = self
            .params
            .argon2()?
            .hash_password_with_salt(password.as_bytes(), &salt)
            .map_err(|e| CryptoError::Kdf(e.to_string()))?;
        Ok(hash.to_string())
    }

    /// Vérifie un mot de passe contre une chaîne PHC stockée, avec les paramètres qu'elle
    /// contient (bornés comme [`KdfParams::new`]). La comparaison finale est en temps constant.
    ///
    /// # Errors
    /// [`AuthError::InvalidCredentials`] si le mot de passe ne correspond pas ;
    /// [`AuthError::InvalidStoredHash`] si la chaîne est illisible ou n'est pas de l'Argon2id.
    pub fn verify(&self, password: &str, stored: &str) -> Result<(), AuthError> {
        if password.is_empty() || password.len() > MAX_PASSWORD_LEN {
            return Err(AuthError::InvalidCredentials);
        }
        let parsed = parse_stored(stored)?;
        let argon2 = stored_params(&parsed)?.argon2()?;
        argon2
            .verify_password(password.as_bytes(), &parsed)
            .map_err(|e| match e {
                argon2::password_hash::Error::PasswordInvalid => AuthError::InvalidCredentials,
                other => AuthError::InvalidStoredHash(other.to_string()),
            })
    }

    /// Vrai si le hachage stocké utilise des paramètres différents de ceux du hacheur : à
    /// rehacher lors de la prochaine connexion réussie.
    ///
    /// # Errors
    /// [`AuthError::InvalidStoredHash`].
    pub fn needs_rehash(&self, stored: &str) -> Result<bool, AuthError> {
        let parsed = parse_stored(stored)?;
        Ok(stored_params(&parsed)? != self.params)
    }
}

impl Default for PasswordHasher {
    /// Profil serveur : [`KdfParams::MODERATE`] (19 Mio, 2 passes, 1 voie — OWASP).
    fn default() -> Self {
        Self::new(KdfParams::MODERATE)
    }
}

/// Analyse une chaîne PHC et exige Argon2id v0x13.
fn parse_stored(stored: &str) -> Result<PasswordHash, AuthError> {
    let parsed =
        PasswordHash::new(stored).map_err(|e| AuthError::InvalidStoredHash(e.to_string()))?;
    if parsed.algorithm.as_str() != Algorithm::Argon2id.ident().as_str() {
        return Err(AuthError::InvalidStoredHash(format!(
            "algorithme {} refusé (Argon2id exigé)",
            parsed.algorithm
        )));
    }
    if parsed.version != Some(Version::V0x13.into()) {
        return Err(AuthError::InvalidStoredHash(
            "version d'Argon2 refusée (0x13 exigée)".into(),
        ));
    }
    Ok(parsed)
}

/// Paramètres d'un hachage stocké, validés par [`KdfParams::new`] (anti-déni de service).
fn stored_params(parsed: &PasswordHash) -> Result<KdfParams, AuthError> {
    let params = argon2::Params::try_from(parsed)
        .map_err(|e| AuthError::InvalidStoredHash(e.to_string()))?;
    KdfParams::new(params.m_cost(), params.t_cost(), params.p_cost())
        .map_err(|e| AuthError::InvalidStoredHash(e.to_string()))
}

/// Jeton d'API en clair, à remettre **une seule fois** à son titulaire.
///
/// Il n'est jamais stocké côté serveur ; il est effacé de la mémoire à sa libération et
/// n'apparaît pas dans `Debug`.
pub struct ApiToken {
    secret: Zeroizing<String>,
}

impl ApiToken {
    /// Texte du jeton (`stx_` suivi de 43 caractères base64url).
    #[must_use]
    pub fn expose(&self) -> &str {
        &self.secret
    }
}

impl fmt::Debug for ApiToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ApiToken(<masqué>)")
    }
}

/// Empreinte SHA-256 d'un jeton : seule forme stockée, utilisable comme clé de recherche.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct TokenDigest([u8; 32]);

impl TokenDigest {
    /// Empreinte du jeton présenté par un client.
    #[must_use]
    pub fn of(presented: &str) -> Self {
        Self(Sha256::digest(presented.as_bytes()).into())
    }

    /// Comparaison en temps constant.
    #[must_use]
    pub fn matches(&self, other: &Self) -> bool {
        bool::from(self.0.ct_eq(&other.0))
    }
}

impl fmt::Display for TokenDigest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.iter().try_for_each(|b| write!(f, "{b:02x}"))
    }
}

impl fmt::Debug for TokenDigest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "TokenDigest({self})")
    }
}

impl From<TokenDigest> for String {
    fn from(digest: TokenDigest) -> Self {
        digest.to_string()
    }
}

impl TryFrom<String> for TokenDigest {
    type Error = AuthError;

    fn try_from(hex: String) -> Result<Self, Self::Error> {
        let invalid = || AuthError::InvalidStoredHash("empreinte de jeton invalide".into());
        if hex.len() != 64 || !hex.is_ascii() {
            return Err(invalid());
        }
        let mut out = [0u8; 32];
        for (i, byte) in out.iter_mut().enumerate() {
            *byte = u8::from_str_radix(&hex[2 * i..2 * i + 2], 16).map_err(|_| invalid())?;
        }
        Ok(Self(out))
    }
}

/// Ce que le serveur conserve d'un jeton d'API : empreinte, libellé et dates.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredToken {
    /// Empreinte SHA-256 du jeton.
    pub digest: TokenDigest,
    /// Libellé choisi par l'utilisateur (« intégration CI »…).
    pub label: String,
    /// Émission, en millisecondes depuis l'époque Unix (UTC).
    pub created_at_ms: u64,
    /// Expiration (exclue), en millisecondes depuis l'époque Unix (UTC).
    pub expires_at_ms: u64,
}

impl StoredToken {
    /// Vérifie un jeton présenté : empreinte comparée en temps constant, puis expiration.
    ///
    /// # Errors
    /// [`AuthError::InvalidCredentials`] si le jeton ne correspond pas ;
    /// [`AuthError::TokenExpired`] s'il correspond mais a expiré.
    pub fn verify(&self, presented: &str, now_ms: u64) -> Result<(), AuthError> {
        if !self.digest.matches(&TokenDigest::of(presented)) {
            return Err(AuthError::InvalidCredentials);
        }
        if now_ms >= self.expires_at_ms {
            return Err(AuthError::TokenExpired);
        }
        Ok(())
    }
}

/// Émet un jeton d'API valable `lifetime` à partir de `now_ms`.
///
/// Renvoie le jeton en clair (à remettre au client) et sa forme stockable.
///
/// # Errors
/// [`AuthError::InvalidLifetime`] pour une durée nulle ou trop longue ; aléa indisponible.
pub fn issue_token(
    label: impl Into<String>,
    now_ms: u64,
    lifetime: Duration,
) -> Result<(ApiToken, StoredToken), AuthError> {
    let lifetime_ms =
        u64::try_from(lifetime.as_millis()).map_err(|_| AuthError::InvalidLifetime)?;
    if lifetime_ms == 0 {
        return Err(AuthError::InvalidLifetime);
    }
    let expires_at_ms = now_ms
        .checked_add(lifetime_ms)
        .ok_or(AuthError::InvalidLifetime)?;
    let mut raw = Zeroizing::new([0u8; TOKEN_BYTES]);
    getrandom::fill(raw.as_mut())?;
    let mut secret = Zeroizing::new(String::with_capacity(TOKEN_PREFIX.len() + 43));
    secret.push_str(TOKEN_PREFIX);
    secret.push_str(&Zeroizing::new(Base64UrlUnpadded::encode_string(
        raw.as_ref(),
    )));
    let stored = StoredToken {
        digest: TokenDigest::of(&secret),
        label: label.into(),
        created_at_ms: now_ms,
        expires_at_ms,
    };
    Ok((ApiToken { secret }, stored))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fast() -> PasswordHasher {
        PasswordHasher::new(KdfParams::new(64, 1, 1).unwrap())
    }

    #[test]
    fn hash_and_verify() {
        let hasher = fast();
        let stored = hasher.hash("tapis à la river").unwrap();
        assert!(stored.starts_with("$argon2id$v=19$m=64,t=1,p=1$"));
        hasher.verify("tapis à la river", &stored).unwrap();
        assert!(matches!(
            hasher.verify("tapis a la river", &stored),
            Err(AuthError::InvalidCredentials)
        ));
        assert!(matches!(
            hasher.verify("", &stored),
            Err(AuthError::InvalidCredentials)
        ));
        assert_ne!(
            stored,
            hasher.hash("tapis à la river").unwrap(),
            "sel aléatoire"
        );
    }

    /// Vecteur de référence (phc-winner-argon2) : Argon2id v0x13, m = 64 Mio, t = 2, p = 1.
    #[test]
    fn verifies_reference_phc_string() {
        let reference = "$argon2id$v=19$m=65536,t=2,p=1$c29tZXNhbHQ$CTFhFdXPJO1aFaMaO6Mm5c8y7cJHAph8ArZWb2GRPPc";
        let hasher = fast();
        hasher.verify("password", reference).unwrap();
        assert!(hasher.verify("passwore", reference).is_err());
        assert!(hasher.needs_rehash(reference).unwrap());
        assert!(
            !PasswordHasher::new(KdfParams::new(65536, 2, 1).unwrap())
                .needs_rehash(reference)
                .unwrap()
        );
    }

    #[test]
    fn rejects_bad_stored_hashes_and_passwords() {
        let hasher = fast();
        assert!(matches!(hasher.hash(""), Err(AuthError::EmptyPassword)));
        assert!(matches!(
            hasher.hash(&"x".repeat(MAX_PASSWORD_LEN + 1)),
            Err(AuthError::PasswordTooLong)
        ));
        for stored in [
            "n'importe quoi",
            // Argon2i : refusé (pas d'affaiblissement possible par la base).
            "$argon2i$v=19$m=65536,t=2,p=1$c29tZXNhbHQ$wWKIMhR9lyDFvRz9YTZweHKfbftvj+qf+YFY4NeBbtA",
            // Mémoire démesurée : refusée avant tout calcul.
            "$argon2id$v=19$m=4294967295,t=2,p=1$c29tZXNhbHQ$CTFhFdXPJO1aFaMaO6Mm5c8y7cJHAph8ArZWb2GRPPc",
        ] {
            assert!(
                matches!(
                    hasher.verify("password", stored),
                    Err(AuthError::InvalidStoredHash(_))
                ),
                "{stored}"
            );
        }
    }

    #[test]
    fn api_token_lifecycle() {
        let now = 1_700_000_000_000;
        let (token, stored) = issue_token("CI", now, Duration::from_secs(3600)).unwrap();
        let text = token.expose();
        assert!(text.starts_with(TOKEN_PREFIX));
        assert_eq!(
            text.len(),
            TOKEN_PREFIX.len() + 43,
            "256 bits en base64url sans remplissage"
        );
        assert_eq!(
            Base64UrlUnpadded::decode_vec(&text[TOKEN_PREFIX.len()..])
                .unwrap()
                .len(),
            TOKEN_BYTES
        );
        assert!(!format!("{token:?}").contains(&text[4..]));

        stored.verify(text, now).unwrap();
        stored.verify(text, now + 3_599_999).unwrap();
        assert!(matches!(
            stored.verify(text, now + 3_600_000),
            Err(AuthError::TokenExpired)
        ));
        let mut forged = text.to_owned();
        forged.pop();
        forged.push(if text.ends_with('A') { 'B' } else { 'A' });
        assert!(matches!(
            stored.verify(&forged, now),
            Err(AuthError::InvalidCredentials)
        ));

        // Seule l'empreinte est sérialisée, jamais le jeton.
        let json = serde_json::to_string(&stored).unwrap();
        assert!(!json.contains(&text[TOKEN_PREFIX.len()..]));
        assert_eq!(serde_json::from_str::<StoredToken>(&json).unwrap(), stored);
        assert_eq!(TokenDigest::of(text), stored.digest);

        let (other, _) = issue_token("CI", now, Duration::from_secs(1)).unwrap();
        assert_ne!(other.expose(), text);
    }

    #[test]
    fn token_digest_is_sha256() {
        assert_eq!(
            TokenDigest::of("abc").to_string(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert!(serde_json::from_str::<TokenDigest>("\"zz\"").is_err());
    }

    #[test]
    fn invalid_lifetimes() {
        assert!(matches!(
            issue_token("x", 0, Duration::ZERO),
            Err(AuthError::InvalidLifetime)
        ));
        assert!(matches!(
            issue_token("x", u64::MAX, Duration::from_millis(1)),
            Err(AuthError::InvalidLifetime)
        ));
    }
}
