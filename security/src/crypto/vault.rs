//! Format versionné des coffres et rotation du mot de passe.
//!
//! ```text
//! signature   8   « STRXVLT\0 »
//! version     2   u16 gros-boutiste (1)
//! contenu     1   1 = blob, 2 = flux par blocs
//! clé         1   1 = clé externe (identifiant seul), 2 = emplacement de clé embarqué
//! [identifiant de clé 16]                      si clé externe
//! [emplacement de clé 133 : identifiant,       si emplacement embarqué
//!  paramètres Argon2id, sel, nonce, clé enveloppée]
//! paramètres du contenu
//!   blob : nonce 24
//!   flux : taille de bloc u32, préfixe de nonce 19
//! contenu chiffré
//! ```
//!
//! Les données associées du contenu couvrent la version, le type de contenu, l'identifiant de
//! clé et les paramètres du contenu, mais **pas** l'emplacement de clé : changer de mot de passe
//! ne réécrit que l'en-tête ([`rotate_password`]), le contenu chiffré est recopié tel quel.

use std::fmt;
use std::io::{self, Read, Write};

use chacha20poly1305::XChaCha20Poly1305;

use super::CryptoError;
use super::kdf::KdfParams;
use super::key::{DataKey, KEY_ID_LEN, KeyId, NONCE_LEN};
use super::slot::{KeySlot, SLOT_LEN};

/// Signature des coffres.
pub const VAULT_MAGIC: [u8; 8] = *b"STRXVLT\0";
/// Version courante du format des coffres.
pub const VAULT_VERSION: u16 = 1;

/// Séparation de domaine des données associées du contenu.
const PAYLOAD_AAD_DOMAIN: &[u8] = b"STRATYX/vault-payload/v1";

/// Taille du préfixe de nonce d'un flux : 24 − 4 (compteur) − 1 (drapeau de bloc final).
pub const STREAM_NONCE_PREFIX_LEN: usize = NONCE_LEN - 5;

const KEY_MODE_EXTERNAL: u8 = 1;
const KEY_MODE_EMBEDDED: u8 = 2;

/// Type de contenu d'un coffre.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PayloadKind {
    /// Message unique, chiffré et authentifié en une fois (tient en mémoire).
    Blob,
    /// Flux découpé en blocs authentifiés un par un (construction STREAM), pour les gros
    /// fichiers comme les vidéos.
    Stream,
}

impl PayloadKind {
    const fn code(self) -> u8 {
        match self {
            Self::Blob => 1,
            Self::Stream => 2,
        }
    }
}

impl fmt::Display for PayloadKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Blob => "bloc unique",
            Self::Stream => "flux par blocs",
        })
    }
}

/// Paramètres propres au contenu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PayloadParams {
    Blob {
        nonce: [u8; NONCE_LEN],
    },
    Stream {
        chunk_size: u32,
        nonce_prefix: [u8; STREAM_NONCE_PREFIX_LEN],
    },
}

impl PayloadParams {
    const fn kind(&self) -> PayloadKind {
        match self {
            Self::Blob { .. } => PayloadKind::Blob,
            Self::Stream { .. } => PayloadKind::Stream,
        }
    }

    fn encode_into(&self, out: &mut Vec<u8>) {
        match self {
            Self::Blob { nonce } => out.extend_from_slice(nonce),
            Self::Stream {
                chunk_size,
                nonce_prefix,
            } => {
                out.extend_from_slice(&chunk_size.to_be_bytes());
                out.extend_from_slice(nonce_prefix);
            }
        }
    }
}

/// Moyen de déverrouiller un coffre.
#[derive(Clone, Copy)]
pub enum Unlock<'a> {
    /// Mot de passe : le coffre doit embarquer un emplacement de clé.
    Password(&'a [u8]),
    /// Clé de données déjà déverrouillée (trousseau local).
    Key(&'a DataKey),
}

impl fmt::Debug for Unlock<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Password(_) => f.write_str("Unlock::Password(<masqué>)"),
            Self::Key(key) => f.debug_tuple("Unlock::Key").field(&key.id()).finish(),
        }
    }
}

