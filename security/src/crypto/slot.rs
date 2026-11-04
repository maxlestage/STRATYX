//! Emplacement de clé : la clé de données enveloppée par une clé dérivée du mot de passe.

use std::fmt;

use zeroize::{Zeroize, Zeroizing};

use super::CryptoError;
use super::kdf::{KDF_PARAMS_LEN, KdfParams, SALT_LEN};
use super::key::{
    DataKey, KEY_ID_LEN, KEY_LEN, KeyId, NONCE_LEN, TAG_LEN, aead_open, aead_seal, cipher_from,
    random_array,
};

/// Signature des fichiers trousseau autonomes (`KeySlot::to_bytes`).
pub const KEYRING_MAGIC: [u8; 8] = *b"STRXKEY\0";
/// Version du format des trousseaux.
pub const KEYRING_VERSION: u16 = 1;

/// Séparation de domaine des données associées de l'enveloppe.
const SLOT_AAD_DOMAIN: &[u8] = b"STRATYX/key-slot/v1";

/// Taille de la clé enveloppée : clé de données et étiquette.
const WRAPPED_LEN: usize = KEY_LEN + TAG_LEN;

/// Taille de l'encodage binaire d'un emplacement de clé.
pub(crate) const SLOT_LEN: usize = KEY_ID_LEN + KDF_PARAMS_LEN + SALT_LEN + NONCE_LEN + WRAPPED_LEN;

/// Clé de données chiffrée par une clé dérivée du mot de passe (Argon2id).
///
/// ```text
/// mot de passe ──Argon2id(sel, paramètres)──▶ clé d'enveloppe
/// clé d'enveloppe ──XChaCha20-Poly1305──▶ clé de données enveloppée
/// clé de données ──XChaCha20-Poly1305──▶ mains, vidéos, sauvegardes
/// ```
///
/// Changer de mot de passe ([`KeySlot::rewrap`]) ne réécrit que cet emplacement : les
/// données restent chiffrées par la même clé. Les données associées de l'enveloppe couvrent
/// l'identifiant de clé, les paramètres et le sel.
///
/// Encodage (133 octets) : identifiant de clé (16), paramètres (13), sel (32), nonce (24),
/// clé enveloppée et étiquette (48).
#[derive(Clone, PartialEq, Eq)]
pub struct KeySlot {
    key_id: KeyId,
    params: KdfParams,
    salt: [u8; SALT_LEN],
    nonce: [u8; NONCE_LEN],
    wrapped: [u8; WRAPPED_LEN],
}

impl KeySlot {
    /// Tire une nouvelle clé de données et la protège par `password`.
    ///
    /// # Errors
    /// Aléa indisponible ou échec d'Argon2id.
    pub fn create(password: &[u8], params: KdfParams) -> Result<(Self, DataKey), CryptoError> {
        let key = DataKey::generate()?;
        let slot = Self::seal(&key, password, params)?;
        Ok((slot, key))
    }

    /// Enveloppe une clé de données existante avec un nouveau sel.
    ///
    /// # Errors
    /// Aléa indisponible ou échec d'Argon2id.
    pub fn seal(key: &DataKey, password: &[u8], params: KdfParams) -> Result<Self, CryptoError> {
        let salt = random_array::<SALT_LEN>()?;
        let nonce = random_array::<NONCE_LEN>()?;
        let wrapping_key = params.derive_key(password, &salt)?;
        let aad = slot_aad(key.id(), &params, &salt);
        let sealed = aead_seal(
            &cipher_from(&wrapping_key),
            &nonce,
            &aad,
            key.secret_bytes(),
        )?;
        let wrapped: [u8; WRAPPED_LEN] = sealed
            .try_into()
            .map_err(|_| CryptoError::Malformed("taille d'enveloppe inattendue"))?;
        Ok(Self {
            key_id: key.id(),
            params,
            salt,
            nonce,
            wrapped,
        })
    }

