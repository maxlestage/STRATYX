//! L'application installable (PWA) : le manifeste, le service worker et les balises du
//! `<head>`, écrits ici plutôt que par `app.pwa(...)` de Paquets pour avoir tout ce qu'une
//! PWA complète déclare : raccourcis, captures d'écran, cible de partage, gestion du
//! lancement, et un service worker qui garde chaque page, le bundle WebAssembly et les
//! polices pour le hors-ligne, avec une mise à jour proposée à l'utilisateur.
//!
//! Le service worker est enregistré par le bundle WebAssembly (src/mouvement.rs) : aucune
//! ligne de JavaScript dans les pages, la Content-Security-Policy reste stricte.

use std::sync::OnceLock;

use crate::contenu::{NOM, PAGES};

/// La couleur du thème et du fond de l'application.
pub const COULEUR: &str = "#07100c";

/// La page montrée hors ligne quand la page demandée n'a jamais été visitée.
pub const HORS_LIGNE: &str = "/hors-ligne";

/// Les pages gardées hors ligne en plus du plan du site (qui contient déjà /app).
const PAGES_EN_PLUS: [&str; 1] = [HORS_LIGNE];

/// Le bundle des îlots, quand il est construit (voir le README).
const BUNDLE: [&str; 2] = ["/pkg/stratyx_site.js", "/pkg/stratyx_site_bg.wasm"];

/// Les fichiers statiques gardés dès l'installation.
const STATIQUES: [&str; 6] = [
    "/style.css",
    "/favicon.svg",
    "/manifest.webmanifest",
    "/icons/icon-192.png",
    "/icons/icon-512.png",
    "/icons/apple-touch-icon.png",
];

/// Les balises du `<head>` de chaque page.
pub const BALISES: &str = concat!(
    r#"<link rel="manifest" href="/manifest.webmanifest"/>"#,
    r##"<meta name="theme-color" content="#07100c"/>"##,
    r#"<meta name="application-name" content="STRATYX"/>"#,
    r#"<meta name="mobile-web-app-capable" content="yes"/>"#,
    r#"<meta name="apple-mobile-web-app-capable" content="yes"/>"#,
    r#"<meta name="apple-mobile-web-app-status-bar-style" content="black-translucent"/>"#,
    r#"<meta name="apple-mobile-web-app-title" content="STRATYX"/>"#,
    r##"<meta name="msapplication-TileColor" content="#07100c"/>"##,
    r#"<meta name="msapplication-TileImage" content="/icons/icon-192.png"/>"#,
);

/// Le manifeste de l'application web.
pub fn manifeste() -> &'static str {
    static MANIFESTE: OnceLock<String> = OnceLock::new();
    MANIFESTE.get_or_init(|| {
        format!(
            r##"{{
  "id": "/",
  "name": "{NOM} · Analyse de Texas Hold'em",
  "short_name": "{NOM}",
  "description": "Analyse de Texas Hold'em en Rust : analyse de parties, équité, ranges, ICM et deux IA spécialisées (Expresso 3-Max, Tournois 6-Max). Les outils marchent hors ligne.",
  "lang": "fr",
  "dir": "ltr",
  "start_url": "/?source=pwa",
  "scope": "/",
  "display": "standalone",
  "display_override": ["window-controls-overlay", "standalone", "minimal-ui"],
  "orientation": "any",
  "theme_color": "{COULEUR}",
  "background_color": "{COULEUR}",
  "categories": ["games", "education", "sports", "utilities"],
  "prefer_related_applications": false,
  "launch_handler": {{ "client_mode": "navigate-existing" }},
  "icons": [
    {{ "src": "/icons/icon-192.png", "sizes": "192x192", "type": "image/png", "purpose": "any" }},
    {{ "src": "/icons/icon-512.png", "sizes": "512x512", "type": "image/png", "purpose": "any" }},
    {{ "src": "/icons/icon-maskable-512.png", "sizes": "512x512", "type": "image/png", "purpose": "maskable" }},
    {{ "src": "/favicon.svg", "sizes": "any", "type": "image/svg+xml", "purpose": "any" }}
  ],
  "shortcuts": [
    {{
      "name": "Analyser une partie",
      "short_name": "Analyser",
      "description": "Collez un historique de main : le moteur et les IA l'analysent.",
      "url": "/app?source=raccourci",
      "icons": [{{ "src": "/icons/raccourci-analyse.png", "sizes": "96x96", "type": "image/png" }}]
    }},
    {{
      "name": "Laboratoire : équité, ranges, ICM",
      "short_name": "Laboratoire",
      "description": "Les outils du moteur, dans le navigateur, même hors ligne.",
      "url": "/laboratoire?source=raccourci",
      "icons": [{{ "src": "/icons/raccourci-laboratoire.png", "sizes": "96x96", "type": "image/png" }}]
    }},
    {{
      "name": "Les deux IA",
      "short_name": "Les IA",
      "description": "IA Expresso 3-Max et IA Tournois 6-Max.",
      "url": "/ia?source=raccourci",
      "icons": [{{ "src": "/icons/raccourci-ia.png", "sizes": "96x96", "type": "image/png" }}]
    }}
  ],
  "screenshots": [
    {{
      "src": "/screenshots/accueil-large.png",
      "sizes": "1280x720",
      "type": "image/png",
      "form_factor": "wide",
      "label": "L'accueil de STRATYX sur ordinateur"
    }},
    {{
      "src": "/screenshots/analyse-etroit.png",
      "sizes": "390x844",
      "type": "image/png",
      "form_factor": "narrow",
      "label": "L'analyse d'une partie sur téléphone"
    }}
  ],
  "share_target": {{
    "action": "/app",
    "method": "GET",
    "params": {{ "title": "titre", "text": "texte", "url": "lien" }}
  }}
}}
"##
        )
    })
}

