//! Dérivation de clé à partir d'un mot de passe : Argon2id (RFC 9106), version 0x13.

use argon2::{Algorithm, Argon2, Params, Version};
use zeroize::Zeroizing;

use super::CryptoError;
use super::key::KEY_LEN;

/// Taille du sel Argon2id stocké dans chaque emplacement de clé, en octets.
///
/// La RFC 9106 recommande 16 octets ; 32 octets écartent toute collision entre coffres.
pub const SALT_LEN: usize = 32;

/// Identifiant d'algorithme stocké dans le format : Argon2id, version 0x13 (19).
pub(crate) const KDF_ARGON2ID_V13: u8 = 1;

/// Taille de l'encodage des paramètres : identifiant d'algorithme puis trois `u32` gros-boutistes.
pub(crate) const KDF_PARAMS_LEN: usize = 1 + 4 * 3;

/// Paramètres de coût d'Argon2id, stockés en clair dans chaque emplacement de clé pour que
/// le coffre reste lisible si les valeurs par défaut évoluent.
///
/// | Profil | Mémoire | Passes | Voies | Usage |
/// |---|---|---|---|---|
/// | [`RECOMMENDED`](Self::RECOMMENDED) | 64 Mio | 3 | 4 | coffres locaux, sauvegardes (RFC 9106, 2ᵉ option) |
/// | [`MODERATE`](Self::MODERATE) | 19 Mio | 2 | 1 | mots de passe du serveur, mobiles (OWASP) |
///
/// Les paramètres lus dans un fichier sont bornés (voir [`KdfParams::new`]) pour qu'un
/// en-tête forgé ne puisse pas épuiser la mémoire ou le processeur.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(try_from = "RawKdfParams", into = "RawKdfParams")]
pub struct KdfParams {
    m_cost_kib: u32,
    t_cost: u32,
    p_cost: u32,
}

impl KdfParams {
    /// Profil par défaut des coffres : 64 Mio, 3 passes, 4 voies (RFC 9106, § 4, 2ᵉ option).
    pub const RECOMMENDED: Self = Self {
        m_cost_kib: 64 * 1024,
        t_cost: 3,
        p_cost: 4,
    };

    /// Profil allégé : 19 Mio, 2 passes, 1 voie (recommandation minimale de l'OWASP).
    pub const MODERATE: Self = Self {
        m_cost_kib: 19 * 1024,
        t_cost: 2,
        p_cost: 1,
    };

    /// Mémoire maximale acceptée : 2 Gio (1ʳᵉ option de la RFC 9106).
    pub const MAX_M_COST_KIB: u32 = 2 * 1024 * 1024;
    /// Nombre maximal de passes accepté.
    pub const MAX_T_COST: u32 = 64;
    /// Nombre maximal de voies accepté.
    pub const MAX_P_COST: u32 = 64;

    /// Construit des paramètres en vérifiant leurs bornes : au moins 8 Kio par voie
    /// (minimum d'Argon2), au plus [`MAX_M_COST_KIB`](Self::MAX_M_COST_KIB),
    /// 1 à [`MAX_T_COST`](Self::MAX_T_COST) passes, 1 à [`MAX_P_COST`](Self::MAX_P_COST) voies.
    ///
    /// Aucune force minimale n'est imposée, pour permettre des tests rapides : en production,
    /// utiliser [`RECOMMENDED`](Self::RECOMMENDED) ou [`MODERATE`](Self::MODERATE).
    ///
    /// # Errors
    /// [`CryptoError::InvalidKdfParams`] si une borne n'est pas respectée.
    pub fn new(m_cost_kib: u32, t_cost: u32, p_cost: u32) -> Result<Self, CryptoError> {
        if !(1..=Self::MAX_P_COST).contains(&p_cost) {
            return Err(CryptoError::InvalidKdfParams(format!(
                "nombre de voies {p_cost} hors de 1..={}",
                Self::MAX_P_COST
            )));
        }
        if !(1..=Self::MAX_T_COST).contains(&t_cost) {
            return Err(CryptoError::InvalidKdfParams(format!(
                "nombre de passes {t_cost} hors de 1..={}",
                Self::MAX_T_COST
            )));
        }
        let min_memory = 8 * p_cost;
        if !(min_memory..=Self::MAX_M_COST_KIB).contains(&m_cost_kib) {
            return Err(CryptoError::InvalidKdfParams(format!(
                "mémoire {m_cost_kib} Kio hors de {min_memory}..={} Kio",
                Self::MAX_M_COST_KIB
            )));
        }
        Ok(Self {
            m_cost_kib,
            t_cost,
            p_cost,
        })
    }

    /// Mémoire utilisée, en kibioctets.
    #[must_use]
    pub const fn m_cost_kib(&self) -> u32 {
        self.m_cost_kib
    }

    /// Nombre de passes sur la mémoire.
    #[must_use]
    pub const fn t_cost(&self) -> u32 {
        self.t_cost
    }

    /// Nombre de voies (parallélisme logique ; le calcul reste séquentiel).
    #[must_use]
    pub const fn p_cost(&self) -> u32 {
        self.p_cost
    }

