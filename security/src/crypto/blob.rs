//! Chiffrement d'un message unique (« blob ») tenant en mémoire : historiques de mains,
//! profils, configuration.

use super::CryptoError;
use super::kdf::KdfParams;
use super::key::{DataKey, NONCE_LEN, aead_open, aead_seal, random_array};
use super::slot::KeySlot;
use super::vault::{PayloadKind, PayloadParams, Unlock, VaultHeader};

/// Chiffre `plaintext` avec une clé de données ; le coffre ne porte que l'identifiant de clé.
///
/// # Errors
/// Aléa indisponible.
pub fn seal(key: &DataKey, plaintext: &[u8]) -> Result<Vec<u8>, CryptoError> {
    seal_inner(key, None, plaintext)
}

/// Chiffre `plaintext` et embarque `slot`, qui doit protéger `key` : le coffre s'ouvre alors
/// aussi avec le mot de passe.
///
/// # Errors
/// [`CryptoError::KeyMismatch`] si `slot` protège une autre clé ; aléa indisponible.
pub fn seal_with_slot(
    key: &DataKey,
    slot: &KeySlot,
    plaintext: &[u8],
) -> Result<Vec<u8>, CryptoError> {
    seal_inner(key, Some(slot), plaintext)
}

/// Chiffre `plaintext` sous une nouvelle clé de données protégée par `password` :
/// coffre autonome, déverrouillable sur n'importe quelle machine avec le mot de passe.
///
/// # Errors
/// Aléa indisponible ; échec d'Argon2id.
pub fn seal_with_password(
    password: &[u8],
    params: KdfParams,
    plaintext: &[u8],
) -> Result<Vec<u8>, CryptoError> {
    let (slot, key) = KeySlot::create(password, params)?;
    seal_inner(&key, Some(&slot), plaintext)
}

/// Vérifie puis déchiffre un coffre de type blob.
///
/// # Errors
/// [`CryptoError::AuthenticationFailed`] si un seul bit du contenu ou de ses paramètres a
/// changé ; [`CryptoError::WrongPassword`], [`CryptoError::KeyMismatch`],
/// [`CryptoError::WrongPayloadKind`] ; en-tête invalide.
pub fn open(unlock: Unlock<'_>, sealed: &[u8]) -> Result<Vec<u8>, CryptoError> {
    let (header, header_len) = VaultHeader::parse(sealed)?;
    header.expect_kind(PayloadKind::Blob)?;
    let PayloadParams::Blob { nonce } = header.payload() else {
        return Err(CryptoError::Malformed("paramètres de contenu incohérents"));
    };
    let cipher = header.cipher(unlock)?;
    aead_open(&cipher, nonce, &header.payload_aad(), &sealed[header_len..])
}