/// Tout ce que le service worker garde dès l'installation.
pub fn precache() -> Vec<String> {
    let mut urls: Vec<String> = PAGES.iter().map(|(p, _)| (*p).to_owned()).collect();
    urls.extend(PAGES_EN_PLUS.iter().map(|p| (*p).to_owned()));
    urls.extend(STATIQUES.iter().map(|p| (*p).to_owned()));
    if std::path::Path::new("pkg").is_dir() {
        urls.extend(BUNDLE.iter().map(|p| (*p).to_owned()));
    }
    urls
}

/// Un hachage court (FNV-1a, 10 caractères hexadécimaux) de ce que le service worker garde :
/// chaque déploiement qui change une de ces parties installe un nouveau worker.
pub fn version<I, B>(parties: I) -> String
where
    I: IntoIterator<Item = B>,
    B: AsRef<[u8]>,
{
    let mut h = 0xcbf2_9ce4_8422_2325_u64;
    for partie in parties {
        for octet in partie.as_ref() {
            h = (h ^ u64::from(*octet)).wrapping_mul(0x0100_0000_01b3);
        }
        h = (h ^ 0xff).wrapping_mul(0x0100_0000_01b3);
    }
    format!("{h:016x}")[..10].to_owned()
}

fn tableau(urls: &[String]) -> String {
    let mut out = String::from("[");
    for (i, u) in urls.iter().enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        out.push('"');
        out.push_str(&u.replace('\\', "\\\\").replace('"', "\\\""));
        out.push('"');
    }
    out.push(']');
    out
}

/// Le service worker.
pub fn service_worker() -> &'static str {
    static WORKER: OnceLock<String> = OnceLock::new();
    WORKER.get_or_init(|| {
        let urls = precache();
        let mut parties: Vec<Vec<u8>> = vec![
            env!("CARGO_PKG_VERSION").as_bytes().to_vec(),
            manifeste().as_bytes().to_vec(),
            SCRIPT.as_bytes().to_vec(),
            urls.join("\n").into_bytes(),
        ];
        for f in [
            "public/style.css",
            "pkg/stratyx_site.js",
            "pkg/stratyx_site_bg.wasm",
        ] {
            if let Ok(octets) = std::fs::read(f) {
                parties.push(octets);
            }
        }
        // Le serveur lui-même : une nouvelle version change aussi les pages rendues.
        if let Ok(exe) = std::env::current_exe()
            && let Ok(octets) = std::fs::read(exe)
        {
            parties.push(octets);
        }
        SCRIPT
            .replace("__VERSION__", &version(&parties))
            .replace("__PRECACHE__", &tableau(&urls))
            .replace("__HORS_LIGNE__", HORS_LIGNE)
    })
}

const SCRIPT: &str = r#"// Service worker de STRATYX : chaque page, le moteur en WebAssembly et les polices restent
// disponibles hors ligne. Écrit par le serveur du site (site/src/pwa.rs).
const VERSION = "__VERSION__";
const PREFIXE = "stratyx-";
const CACHE = PREFIXE + "site-" + VERSION;
const POLICES = PREFIXE + "polices";
const PRECACHE = __PRECACHE__;
const HORS_LIGNE = "__HORS_LIGNE__";
// Toujours le réseau (en plus des requêtes POST : formulaires).
const RESEAU = ["/health"];
const POLICES_ORIGINES = ["https://fonts.googleapis.com", "https://fonts.gstatic.com"];

