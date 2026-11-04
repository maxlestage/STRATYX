# Le site de STRATYX

Le site web de STRATYX, écrit **entièrement en Rust** avec [Paquets](https://github.com/maxlestage/Paquets),
sans une ligne de JavaScript écrite à la main : pages rendues sur le serveur avec leurs balises
SEO, PWA installable et hors ligne, scènes 3D en WebAssembly (`paquets::three`), et le **vrai
moteur `stratyx-poker`** compilé en WebAssembly pour les démos.

## Pages

Accueil, Fonctionnalités, Les IA, Laboratoire, Technologie, Feuille de route, Fair-play,
Contact, Mentions légales, Confidentialité, Plan du site, Merci, 404.

Les textes viennent du cahier des charges et des décisions (`../docs/`) : rien d'inventé.
Les mentions légales attendent les informations de l'éditeur (`src/contenu.rs`).

## Démos du moteur (îlots, dans le navigateur)

| Îlot | Ce qu'il fait | Moteur |
|---|---|---|
| `table3d` | Table 3D (3, 6 ou 9 joueurs), main distribuée, flop/turn/river, évaluation et équité contre une main au hasard | `best_five`, `equity` |
| `equite` | Équité de 2 ou 3 mains ou ranges, board facultatif, exact ou Monte-Carlo | `Range::parse`, `equity` |
| `grille` | Grille des 169 mains, colorée en direct depuis une range | `Range`, `HandClass` |
| `icm` | Équité ICM de chaque tapis selon les paiements | `icm_equities` |
| `jetons` | Jetons qui tombent et s'empilent (physique) | `paquets::three::Physics` |
| `hero3d` | Jetons et cartes en orbite, qui suivent le pointeur et le défilement | `paquets::three` |
| `mouvement` | Les mouvements de chaque page (apparitions, compteurs, curseur, menu…) | `web-sys` |

## Organisation

| Fichier | Rôle |
|---|---|
| `src/main.rs` | Le serveur : mise en page, pages, liste d'attente, contact, PWA, sitemap |
| `src/contenu.rs` | Tous les textes |
| `src/lib.rs` | Les îlots (serveur et navigateur) et leurs calculs |
| `src/scenes.rs` | Les scènes 3D (navigateur) |
| `src/mouvement.rs` | Les mouvements des pages (navigateur) |
| `src/icones.rs` | Les icônes SVG |
| `public/` | Feuille de style, favicon, icônes de la PWA, image Open Graph |

Le site est un crate autonome, exclu du workspace : l'audit `cargo deny` du produit reste
inchangé. Paquets y est utilisé sans SQLite ni PostgreSQL (aucune bibliothèque C) : la base
est en mémoire.

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

## GitHub Pages

`.github/workflows/pages.yml` déploie le site à chaque push sur `master` qui touche `site/`,
`poker/` ou `shared/` (ou à la main, onglet Actions) : tests, bundle WebAssembly, export
statique, publication.

**Une seule fois** : Settings → Pages → Source : « GitHub Actions ».

L'export rend chaque page avec le vrai serveur Paquets (sans ouvrir de port) :

```bash
BASE_PATH=/stratyx SITE_URL=https://maxlestage.github.io/stratyx \
  cargo run --release -- export dist
```

`BASE_PATH` préfixe chaque adresse (liens, manifeste, service worker, bundle) ; le workflow le
prend de GitHub (`/stratyx`, ou vide sur un domaine personnalisé). Pages n'a pas de serveur :
dans l'export, la liste d'attente et le contact deviennent des liens vers le dépôt. Les démos,
la 3D et la PWA fonctionnent à l'identique.

## Mobile first

La feuille de style vise d'abord le téléphone ; les écrans plus larges ajoutent des colonnes
(`min-width` : 641, 961 et 1101 px). Sur mobile, rien ne glisse horizontalement : les
apparitions sont verticales, `html` et `body` coupent tout débordement. Vérifié de 320 à
1440 px sur chaque page.

## Vérifier

```bash
cargo fmt --check
cargo clippy --all-targets
cargo clippy --lib --target wasm32-unknown-unknown
cargo test      # chaque page, chaque îlot, et les calculs du moteur (AA contre KK ≈ 82 %…)
```
