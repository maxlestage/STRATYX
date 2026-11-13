//! Durées de conservation et purge des données expirées (minimisation, art. 5.1.e).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::GdprError;
use super::audit::{AuditAction, AuditLog, AuditOutcome};
use super::inventory::DataCategory;
use super::store::{PersonalDataStore, RecordRef};

/// Une journée en millisecondes.
pub const DAY_MS: u64 = 24 * 60 * 60 * 1000;

/// Durée de conservation d'une catégorie.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Retention {
    /// Conservé jusqu'à l'effacement demandé par l'utilisateur.
    UntilErasure,
    /// Purgé ce nombre de jours après sa date de référence (création, ou dernière
    /// observation pour un profil d'adversaire). `0` : purgé dès la purge suivante.
    Days(u32),
}

impl Retention {
    /// Date limite : les enregistrements dont la date de référence est **strictement
    /// antérieure** sont expirés. `None` pour [`Retention::UntilErasure`].
    #[must_use]
    pub fn cutoff_ms(self, now_ms: u64) -> Option<u64> {
        match self {
            Self::UntilErasure => None,
            Self::Days(days) => Some(now_ms.saturating_sub(u64::from(days) * DAY_MS)),
        }
    }
}

/// Politique de conservation configurable, par catégorie. Une catégorie absente suit la
/// durée par défaut de l'inventaire.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RetentionPolicy {
    rules: BTreeMap<DataCategory, Retention>,
}

impl Default for RetentionPolicy {
    fn default() -> Self {
        Self {
            rules: DataCategory::ALL
                .into_iter()
                .map(|c| (c, c.info().default_retention))
                .collect(),
        }
    }
}

impl RetentionPolicy {
    /// Durée applicable à une catégorie.
    #[must_use]
    pub fn get(&self, category: DataCategory) -> Retention {
        self.rules
            .get(&category)
            .copied()
            .unwrap_or(category.info().default_retention)
    }

    /// Fixe la durée d'une catégorie.
    pub fn set(&mut self, category: DataCategory, retention: Retention) -> &mut Self {
        self.rules.insert(category, retention);
        self
    }

    /// Calcule, sans rien supprimer, les enregistrements expirés à `now_ms`.
    ///
    /// # Errors
    /// [`GdprError::Store`] si la base ne répond pas.
    pub fn plan_purge<S: PersonalDataStore + ?Sized>(
        &self,
        store: &S,
        now_ms: u64,
    ) -> Result<PurgePlan, GdprError> {
        let mut items = Vec::new();
        for category in DataCategory::ALL {
            if let Some(cutoff) = self.get(category).cutoff_ms(now_ms) {
                let expired = store
                    .list_records(category, Some(cutoff))
                    .map_err(GdprError::store)?;
                // Défense en profondeur : on ne fait pas confiance au filtre de la base.
                items.extend(
                    expired
                        .into_iter()
                        .filter(|r| r.category == category && r.reference_ms < cutoff),
                );
            }
        }
        Ok(PurgePlan {
            computed_at_ms: now_ms,
            items,
        })
    }
}

/// Enregistrements à purger, calculés par [`RetentionPolicy::plan_purge`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PurgePlan {
    /// Date du calcul.
    pub computed_at_ms: u64,
    /// Enregistrements expirés.
    pub items: Vec<RecordRef>,
}

impl PurgePlan {
    /// Vrai si rien n'est à purger.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Nombre d'enregistrements à purger dans une catégorie.
    #[must_use]
    pub fn count(&self, category: DataCategory) -> usize {
        self.items.iter().filter(|r| r.category == category).count()
    }

