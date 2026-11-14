//! Les archives : lecture (historiques texte des salles, JSON `HandRecord`), nettoyage,
//! déduplication et validation par rejeu ; écriture en JSON Lines.
//!
//! Dossier attendu (`--archives <dossier>`) :
//!
//! ```text
//! <dossier>/expresso-3max/   historiques Expresso (3 joueurs)
//! <dossier>/tournois-6max/   historiques de tournois 6-max
//! ```
//!
//! Chaque fichier `.txt` est lu par `stratyx_poker::import`, chaque `.json` ou `.jsonl`
//! comme des `HandRecord`. Une main dont le format ne correspond pas à l'IA visée est
//! écartée. N'y mettez que vos propres mains ou des données sous licence vérifiée
//! (cahier des charges, section 8 ; points de vigilance de `docs/decisions.md`).

use std::collections::HashSet;
use std::io::Write;
use std::path::Path;

use stratyx_poker::Specialist;
use stratyx_poker::history::HandRecord;
use stratyx_poker::import::import_text;

/// Bilan de lecture d'un dossier d'archives.
#[derive(Debug, Default, Clone)]
pub struct LoadReport {
    pub files: usize,
    pub read: usize,
    pub duplicates: usize,
    pub invalid: usize,
    pub other_format: usize,
}

/// Lit les archives d'un dossier pour une IA ; renvoie les mains valides et dédupliquées.
pub fn load(dir: &Path, specialist: Specialist) -> std::io::Result<(Vec<HandRecord>, LoadReport)> {
    let mut report = LoadReport::default();
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    if !dir.is_dir() {
        return Ok((out, report));
    }
    let mut files: Vec<_> = std::fs::read_dir(dir)?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .collect();
    files.sort();
    for path in files {
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();
        match ext.as_str() {
            "txt" | "json" | "jsonl" => {}
            _ => continue,
        }
        report.files += 1;
        // Un fichier illisible (droits, sous-dossier nommé « x.json »…) compte comme
        // invalide : il n'arrête plus la lecture des autres archives.
        let Ok(bytes) = std::fs::read(&path) else {
            report.invalid += 1;
            continue;
        };
        let records: Vec<HandRecord> = match ext.as_str() {
            "txt" => {
                let text = String::from_utf8_lossy(&bytes);
                let r = import_text(&text);
                report.invalid += r.errors.len();
                r.hands.into_iter().map(|h| h.record).collect()
            }
            "json" => match serde_json::from_slice::<Vec<HandRecord>>(&bytes) {
                Ok(v) => v,
                Err(_) => serde_json::from_slice::<HandRecord>(&bytes)
                    .map(|h| vec![h])
                    .unwrap_or_else(|_| {
                        report.invalid += 1;
                        Vec::new()
                    }),
            },
            _ => read_jsonl(&bytes, &mut report.invalid),
        };
        for r in records {
            report.read += 1;
            if r.format.specialist() != Some(specialist) {
                report.other_format += 1;
                continue;
            }
            if !seen.insert(r.id.clone()) {
                report.duplicates += 1;
                continue;
            }
            if r.replay().is_err() {
                report.invalid += 1;
                continue;
            }
            out.push(r);
        }
    }
    Ok((out, report))
}

/// Une main JSON par ligne ; une ligne illisible (JSON invalide, octets non UTF-8) compte
/// comme invalide sans écarter les autres.
fn read_jsonl(bytes: &[u8], invalid: &mut usize) -> Vec<HandRecord> {
    let mut out = Vec::new();
    for line in bytes.split(|b| *b == b'\n') {
        if line.trim_ascii().is_empty() {
            continue;
        }
        match serde_json::from_slice(line) {
            Ok(r) => out.push(r),
            Err(_) => *invalid += 1,
        }
    }
    out
}

/// Écrit des mains en JSON Lines (une main par ligne).
pub fn write_jsonl(path: &Path, records: &[HandRecord]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut f = std::io::BufWriter::new(std::fs::File::create(path)?);
    for r in records {
        serde_json::to_writer(&mut f, r).map_err(std::io::Error::other)?;
        f.write_all(b"\n")?;
    }
    f.flush()
}

#[cfg(test)]
mod tests {
    use super::*;
    use stratyx_poker::import::SAMPLE_WINAMAX;

    #[test]
    fn unreadable_files_do_not_stop_the_load() {
        let dir = std::env::temp_dir().join(format!(
            "stratyx-archives-{}-{}",
            std::process::id(),
            unique()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        // Une main Expresso valide en texte, et la même en JSON Lines (doublon).
        std::fs::write(dir.join("a.txt"), SAMPLE_WINAMAX).unwrap();
        let hand = import_text(SAMPLE_WINAMAX).hands.remove(0).record;
        write_jsonl(&dir.join("b.jsonl"), std::slice::from_ref(&hand)).unwrap();
        // Des fichiers abîmés : JSON non UTF-8, une ligne illisible au milieu d'un JSON
        // Lines, un sous-dossier qui porte une extension d'archive, un fichier sans rapport.
        std::fs::write(dir.join("c.json"), [0xff, 0xfe, b'[', 0x80]).unwrap();
        let mut lines = serde_json::to_vec(&hand).unwrap();
        lines.extend_from_slice(b"\n\xff\xfe pas du json\n\n");
        std::fs::write(dir.join("d.jsonl"), lines).unwrap();
        std::fs::create_dir_all(dir.join("piege.json")).unwrap();
        std::fs::write(dir.join("notes.md"), "rien à lire").unwrap();

        let (hands, report) = load(&dir, Specialist::Expresso3Max).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(hands.len(), 1, "{report:?}");
        assert_eq!(report.files, 5, "{report:?}"); // a.txt b.jsonl c.json d.jsonl piege.json
        assert_eq!(report.read, 3, "{report:?}");
        assert_eq!(report.duplicates, 2, "{report:?}");
        // c.json, la ligne abîmée de d.jsonl, et piege.json (illisible comme fichier).
        assert_eq!(report.invalid, 3, "{report:?}");
    }

    /// Un suffixe propre à chaque appel (tests lancés en parallèle).
    pub(super) fn unique() -> u64 {
        use std::sync::atomic::{AtomicU64, Ordering};
        static N: AtomicU64 = AtomicU64::new(0);
        N.fetch_add(1, Ordering::Relaxed)
    }
}
