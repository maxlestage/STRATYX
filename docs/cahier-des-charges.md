# Cahier des charges — Stratyx

**Version 1.1 — 2026-11-04** (D8 : couverture complète du Texas Hold'em). Les décisions de cadrage D1 à D8 sont détaillées dans
[`decisions.md`](decisions.md).

## 1. Objet

Logiciel professionnel d'analyse de **Texas Hold'em**. Il capture l'écran ou importe une vidéo, retranscrit
automatiquement chaque main, l'analyse en profondeur et aide le joueur à progresser grâce à deux IA
spécialisées.

**Règle fondamentale.** Tout est écrit en Rust : moteurs, IA, interfaces, serveur, outils. Seules
les API des systèmes d'exploitation et les pilotes sont utilisés depuis Rust (D1). Aucune
dépendance native tierce en C, C++, Python, Swift ou Kotlin. La règle est contrôlée
automatiquement par `cargo deny check`.

## 2. Décisions validées

| # | Sujet | Décision |
|---|---|---|
| D1 | Politique Rust | Rust partout, API natives des systèmes autorisées |
| D2 | Plateformes | Ordinateur et mobile, priorité à l'ordinateur (Windows, macOS, Linux, puis iOS, Android) |
| D3 | Calcul IA | CPU + GPU selon le matériel (Burn : `ndarray` / `wgpu`) |
| D8 | Variante | Texas Hold'em uniquement, sous toutes ses formes : no-limit, pot-limit, limite fixe ; 2 à 10 joueurs ; cash game, tournoi, sit & go, Expresso (remplace D4) |
| D5 | Entraînement IA | Archives + simulations |
| D6 | Utilisation | Analyse après partie et entraînement, en modes distincts |
| D7 | Ambition | Plateforme complète avec API et services professionnels, livrée par étapes |

## 3. Les deux IA

Elles partagent le moteur de calcul commun (`poker/`). Chacune a ses propres archives,
entraînements, simulations et évaluations. Les autres formats de Texas Hold'em (cash game,
tables pleines, pot-limit, limite fixe…) sont analysés par le moteur commun, sans IA dédiée
(D8).

**IA Expresso 3-Max** (`ai/expresso_3max`) : push/fold et tapis courts, ranges par position,
heads-up et jeu à trois, stratégie préflop et postflop, multiplicateurs et structures de paiement.

**IA Tournois 6-Max** (`ai/tournament_6max`) : open, 3-bet et 4-bet, ranges par position,
postflop, gestion des tapis, ICM, bulle et table finale, adaptation aux adversaires.

## 4. Capture vidéo et vision

- Capture de l'écran pendant la partie, sans aucune aide affichée (D6). Import de vidéos existantes.
- Reconnaissance des cartes, joueurs, positions (de UTG à BB, tables de 2 à 10 joueurs),
  blindes, antes, tapis, mises et actions (fold, check, call, bet, raise, all-in).
- Pots principaux et secondaires calculés par le moteur.
- Événements synchronisés avec la vidéo (horodatage en millisecondes).
- Détections incertaines signalées : confiance sous le seuil, ou transition refusée par les règles.
- **Aucune carte privée invisible n'est inventée.** Si une carte manque pour désigner un gagnant,
  le résultat est déclaré « non résolu ».

## 5. Retranscription

