# Le site de STRATYX

Le site web de STRATYX, écrit **entièrement en Rust** avec [Paquets](https://github.com/maxlestage/Paquets),
sans une ligne de JavaScript écrite à la main : pages rendues sur le serveur avec leurs balises
SEO, PWA complète (installable, hors ligne, raccourcis, partage), dessins 2D en WebAssembly
(Canvas 2D), le **vrai moteur `stratyx-poker`** compilé en WebAssembly pour les démos, et
la page **Analyse** (`/app`) où les modèles entraînés des deux IA analysent vos mains dans
le navigateur.

## Pages

Accueil, Analyse (`/app`), Fonctionnalités, Les IA, Laboratoire, Technologie, Feuille de
route, Fair-play, Contact, Mentions légales, Confidentialité, Plan du site, Merci, Hors
ligne, 404.

## Analyse d'une partie (`/app`)

L'îlot `analyse` (`src/analyse.rs`) lit un ou plusieurs historiques texte (PokerStars, Winamax
et formats proches) collés, importés d'un fichier `.txt` ou partagés vers l'application
installée. Chaque main est rejouée par le moteur, puis chaque décision du joueur est comparée
à la référence de l'IA du format (`stratyx-analytics`) : équilibre push/fold ou référence EV,
modèle de décision, tendances de la population, EV perdue, gravité, pistes de travail. Tout
tourne en WebAssembly : rien n'est envoyé au serveur, et la page marche hors ligne (le
service worker sert `/app` sans l'adresse complète d'un partage, voir la décision T12).

La section **Vidéo et captures** filme l'écran ou la caméra, ou lit une vidéo ou des captures
choisies sur l'appareil. Pendant l'enregistrement, rien n'est lu ni conseillé ; à l'arrêt, les
cartes sont reconnues (calibration en touchant les cartes douteuses, gardée dans le
navigateur) et chaque image devient un spot à analyser (décision T13). Seule `/app` ouvre la
caméra et la capture d'écran dans ses en-têtes.


Les textes viennent du cahier des charges et des décisions (`../docs/`) : rien d'inventé.
Les mentions légales attendent les informations de l'éditeur (`src/contenu.rs`).

## Démos du moteur (îlots, dans le navigateur)

| Îlot | Ce qu'il fait | Moteur |
|---|---|---|
| `analyse` | Analyse d'une main collée, importée ou partagée, et bilan de toute la session | `stratyx-analytics` |
| `capture` | Filmer l'écran ou la caméra, importer une vidéo ou des captures ; à l'arrêt, cartes reconnues, calibration, analyse de chaque spot | `stratyx-vision`, `analytics::spot` |
| `table2d` | Table vue de dessus en 2D (3, 6 ou 9 joueurs), main distribuée, flop/turn/river, évaluation et équité contre une main au hasard | `best_five`, `equity` |
| `equite` | Équité de 2 ou 3 mains ou ranges, board facultatif, exact ou Monte-Carlo | `Range::parse`, `equity` |
| `grille` | Grille des 169 mains, colorée en direct depuis une range | `Range`, `HandClass` |
| `icm` | Équité ICM de chaque tapis selon les paiements | `icm_equities` |
| `jetons` | Jetons qui tombent et s'empilent (physique 2D écrite en Rust) | `src/scenes.rs` |
| `hero2d` | Jetons et cartes à plat qui flottent, suivent le pointeur et le défilement | Canvas 2D |
| `mouvement` | Les mouvements de chaque page (apparitions, compteurs, curseur, menu…) | `web-sys` |

## Application installable (PWA)

Le site est une PWA complète, sans une ligne de JavaScript écrite à la main dans les pages :

- **Manifeste** (`/manifest.webmanifest`, `src/pwa.rs`) : `id`, `scope`, icônes (dont
  masquable et SVG), `display_override` (dont `window-controls-overlay`), raccourcis
  (Analyser une partie, Laboratoire, Les IA), captures d'écran large et étroite,
  `share_target` (un texte partagé depuis une autre application arrive sur `/app?texte=…`),
  `launch_handler`.