/// En-tête d'un coffre.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VaultHeader {
    key_id: KeyId,
    slot: Option<KeySlot>,
    payload: PayloadParams,
}

impl VaultHeader {
    /// En-tête d'un nouveau coffre ; `slot`, s'il est fourni, doit protéger `key`.
    pub(crate) fn new(
        key: &DataKey,
        slot: Option<&KeySlot>,
        payload: PayloadParams,
    ) -> Result<Self, CryptoError> {
        if slot.is_some_and(|s| s.key_id() != key.id()) {
            return Err(CryptoError::KeyMismatch);
        }
        Ok(Self {
            key_id: key.id(),
            slot: slot.cloned(),
            payload,
        })
    }

    /// Type de contenu.
    #[must_use]
    pub const fn kind(&self) -> PayloadKind {
        self.payload.kind()
    }

    /// Identifiant de la clé de données qui chiffre le contenu.
    #[must_use]
    pub const fn key_id(&self) -> KeyId {
        self.key_id
    }

    /// Emplacement de clé embarqué, s'il existe (coffre déverrouillable par mot de passe).
    #[must_use]
    pub const fn key_slot(&self) -> Option<&KeySlot> {
        self.slot.as_ref()
    }

    pub(crate) const fn payload(&self) -> &PayloadParams {
        &self.payload
    }

    /// Vérifie le type de contenu attendu.
    pub(crate) fn expect_kind(&self, expected: PayloadKind) -> Result<(), CryptoError> {
        let found = self.kind();
        if found == expected {
            Ok(())
        } else {
            Err(CryptoError::WrongPayloadKind { expected, found })
        }
    }

    /// Encodage binaire de l'en-tête.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(12 + SLOT_LEN + NONCE_LEN);
        out.extend_from_slice(&VAULT_MAGIC);
        out.extend_from_slice(&VAULT_VERSION.to_be_bytes());
        out.push(self.kind().code());
        match &self.slot {
            None => {
                out.push(KEY_MODE_EXTERNAL);
                out.extend_from_slice(self.key_id.as_bytes());
            }
            Some(slot) => {
                out.push(KEY_MODE_EMBEDDED);
                out.extend_from_slice(&slot.encode());
            }
        }
        self.payload.encode_into(&mut out);
        out
    }

    /// Lit un en-tête au début de `reader` ; le lecteur est ensuite positionné sur le contenu.
    ///
    /// # Errors
    /// Signature, version, type ou paramètres invalides ; en-tête tronqué.
    pub fn read_from<R: Read + ?Sized>(reader: &mut R) -> Result<Self, CryptoError> {
        let magic: [u8; 8] = read_array(reader)?;
        if magic != VAULT_MAGIC {
            return Err(CryptoError::BadMagic);
        }
        let version = u16::from_be_bytes(read_array(reader)?);
        if version != VAULT_VERSION {
            return Err(CryptoError::UnsupportedVersion(version));
        }
        let [kind, key_mode] = read_array(reader)?;
        let (key_id, slot) = match key_mode {
            KEY_MODE_EXTERNAL => (
                KeyId::from_bytes(read_array::<KEY_ID_LEN, _>(reader)?),
                None,
            ),
            KEY_MODE_EMBEDDED => {
                let slot = KeySlot::decode(&read_array::<SLOT_LEN, _>(reader)?)?;
                (slot.key_id(), Some(slot))
            }
            _ => return Err(CryptoError::Malformed("mode de clé inconnu")),
        };
        let payload = match kind {
            1 => PayloadParams::Blob {
                nonce: read_array(reader)?,
            },
            2 => PayloadParams::Stream {
                chunk_size: u32::from_be_bytes(read_array(reader)?),
                nonce_prefix: read_array(reader)?,
            },
            _ => return Err(CryptoError::Malformed("type de contenu inconnu")),
        };
        Ok(Self {
            key_id,
            slot,
            payload,
        })
    }

    /// Lit l'en-tête au début de `bytes` ; renvoie aussi sa longueur.
    ///
    /// # Errors
    /// Voir [`VaultHeader::read_from`].
    pub fn parse(bytes: &[u8]) -> Result<(Self, usize), CryptoError> {
        let mut cursor = bytes;
        let header = Self::read_from(&mut cursor)?;
        Ok((header, bytes.len() - cursor.len()))
    }

    /// Données associées authentifiées avec le contenu (hors emplacement de clé).
    pub(crate) fn payload_aad(&self) -> Vec<u8> {
        let mut aad = Vec::with_capacity(PAYLOAD_AAD_DOMAIN.len() + 3 + KEY_ID_LEN + NONCE_LEN);
        aad.extend_from_slice(PAYLOAD_AAD_DOMAIN);
        aad.extend_from_slice(&VAULT_VERSION.to_be_bytes());
        aad.push(self.kind().code());
        aad.extend_from_slice(self.key_id.as_bytes());
        self.payload.encode_into(&mut aad);
        aad
    }

    /// Instance AEAD du contenu, après déverrouillage.
    pub(crate) fn cipher(&self, unlock: Unlock<'_>) -> Result<XChaCha20Poly1305, CryptoError> {
        match unlock {
            Unlock::Key(key) if key.id() == self.key_id => Ok(key.cipher()),
            Unlock::Key(_) => Err(CryptoError::KeyMismatch),
            Unlock::Password(password) => {
                let slot = self.slot.as_ref().ok_or(CryptoError::NoPasswordSlot)?;
                Ok(slot.unlock(password)?.cipher())
            }
        }
    }
}

