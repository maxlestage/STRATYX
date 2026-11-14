//! Robustesse de la base locale face à un fichier abîmé (disque plein, copie interrompue,
//! fichier remplacé) : l'ouverture et la lecture devraient répondre par une erreur ou par des
//! données lisibles, jamais par une panique qui ferait planter l'application.
//!
//! La base est un dossier Nuage DB ; chacun de ses fichiers est abîmé tour à tour. Une
//! ancienne base redb abîmée, ouverte pour être importée, passe par le même chemin
//! (`Database::open` d'un fichier).
//!
//! Jusqu'à Nuage DB 0.4, les pages de la base étaient un fichier redb, et redb 4.x paniquait
//! sur certains fichiers dont un seul octet était abîmé (index hors limites dans son allocateur
//! de pages) : ce test était ignoré. Nuage écrit maintenant ses propres pages, avec une somme
//! de contrôle sur chaque nœud : un fichier abîmé donne une erreur, et le test tourne à chaque
//! fois. Parade côté application dans tous les cas : les sauvegardes chiffrées vérifiées
//! (`stratyx_security::backup`).

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};

use stratyx_database::Database;
use stratyx_database::text::{ImportOptions, import_pokerstars};

const CASH: &str = include_str!("data/pokerstars_cash_nl6.txt");

/// Un dossier temporaire propre au test, supprimé à la fin.
struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> TempDir {
        let dir = std::env::temp_dir().join(format!("stratyx-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        TempDir(dir)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Une base réelle contenant les mains de l'historique d'exemple ; renvoie son dossier et
/// les identifiants des mains.
fn sample_database(dir: &Path) -> (PathBuf, Vec<String>) {
    let path = dir.join("modele");
    let mut ids = Vec::new();
    {
        let db = Database::open(&path).unwrap();
        for hand in import_pokerstars(CASH, &ImportOptions::default()) {
            let record = hand.unwrap().record;
            db.put_hand(&record).unwrap();
            ids.push(record.id.clone());
        }
        assert_eq!(db.count_hands(None).unwrap(), ids.len() as u64);
    }
    (path, ids)
}

/// Copie du dossier `from` en `to`.
fn copy_dir(from: &Path, to: &Path) {
    let _ = std::fs::remove_dir_all(to);
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), target).unwrap();
        }
    }
}

/// Fichiers du dossier (chemins relatifs), sous-dossiers compris.
fn files(dir: &Path, base: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let entry = entry.unwrap();
        if entry.file_type().unwrap().is_dir() {
            files(&entry.path(), base, out);
        } else {
            out.push(entry.path().strip_prefix(base).unwrap().to_path_buf());
        }
    }
}

/// Ouvre la base `path` et lit tout ce qu'on peut ; renvoie la panique éventuelle.
fn survives(path: &Path, ids: &[String]) -> Result<(), String> {
    let outcome = catch_unwind(AssertUnwindSafe(|| {
        if let Ok(db) = Database::open(path) {
            let _ = db.schema_version();
            let _ = db.count_hands(None);
            for id in ids {
                let _ = db.get_hand(id);
                let _ = db.hand_summary(id);
            }
        }
    }));
    outcome.map_err(|e| {
        e.downcast_ref::<String>()
            .cloned()
            .or_else(|| e.downcast_ref::<&str>().map(|s| (*s).to_owned()))
            .unwrap_or_default()
    })
}

#[test]
fn damaged_database_files_never_panic() {
    let dir = TempDir::new("base-abimee");
    let (model, ids) = sample_database(&dir.0);
    let damaged = dir.0.join("abimee");
    let mut names = Vec::new();
    files(&model, &model, &mut names);
    assert!(!names.is_empty());
    let mut failures = Vec::new();
    let mut check = |what: String, name: &Path, bytes: &[u8]| {
        copy_dir(&model, &damaged);
        std::fs::write(damaged.join(name), bytes).unwrap();
        if let Err(msg) = survives(&damaged, &ids) {
            failures.push(format!("{} {what} : {msg}", name.display()));
        }
    };
    for name in &names {
        let original = std::fs::read(model.join(name)).unwrap();
        let len = original.len();
        // Fichier vide, remplacé par du texte, coupé à différentes positions.
        check("vide".into(), name, &[]);
        check("texte".into(), name, CASH.as_bytes());
        for cut in [1, 16, 512, len / 2, len.saturating_sub(1)] {
            if cut > 0 && cut < len {
                check(format!("coupé à {cut}"), name, &original[..cut]);
            }
        }
        // Octets abîmés et blocs de 64 octets mis à zéro, répartis dans le fichier.
        for at in (0..len).step_by(509) {
            let mut bytes = original.clone();
            bytes[at] ^= 0xff;
            check(format!("octet {at} inversé"), name, &bytes);
            let end = (at + 64).min(len);
            bytes[at..end].fill(0);
            check(format!("bloc {at}..{end} à zéro"), name, &bytes);
        }
    }
    // Une ancienne base redb remplacée par n'importe quoi : refusée, sans panique.
    let legacy = dir.0.join("ancienne.redb");
    std::fs::write(&legacy, CASH.as_bytes()).unwrap();
    if let Err(msg) = survives(&legacy, &ids) {
        failures.push(format!("ancienne base illisible : {msg}"));
    }
    assert!(failures.is_empty(), "{failures:#?}");
}