fn seal_inner(
    key: &DataKey,
    slot: Option<&KeySlot>,
    plaintext: &[u8],
) -> Result<Vec<u8>, CryptoError> {
    let nonce = random_array::<NONCE_LEN>()?;
    let header = VaultHeader::new(key, slot, PayloadParams::Blob { nonce })?;
    let mut out = header.encode();
    let ciphertext = aead_seal(&key.cipher(), &nonce, &header.payload_aad(), plaintext)?;
    out.extend_from_slice(&ciphertext);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::super::slot::tests::fast;
    use super::super::vault::{VaultHeader, rotate_password_in_memory};
    use super::*;

    const HAND: &[u8] = b"{\"id\":\"main-42\",\"board\":\"AhKd7c\"}";

    #[test]
    fn round_trip_with_key_and_password() {
        let key = DataKey::generate().unwrap();
        let sealed = seal(&key, HAND).unwrap();
        assert_eq!(open(Unlock::Key(&key), &sealed).unwrap(), HAND);
        assert!(
            !sealed.windows(HAND.len()).any(|w| w == HAND),
            "texte clair absent"
        );

        let sealed = seal_with_password(b"mot de passe", fast(), HAND).unwrap();
        assert_eq!(
            open(Unlock::Password(b"mot de passe"), &sealed).unwrap(),
            HAND
        );

        let empty = seal(&key, b"").unwrap();
        assert_eq!(open(Unlock::Key(&key), &empty).unwrap(), b"");
    }

    #[test]
    fn same_plaintext_gives_different_ciphertexts() {
        let key = DataKey::generate().unwrap();
        assert_ne!(seal(&key, HAND).unwrap(), seal(&key, HAND).unwrap());
    }

    #[test]
    fn any_flipped_bit_is_detected() {
        let (slot, key) = KeySlot::create(b"pw", fast()).unwrap();
        let sealed = seal_with_slot(&key, &slot, HAND).unwrap();
        let (_, header_len) = VaultHeader::parse(&sealed).unwrap();
        for i in 0..sealed.len() {
            for bit in [0x01, 0x80] {
                let mut tampered = sealed.clone();
                tampered[i] ^= bit;
                // Avec la clé : tout bit du contenu ou de ses paramètres est détecté.
                // (L'emplacement de clé n'est pas utilisé lors d'une ouverture par clé.)
                let by_key = open(Unlock::Key(&key), &tampered);
                let by_password = open(Unlock::Password(b"pw"), &tampered);
                assert!(
                    by_password.is_err(),
                    "octet {i} bit {bit:#x} non détecté (mot de passe)"
                );
                if !(12..12 + super::super::slot::SLOT_LEN).contains(&i) {
                    assert!(by_key.is_err(), "octet {i} bit {bit:#x} non détecté (clé)");
                }
            }
        }
        assert!(header_len < sealed.len());
    }

    #[test]
    fn wrong_credentials_and_kind() {
        let key = DataKey::generate().unwrap();
        let other = DataKey::generate().unwrap();
        let sealed = seal(&key, HAND).unwrap();
        assert!(matches!(
            open(Unlock::Key(&other), &sealed),
            Err(CryptoError::KeyMismatch)
        ));
        assert!(matches!(
            open(Unlock::Password(b"pw"), &sealed),
            Err(CryptoError::NoPasswordSlot)
        ));

        let sealed = seal_with_password(b"bon", fast(), HAND).unwrap();
        assert!(matches!(
            open(Unlock::Password(b"mauvais"), &sealed),
            Err(CryptoError::WrongPassword)
        ));

        // Tronquer l'étiquette ou ajouter des octets est détecté.
        let key = DataKey::generate().unwrap();
        let sealed = seal(&key, HAND).unwrap();
        assert!(open(Unlock::Key(&key), &sealed[..sealed.len() - 1]).is_err());
        let mut longer = sealed.clone();
        longer.push(0);
        assert!(open(Unlock::Key(&key), &longer).is_err());
    }

    #[test]
    fn password_rotation_does_not_touch_ciphertext() {
        let sealed = seal_with_password(b"ancien", fast(), HAND).unwrap();
        let rotated = rotate_password_in_memory(&sealed, b"ancien", b"nouveau", fast()).unwrap();
        let (_, old_len) = VaultHeader::parse(&sealed).unwrap();
        let (_, new_len) = VaultHeader::parse(&rotated).unwrap();
        assert_eq!(old_len, new_len);
        assert_eq!(
            &sealed[old_len..],
            &rotated[new_len..],
            "contenu recopié tel quel"
        );
        assert_ne!(
            &sealed[..old_len],
            &rotated[..new_len],
            "emplacement réenveloppé"
        );

        assert_eq!(open(Unlock::Password(b"nouveau"), &rotated).unwrap(), HAND);
        assert!(matches!(
            open(Unlock::Password(b"ancien"), &rotated),
            Err(CryptoError::WrongPassword)
        ));
        assert!(matches!(
            rotate_password_in_memory(&sealed, b"faux", b"nouveau", fast()),
            Err(CryptoError::WrongPassword)
        ));

        let key = DataKey::generate().unwrap();
        assert!(matches!(
            rotate_password_in_memory(&seal(&key, HAND).unwrap(), b"a", b"b", fast()),
            Err(CryptoError::NoPasswordSlot)
        ));
    }
}
