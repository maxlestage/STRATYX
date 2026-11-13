//! IA Expresso 3-Max : sit & go rapides à trois joueurs, jusqu'au tête-à-tête final.
//!
//! Les modèles entraînés par `stratyx-train` (`ai/training`) sont embarqués dans le binaire
//! (`models/expresso-3max.json`) :
//!
//! - **équilibre push/fold** à 3 joueurs, de 2 à 20 grosses blindes (le tête-à-tête s'y
//!   lit en ramenant chaque position à celle qui a le même nombre de joueurs derrière) ;
//! - **modèle de décision** : perceptron multicouche qui prédit la décision de référence
//!   (et une régression softmax de comparaison) ;
//! - **modèle de population** : ce que jouent les joueurs dans un spot.
//!
//! Voir `docs/entrainement.md` pour les données et les mesures de chaque modèle.

use std::sync::OnceLock;

use stratyx_strategy::ModelBundle;

/// Le fichier de modèles, tel qu'embarqué.
pub const MODELS_JSON: &str = include_str!("../models/expresso-3max.json");

/// Les modèles de l'IA (lus une seule fois).
///
/// # Panics
/// Si le fichier embarqué est invalide (vérifié par les tests).
pub fn models() -> &'static ModelBundle {
    static MODELS: OnceLock<ModelBundle> = OnceLock::new();
    MODELS.get_or_init(|| {
        ModelBundle::from_json(MODELS_JSON).expect("modèles Expresso 3-Max valides")
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
        assert_eq!(m.format, "expresso-3max");
        assert_eq!(m.pushfold.players, 3);
        for metric in &m.metrics {
            assert!(metric.accuracy > metric.baseline, "{metric:?}");
        }
        let aa = class_index(HandClass::Pair(Rank::Ace));
        let s2o = class_index(HandClass::Offsuit(Rank::Seven, Rank::Two));
        // Bouton à 10 BB : AA toujours, 72o jamais.
        assert!(m.pushfold.probability(Node::FirstIn, 10.0, 0, 3, aa) > 0.9);
        assert!(m.pushfold.probability(Node::FirstIn, 10.0, 0, 3, s2o) < 0.1);
        // En tête-à-tête à 5 BB, le bouton pousse beaucoup plus de mains.
        assert!(m.pushfold.range_fraction(Node::FirstIn, 5.0, 0, 2) > 0.6);
    }
}