/// Lit exactement `N` octets ; une fin prématurée signale un en-tête tronqué.
fn read_array<const N: usize, R: Read + ?Sized>(reader: &mut R) -> Result<[u8; N], CryptoError> {
    let mut out = [0u8; N];
    reader.read_exact(&mut out).map_err(|e| match e.kind() {
        io::ErrorKind::UnexpectedEof => CryptoError::Malformed("en-tête tronqué"),
        _ => CryptoError::from_io(e),
    })?;
    Ok(out)
}

/// Change le mot de passe d'un coffre (blob ou flux) **sans rechiffrer son contenu** :
/// l'emplacement de clé est réenveloppé (nouveau sel, nouveaux paramètres) et le contenu
/// chiffré est recopié octet pour octet. Renvoie le nombre d'octets de contenu recopiés.
///
/// Le contenu n'est pas déchiffré : sa vérification reste à la charge de la lecture.
///
/// # Errors
/// [`CryptoError::NoPasswordSlot`] pour un coffre à clé externe (changer alors le mot de passe
/// du trousseau avec [`KeySlot::rewrap`]) ; [`CryptoError::WrongPassword`] ; entrée-sortie.
pub fn rotate_password<R: Read, W: Write>(
    mut input: R,
    mut output: W,
    old_password: &[u8],
    new_password: &[u8],
    new_params: KdfParams,
) -> Result<u64, CryptoError> {
    let header = VaultHeader::read_from(&mut input)?;
    let slot = header.slot.as_ref().ok_or(CryptoError::NoPasswordSlot)?;
    let new_slot = slot.rewrap(old_password, new_password, new_params)?;
    let rotated = VaultHeader {
        slot: Some(new_slot),
        ..header
    };
    output.write_all(&rotated.encode())?;
    let copied = io::copy(&mut input, &mut output)?;
    output.flush()?;
    Ok(copied)
}

