//! Sécurité et conformité de Stratyx : chiffrement au repos, authentification, autorisations
//! de capture, verrou d'analyse (décision D6), RGPD, droit à l'image et sauvegardes.
//!
//! | Module | Rôle |
//! |---|---|
//! | [`crypto`] | XChaCha20-Poly1305, Argon2id, coffres versionnés, chiffrement en flux, rotation du mot de passe |
//! | [`auth`] | mots de passe (Argon2id, PHC), jetons d'API stockés en SHA-256, expiration |
//! | [`capture`] | consentement, conditions des salles, [`AnalysisLock`](capture::AnalysisLock) (D6) |
//! | [`gdpr`] | inventaire, conservation, export, effacement, journal d'audit chaîné |
//! | [`masking`] | masquage des zones hors table (webcam, chat) dans les images RGBA |
//! | [`backup`] | archives de sauvegarde chiffrées, restauration vérifiée |
//!
//! **Dépendances.** Uniquement des crates RustCrypto en Rust pur (sans script de build ni
//! code C), conformément à la décision D1 ; voir [`crypto`] pour le détail.
//!
//! **Horloge.** Les fonctions datées prennent l'heure en paramètre (`now_ms`, millisecondes
//! depuis l'époque Unix, UTC), comme [`stratyx_poker::session::Session`], pour rester
//! testables ; [`now_ms`] lit l'horloge système.

pub mod auth;
pub mod backup;
pub mod capture;
pub mod crypto;
pub mod gdpr;
pub mod masking;

/// Heure système en millisecondes depuis l'époque Unix (UTC) ; 0 si l'horloge est antérieure
/// à 1970.
#[must_use]
pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

#[cfg(test)]
mod tests {
    #[test]
    fn clock_is_after_2025() {
        assert!(super::now_ms() > 1_735_689_600_000);
    }
}