    /// Exécute la purge et la consigne au journal d'audit (succès ou échec) ; renvoie le
    /// nombre d'enregistrements supprimés.
    ///
    /// # Errors
    /// [`GdprError::Store`].
    pub fn execute<S: PersonalDataStore + ?Sized>(
        &self,
        store: &mut S,
        audit: &mut AuditLog,
        now_ms: u64,
    ) -> Result<usize, GdprError> {
        let summary = DataCategory::ALL
            .into_iter()
            .filter_map(|c| match self.count(c) {
                0 => None,
                n => Some(format!("{} : {n}", c.label())),
            })
            .collect::<Vec<_>>()
            .join(", ");
        match store.delete_records(&self.items) {
            Ok(deleted) => {
                audit.append(
                    now_ms,
                    AuditAction::RetentionPurge,
                    None,
                    AuditOutcome::Completed,
                    deleted as u64,
                    format!("purge des données expirées ({summary})"),
                );
                Ok(deleted)
            }
            Err(e) => {
                let error = GdprError::store(e);
                audit.append(
                    now_ms,
                    AuditAction::RetentionPurge,
                    None,
                    AuditOutcome::Failed,
                    0,
                    error.to_string(),
                );
                Err(error)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::store::{DataSubject, InMemoryStore};
    use super::*;

    const NOW: u64 = 400 * DAY_MS;

    #[test]
    fn cutoffs() {
        assert_eq!(Retention::UntilErasure.cutoff_ms(NOW), None);
        assert_eq!(Retention::Days(90).cutoff_ms(NOW), Some(310 * DAY_MS));
        assert_eq!(Retention::Days(u32::MAX).cutoff_ms(NOW), Some(0));
    }

    #[test]
    fn purge_plan_respects_policy() {
        let user = DataSubject::user("u1");
        let mut store = InMemoryStore::new();
        // Vidéos : 90 jours par défaut.
        store.insert(
            DataCategory::Videos,
            "vieille",
            NOW - 91 * DAY_MS,
            [user.clone()],
            "video/x-stratyx",
            vec![1],
        );
        store.insert(
            DataCategory::Videos,
            "récente",
            NOW - 89 * DAY_MS,
            [user.clone()],
            "video/x-stratyx",
            vec![2],
        );
        // Pile à la limite : conservée (strictement antérieure seulement).
        store.insert(
            DataCategory::Videos,
            "limite",
            NOW - 90 * DAY_MS,
            [user.clone()],
            "video/x-stratyx",
            vec![3],
        );
        // Mains : conservées jusqu'à effacement.
        store.insert(
            DataCategory::HandHistories,
            "main",
            0,
            [user.clone()],
            "application/json",
            vec![],
        );
        // Profil d'adversaire non revu depuis 400 jours.
        let villain = DataSubject::opponent("winamax", "Villain").unwrap();
        store.insert(
            DataCategory::OpponentProfiles,
            "p1",
            0,
            [villain],
            "application/json",
            vec![],
        );

        let policy = RetentionPolicy::default();
        let plan = policy.plan_purge(&store, NOW).unwrap();
        assert_eq!(plan.count(DataCategory::Videos), 1);
        assert_eq!(plan.count(DataCategory::OpponentProfiles), 1);
        assert_eq!(plan.count(DataCategory::HandHistories), 0);
        assert_eq!(store.len(), 5, "le calcul ne supprime rien");

        let mut audit = AuditLog::new();
        assert_eq!(plan.execute(&mut store, &mut audit, NOW).unwrap(), 2);
        assert!(!store.contains(DataCategory::Videos, "vieille"));
        assert!(store.contains(DataCategory::Videos, "limite"));
        assert_eq!(audit.entries().len(), 1);
        assert!(audit.entries()[0].detail.contains("Vidéos : 1"));

        // Politique durcie : mains conservées 30 jours.
        let mut strict = RetentionPolicy::default();
        strict.set(DataCategory::HandHistories, Retention::Days(30));
        assert_eq!(
            strict
                .plan_purge(&store, NOW)
                .unwrap()
                .count(DataCategory::HandHistories),
            1
        );
        let json = serde_json::to_string(&strict).unwrap();
        assert_eq!(
            serde_json::from_str::<RetentionPolicy>(&json).unwrap(),
            strict
        );
    }
}
