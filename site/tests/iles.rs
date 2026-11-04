//! Les îlots se rendent sur le serveur, et leurs calculs sont ceux du moteur STRATYX.

use paquets::prelude::*;
use stratyx_site as site;

#[test]
fn chaque_ile_se_rend() {
    for (nom, ile) in site::ISLANDS {
        let html = island(nom, "", ile).render();
        assert!(html.contains(&format!("data-island=\"{nom}\"")), "{nom}");
    }
}

#[test]
fn la_main_initiale_est_une_quinte_flush_royale_a_la_river() {
    let (titre, eq) = site::analyser(&site::MAIN_INITIALE, 3);
    assert!(titre.to_lowercase().contains("quinte flush"), "{titre}");
    assert!(eq.starts_with("100"), "{eq}");
    let (preflop, _) = site::analyser(&site::MAIN_INITIALE, 0);
    assert!(preflop.contains("AKs"), "{preflop}");
}

#[test]
fn donner_tire_des_cartes_differentes() {
    let cartes = site::donner(7);
    assert_eq!(cartes.len(), 7);
    let mut uniques = cartes.clone();
    uniques.sort();
    uniques.dedup();
    assert_eq!(uniques.len(), 7);
    assert!(cartes.iter().all(|c| *c < 52));
}

#[test]
fn equite_as_contre_rois() {
    let (lignes, info) =
        site::calculer_equite(&["AsAh".into(), "KsKh".into(), String::new()], "").unwrap();
    assert_eq!(lignes.len(), 2);
    assert!((lignes[0].1 - 0.82).abs() < 0.02, "{lignes:?}");
    assert!(info.contains("stratyx-poker"));
    assert!(site::calculer_equite(&["XX".into(), "KK".into()], "").is_err());
}

#[test]
fn grille_et_icm() {
    assert_eq!(site::classes_grille().len(), 169);
    let (poids, combos, _) = site::poids_grille("AA,AKs").unwrap();
    assert_eq!(combos, 10.0);
    assert_eq!(poids.iter().filter(|p| **p > 0.0).count(), 2);
    let icm = site::calculer_icm("5000,3000,2000", "50,30,20").unwrap();
    let total: f64 = icm.iter().map(|l| l.1).sum();
    assert!((total - 100.0).abs() < 1e-6);
    assert!(icm[0].1 < 50.0 && icm[0].1 > icm[1].1);
}