self.addEventListener("install", (event) => {
  event.waitUntil(
    caches.open(CACHE)
      // Une adresse qui manque n'empêche pas l'installation.
      .then((cache) => Promise.allSettled(
        PRECACHE.map((url) => cache.add(new Request(url, { cache: "reload" }))),
      ))
      // Première installation : le worker prend la main tout de suite. Une mise à jour
      // attend que la page la propose (message SKIP_WAITING).
      .then(() => (self.registration.active ? undefined : self.skipWaiting())),
  );
});

self.addEventListener("activate", (event) => {
  event.waitUntil(
    caches.keys()
      .then((cles) => Promise.all(
        cles.filter((cle) => cle.startsWith(PREFIXE) && cle !== CACHE && cle !== POLICES)
          .map((cle) => caches.delete(cle)),
      ))
      .then(() => self.clients.claim()),
  );
});

self.addEventListener("message", (event) => {
  if (event.data && event.data.type === "SKIP_WAITING") self.skipWaiting();
});

function garder(requete, reponse) {
  if (reponse.ok && reponse.type === "basic") {
    const copie = reponse.clone();
    caches.open(CACHE).then((cache) => cache.put(requete, copie));
  }
  return reponse;
}

self.addEventListener("fetch", (event) => {
  const requete = event.request;
  if (requete.method !== "GET") return;
  const url = new URL(requete.url);

  // Les polices : d'abord le cache, gardées d'une version à l'autre.
  if (POLICES_ORIGINES.includes(url.origin)) {
    event.respondWith(
      caches.open(POLICES).then((cache) => cache.match(requete).then((vue) => vue || fetch(requete)
        .then((reponse) => {
          if (reponse.ok || reponse.type === "opaque") cache.put(requete, reponse.clone());
          return reponse;
        }))),
    );
    return;
  }
  if (url.origin !== self.location.origin) return;
  if (RESEAU.some((chemin) => url.pathname.startsWith(chemin))) return;

  // Un historique partagé vers l'application (/app?texte=…) reste sur l'appareil : la page
  // /app est servie sans l'adresse complète, et l'îlot lit le texte dans le navigateur.
  if (requete.mode === "navigate" && url.pathname === "/app" && url.search.length > 1) {
    event.respondWith(
      fetch("/app")
        .then((reponse) => garder(new Request("/app"), reponse))
        .catch(() => caches.match("/app"))
        .then((vue) => vue || caches.match(HORS_LIGNE))
        .then((vue) => vue || Response.error()),
    );
    return;
  }

  // Les pages : d'abord le réseau, puis la copie gardée, puis la page hors ligne.
  if (requete.mode === "navigate") {
    event.respondWith(
      fetch(requete)
        .then((reponse) => garder(requete, reponse))
        .catch(() => caches.match(requete)
          .then((vue) => vue || caches.match(requete, { ignoreSearch: true }))
          .then((vue) => vue || caches.match(HORS_LIGNE))
          .then((vue) => vue || Response.error())),
    );
    return;
  }

  // Le reste (style, WebAssembly, icônes) : la copie tout de suite, mise à jour derrière.
  event.respondWith(
    caches.match(requete).then((vue) => {
      const reseau = fetch(requete).then((reponse) => garder(requete, reponse));
      if (vue) {
        event.waitUntil(reseau.catch(() => {}));
        return vue;
      }
      return reseau;
    }),
  );
});
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn le_manifeste_est_complet() {
        let m = manifeste();
        for cle in [
            "\"id\"",
            "\"shortcuts\"",
            "\"screenshots\"",
            "\"share_target\"",
            "\"display_override\"",
            "\"launch_handler\"",
            "\"maskable\"",
            "\"form_factor\": \"wide\"",
            "\"form_factor\": \"narrow\"",
        ] {
            assert!(m.contains(cle), "{cle}");
        }
        // Chaque fichier cité existe dans public/.
        for chemin in m.split("\"src\": \"").skip(1) {
            let chemin = chemin.split('"').next().unwrap_or_default();
            assert!(
                std::path::Path::new("public")
                    .join(chemin.trim_start_matches('/'))
                    .is_file(),
                "{chemin} manque dans public/"
            );
        }
    }

    #[test]
    fn le_service_worker_garde_chaque_page() {
        let sw = service_worker();
        for (page, _) in PAGES {
            assert!(sw.contains(&format!("\"{page}\"")), "{page}");
        }
        assert!(sw.contains("\"/app\""));
        assert!(sw.contains("\"/hors-ligne\""));
        assert!(sw.contains("SKIP_WAITING"));
        assert!(!sw.contains("__"), "un gabarit n'a pas été remplacé");
        assert_eq!(version([b"a".as_slice()]).len(), 10);
        assert_ne!(version([b"a".as_slice()]), version([b"b".as_slice()]));
    }
}