- **Service worker** (`/sw.js`) : chaque page du plan du site, `/app`, la page
  `/hors-ligne`, la feuille de style, les icônes et le bundle WebAssembly sont gardés dès
  l'installation ; les pages passent d'abord par le réseau, le reste est servi depuis le
  cache et mis à jour derrière ; les polices Google sont gardées à part. La version du
  cache est un hachage de ce qu'il garde : chaque déploiement installe un nouveau worker.
- **Dans la page** (`src/mouvement.rs`, en Rust) : enregistrement du worker, message
  « Nouvelle version disponible » avec mise à jour en un clic, messages hors ligne / en
  ligne, bouton « Installer l'application » (et explication sur iPhone et iPad).

Les captures d'écran du manifeste (`public/screenshots/`) et les icônes des raccourcis
(`public/icons/raccourci-*.png`) sont faites avec Chromium sur le vrai site.

## Organisation

| Fichier | Rôle |
|---|---|
| `src/main.rs` | Le serveur : mise en page, pages, liste d'attente, contact, PWA, sitemap |
| `src/contenu.rs` | Tous les textes |
| `src/lib.rs` | Les îlots (serveur et navigateur) et leurs calculs |
| `src/analyse.rs` | L'îlot de la page Analyse (`/app`) : historiques et bilan de session |
| `src/capture.rs`, `src/media.rs` | Vidéo et captures : enregistrement, import, revue, spots |
| `src/scenes.rs` | Les scènes 2D en Canvas 2D (navigateur) |
| `src/anim.rs` | La boucle d'animation (`requestAnimationFrame`) |
| `src/pwa.rs` | La PWA : manifeste, service worker, balises du `<head>` |
| `src/mouvement.rs` | Les mouvements des pages (navigateur) |
| `src/icones.rs` | Les icônes SVG |
| `public/` | Feuille de style, favicon, icônes de la PWA, image Open Graph |

Le site réutilise les crates du produit : `stratyx-poker`, `stratyx-strategy` et
`stratyx-analytics` (qui embarque les modèles de `ai/expresso_3max` et `ai/tournament_6max`).

Le site est un crate autonome, exclu du workspace : l'audit `cargo deny` du produit reste
inchangé. Paquets y est utilisé sans SQLite ni PostgreSQL ; les données du site sont dans
Nuage DB embarqué (voir « Base de données »).

## Lancer

Depuis ce dossier (`site/`) :

```bash
# Une fois
rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli --version 0.2.129

# 1. Le bundle du navigateur → pkg/
cargo build --lib --target wasm32-unknown-unknown --profile wasm-release
wasm-bindgen --target web --no-typescript --out-dir pkg \
  target/wasm32-unknown-unknown/wasm-release/stratyx_site.wasm

# 2. Le serveur : http://localhost:3000
cargo run --release
```

`.cargo/config.toml` donne à `getrandom` (dépendance de `rand`, utilisé par le moteur) le
hasard du navigateur en WebAssembly. Variables : `PORT`, `SITE_URL` (URL canoniques et
sitemap).

## Heroku (déploiement principal)

Voir la section « Déployer le site depuis son téléphone » du [README](../README.md) :
bouton *Deploy to Heroku*, buildpack emk/rust (`RustConfig` à la racine lance
`heroku-build.sh`, qui compile le serveur et le WebAssembly ; le `Procfile` démarre
`site/target/release/stratyx-site`), puis déploiement automatique de `master` par
`.github/workflows/heroku.yml`.

**Fusion automatique** : la CI (`.github/workflows/ci.yml`) teste chaque push, sur toutes les
branches. Quand tout est vert, la branche est fusionnée dans `master` (sauf `wip/...`) et le
site est déployé sur Heroku.

## Base de données (liste d'attente, messages de contact)

Le serveur garde les inscriptions à la liste d'attente (table `inscrit`) et les messages de
contact (table `message`) dans **Nuage DB**, la base de Memoire, en Rust pur, **embarquée
dans le serveur** (module `base` de `src/main.rs`) : pas de serveur de base à part, pas
d'abonnement.

Heroku efface le disque à chaque redémarrage (au moins une fois par jour, et à chaque
déploiement). Nuage DB s'en accommode : il envoie ses sauvegardes (instantanés et journal,
chiffrés) vers un **stockage compatible S3** et, quand son dossier est vide au démarrage, il
s'y **restaure tout seul**. À l'arrêt (`SIGTERM`), le serveur fait un dernier point de
contrôle et une dernière sauvegarde.

