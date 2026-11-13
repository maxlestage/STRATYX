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
use std::io::{BufRead, Write};
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
        let records: Vec<HandRecord> = match ext.as_str() {
            "txt" => {
                report.files += 1;
                let bytes = std::fs::read(&path)?;
                let text = String::from_utf8_lossy(&bytes);
                let r = import_text(&text);
                report.invalid += r.errors.len();
                r.hands.into_iter().map(|h| h.record).collect()
            }
            "json" => {
                report.files += 1;
                let text = std::fs::read_to_string(&path)?;
                match serde_json::from_str::<Vec<HandRecord>>(&text) {
                    Ok(v) => v,
                    Err(_) => serde_json::from_str::<HandRecord>(&text)
                        .map(|h| vec![h])
                        .unwrap_or_else(|_| {
                            report.invalid += 1;
                            Vec::new()
                        }),
                }
            }
            "jsonl" => {
                report.files += 1;
                read_jsonl(&path, &mut report.invalid)?
            }
            _ => continue,
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

fn read_jsonl(path: &Path, invalid: &mut usize) -> std::io::Result<Vec<HandRecord>> {
    let file = std::fs::File::open(path)?;
    let mut out = Vec::new();
    for line in std::io::BufReader::new(file).lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str(&line) {
            Ok(r) => out.push(r),
            Err(_) => *invalid += 1,
        }
    }
    Ok(out)
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