/// Variante en mémoire de [`rotate_password`].
///
/// # Errors
/// Voir [`rotate_password`].
pub fn rotate_password_in_memory(
    sealed: &[u8],
    old_password: &[u8],
    new_password: &[u8],
    new_params: KdfParams,
) -> Result<Vec<u8>, CryptoError> {
    let mut out = Vec::with_capacity(sealed.len());
    rotate_password(sealed, &mut out, old_password, new_password, new_params)?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::super::slot::tests::fast;
    use super::*;

    #[test]
    fn header_round_trip_both_key_modes() {
        let (slot, key) = KeySlot::create(b"pw", fast()).unwrap();
        let stream = PayloadParams::Stream {
            chunk_size: 4096,
            nonce_prefix: [9; STREAM_NONCE_PREFIX_LEN],
        };
        for slot in [None, Some(&slot)] {
            let header = VaultHeader::new(&key, slot, stream.clone()).unwrap();
            let mut bytes = header.encode();
            bytes.extend_from_slice(b"contenu");
            let (parsed, len) = VaultHeader::parse(&bytes).unwrap();
            assert_eq!(parsed, header);
            assert_eq!(&bytes[len..], b"contenu");
            assert_eq!(parsed.kind(), PayloadKind::Stream);
            assert_eq!(parsed.key_id(), key.id());
        }
    }

    #[test]
    fn header_errors() {
        let key = DataKey::generate().unwrap();
        let header = VaultHeader::new(&key, None, PayloadParams::Blob { nonce: [0; 24] }).unwrap();
        let bytes = header.encode();

        assert!(matches!(
            VaultHeader::parse(b"PK\x03\x04"),
            Err(CryptoError::Malformed(_))
        ));
        let mut magic = bytes.clone();
        magic[3] = b'?';
        assert!(matches!(
            VaultHeader::parse(&magic),
            Err(CryptoError::BadMagic)
        ));
        let mut version = bytes.clone();
        version[9] = 7;
        assert!(matches!(
            VaultHeader::parse(&version),
            Err(CryptoError::UnsupportedVersion(7))
        ));
        let mut kind = bytes.clone();
        kind[10] = 3;
        assert!(matches!(
            VaultHeader::parse(&kind),
            Err(CryptoError::Malformed(_))
        ));
        let mut mode = bytes.clone();
        mode[11] = 0;
        assert!(matches!(
            VaultHeader::parse(&mode),
            Err(CryptoError::Malformed(_))
        ));
        assert!(matches!(
            VaultHeader::parse(&bytes[..bytes.len() - 1]),
            Err(CryptoError::Malformed(_))
        ));

        let (other_slot, _) = KeySlot::create(b"pw", fast()).unwrap();
        assert!(matches!(
            VaultHeader::new(
                &key,
                Some(&other_slot),
                PayloadParams::Blob { nonce: [0; 24] }
            ),
            Err(CryptoError::KeyMismatch)
        ));
    }

    #[test]
    fn unlock_variants() {
        let (slot, key) = KeySlot::create(b"pw", fast()).unwrap();
        let other = DataKey::generate().unwrap();
        let blob = PayloadParams::Blob { nonce: [0; 24] };
        let embedded = VaultHeader::new(&key, Some(&slot), blob.clone()).unwrap();
        let external = VaultHeader::new(&key, None, blob).unwrap();

        assert!(embedded.cipher(Unlock::Password(b"pw")).is_ok());
        assert!(embedded.cipher(Unlock::Key(&key)).is_ok());
        assert!(matches!(
            embedded.cipher(Unlock::Password(b"faux")),
            Err(CryptoError::WrongPassword)
        ));
        assert!(matches!(
            external.cipher(Unlock::Password(b"pw")),
            Err(CryptoError::NoPasswordSlot)
        ));
        assert!(matches!(
            external.cipher(Unlock::Key(&other)),
            Err(CryptoError::KeyMismatch)
        ));
        assert_eq!(
            format!("{:?}", Unlock::Password(b"secret")),
            "Unlock::Password(<masqué>)"
        );
    }
}