    /// Instance Argon2id v0x13 avec ces paramètres et une sortie de 32 octets.
    pub(crate) fn argon2(&self) -> Result<Argon2<'static>, CryptoError> {
        let params = Params::new(self.m_cost_kib, self.t_cost, self.p_cost, Some(KEY_LEN))
            .map_err(|e| CryptoError::InvalidKdfParams(e.to_string()))?;
        Ok(Argon2::new(Algorithm::Argon2id, Version::V0x13, params))
    }

    /// Dérive une clé de 32 octets, effacée de la mémoire à sa libération.
    pub(crate) fn derive_key(
        &self,
        password: &[u8],
        salt: &[u8],
    ) -> Result<Zeroizing<[u8; KEY_LEN]>, CryptoError> {
        let mut out = Zeroizing::new([0u8; KEY_LEN]);
        self.argon2()?
            .hash_password_into(password, salt, out.as_mut())
            .map_err(|e| CryptoError::Kdf(e.to_string()))?;
        Ok(out)
    }

    /// Encodage binaire : identifiant d'algorithme, mémoire, passes, voies (gros-boutiste).
    pub(crate) fn encode(&self) -> [u8; KDF_PARAMS_LEN] {
        let mut out = [0u8; KDF_PARAMS_LEN];
        out[0] = KDF_ARGON2ID_V13;
        out[1..5].copy_from_slice(&self.m_cost_kib.to_be_bytes());
        out[5..9].copy_from_slice(&self.t_cost.to_be_bytes());
        out[9..13].copy_from_slice(&self.p_cost.to_be_bytes());
        out
    }

    /// Décode et valide l'encodage produit par [`KdfParams::encode`].
    pub(crate) fn decode(bytes: &[u8; KDF_PARAMS_LEN]) -> Result<Self, CryptoError> {
        if bytes[0] != KDF_ARGON2ID_V13 {
            return Err(CryptoError::InvalidKdfParams(format!(
                "algorithme de dérivation inconnu ({})",
                bytes[0]
            )));
        }
        let word =
            |i: usize| u32::from_be_bytes([bytes[i], bytes[i + 1], bytes[i + 2], bytes[i + 3]]);
        Self::new(word(1), word(5), word(9))
    }
}

impl Default for KdfParams {
    fn default() -> Self {
        Self::RECOMMENDED
    }
}

/// Forme sérialisée de [`KdfParams`], validée à la désérialisation.
#[derive(serde::Serialize, serde::Deserialize)]
struct RawKdfParams {
    m_cost_kib: u32,
    t_cost: u32,
    p_cost: u32,
}

impl TryFrom<RawKdfParams> for KdfParams {
    type Error = CryptoError;

    fn try_from(raw: RawKdfParams) -> Result<Self, Self::Error> {
        Self::new(raw.m_cost_kib, raw.t_cost, raw.p_cost)
    }
}

impl From<KdfParams> for RawKdfParams {
    fn from(params: KdfParams) -> Self {
        Self {
            m_cost_kib: params.m_cost_kib,
            t_cost: params.t_cost,
            p_cost: params.p_cost,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }

    /// Vecteur de référence de l'implémentation Argon2 (phc-winner-argon2, `test.c`) :
    /// Argon2id v0x13, t = 1, m = 2¹⁶ Kio, p = 1, « password », « somesalt ».
    #[test]
    fn argon2id_reference_vector() {
        let params = KdfParams::new(1 << 16, 1, 1).unwrap();
        let key = params.derive_key(b"password", b"somesalt").unwrap();
        assert_eq!(
            hex(key.as_ref()),
            "f6a5adc1ba723dddef9b5ac1d464e180fcd9dffc9d1cbf76cca2fed795d9ca98"
        );
    }

    #[test]
    fn bounds_are_enforced() {
        assert!(KdfParams::new(8, 1, 1).is_ok());
        assert!(KdfParams::new(7, 1, 1).is_err());
        assert!(
            KdfParams::new(16, 1, 4).is_err(),
            "8 Kio par voie au minimum"
        );
        assert!(KdfParams::new(1024, 0, 1).is_err());
        assert!(KdfParams::new(1024, 1, 0).is_err());
        assert!(KdfParams::new(KdfParams::MAX_M_COST_KIB + 1, 1, 1).is_err());
        assert!(KdfParams::new(1024, KdfParams::MAX_T_COST + 1, 1).is_err());
        assert!(KdfParams::new(1 << 20, 1, KdfParams::MAX_P_COST + 1).is_err());
        for profile in [KdfParams::RECOMMENDED, KdfParams::MODERATE] {
            let p = KdfParams::new(profile.m_cost_kib(), profile.t_cost(), profile.p_cost());
            assert_eq!(p.unwrap(), profile);
        }
    }

    #[test]
    fn encoding_round_trip_and_validation() {
        let p = KdfParams::new(4096, 2, 2).unwrap();
        let bytes = p.encode();
        assert_eq!(KdfParams::decode(&bytes).unwrap(), p);

        let mut unknown = bytes;
        unknown[0] = 9;
        assert!(KdfParams::decode(&unknown).is_err());

        let mut huge = bytes;
        huge[1..5].copy_from_slice(&u32::MAX.to_be_bytes());
        assert!(
            KdfParams::decode(&huge).is_err(),
            "mémoire bornée à la lecture"
        );

        let json = serde_json::to_string(&p).unwrap();
        assert_eq!(serde_json::from_str::<KdfParams>(&json).unwrap(), p);
        assert!(
            serde_json::from_str::<KdfParams>(r#"{"m_cost_kib":1,"t_cost":1,"p_cost":1}"#).is_err()
        );
    }
}
