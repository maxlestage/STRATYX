//! IA Tournois 6-Max : tables de tournoi à six joueurs, avec antes.
//!
//! Les modèles entraînés par `stratyx-train` (`ai/training`) sont embarqués dans le binaire
//! (`models/tournois-6max.json`) :
//!
//! - **équilibre push/fold** à 6 joueurs (ante de 12,5 %), de 3 à 20 grosses blindes ; les
//!   tables plus courtes s'y lisent par le nombre de joueurs derrière chaque position ;
//! - **modèle de décision** : perceptron multicouche qui prédit la décision de référence
//!   (et une régression softmax de comparaison) ;
//! - **modèle de population** : ce que jouent les joueurs dans un spot.
//!
//! Voir `docs/entrainement.md` pour les données et les mesures de chaque modèle.

use std::sync::OnceLock;

use stratyx_strategy::ModelBundle;

/// Le fichier de modèles, tel qu'embarqué.
pub const MODELS_JSON: &str = include_str!("../models/tournois-6max.json");

/// Les modèles de l'IA (lus une seule fois).
///
/// # Panics
/// Si le fichier embarqué est invalide (vérifié par les tests).
pub fn models() -> &'static ModelBundle {
    static MODELS: OnceLock<ModelBundle> = OnceLock::new();
    MODELS.get_or_init(|| {
        ModelBundle::from_json(MODELS_JSON).expect("modèles Tournois 6-Max valides")
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use stratyx_poker::{HandClass, Rank};
    use stratyx_strategy::charts::Node;
    use stratyx_strategy::classes::class_index;

    #[test]
    fn models_load_and_make_sense() {
        let m = models();
        assert_eq!(m.format, "tournois-6max");
        assert_eq!(m.pushfold.players, 6);
        for metric in &m.metrics {
            assert!(metric.accuracy > metric.baseline, "{metric:?}");
        }
        let aa = class_index(HandClass::Pair(Rank::Ace));
        let s2o = class_index(HandClass::Offsuit(Rank::Seven, Rank::Two));
        assert!(m.pushfold.probability(Node::FirstIn, 10.0, 0, 6, aa) > 0.9);
        assert!(m.pushfold.probability(Node::FirstIn, 10.0, 0, 6, s2o) < 0.1);
        // Plus on parle tard, plus on pousse large.
        let utg = m.pushfold.range_fraction(Node::FirstIn, 10.0, 0, 6);
        let btn = m.pushfold.range_fraction(Node::FirstIn, 10.0, 3, 6);
        assert!(btn > utg);
    }
}
