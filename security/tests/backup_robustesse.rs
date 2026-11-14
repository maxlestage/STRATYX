//! Robustesse de la restauration des sauvegardes chiffrées : une archive abîmée (un octet
//! modifié n'importe où, en-tête compris, ou un fichier coupé à n'importe quelle position)
//! doit être refusée — erreur, jamais de panique, et rien de restauré. Les tests unitaires de
//! `backup.rs` vérifient quelques positions ; celui-ci les essaie toutes.

use std::panic::{AssertUnwindSafe, catch_unwind};

use stratyx_security::backup::{BackupWriter, restore_to_memory, verify_backup};
use stratyx_security::crypto::{DataKey, Unlock};

/// Une petite archive de trois fichiers (dont un vide et un de plusieurs blocs), déverrouillée
/// par clé : pas de dérivation de mot de passe à chaque essai.
fn archive(key: &DataKey) -> Vec<u8> {
    let mut writer = BackupWriter::with_key(key, None, Vec::new()).unwrap();
    writer
        .add_bytes("mains/2026-11-14.json", b"[{\"id\":1},{\"id\":2}]")
        .unwrap();
    writer.add_bytes("vide.txt", b"").unwrap();
    let video: Vec<u8> = (0..150_000u32).map(|i| (i * 31 % 251) as u8).collect();
    writer.add_bytes("videos/session-1.stxv", &video).unwrap();
    writer.finish().unwrap().0
}

/// Vérifie puis restaure `bytes` ; `Ok(true)` si l'archive est acceptée, la panique sinon.
fn accepted(bytes: &[u8], key: &DataKey) -> Result<bool, String> {
    catch_unwind(AssertUnwindSafe(|| {
        let verified = verify_backup(bytes, Unlock::Key(key)).is_ok();
        let restored = restore_to_memory(bytes, Unlock::Key(key)).is_ok();
        assert_eq!(
            verified, restored,
            "vérification et restauration en désaccord"
        );
        verified
    }))
    .map_err(|e| {
        e.downcast_ref::<String>()
            .cloned()
            .or_else(|| e.downcast_ref::<&str>().map(|s| (*s).to_owned()))
            .unwrap_or_default()
    })
}

#[test]
fn every_altered_byte_is_rejected() {
    let key = DataKey::generate().unwrap();
    let original = archive(&key);
    assert_eq!(
        accepted(&original, &key),
        Ok(true),
        "archive intacte refusée"
    );
    // Chaque octet de l'en-tête et du début, puis un sur 97 dans le reste (l'archive fait
    // ~150 Ko : tout essayer prendrait plusieurs minutes en mode test).
    let positions = (0..original.len().min(4_096)).chain((4_096..original.len()).step_by(97));
    let mut failures = Vec::new();
    for at in positions.chain([original.len() - 1]) {
        for flip in [0x01u8, 0x80] {
            let mut altered = original.clone();
            altered[at] ^= flip;
            match accepted(&altered, &key) {
                Ok(false) => {}
                Ok(true) => failures.push(format!("octet {at} (^{flip:#04x}) accepté")),
                Err(msg) => failures.push(format!("octet {at} (^{flip:#04x}) : panique {msg}")),
            }
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}

#[test]
fn every_truncation_is_rejected() {
    let key = DataKey::generate().unwrap();
    let original = archive(&key);
    let cuts = (0..original.len().min(4_096))
        .chain((4_096..original.len()).step_by(61))
        .chain(original.len() - 64..original.len());
    let mut failures = Vec::new();
    for cut in cuts {
        match accepted(&original[..cut], &key) {
            Ok(false) => {}
            Ok(true) => failures.push(format!("coupée à {cut} : acceptée")),
            Err(msg) => failures.push(format!("coupée à {cut} : panique {msg}")),
        }
    }
    // Des octets en trop après la fin ne doivent pas passer non plus.
    let mut longer = original.clone();
    longer.extend_from_slice(b"octets en trop");
    if accepted(&longer, &key) != Ok(false) {
        failures.push("octets ajoutés après la fin : acceptés".into());
    }
    assert!(failures.is_empty(), "{failures:#?}");
}

#[test]
fn a_wrong_key_is_rejected() {
    let key = DataKey::generate().unwrap();
    let other = DataKey::generate().unwrap();
    assert_eq!(accepted(&archive(&key), &other), Ok(false));
}
