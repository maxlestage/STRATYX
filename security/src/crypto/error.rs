//! Erreurs du chiffrement au repos.

use std::io;

use super::vault::PayloadKind;

/// Erreur de chiffrement, de déchiffrement ou de lecture d'un coffre.
///
/// Les échecs d'authentification ne révèlent volontairement pas leur cause exacte
/// (mot de passe faux ou en-tête modifié, bloc altéré ou réordonné) : distinguer ces cas
/// aiderait un attaquant sans aider l'utilisateur.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum CryptoError {
    /// Les premiers octets ne correspondent à aucun format Stratyx.
    #[error("format non reconnu : ce fichier n'est pas un coffre Stratyx")]
    BadMagic,
    /// Le coffre a été écrit par une version plus récente (ou inconnue) du format.
    #[error("version de format non prise en charge : {0}")]
    UnsupportedVersion(u16),
    /// L'en-tête est incohérent ou incomplet.
    #[error("coffre malformé : {0}")]
    Malformed(&'static str),
    /// Paramètres Argon2id hors des bornes acceptées.
    #[error("paramètres de dérivation de clé invalides : {0}")]
    InvalidKdfParams(String),
    /// La dérivation Argon2id a échoué (mémoire insuffisante, par exemple).
    #[error("échec de la dérivation de clé Argon2id : {0}")]
    Kdf(String),
    /// Le mot de passe ne déverrouille pas l'emplacement de clé (ou celui-ci a été modifié).
    #[error("mot de passe incorrect, ou emplacement de clé altéré")]
    WrongPassword,
    /// Le coffre a été chiffré avec une autre clé de données que celle fournie.
    #[error("ce coffre a été chiffré avec une autre clé de données")]
    KeyMismatch,
    /// Déverrouillage par mot de passe demandé sur un coffre sans emplacement de clé.
    #[error("ce coffre ne contient pas d'emplacement de clé protégé par mot de passe")]
    NoPasswordSlot,
    /// Le contenu ne correspond pas au type de coffre attendu.
    #[error("type de coffre inattendu : {found} au lieu de {expected}")]
    WrongPayloadKind {
        /// Type attendu par l'appelant.
        expected: PayloadKind,
        /// Type lu dans l'en-tête.
        found: PayloadKind,
    },
    /// Le texte chiffré, l'en-tête ou un bloc a été modifié, supprimé ou réordonné.
    #[error("données altérées : échec de l'authentification")]
    AuthenticationFailed,
    /// Le flux s'arrête avant son bloc final : fichier tronqué ou écriture interrompue.
    #[error("flux chiffré tronqué : le bloc final est absent")]
    Truncated,
    /// Taille de bloc de flux hors des bornes acceptées.
    #[error("taille de bloc invalide : {0} octets")]
    InvalidChunkSize(u32),
    /// Le compteur de blocs du flux est épuisé (plus de 2³² blocs).
    #[error("flux trop long : compteur de blocs épuisé")]
    StreamTooLong,
    /// Le générateur aléatoire du système d'exploitation n'a pas répondu.
    #[error("générateur aléatoire du système indisponible")]
    Random,
    /// Erreur d'entrée-sortie sous-jacente.
    #[error("erreur d'entrée-sortie : {0}")]
    Io(#[from] io::Error),
}

impl CryptoError {
    /// Récupère une [`CryptoError`] transportée dans une [`io::Error`] (adaptateurs
    /// [`Read`](io::Read) et [`Write`](io::Write)), ou enveloppe l'erreur d'entrée-sortie.
    #[must_use]
    pub fn from_io(error: io::Error) -> Self {
        if error.get_ref().is_some_and(|inner| inner.is::<Self>()) {
            if let Some(inner) = error.into_inner() {
                return match inner.downcast::<Self>() {
                    Ok(crypto) => *crypto,
                    Err(other) => Self::Io(io::Error::other(other)),
                };
            }
            return Self::Malformed("erreur interne d'entrée-sortie");
        }
        Self::Io(error)
    }
}

impl From<CryptoError> for io::Error {
    fn from(error: CryptoError) -> Self {
        let kind = match error {
            CryptoError::Io(inner) => return inner,
            CryptoError::Truncated => io::ErrorKind::UnexpectedEof,
            CryptoError::Random | CryptoError::Kdf(_) => io::ErrorKind::Other,
            _ => io::ErrorKind::InvalidData,
        };
        io::Error::new(kind, error)
    }
}

impl From<getrandom::Error> for CryptoError {
    fn from(_: getrandom::Error) -> Self {
        Self::Random
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn io_round_trip_preserves_variant() {
        let io_err: io::Error = CryptoError::Truncated.into();
        assert_eq!(io_err.kind(), io::ErrorKind::UnexpectedEof);
        assert!(matches!(
            CryptoError::from_io(io_err),
            CryptoError::Truncated
        ));

        let plain = io::Error::new(io::ErrorKind::NotFound, "absent");
        assert!(matches!(CryptoError::from_io(plain), CryptoError::Io(_)));
    }
}
