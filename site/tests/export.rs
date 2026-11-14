//! L'export statique (`stratyx-site export <dossier>`, copie GitHub Pages) servi sous un
//! préfixe : chaque adresse du site écrite dans les pages, la feuille de style, le manifeste
//! et le service worker porte le préfixe et désigne un fichier exporté.
//!
//! Le binaire est lancé comme le fait `pages.yml` : l'export bascule un état global du
//! serveur, il a donc son propre processus.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;

const PREFIXE: &str = "/prefixe";

struct Dossier(PathBuf);

impl Drop for Dossier {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Les valeurs qui suivent `marque` dans `texte`, jusqu'à `fin`.
fn valeurs<'a>(texte: &'a str, marque: &str, fin: char) -> Vec<&'a str> {
    texte
        .split(marque)
        .skip(1)
        .filter_map(|suite| suite.split(fin).next())
        .collect()
}

fn fichiers(dossier: &Path, extension: &str, dans: &mut Vec<PathBuf>) {
    for entree in std::fs::read_dir(dossier).unwrap() {
        let chemin = entree.unwrap().path();
        if chemin.is_dir() {
            fichiers(&chemin, extension, dans);
        } else if chemin.extension().is_some_and(|e| e == extension) {
            dans.push(chemin);
        }
    }
}

/// Le fichier exporté que désigne l'adresse `adresse` du site (préfixe compris), s'il existe.
fn existe(racine: &Path, adresse: &str) -> bool {
    let chemin = adresse.split(['?', '#']).next().unwrap_or_default();
    let Some(relatif) = chemin.strip_prefix(PREFIXE) else {
        return false;
    };
    let cible = racine.join(relatif.trim_start_matches('/'));
    cible.is_file() || cible.join("index.html").is_file()
}

#[test]
fn l_export_sous_un_prefixe_ne_laisse_aucune_adresse_cassee() {
    let temp = Dossier(std::env::temp_dir().join(format!("stratyx-export-{}", std::process::id())));
    let racine = temp.0.join("site");
    let sortie = Command::new(env!("CARGO_BIN_EXE_stratyx-site"))
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .arg("export")
        .arg(&racine)
        .env("BASE_PATH", PREFIXE)
        .env("SITE_URL", format!("https://exemple.test{PREFIXE}"))
        .env("NUAGE_DATA_DIR", temp.0.join("base"))
        .env_remove("NUAGE_URL")
        .env_remove("NUAGE_TOKEN")
        .output()
        .unwrap();
    assert!(
        sortie.status.success(),
        "{}",
        String::from_utf8_lossy(&sortie.stderr)
    );

    let mut cassees = BTreeSet::new();
    let mut vues = 0;
    let mut verifier = |fichier: &Path, adresse: &str, doit_exister: bool| {
        // Les adresses complètes, les ancres et les données ne sont pas des fichiers du site.
        if !adresse.starts_with('/') || adresse.starts_with("//") {
            return;
        }
        vues += 1;
        let sous_prefixe = adresse == PREFIXE || adresse.starts_with(&format!("{PREFIXE}/"));
        if !sous_prefixe || (doit_exister && !existe(&racine, adresse)) {
            cassees.insert(format!(
                "{} : {adresse}",
                fichier.strip_prefix(&racine).unwrap().display()
            ));
        }
    };

    // Les pages : liens, feuilles de style, scripts, images, préchargements, formulaires.
    let mut pages = Vec::new();
    fichiers(&racine, "html", &mut pages);
    assert!(pages.len() >= 10, "{} pages", pages.len());
    for page in &pages {
        let html = std::fs::read_to_string(page).unwrap();
        for attribut in [" href=\"", " src=\"", " poster=\"", " content=\""] {
            for adresse in valeurs(&html, attribut, '"') {
                verifier(page, adresse, true);
            }
        }
        // Un formulaire s'adresse au serveur : préfixé, mais sans fichier dans l'export.
        for adresse in valeurs(&html, " action=\"", '"') {
            verifier(page, adresse, false);
        }
    }

    // Les feuilles de style : adresses absolues préfixées, adresses relatives à la feuille.
    let mut feuilles = Vec::new();
    fichiers(&racine, "css", &mut feuilles);
    assert!(!feuilles.is_empty());
    let mut polices = 0;
    for feuille in &feuilles {
        let css = std::fs::read_to_string(feuille).unwrap();
        for brut in valeurs(&css, "url(", ')') {
            let adresse = brut.trim_matches(['"', '\'']);
            // `url(%23n)` : renvoi interne d'une image SVG écrite dans la feuille (`data:`).
            if adresse.starts_with("data:")
                || adresse.starts_with("%23")
                || adresse.starts_with('#')
            {
                continue;
            }
            if adresse.starts_with('/') {
                verifier(feuille, adresse, true);
            } else {
                // Relative à la feuille : la même adresse, écrite depuis la racine du site.
                let dossier = feuille.parent().unwrap().strip_prefix(&racine).unwrap();
                let depuis_la_racine = Path::new(PREFIXE).join(dossier).join(adresse);
                verifier(
                    feuille,
                    &depuis_la_racine.to_string_lossy().replace('\\', "/"),
                    true,
                );
            }
            polices += usize::from(adresse.ends_with(".woff2"));
        }
    }
    assert_eq!(polices, 4, "polices de style.css");

    // Le manifeste et le service worker : toute chaîne qui commence par `/`.
    for nom in ["manifest.webmanifest", "sw.js"] {
        let fichier = racine.join(nom);
        let texte = std::fs::read_to_string(&fichier).unwrap();
        let mut dans_ce_fichier = 0;
        for adresse in valeurs(&texte, "\"/", '"') {
            let adresse = format!("/{adresse}");
            // `/health` (jamais gardé par le worker) n'existe que sur le serveur.
            let serveur = adresse.ends_with("/health");
            verifier(&fichier, &adresse, !serveur);
            dans_ce_fichier += 1;
        }
        assert!(dans_ce_fichier >= 5, "{nom} : {dans_ce_fichier} adresses");
    }

    assert!(vues >= 200, "{vues} adresses vérifiées");
    assert!(
        cassees.is_empty(),
        "{} adresses cassées sur {vues} :\n{}",
        cassees.len(),
        cassees.into_iter().collect::<Vec<_>>().join("\n")
    );
}
