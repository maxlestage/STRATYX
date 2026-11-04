//! Inventaire des catégories de données traitées (registre des traitements, art. 30).

use serde::{Deserialize, Serialize};

use super::retention::Retention;

/// Catégorie de données personnelles.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DataCategory {
    /// Historiques de mains (actions, cartes, montants, pseudonymes des joueurs de la table).
    HandHistories,
    /// Enregistrements vidéo des parties.
    Videos,
    /// Profils d'adversaires : statistiques rattachées à un pseudonyme.
    OpponentProfiles,
    /// Statistiques personnelles de l'utilisateur.
    Statistics,
    /// Progression dans le coaching (exercices, quiz, parcours).
    Progress,
}

impl DataCategory {
    /// Toutes les catégories, dans l'ordre de l'inventaire.
    pub const ALL: [Self; 5] = [
        Self::HandHistories,
        Self::Videos,
        Self::OpponentProfiles,
        Self::Statistics,
        Self::Progress,
    ];

    /// Libellé en français.
    #[must_use]
    pub const fn label(self) -> &'static str {
        self.info().label
    }

    /// Fiche d'inventaire de la catégorie.
    #[must_use]
    pub const fn info(self) -> &'static CategoryInfo {
        match self {
            Self::HandHistories => &INVENTORY[0],
            Self::Videos => &INVENTORY[1],
            Self::OpponentProfiles => &INVENTORY[2],
            Self::Statistics => &INVENTORY[3],
            Self::Progress => &INVENTORY[4],
        }
    }
}

/// Base légale du traitement (RGPD, art. 6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LegalBasis {
    /// Art. 6.1.a : consentement.
    Consent,
    /// Art. 6.1.b : exécution du service demandé par l'utilisateur.
    Contract,
    /// Art. 6.1.f : intérêt légitime ; la mise en balance doit être documentée.
    LegitimateInterest,
}

/// Personnes concernées par une catégorie.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Subjects {
    /// L'utilisateur seul.
    User,
    /// L'utilisateur et les adversaires (pseudonymes).
    UserAndOpponents,
    /// Les adversaires seuls.
    Opponents,
}

/// Fiche d'inventaire d'une catégorie de données.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct CategoryInfo {
    /// Catégorie.
    pub category: DataCategory,
    /// Libellé.
    pub label: &'static str,
    /// Contenu.
    pub description: &'static str,
    /// Finalité.
    pub purpose: &'static str,
    /// Personnes concernées.
    pub subjects: Subjects,
    /// Base légale pour les données de l'utilisateur.
    pub user_basis: Option<LegalBasis>,
    /// Base légale pour les données des adversaires.
    pub third_party_basis: Option<LegalBasis>,
    /// Conservation par défaut (modifiable dans la [`RetentionPolicy`](super::RetentionPolicy)).
    pub default_retention: Retention,
    /// Mesures de protection.
    pub safeguards: &'static str,
}

/// Inventaire des données traitées par Stratyx. Stockage local et chiffré par défaut.
pub static INVENTORY: [CategoryInfo; 5] = [
    CategoryInfo {
        category: DataCategory::HandHistories,
        label: "Historiques de mains",
        description: "Actions, cartes, montants et pseudonymes des joueurs présents à la table",
        purpose: "Retranscription, revue et analyse des parties de l'utilisateur",
        subjects: Subjects::UserAndOpponents,
        user_basis: Some(LegalBasis::Contract),
        third_party_basis: Some(LegalBasis::LegitimateInterest),
        default_retention: Retention::UntilErasure,
        safeguards: "Chiffrement au repos ; pseudonymes d'un adversaire anonymisés sur demande",
    },
    CategoryInfo {
        category: DataCategory::Videos,
        label: "Vidéos",
        description: "Enregistrements d'écran des parties (peuvent montrer webcams et discussions)",
        purpose: "Retranscription automatique et revue visuelle des mains",
        subjects: Subjects::UserAndOpponents,
        user_basis: Some(LegalBasis::Consent),
        third_party_basis: Some(LegalBasis::LegitimateInterest),
        default_retention: Retention::Days(90),
        safeguards: "Masquage des zones hors table dès la capture ; chiffrement en flux ; \
                     suppression automatique après retranscription et délai",
    },
    CategoryInfo {
        category: DataCategory::OpponentProfiles,
        label: "Profils d'adversaires",
        description: "Statistiques de jeu (VPIP, PFR, agressivité…) rattachées à un pseudonyme",
        purpose: "Adaptation de la stratégie de l'utilisateur après partie",
        subjects: Subjects::Opponents,
        user_basis: None,
        third_party_basis: Some(LegalBasis::LegitimateInterest),
        default_retention: Retention::Days(365),
        safeguards: "Minimisation (statistiques agrégées seulement) ; purge un an après la \
                     dernière observation ; effacement sur demande",
    },
    CategoryInfo {
        category: DataCategory::Statistics,
        label: "Statistiques personnelles",
        description: "Gains, ROI, ITM, volume, erreurs récurrentes de l'utilisateur",
        purpose: "Suivi des résultats et de la progression de l'utilisateur",
        subjects: Subjects::User,
        user_basis: Some(LegalBasis::Contract),
        third_party_basis: None,
        default_retention: Retention::UntilErasure,
        safeguards: "Chiffrement au repos ; export et effacement à la demande",
    },
    CategoryInfo {
        category: DataCategory::Progress,
        label: "Progression",
        description: "Résultats des exercices, quiz et parcours de coaching",
        purpose: "Parcours d'entraînement personnalisé",
        subjects: Subjects::User,
        user_basis: Some(LegalBasis::Contract),
        third_party_basis: None,
        default_retention: Retention::UntilErasure,
        safeguards: "Chiffrement au repos ; export et effacement à la demande",
    },
];

/// Inventaire complet.
#[must_use]
pub fn inventory() -> &'static [CategoryInfo] {
    &INVENTORY
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inventory_is_consistent() {
        for category in DataCategory::ALL {
            let info = category.info();
            assert_eq!(info.category, category);
            let third_parties = matches!(
                info.subjects,
                Subjects::UserAndOpponents | Subjects::Opponents
            );
            assert_eq!(
                info.third_party_basis.is_some(),
                third_parties,
                "{category:?}"
            );
            assert_eq!(
                info.user_basis.is_some(),
                info.subjects != Subjects::Opponents
            );
        }
        assert_eq!(inventory().len(), DataCategory::ALL.len());
        assert_eq!(
            DataCategory::OpponentProfiles.label(),
            "Profils d'adversaires"
        );
        assert!(
            serde_json::to_string(inventory())
                .unwrap()
                .contains("legitimate_interest")
        );
    }
}
