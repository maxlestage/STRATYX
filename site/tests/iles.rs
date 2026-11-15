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

/// Chaque joueur garde le numéro de son champ, dans les résultats comme dans les messages.
#[test]
fn l_equite_nomme_les_joueurs_comme_a_l_ecran() {
    let champs = |a: &str, b: &str, c: &str| [a.to_owned(), b.to_owned(), c.to_owned()];
    // Champ 1 vide : les deux lignes sont celles des joueurs 2 et 3.
    let (lignes, _) = site::calculer_equite(&champs("", "AsAh", "KsKh"), "").unwrap();
    assert_eq!(lignes[0].0, "Joueur 2 · AsAh");
    assert_eq!(lignes[1].0, "Joueur 3 · KsKh");
    let (lignes, _) = site::calculer_equite(&champs("AsAh", "", "KsKh"), "").unwrap();
    assert_eq!(lignes[0].0, "Joueur 1 · AsAh");
    assert_eq!(lignes[1].0, "Joueur 3 · KsKh");
    for (saisie, board, attendu) in [
        (
            champs("AhKh", "AhKh", ""),
            "AhKh2c",
            "la range du joueur 1 est vide",
        ),
        (
            champs("QQ", "", "AhKh"),
            "AhKh2c",
            "la range du joueur 3 est vide",
        ),
        (champs("AA", "ZZ", ""), "", "joueur 2 : "),
        (champs("", "", "AhAh"), "", "joueur 3 : "),
        (champs("", "", ""), "", "au moins deux joueurs (reçu : 0)"),
        (champs("", "AA", ""), "", "au moins deux joueurs (reçu : 1)"),
    ] {
        let erreur = site::calculer_equite(&saisie, board).unwrap_err();
        assert!(erreur.contains(attendu), "{saisie:?} : {erreur}");
        assert!(!erreur.contains("joueur 0"), "{erreur}");
    }
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

/// Le calculateur ICM refuse ce qui n'a pas de sens, avec un message qui dit quoi corriger et
/// qui numérote les joueurs comme à l'écran (J1, J2…).
#[test]
fn l_icm_refuse_les_saisies_absurdes() {
    for (tapis, paiements, attendu) in [
        ("5000,3000,2000", "", "au moins un paiement"),
        ("1000,1000", "0,0", "au moins un paiement"),
        ("1,1", "1e308,1e308", "paiement invalide"),
        ("1,1", "inf", "paiement invalide"),
        ("1,1", "nan", "paiement invalide"),
        ("1000,1000", "100,-50", "paiement invalide"),
        ("1000,1000", "1000000000001", "paiement invalide"),
        ("5000,0,2000", "50,30,20", "le joueur 2 a un tapis nul"),
        ("0,0,0", "50,30,20", "le joueur 1 a un tapis nul"),
        ("abc", "50", "tapis illisible"),
        (
            "99999999999999999999999999999999999999999999999999999999999999",
            "50",
            "tapis illisible",
        ),
        ("1000", "def", "paiement illisible"),
        ("1,2,3,4,5,6,7,8,9,10", "50", "9 joueurs au plus"),
    ] {
        let erreur = site::calculer_icm(tapis, paiements).unwrap_err();
        assert!(erreur.contains(attendu), "{tapis} / {paiements} : {erreur}");
        assert!(erreur.chars().count() < 120, "{erreur}");
    }
    // Ce qui a un sens reste accepté : montants, espaces, plus de prix que de joueurs.
    for (tapis, paiements) in [
        ("5000,3000,2000", "50,30,20"),
        ("5 000, 3 000", "60,40"),
        ("5000,3000", "60,40,20,10"),
        ("1000", "100"),
        ("1500,1500,1500", "1000000000000, 0"),
        ("10,20", "0, 0.5"),
    ] {
        let lignes = site::calculer_icm(tapis, paiements).unwrap();
        assert!(
            lignes.iter().all(|l| l.1.is_finite() && l.1 >= 0.0),
            "{tapis} / {paiements}"
        );
        assert!(
            lignes.iter().map(|l| l.1).sum::<f64>() > 0.0,
            "{tapis} / {paiements}"
        );
    }
}
