# STRATYX

Plateforme professionnelle d'analyse de **Texas Hold'em**, **entièrement écrite en Rust** : capture et
retranscription automatique des parties, analyse stratégique, profils d'adversaires, coaching,
avec deux IA spécialisées (**Expresso 3-Max** et **Tournois 6-Max**) adossées à un moteur de
calcul commun.

- Cahier des charges : [`docs/cahier-des-charges.md`](docs/cahier-des-charges.md)
- Décisions et audit des dépendances : [`docs/decisions.md`](docs/decisions.md)

Couverture : no-limit, pot-limit et limite fixe ; tables de 2 à 10 joueurs ; cash game,
tournois, sit & go et Expresso.

> Périmètre : analyse **après** partie et entraînement. Aucune assistance stratégique en direct
> pendant une partie réelle.

## État

- Le moteur poker (`poker/`) est opérationnel et testé, avec l'import des historiques texte
  PokerStars et Winamax.
- Les deux IA existent en première version (phase 4) : équilibres push/fold, modèle de
  décision et modèle de population, entraînés en Rust pur (voir
  [`docs/entrainement.md`](docs/entrainement.md)).
- L'analyse des mains après la partie (`analytics`) est livrée : `stratyx analyse` et la page
  `/app` du site.
- Les autres crates sont des squelettes qui matérialisent l'architecture.

## Démarrer

```sh
cargo test --workspace                                # tests (≈ 1 s)
cargo test --release -p stratyx-poker -- --ignored    # évaluateur exhaustif, 133 M de mains
cargo deny check                                      # audit « 100 % Rust » (cargo install cargo-deny)

cargo run --release -p stratyx-cli -- eval AsKsQsJsTs9h2c
cargo run --release -p stratyx-cli -- equity AhKh "QQ+,AKo" --board Qs7h2h
cargo run --release -p stratyx-cli -- range "22+,A2s+,KTo+"
cargo run --release -p stratyx-cli -- icm --stacks 5000,3000,2000 --payouts 50,30,20
cargo run --release -p stratyx-cli -- analyse mes-mains.txt --joueur MonPseudo

# Entraîner les modèles des deux IA (≈ 5 min sur 4 cœurs), avec vos propres historiques
# rangés dans archives/expresso-3max/ et archives/tournois-6max/ :
cargo run --release -p stratyx-ai-training --bin stratyx-train -- --archives archives
```

## Organisation

| Crate | Rôle | Phase |
|---|---|---|
| `shared` | jetons, confiance, provenance des données | 1 ✅ |
| `poker` | moteur Texas Hold'em : cartes, évaluation, ranges, équité, règles NL/PL/FL, formats, pots, historiques, import des historiques texte, ICM | 1 ✅ |
| `tools/cli` | outil `stratyx` (dont `stratyx analyse`) | 1 ✅ |
| `strategy` | 169 classes, ranges adverses, équilibres push/fold, référence EV, modèles en Rust pur | 4 ✅ |
| `ai/training` | `stratyx-train` : simulations, archives, apprentissage, rapport | 4 ✅ |
| `ai/expresso_3max`, `ai/tournament_6max` | les modèles entraînés de chaque IA, embarqués | 4 ✅ |
| `analytics` | analyse des mains après la partie (profils et statistiques à venir) | 5 🟡 |
| `capture`, `video`, `vision` | capture d'écran, vidéo, reconnaissance | 2 |
| `database` | quatre bases : Expresso, Tournois, Stratégie, Utilisateurs | 3 |
| `ai/inference` | exécution des modèles lourds sur CPU ou GPU (Burn) | 4 |
| `coaching` | exercices, quiz, parcours | 6 |
| `apps/*` | bureau, mobile, serveur et API | 7 |
| `security` | chiffrement, authentification, RGPD | 8 |
| `site` | site web en Rust (Paquets, WebAssembly), PWA complète, dessins en Canvas 2D, démos du moteur et page d'analyse `/app` : voir [`site/README.md`](site/README.md) | — |

## Déployer le site depuis son téléphone (Heroku)

[![Deploy to Heroku](https://www.herokucdn.com/deploy/button.svg)](https://heroku.com/deploy?template=https://github.com/maxlestage/STRATYX)

Le site web (`site/`) se déploie sur Heroku avec le buildpack
[emk/rust](https://elements.heroku.com/buildpacks/emk/heroku-buildpack-rust), stack
heroku-26 : [`app.json`](app.json), [`RustConfig`](RustConfig),
[`heroku-build.sh`](heroku-build.sh) (serveur + WebAssembly), [`Procfile`](Procfile).

1. **Première mise en ligne** : touchez le bouton ci-dessus, choisissez un nom d'app,
   touchez *Deploy app*. La première compilation prend une quinzaine de minutes.
2. **Déploiements automatiques** : sur GitHub, *Settings → Secrets and variables →
   Actions* :
   - onglet *Secrets* : `HEROKU_API_KEY` (Heroku → *Account settings* → *API Key* →
     *Reveal*) ;
   - onglet *Variables* : `HEROKU_APP_NAME`, le nom de l'app (créée si elle n'existe pas).

   Ensuite, chaque branche dont les tests passent est fusionnée dans `master`, puis
   [`heroku.yml`](.github/workflows/heroku.yml) déploie le site. À la main : *Actions →
   Déploiement Heroku → Run workflow* (l'application GitHub mobile le permet).

Heroku n'a pas d'offre gratuite (un dyno Eco coûte environ 5 $ par mois). La liste
d'attente et les messages sont gardés en mémoire : ils sont perdus à chaque redémarrage du
dyno (au moins une fois par jour sur Heroku).

## Règle « 100 % Rust »

`deny.toml` bloque toute dépendance qui compile du C/C++ ou embarque une bibliothèque native
(OpenCV, FFmpeg, SQLite, OpenSSL, ring, aws-lc, libtorch, ONNX Runtime, Skia…). Il impose aussi
l'approbation nominative de chaque script de build. Seules les API des systèmes d'exploitation
et les pilotes sont autorisés (décision D1). Le contrôle s'exécute à chaque push.
