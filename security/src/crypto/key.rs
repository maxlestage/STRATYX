//! Clé de données et primitives AEAD (XChaCha20-Poly1305).

use std::fmt;

use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

use super::CryptoError;

/// Taille des clés symétriques, en octets (256 bits).
pub const KEY_LEN: usize = 32;
/// Taille du nonce de XChaCha20-Poly1305, en octets (192 bits : tirage aléatoire sans risque).
pub const NONCE_LEN: usize = 24;
/// Taille de l'étiquette d'authentification Poly1305, en octets.
pub const TAG_LEN: usize = 16;
/// Taille d'un identifiant de clé, en octets.
pub const KEY_ID_LEN: usize = 16;

/// Remplit un tableau avec l'aléa du système d'exploitation.
pub(crate) fn random_array<const N: usize>() -> Result<[u8; N], CryptoError> {
    let mut out = [0u8; N];
    getrandom::fill(&mut out)?;
    Ok(out)
}

/// Identifiant public et aléatoire d'une clé de données.
///
/// Il est écrit en clair dans chaque coffre (et authentifié) : il permet de signaler
/// « mauvaise clé » sans tenter de déchiffrer, et de retrouver la clé dans un trousseau.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct KeyId([u8; KEY_ID_LEN]);

impl KeyId {
    /// Construit un identifiant à partir de ses octets.
    #[must_use]
    pub const fn from_bytes(bytes: [u8; KEY_ID_LEN]) -> Self {
        Self(bytes)
    }

    /// Octets de l'identifiant.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; KEY_ID_LEN] {
        &self.0
    }
}

impl fmt::Display for KeyId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.iter().try_for_each(|b| write!(f, "{b:02x}"))
    }
}

impl fmt::Debug for KeyId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "KeyId({self})")
    }
}

/// Clé de données : chiffre réellement les mains, vidéos et sauvegardes.
///
/// Elle est tirée au hasard ([`DataKey::generate`]) puis protégée par un mot de passe via un
/// [`KeySlot`](super::KeySlot). Changer de mot de passe ne change pas la clé de données :
/// aucune donnée n'est à rechiffrer. Le secret est effacé de la mémoire à la libération et
/// n'apparaît jamais dans `Debug`.
pub struct DataKey {
    id: KeyId,
    secret: Zeroizing<[u8; KEY_LEN]>,
}

impl DataKey {
    /// Tire une nouvelle clé de données et son identifiant.
    ///
    /// # Errors
    /// [`CryptoError::Random`] si l'aléa du système est indisponible.
    pub fn generate() -> Result<Self, CryptoError> {
        let id = KeyId(random_array()?);
        let mut secret = Zeroizing::new([0u8; KEY_LEN]);
        getrandom::fill(secret.as_mut())?;
        Ok(Self { id, secret })
    }

    /// Reconstitue une clé à partir de son identifiant et de son secret ; le tableau source
    /// est effacé.
    #[must_use]
    pub fn from_parts(id: KeyId, secret: &mut [u8; KEY_LEN]) -> Self {
        let key = Self {
            id,
            secret: Zeroizing::new(*secret),
        };
        secret.zeroize();
        key
    }

    /// Identifiant public de la clé.
    #[must_use]
    pub const fn id(&self) -> KeyId {
        self.id
    }

    /// Octets secrets, pour l'enveloppe uniquement.
    pub(crate) fn secret_bytes(&self) -> &[u8; KEY_LEN] {
        &self.secret
    }

    /// Instance AEAD (sa copie de la clé est effacée à sa libération).
    pub(crate) fn cipher(&self) -> XChaCha20Poly1305 {
        cipher_from(&self.secret)
    }
}

impl fmt::Debug for DataKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DataKey")
            .field("id", &self.id)
            .field("secret", &"<masqué>")
            .finish()
    }
}

impl ZeroizeOnDrop for DataKey {}

/// Instance XChaCha20-Poly1305 pour une clé brute.
pub(crate) fn cipher_from(key: &[u8; KEY_LEN]) -> XChaCha20Poly1305 {
    XChaCha20Poly1305::new(&(*key).into())
}