    /// Déverrouille la clé de données.
    ///
    /// # Errors
    /// [`CryptoError::WrongPassword`] si le mot de passe est faux ou si l'emplacement a été
    /// modifié (identifiant, paramètres, sel ou clé enveloppée).
    pub fn unlock(&self, password: &[u8]) -> Result<DataKey, CryptoError> {
        let wrapping_key = self.params.derive_key(password, &self.salt)?;
        let aad = slot_aad(self.key_id, &self.params, &self.salt);
        let opened = aead_open(
            &cipher_from(&wrapping_key),
            &self.nonce,
            &aad,
            &self.wrapped,
        )
        .map_err(|_| CryptoError::WrongPassword)?;
        let mut opened = Zeroizing::new(opened);
        let mut secret: [u8; KEY_LEN] = opened
            .as_slice()
            .try_into()
            .map_err(|_| CryptoError::Malformed("taille de clé inattendue"))?;
        opened.zeroize();
        Ok(DataKey::from_parts(self.key_id, &mut secret))
    }

    /// Change le mot de passe (et éventuellement les paramètres) sans changer la clé de
    /// données : renvoie un nouvel emplacement, avec un nouveau sel et un nouveau nonce.
    ///
    /// # Errors
    /// [`CryptoError::WrongPassword`] si `old_password` est faux.
    pub fn rewrap(
        &self,
        old_password: &[u8],
        new_password: &[u8],
        new_params: KdfParams,
    ) -> Result<Self, CryptoError> {
        let key = self.unlock(old_password)?;
        Self::seal(&key, new_password, new_params)
    }

    /// Identifiant de la clé de données protégée.
    #[must_use]
    pub const fn key_id(&self) -> KeyId {
        self.key_id
    }

    /// Paramètres Argon2id de cet emplacement.
    #[must_use]
    pub const fn params(&self) -> KdfParams {
        self.params
    }

    /// Encodage binaire de l'emplacement (sans signature ni version).
    pub(crate) fn encode(&self) -> [u8; SLOT_LEN] {
        let mut out = [0u8; SLOT_LEN];
        let mut at = 0;
        for part in [
            self.key_id.as_bytes().as_slice(),
            &self.params.encode(),
            &self.salt,
            &self.nonce,
            &self.wrapped,
        ] {
            out[at..at + part.len()].copy_from_slice(part);
            at += part.len();
        }
        out
    }

    /// Décode l'encodage produit par [`KeySlot::encode`].
    pub(crate) fn decode(bytes: &[u8; SLOT_LEN]) -> Result<Self, CryptoError> {
        let mut cursor = bytes.as_slice();
        let mut take = |n: usize| {
            let (head, tail) = cursor.split_at(n);
            cursor = tail;
            head
        };
        let key_id = KeyId::from_bytes(fixed(take(KEY_ID_LEN)));
        let params = KdfParams::decode(&fixed(take(KDF_PARAMS_LEN)))?;
        let salt = fixed(take(SALT_LEN));
        let nonce = fixed(take(NONCE_LEN));
        let wrapped = fixed(take(WRAPPED_LEN));
        Ok(Self {
            key_id,
            params,
            salt,
            nonce,
            wrapped,
        })
    }

    /// Fichier trousseau autonome : signature `STRXKEY\0`, version (`u16`), emplacement.
    ///
    /// Sert à protéger la clé des données locales (base, vidéos) : chaque fichier de données
    /// ne porte alors que l'identifiant de clé, et changer de mot de passe ne réécrit que
    /// ce trousseau.
    #[must_use]
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(KEYRING_MAGIC.len() + 2 + SLOT_LEN);
        out.extend_from_slice(&KEYRING_MAGIC);
        out.extend_from_slice(&KEYRING_VERSION.to_be_bytes());
        out.extend_from_slice(&self.encode());
        out
    }

    /// Lit un fichier trousseau produit par [`KeySlot::to_bytes`].
    ///
    /// # Errors
    /// Signature, version ou taille invalides ; paramètres hors bornes.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, CryptoError> {
        let rest = bytes
            .strip_prefix(KEYRING_MAGIC.as_slice())
            .ok_or(CryptoError::BadMagic)?;
        let (version, body) = rest
            .split_first_chunk::<2>()
            .ok_or(CryptoError::Malformed("trousseau tronqué"))?;
        let version = u16::from_be_bytes(*version);
        if version != KEYRING_VERSION {
            return Err(CryptoError::UnsupportedVersion(version));
        }
        let body: &[u8; SLOT_LEN] = body
            .try_into()
            .map_err(|_| CryptoError::Malformed("taille de trousseau invalide"))?;
        Self::decode(body)
    }
}

impl fmt::Debug for KeySlot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("KeySlot")
            .field("key_id", &self.key_id)
            .field("params", &self.params)
            .finish_non_exhaustive()
    }
}

