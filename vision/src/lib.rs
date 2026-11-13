//! Vision artificielle : lecture des cartes sur des captures d'écran ou des images d'une
//! vidéo de partie, **après la partie** (décision D6 : aucune lecture ni conseil pendant
//! une partie réelle).
//!
//! | Étape | Fonction |
//! |---|---|
//! | trier les images d'un enregistrement | [`Keyframer`] |
//! | trouver les cartes face visible | [`detect_cards`] |
//! | cartes du joueur, board, autres | [`arrange`] |
//! | rang et couleur, avec confiance | [`recognize`], [`Templates`] |
//!
//! La reconnaissance s'appuie d'abord sur les **modèles appris** : l'utilisateur étiquette
//! quelques cartes de sa salle, chaque carte apprend son rang et sa couleur, et les lectures
//! suivantes se font au plus proche voisin. Sans calibration, une police générique donne
//! une hypothèse à faible confiance (0,5 au plus). Chaque lecture porte sa confiance
//! (décision T7) : sous le seuil de revue, elle est signalée, jamais corrigée en silence.
//! Une carte non visible reste inconnue.
//!
//! Rust pur, sans dépendance native : le crate tourne aussi dans le navigateur
//! (WebAssembly).

mod detect;
mod font;
mod frame;
mod keyframe;
mod layout;
mod recognize;

pub use detect::{DetectedCard, detect_cards, read_corner};
pub use frame::{Frame, Rect};
pub use keyframe::Keyframer;
pub use layout::{TableRead, arrange};
pub use recognize::{InkColor, Patch, Recognized, Templates, recognize};

#[cfg(test)]
mod tests;