/// Chiffre et authentifie `plaintext` ; renvoie le texte chiffré suivi de l'étiquette.
pub(crate) fn aead_seal(
    cipher: &XChaCha20Poly1305,
    nonce: &[u8; NONCE_LEN],
    aad: &[u8],
    plaintext: &[u8],
) -> Result<Vec<u8>, CryptoError> {
    let nonce = XNonce::from(*nonce);
    cipher
        .encrypt(
            &nonce,
            Payload {
                msg: plaintext,
                aad,
            },
        )
        .map_err(|_| CryptoError::Malformed("message trop long pour XChaCha20-Poly1305"))
}

/// Vérifie puis déchiffre `ciphertext` (étiquette incluse).
pub(crate) fn aead_open(
    cipher: &XChaCha20Poly1305,
    nonce: &[u8; NONCE_LEN],
    aad: &[u8],
    ciphertext: &[u8],
) -> Result<Vec<u8>, CryptoError> {
    let nonce = XNonce::from(*nonce);
    cipher
        .decrypt(
            &nonce,
            Payload {
                msg: ciphertext,
                aad,
            },
        )
        .map_err(|_| CryptoError::AuthenticationFailed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unhex(s: &str) -> Vec<u8> {
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
            .collect()
    }

    /// Vecteur de draft-irtf-cfrg-xchacha-03, annexe A.3.1 (AEAD_XChaCha20_Poly1305).
    #[test]
    fn xchacha20poly1305_draft_vector() {
        let key: [u8; KEY_LEN] =
            unhex("808182838485868788898a8b8c8d8e8f909192939495969798999a9b9c9d9e9f")
                .try_into()
                .unwrap();
        let nonce: [u8; NONCE_LEN] = unhex("404142434445464748494a4b4c4d4e4f5051525354555657")
            .try_into()
            .unwrap();
        let aad = unhex("50515253c0c1c2c3c4c5c6c7");
        let plaintext = b"Ladies and Gentlemen of the class of '99: If I could offer you only one tip for the future, sunscreen would be it.";
        let expected = unhex(concat!(
            "bd6d179d3e83d43b9576579493c0e939572a1700252bfaccbed2902c21396cbb",
            "731c7f1b0b4aa6440bf3a82f4eda7e39ae64c6708c54c216cb96b72e1213b452",
            "2f8c9ba40db5d945b11b69b982c1bb9e3f3fac2bc369488f76b2383565d3fff9",
            "21f9664c97637da9768812f615c68b13b52e",
            "c0875924c1c7987947deafd8780acf49",
        ));

        let cipher = cipher_from(&key);
        let sealed = aead_seal(&cipher, &nonce, &aad, plaintext).unwrap();
        assert_eq!(sealed, expected);
        assert_eq!(
            aead_open(&cipher, &nonce, &aad, &sealed).unwrap(),
            plaintext
        );

        let mut tampered = sealed.clone();
        tampered[10] ^= 1;
        assert!(matches!(
            aead_open(&cipher, &nonce, &aad, &tampered),
            Err(CryptoError::AuthenticationFailed)
        ));
        assert!(aead_open(&cipher, &nonce, b"autre", &sealed).is_err());
    }

    #[test]
    fn data_key_debug_hides_secret() {
        let mut raw = [7u8; KEY_LEN];
        let key = DataKey::from_parts(KeyId::from_bytes([1; KEY_ID_LEN]), &mut raw);
        assert_eq!(raw, [0u8; KEY_LEN], "la source est effacée");
        let debug = format!("{key:?}");
        assert!(debug.contains("masqué"));
        assert!(!debug.contains("7, 7"));
        assert_eq!(key.id().to_string(), "01".repeat(KEY_ID_LEN));
    }

    #[test]
    fn generated_keys_differ() {
        let a = DataKey::generate().unwrap();
        let b = DataKey::generate().unwrap();
        assert_ne!(a.id(), b.id());
        assert_ne!(*a.secret, *b.secret);
    }
}
