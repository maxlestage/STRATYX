//! Les modèles entraînés d'une IA spécialisée, réunis dans un seul fichier JSON.

use serde::{Deserialize, Serialize};

use crate::model::Classifier;
use crate::pushfold::Chart;

/// La mesure d'un modèle sur les exemples de validation (jamais vus à l'entraînement).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Metric {
    pub model: String,
    /// Exemples d'entraînement et de validation.
    pub train: usize,
    pub validation: usize,
    /// Part des décisions bien prédites.
    pub accuracy: f32,
    /// Entropie croisée moyenne.
    pub log_loss: f32,
    /// Part de la classe la plus fréquente (le score d'un modèle qui répond toujours pareil).
    pub baseline: f32,
    pub parameters: usize,
}

/// D'où viennent les données d'entraînement.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Provenance {
    /// Mains simulées (auto-jeu dans le moteur de règles).
    pub simulated_hands: usize,
    /// Mains importées d'archives réelles (historiques de l'utilisateur ou sous licence).
    pub archive_hands: usize,
    /// Décisions extraites.
    pub decisions: usize,
    pub seed: u64,
    /// Date de l'entraînement (AAAA-MM-JJ).
    pub date: String,
}

/// Tous les modèles d'une IA.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelBundle {
    /// `expresso-3max` ou `tournois-6max`.
    pub format: String,
    pub provenance: Provenance,
    /// Équilibre push/fold (simulations).
    pub pushfold: Chart,
    /// Décision de référence, perceptron multicouche (caractéristiques du spot et de la main).
    pub decision: Classifier,
    /// Même tâche, régression softmax (modèle linéaire de comparaison).
    pub decision_linear: Classifier,
    /// Ce que joue la population dans un spot (contexte seulement, appris sur les archives).
    pub population: Classifier,
    pub metrics: Vec<Metric>,
}

impl ModelBundle {
    /// Lit un fichier de modèles.
    pub fn from_json(json: &str) -> Result<ModelBundle, serde_json::Error> {
        serde_json::from_str(json)
    }
}