| Variable | Rôle |
|---|---|
| `NUAGE_ENCRYPTION_KEY` | clé de chiffrement de la base et des sauvegardes (`openssl rand -hex 32`) : **à garder**, sans elle rien ne se relit |
| `NUAGE_BACKUP_S3_BUCKET`, `NUAGE_BACKUP_S3_ENDPOINT`, `NUAGE_BACKUP_S3_REGION` | le seau S3 et son adresse |
| `NUAGE_BACKUP_S3_ACCESS_KEY_ID`, `NUAGE_BACKUP_S3_SECRET_ACCESS_KEY` | la clé d'accès au seau |
| `NUAGE_DATA_DIR` | dossier de travail de la base (`nuage-data` par défaut) |
| `NUAGE_BACKUP_DIR` | sauvegarde vers un dossier, au lieu de S3 (disque persistant, essais en local) |

Sans sauvegarde configurée, la base vit seulement dans `nuage-data` : en local, elle
persiste ; sur Heroku, elle est perdue au redémarrage (le serveur l'écrit au démarrage).

**Gratuitement, sur Heroku :**

1. Créer un seau sur un stockage compatible S3 qui a une offre gratuite (par exemple
   Backblaze B2, 10 Go gratuits au moment d'écrire ces lignes ; vérifier ses conditions),
   puis une clé d'application limitée à ce seau. Noter le nom du seau, l'adresse S3
   (`https://s3.<région>.backblazeb2.com` pour B2), la région, l'identifiant et le secret
   de la clé.
2. Générer la clé de chiffrement : `openssl rand -hex 32` (et la garder en lieu sûr).
3. Dans le dépôt GitHub : *Settings → Secrets and variables → Actions*, ajouter les secrets
   `NUAGE_ENCRYPTION_KEY`, `NUAGE_BACKUP_S3_BUCKET`, `NUAGE_BACKUP_S3_ENDPOINT`,
   `NUAGE_BACKUP_S3_REGION`, `NUAGE_BACKUP_S3_ACCESS_KEY_ID`,
   `NUAGE_BACKUP_S3_SECRET_ACCESS_KEY`.
4. Relancer le déploiement (*Actions → Déploiement Heroku → Run workflow*) : il transmet ces
   valeurs à Heroku (et retire l'ancien `DATABASE_URL`), et le serveur affiche
   `base Nuage DB : … (sauvegardes vers …, restaurées au démarrage)`.

Un seul dyno web : deux dynos seraient deux bases écrivant dans les mêmes sauvegardes.

**Dépendances.** Nuage DB chiffre avec AES et ses sauvegardes S3 passent par rustls avec
`ring` (C et assembleur) : exception propre au serveur du site, hors de l'audit `cargo deny`
du produit. Nuage a sa propre version de Memoire, distincte de celle de Paquets : le site
lui parle en SQL (valeurs toujours en paramètres), et la base de Paquets reste en mémoire,
inutilisée.

## Copie statique (GitHub Pages, à la main)

`.github/workflows/pages.yml` ne se lance plus qu'à la main (*Actions → Site (GitHub Pages) →
Run workflow*). L'export rend chaque page avec le vrai serveur Paquets :

```bash
BASE_PATH=/STRATYX SITE_URL=https://maxlestage.github.io/STRATYX \
  cargo run --release -- export dist
```

Sans serveur, la liste d'attente et le contact y deviennent des liens vers le dépôt.

## Mobile first

La feuille de style vise d'abord le téléphone ; les écrans plus larges ajoutent des colonnes
(`min-width` : 641, 961 et 1101 px). Sur mobile, rien ne glisse horizontalement : les
apparitions sont verticales, `html` et `body` coupent tout débordement (avec un repli
`overflow-x: hidden` pour les navigateurs sans `clip`). Aucun bloc ne défile en largeur
non plus : le terminal passe à la ligne, et les grilles utilisent `minmax(0, …)` pour leurs colonnes.
Vérifié de 320 à 1440 px sur chaque page.

## Vérifier

```bash
cargo fmt --check
cargo clippy --all-targets
cargo clippy --lib --target wasm32-unknown-unknown
cargo test      # chaque page, chaque îlot, et les calculs du moteur (AA contre KK ≈ 82 %…)
```