/// Données associées de l'enveloppe : domaine, identifiant de clé, paramètres, sel.
fn slot_aad(key_id: KeyId, params: &KdfParams, salt: &[u8; SALT_LEN]) -> Vec<u8> {
    let mut aad =
        Vec::with_capacity(SLOT_AAD_DOMAIN.len() + KEY_ID_LEN + KDF_PARAMS_LEN + SALT_LEN);
    aad.extend_from_slice(SLOT_AAD_DOMAIN);
    aad.extend_from_slice(key_id.as_bytes());
    aad.extend_from_slice(&params.encode());
    aad.extend_from_slice(salt);
    aad
}

/// Copie une tranche de taille connue dans un tableau.
fn fixed<const N: usize>(bytes: &[u8]) -> [u8; N] {
    let mut out = [0u8; N];
    out.copy_from_slice(bytes);
    out
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Paramètres réduits pour des tests rapides (ne jamais utiliser en production).
    pub(crate) fn fast() -> KdfParams {
        KdfParams::new(64, 1, 1).unwrap()
    }

    #[test]
    fn create_unlock_round_trip() {
        let (slot, key) = KeySlot::create(b"correct horse", fast()).unwrap();
        assert_eq!(slot.key_id(), key.id());
        let unlocked = slot.unlock(b"correct horse").unwrap();
        assert_eq!(unlocked.id(), key.id());
        assert_eq!(unlocked.secret_bytes(), key.secret_bytes());
    }

    #[test]
    fn wrong_password_is_rejected() {
        let (slot, _) = KeySlot::create(b"correct horse", fast()).unwrap();
        assert!(matches!(
            slot.unlock(b"correct horsE"),
            Err(CryptoError::WrongPassword)
        ));
        assert!(matches!(slot.unlock(b""), Err(CryptoError::WrongPassword)));
    }

    #[test]
    fn every_field_is_authenticated() {
        let (slot, _) = KeySlot::create(b"pw", fast()).unwrap();
        let encoded = slot.encode();
        // Chaque octet modifié (identifiant, paramètres, sel, nonce, enveloppe) est détecté.
        for i in 0..SLOT_LEN {
            let mut bytes = encoded;
            bytes[i] ^= 0x01;
            // Des paramètres devenus invalides sont refusés dès la lecture.
            if let Ok(tampered) = KeySlot::decode(&bytes) {
                assert!(
                    tampered.unlock(b"pw").is_err(),
                    "octet {i} modifié sans être détecté"
                );
            }
        }
    }

    #[test]
    fn rewrap_keeps_data_key() {
        let (slot, key) = KeySlot::create(b"ancien", fast()).unwrap();
        let new_params = KdfParams::new(128, 2, 1).unwrap();
        let rotated = slot.rewrap(b"ancien", b"nouveau", new_params).unwrap();
        assert_eq!(rotated.key_id(), key.id());
        assert_eq!(rotated.params(), new_params);
        assert_ne!(rotated.salt, slot.salt, "nouveau sel à chaque rotation");
        assert!(rotated.unlock(b"ancien").is_err());
        assert_eq!(
            rotated.unlock(b"nouveau").unwrap().secret_bytes(),
            key.secret_bytes()
        );
        assert!(matches!(
            slot.rewrap(b"faux", b"nouveau", fast()),
            Err(CryptoError::WrongPassword)
        ));
    }

    #[test]
    fn keyring_file_round_trip() {
        let (slot, _) = KeySlot::create(b"pw", fast()).unwrap();
        let bytes = slot.to_bytes();
        assert_eq!(bytes.len(), 8 + 2 + SLOT_LEN);
        assert_eq!(KeySlot::from_bytes(&bytes).unwrap(), slot);

        let mut bad_magic = bytes.clone();
        bad_magic[0] = b'X';
        assert!(matches!(
            KeySlot::from_bytes(&bad_magic),
            Err(CryptoError::BadMagic)
        ));

        let mut bad_version = bytes.clone();
        bad_version[9] = 2;
        assert!(matches!(
            KeySlot::from_bytes(&bad_version),
            Err(CryptoError::UnsupportedVersion(2))
        ));
        assert!(KeySlot::from_bytes(&bytes[..bytes.len() - 1]).is_err());
        assert!(!format!("{slot:?}").contains("wrapped"));
    }
}
