//! Chiffrement au repos : XChaCha20-Poly1305, Argon2id, coffres versionnés.
//!
//! ## Hiérarchie de clés
//!
//! - Une **clé de données** ([`DataKey`], 256 bits aléatoires) chiffre le contenu.
//! - Elle est **enveloppée** par une clé dérivée du mot de passe avec Argon2id
//!   ([`KeySlot`]). Changer de mot de passe ne réenveloppe que la clé : aucune donnée n'est
//!   rechiffrée ([`KeySlot::rewrap`], [`rotate_password`]).
//!
//! ## Deux usages
//!
//! - **Données locales** : un trousseau ([`KeySlot::to_bytes`]) protège la clé de données ;
//!   chaque fichier ([`blob::seal`], [`StreamEncryptor::new`]) ne porte que l'identifiant de
//!   clé. Argon2id n'est calculé qu'une fois, au déverrouillage.
//! - **Coffres autonomes** (sauvegardes, exports) : l'emplacement de clé est embarqué dans
//!   l'en-tête ([`blob::seal_with_password`], [`StreamEncryptor::with_password`]) ; le coffre
//!   s'ouvre sur n'importe quelle machine avec le mot de passe.
//!
//! ## Choix cryptographiques
//!
//! | Élément | Choix | Raison |
//! |---|---|---|
//! | Chiffrement authentifié | XChaCha20-Poly1305 | nonce de 192 bits tiré au hasard sans risque de collision ; rapide sans AES matériel (mobiles) |
//! | Dérivation | Argon2id v0x13, sel de 32 octets | RFC 9106 ; paramètres stockés dans chaque coffre |
//! | Gros fichiers | STREAM (BE32) par blocs de 64 Kio | troncature, suppression et réordonnancement détectés |
//! | Effacement | `zeroize` | clés dérivées et clés de données effacées à leur libération |
//!
//! Toutes ces crates sont du Rust pur (RustCrypto), sans script de build ni code C. Certaines
//! dépendances (`cmov` pour les sélections en temps constant, la barrière de `zeroize`)
//! utilisent la macro `asm!` du langage, compilée par `rustc` : aucun assembleur externe.
//!
//! ## Limites
//!
//! - L'effacement mémoire est « au mieux » : des copies temporaires peuvent subsister dans
//!   des registres ou sur la pile ; les pages ne sont pas verrouillées contre le swap.
//! - Les mots de passe sont pris tels quels (octets UTF-8) : l'interface doit les normaliser
//!   (NFC) si la saisie peut varier d'un système à l'autre.

pub mod blob;
mod error;
mod kdf;
mod key;
mod slot;
pub mod stream;
mod vault;

pub use error::CryptoError;
pub use kdf::{KdfParams, SALT_LEN};
pub use key::{DataKey, KEY_ID_LEN, KEY_LEN, KeyId, NONCE_LEN, TAG_LEN};
pub use slot::{KEYRING_MAGIC, KEYRING_VERSION, KeySlot};
pub use stream::{StreamDecryptor, StreamEncryptor, decrypt_stream, encrypt_stream};
pub use vault::{
    PayloadKind, Unlock, VAULT_MAGIC, VAULT_VERSION, VaultHeader, rotate_password,
    rotate_password_in_memory,
};
