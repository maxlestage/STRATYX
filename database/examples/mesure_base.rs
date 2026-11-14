//! Mesure des opérations de la base locale (Nuage DB) sur `n` mains distinctes (1 000 par
//! défaut) : écriture une à une, ingestion d'un lot, recherche, lecture, revalidation.
//!
//! `cargo run --release -p stratyx-database --example mesure_base -- 5000`

use std::time::Instant;

use stratyx_database::text::{ImportOptions, import_pokerstars};
use stratyx_database::{ArchiveBatch, Database, HandQuery, IngestPolicy, Provenance};

const CASH: &str = include_str!("../tests/data/pokerstars_cash_nl6.txt");

fn main() {
    let n: usize = std::env::args()
        .nth(1)
        .and_then(|a| a.parse().ok())
        .unwrap_or(1_000)
        .max(2);
    let modeles: Vec<_> = import_pokerstars(CASH, &ImportOptions::default())
        .into_iter()
        .map(|h| h.expect("historique d'exemple lisible").record)
        .collect();
    // Des mains distinctes (un pseudonyme différent change l'empreinte), datées.
    let hands: Vec<_> = (0..n)
        .map(|i| {
            let mut h = modeles[i % modeles.len()].clone();
            h.id = format!("m{i}");
            h.meta.played_at_ms = Some(i as u64 * 1_000);
            h.seats[0].player = format!("Joueur{i}");
            h
        })
        .collect();
    let db = Database::in_memory().expect("base temporaire");

    let t = Instant::now();
    for h in &hands[..n / 2] {
        db.put_hand(h).expect("écriture");
    }
    let half = u32::try_from(n / 2).unwrap_or(u32::MAX);
    println!(
        "put_hand ×{} : {:?} ({:?} par main)",
        n / 2,
        t.elapsed(),
        t.elapsed() / half
    );

    let t = Instant::now();
    let policy = IngestPolicy {
        accept_incomplete: true,
        accept_needing_review: true,
        ..IngestPolicy::default()
    };
    let provenance = Provenance::new("mesure", "CC0").expect("provenance");
    let report = db
        .ingest(
            ArchiveBatch::new(provenance, hands[n / 2..].to_vec()),
            policy,
        )
        .expect("ingestion");
    println!(
        "ingest ×{} : {:?} ({} acceptées)",
        n - n / 2,
        t.elapsed(),
        report.accepted.len()
    );

    let player = hands[0].seats[1].player.clone();
    for (name, query) in [
        ("joueur", HandQuery::new().player(player).page(0, 20)),
        ("toutes", HandQuery::new().page(0, 20)),
        (
            "salle (parcours)",
            HandQuery::new().site("PokerStars").page(0, 20),
        ),
    ] {
        let t = Instant::now();
        for _ in 0..20 {
            db.search(&query).expect("recherche");
        }
        println!("search {name} : {:?} par page", t.elapsed() / 20);
    }

    let t = Instant::now();
    for i in 0..200 {
        db.get_hand(&format!("m{i}")).expect("lecture");
    }
    println!("get_hand : {:?} par main", t.elapsed() / 200);

    let t = Instant::now();
    let changed = db.revalidate().expect("revalidation");
    println!("revalidate ({changed} changées) : {:?}", t.elapsed());
}