Chaque main est reconstruite (préflop, flop, turn, river, abattage, résultat), puis **validée par
rejeu** dans le moteur de règles. L'utilisateur peut revoir, corriger, valider, rechercher et
exporter (JSON aujourd'hui, formats texte des salles ensuite).

## 6. Analyse stratégique

GTO et jeu exploitant, équité et probabilités, cotes du pot et fold equity, EV, sizings, erreurs,
bluffs et value bets, ICM et gestion du risque. La décision réelle est comparée à la décision de
référence, **indépendamment du résultat de la main**.

## 7. Adversaires, statistiques et coaching

- **Profils** : VPIP, PFR, 3-bet, 4-bet, agressivité, défense des blindes, c-bet, tendances au
  bluff ou au call, comportement par position, historique des confrontations. Chaque statistique
  porte un indicateur de fiabilité (taille d'échantillon, intervalle de confiance).
- **Statistiques personnelles** : gains, ROI, ITM, volume, résultats par position, évolution,
  erreurs récurrentes, décisions coûteuses, points forts et faibles.
- **Coaching** : exercices préflop et postflop, push/fold, ICM, quiz, simulations, parcours
  personnalisé, détection des faiblesses, suivi de progression.

## 8. Bases de données

| Base | Contenu |
|---|---|
| Expresso | Archives 3-Max |
| Tournois | Archives 6-Max |
| Hold'em  | Autres formats de Texas Hold'em (cash game, tables pleines, PL, FL) |
| Stratégie | Ranges, simulations, références |
| Utilisateurs | Parties, statistiques, progression |

Avant tout entraînement, les archives sont nettoyées, structurées, dédupliquées et validées par
rejeu. Licence vérifiée pour chaque source.

## 9. Architecture

```
STRATYX/
├── apps/            desktop · mobile · server
├── capture/         capture d'écran (API système)
├── video/           import, décodage, enregistrement, horodatage
├── vision/          reconnaissance avec score de confiance
├── poker/           moteur commun Texas Hold'em : cartes, évaluation, ranges, équité,
│                    règles NL/PL/FL, formats, pots, ICM
├── ai/              training · inference · expresso_3max · tournament_6max
├── strategy/        ranges de référence, push/fold, solveurs
├── analytics/       analyse des mains, profils, statistiques
├── coaching/        exercices, quiz, parcours
├── database/        persistance des quatre bases
├── security/        chiffrement, authentification, RGPD
├── shared/          types transverses (jetons, confiance, provenance)
└── tools/cli/       outil en ligne de commande `stratyx`
```

Dépendances autorisées : `shared` ← `poker` ← (`vision`, `strategy`, `ai/*`, `analytics`,
`database`) ← `coaching` ← `apps/*`. Aucun cycle. Le moteur `poker` ne dépend d'aucun module
d'entrée-sortie.

## 10. Sécurité et conformité

Chiffrement et authentification, autorisations de capture, protection des vidéos et historiques,
suppression et export des données, sauvegardes. Respect du RGPD et du droit à l'image,
vérification des licences des archives, respect des conditions des salles. **Hors périmètre :
toute assistance stratégique en direct pendant une partie réelle.** Voir les points de vigilance
de `decisions.md`.

## 11. Feuille de route

| Phase | Contenu | Statut |
|---|---|---|
| 1 | Fondations Rust et moteur poker | **En cours** : moteur livré (voir ci-dessous) |
| 2 | Capture vidéo et reconnaissance | À faire |
| 3 | Retranscription automatique | Format pivot, validation par rejeu et import des historiques texte (PokerStars, Winamax) livrés |
| 4 | IA Expresso et IA Tournois | **En cours** : première version livrée (voir ci-dessous) |
| 5 | Analyses et statistiques | Analyse des mains livrée ; profils et statistiques à faire |
| 6 | Coaching et simulations | À faire |
| 7 | Interfaces et API | Site web : PWA complète, page d'analyse `/app` (WebAssembly, hors ligne) |
| 8 | Tests, sécurité et optimisation | Audit des dépendances et CI déjà en place |

### Phase 1 — livré

- Workspace Cargo (tous les modules), audit `cargo-deny`, intégration continue.
- `shared` : `Chips`, `Confidence`, `Source`, `Observed<T>`.
- `poker` :
  - cartes et ensembles de cartes ;
  - évaluateur 5 à 7 cartes, vérifié sur les 133 784 560 mains à 7 cartes ;
  - ranges (syntaxe standard, pondérations) ;
  - équité exacte ou Monte-Carlo, parallèle et reproductible ;
  - règles du Texas Hold'em en no-limit, pot-limit et limite fixe : ordre de parole, relance
    minimale, plafond de mises, tapis incomplets, antes classiques et de grosse blinde ;
  - tables de 2 à 10 joueurs (positions UTG à BB) ;
  - formats : cash game, tournoi, sit & go, Expresso, avec l'IA compétente pour chacun ;
  - pots secondaires et mises non suivies ;
  - historique `HandRecord` avec provenance et confiance, validé par rejeu ;
  - ICM exact (Malmuth-Harville) ;
  - formules de cotes et d'EV.
- `tools/cli` : `stratyx eval | equity | range | icm`.

### Phase 1 — reste à faire

- Export des historiques texte des salles (l'import PokerStars et Winamax est livré :
  `poker::import`).
- Straddles, blindes mortes et blindes postées à l'arrivée (cash game).
- Banc d'essai de performance (évaluations par seconde, équité par range).

### Phase 4 — livré (première version)

- `strategy` : 169 classes et leur force, ranges probables des adversaires d'après leurs
  actions, équité Monte-Carlo, **équilibres push/fold** à 3 et à 6 joueurs (jeu fictif sur
  simulations, chip EV), **décision de référence** de chaque spot (push/fold ou EV),
  caractéristiques des situations et modèles (softmax, perceptron multicouche) en Rust pur
  (T10).
- `ai/training` (`stratyx-train`) : simulateur d'auto-jeu (sit & go Expresso complets, tables
  de tournoi 6-max), lecture des archives réelles (texte des salles, JSON), apprentissage
  (Adam, validation sur des mains jamais vues), rapport [`entrainement.md`](entrainement.md).
- `ai/expresso_3max`, `ai/tournament_6max` : les modèles entraînés de chaque IA (équilibre
  push/fold, décision, décision linéaire, population), embarqués.
- `analytics` : analyse d'une main après la partie, décision par décision, avec gravité, EV
  perdue, avis des modèles, tendances de la population et pistes de travail ; `stratyx
  analyse` et page `/app` du site.

### Phase 4 — reste à faire

- Archives réelles sous licence vérifiée (T11), en plus des simulations.
- Postflop approfondi (solveur CFR sur arbres réduits), ICM dans les décisions de tournoi,
  multiplicateurs Expresso.
- Modèles plus lourds sur GPU avec Burn (D3).
