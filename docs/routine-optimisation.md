# Routine d'optimisation horaire

Une itération par heure : une amélioration ciblée, vérifiée comme la CI (`cargo fmt`,
`cargo clippy -D warnings`, tests du workspace et du site, lint WebAssembly), puis poussée sur
`claude/stratyx-hourly-routine-ejwj2w` (la CI la fusionne dans `master` quand tout est vert).

Chaque entrée note ce qui a été fait, les mesures, et les pistes pour les itérations suivantes.

## 2026-11-13 — Itération 1 : équité exacte répartie sur tous les cœurs

**Constat.** L'énumération exacte (`poker/src/equity.rs`) ne parallélisait que sur les
combinaisons du premier joueur. Avec une main précise en premier (« AhKh contre … », le cas
le plus courant du CLI et du calculateur), tout le calcul tournait sur un seul cœur.

**Changement.** `exact_tasks` découpe le travail en au moins 256 tâches : on fixe les mains
joueur après joueur, puis, si toutes les mains sont fixées, les premières cartes du board
(les tirages sont énumérés dans l'ordre du paquet, chaque tâche reprend à l'indice suivant).
Deux tests : le découpage donne exactement le même résultat que l'énumération séquentielle
(5 cas, ranges pondérées, 2 à 3 joueurs, flop/turn/river), et les tâches couvrent les
1 712 304 boards une seule fois.

**Mesures** (`stratyx equity`, binaire release, 4 cœurs, meilleur de 5) :

| Calcul                              | Avant    | Après    |
| ----------------------------------- | -------- | -------- |
| `AhKh QsQd` (préflop, exact)        | 78 ms    | 24 ms    |
| `AhKh QsQd JcTc`                    | 83 ms    | 24 ms    |
| `AhKh QsQd JcTc 9d9c`               | 86 ms    | 24 ms    |
| `AhKh QQ+,AKo --board Qs7h2h`       | 3,6 ms   | 4,0 ms   |

Résultats identiques au centième près.

**Pistes suivantes.**

- Vitesse mono-cœur de la feuille d'énumération (compte aussi pour le WebAssembly du site,
  qui n'a qu'un fil) : éviter le `Vec<Card>` du paquet à chaque feuille, énumérer les
  combinaisons par masques de bits. → fait à l'itération 2.
- `strategy/src/preflop.rs` : construction de la matrice préflop (temps, parallélisme).
- Monte-Carlo : le tirage des cartes du board par rejet pourrait passer par un mélange
  partiel (Fisher-Yates) du paquet restant.

## 2026-11-13 — Itération 2 : abattage et énumération plus légers (un seul cœur)

**Constat** (callgrind, 1 fil, `AhKh QsQd` exact) : 971 M instructions, dont `evaluate`
54 %, mais aussi `Acc::showdown` 30 % (~170 instructions par board hors évaluation) et
l'énumération des boards 11 %.

**Changements** (`poker/src/equity.rs`) :

- `Acc` regroupe les sommes d'un joueur dans un `PlayerAcc` (une seule vérification de
  bornes au lieu de quatre `Vec`) ; `showdown` repère le gagnant pendant l'évaluation et,
  quand il est seul, ne met à jour que lui.
- Les mains arrivent dans `showdown` déjà converties en `CardSet` (une fois par feuille, et
  non plus à chaque board : la conversion id → bit coûte une division par 4), et le paquet
  restant est une liste d'ensembles à une carte.
- `for_each_combination` traite son dernier niveau par une boucle simple au lieu d'un appel
  récursif par board.

**Mesures** (binaire release, meilleur de 15) :

| Calcul                                      | Avant     | Après     |
| ------------------------------------------- | --------- | --------- |
| `AhKh QsQd`, 1 fil (instructions)           | 971 M     | 781 M     |
| `AhKh QsQd`, 1 fil                          | 75,0 ms   | 59,4 ms   |
| `AhKh QsQd JcTc 9d9c`, 1 fil                | 86,1 ms   | 70,6 ms   |
| `AhKh QsQd`, 4 cœurs                        | 22,9 ms   | 19,1 ms   |
| Monte-Carlo 3 ranges, 2 M tirages, 4 cœurs  | 77,1 ms   | 70,3 ms   |

Résultats identiques, y compris en Monte-Carlo à graine égale.

**Pistes suivantes.**

- `evaluate` pèse désormais 67 % des instructions (~150 par main) : tables de 8 192 entrées
  calculées à la compilation (`const fn`) pour la quinte et les N meilleurs rangs d'un
  masque de 13 bits, à valider par le test exhaustif des 133 M mains (`--ignored`).
  → itération 3 : la vraie cause était ailleurs (pas de `popcnt`), sans table.
- `strategy/src/preflop.rs` : construction de la matrice préflop (temps, parallélisme).
- Monte-Carlo : tirage du board par mélange partiel (Fisher-Yates) au lieu du rejet.

## 2026-11-13 — Itération 3 : évaluateur sans `popcnt`

**Constat.** Le désassemblage d'`evaluate` montre que la cible x86-64 de base n'a pas
l'instruction `popcnt` : chacun des quatre `count_ones()` de la détection de couleur se
compilait en ~18 instructions, soit ~70 par main avant même de regarder les paires.

**Changements** (`poker/src/eval.rs`) :

- `flush_mask` compte les cartes des quatre couleurs d'un coup sur le `u64` du `CardSet`
  (« SWAR », un compteur par tranche de 16 bits), puis teste « au moins 5 » par une seule
  addition et un masque. Test dédié contre le comptage naïf (couleurs pleines, cas
  limites, 1,4 M mains au hasard de 0 à 7 cartes).
- Multiplicités des rangs (au moins 2, 3, 4 fois) factorisées par paires de couleurs
  ({♣, ♦} et {♥, ♠}) : 11 opérations au lieu de 25.
- Toujours sans table précalculée. Le test exhaustif des 133 784 560 mains de 7 cartes
  (`--ignored`) passe.

**Mesures** (binaire release, meilleur de 15, avant = itération 2) :

| Calcul                                      | Avant     | Après     |
| ------------------------------------------- | --------- | --------- |
| `AhKh QsQd`, 1 fil (instructions)           | 781 M     | 597 M     |
| dont `evaluate`                             | 523 M     | 339 M     |
| `AhKh QsQd`, 1 fil                          | 59,0 ms   | 46,9 ms   |
| `AhKh QsQd JcTc 9d9c`, 1 fil                | 67,8 ms   | 50,6 ms   |
| `AhKh QsQd`, 4 cœurs                        | 21,0 ms   | 16,4 ms   |
| Monte-Carlo 3 ranges, 2 M tirages, 1 fil    | 267,9 ms  | 251,5 ms  |

Depuis le début de la routine, `AhKh QsQd` : 78 ms → 16 ms (4 cœurs).

**Pistes suivantes.**

- Monte-Carlo : le coût est maintenant dans le tirage (ChaCha8, rejet des cartes déjà
  prises, recherche dichotomique des combos pondérées) plus que dans l'évaluation :
  profiler, puis mélange partiel (Fisher-Yates) du paquet restant.
- `top::<N>` n'est pas toujours inliné (~8 % des instructions) : extraction des rangs par
  `leading_zeros` déroulée.
- `strategy/src/preflop.rs` : construction de la matrice préflop (temps, parallélisme).

## 2026-11-13 — Itération 4 : Monte-Carlo, tirages moins coûteux

**Constat** (callgrind, 1 fil, `AA,KK QQ,JJ TT,99`, 400 000 tirages) : 462 M instructions,
dont seulement 26 % d'évaluation. Le reste : la recherche dichotomique du tirage pondéré
(~105 instructions par tirage), la conversion carte → bit à chaque `contains` / `insert`
pendant le tirage du board (~90), le `Vec` des mains (~50).

**Changements** (`poker/src/equity.rs`), le générateur étant consommé exactement de la même
façon (résultats identiques bit pour bit, vérifié sur 4 cas dont des ranges pondérées) :

- `WeightedCombos` garde les combinaisons en `CardSet` et, quand tous les poids valent 1
  (range sans pondération, le cas courant), remplace la recherche dichotomique par la partie
  entière du tirage : les cumuls 1, 2, 3… sont exacts en `f64`, l'indice est le même. Test
  dédié : même combinaison tirée que la recherche, 100 000 tirages × 4 ranges.
- Le paquet est une liste d'ensembles à une carte ; le tirage du board ne fait plus que des
  opérations de masques.
- Les mains d'un tirage vivent dans un tableau fixe au lieu d'un `Vec`.

**Mesures** (binaire release, meilleur de 15, 2 M tirages, avant = itération 3) :

| Calcul                                  | 1 fil avant | 1 fil après | 4 cœurs avant | 4 cœurs après |
| --------------------------------------- | ----------- | ----------- | ------------- | ------------- |
| `AA,KK QQ,JJ TT,99`                     | 240,6 ms    | 194,8 ms    | 60,3 ms       | 51,1 ms       |
| `22+,A2s+,KTo+ random QQ+,AK`           | 374,0 ms    | 238,4 ms    | 100,2 ms      | 67,5 ms       |
| `AhKh QQ+,AKo:0.5,AQs:0.3 random`       | 403,3 ms    | 312,5 ms    | 109,7 ms      | 87,8 ms       |

Instructions (1 fil, 400 000 tirages) : 462 M → 370 M.

**Pistes suivantes.**

- `strategy/src/mc.rs` : le Monte-Carlo propre à la stratégie et à l'entraînement des IA
  (appelé à chaque décision simulée) mérite le même profilage.
- `top::<N>` n'est pas toujours inliné : extraction des rangs déroulée.
- `strategy/src/preflop.rs` : construction de la matrice préflop (temps, parallélisme).

## 2026-11-13 — Itération 5 : entraînement des IA (conversions de cartes, allocations)

**Constat** (callgrind sur `stratyx-train`, 1 fil) : l'entraînement passe 79 % de son temps
dans `strategy::charts::best_responses` (les simulations du solveur push/fold), pas dans
`mc.rs`. Hors évaluation, les coûts propres viennent de conversions élémentaires :

- `CardSet::from_card` lisait `Rank::ALL[id / 4]` et `Suit::ALL[id % 4]`, avec contrôles de
  bornes, à chaque carte tirée (`card.rs` : 7 % des instructions à lui seul) ;
- `HoleCards::from_index` retrouvait la main par une boucle (jusqu'à 52 tours), pour chacune
  des 1 326 combinaisons à chaque parcours de range (`Range::iter`, `Sampler::new`) ;
- `hole_class` construisait une `HandClass` pour la ranger dans la grille ;
- `simulate` allouait un `Vec` par main simulée (`malloc` + `free` : ~6 %).

**Changements**, tous sans effet sur le générateur aléatoire :

- `poker/src/cardset.rs` : le bit d'une carte se calcule directement depuis son
  identifiant (`(id & 3) * 16 + (id >> 2)`). Test : égalité avec l'ancienne formule pour
  les 52 cartes.
- `poker/src/range.rs` : table des 1 326 combinaisons calculée à la compilation ;
  `from_index` est une lecture (le test de bijection existant la couvre).
- `strategy/src/classes.rs` : `hole_class` calculée depuis les identifiants. Test : égalité
  avec `class_index(h.class())` pour les 1 326 combinaisons.
- `strategy/src/charts.rs` : joueurs engagés dans un tableau fixe.

**Mesures.** `stratyx-train --rapide` (2 000 mains par IA, 4 cœurs, meilleur de 3) :
**4,58 s → 3,80 s** (−17 %). Les deux modèles produits (`expresso-3max.json`,
`tournois-6max.json`) sont **identiques octet pour octet** à ceux de la version précédente.
Ces conversions servent partout (analyse, vision, base locale) : le gain ne se limite pas à
l'entraînement.

**Pistes suivantes.**

- `draw` (`strategy/src/classes.rs`) et `draw_for` (`charts.rs`) : le rejet des cartes déjà
  prises et la conversion carte → bit restent sur le chemin chaud des simulations.
- `Sampler::new` (`mc.rs`) reconstruit les ranges à chaque équité de la simulation : les
  préparer une fois par main simulée (`equity_with`).
- `top::<N>` n'est pas toujours inliné : extraction des rangs déroulée.

## 2026-11-13 — Itération 6 : empaquetage direct des rangs dans l'évaluateur

**Constat.** Nouveau profil de l'entraînement : 12,4 G instructions (15,9 G avant
l'itération 5). Le générateur ChaCha pèse ~12 % mais n'est pas modifiable sans changer les
modèles. Le reste est très dispersé, sauf autour de l'évaluateur : `top::<3>` et `top::<5>`
n'étaient pas inlinés, `HandRank::pack` empaquetait les rangs par une boucle sur un tableau,
et `r2.count_ones() >= 2` émulait encore un `popcnt`.

**Changements** (`poker/src/eval.rs`) : la valeur est construite directement (`at`,
`top_at`, `#[inline(always)]`), « au moins deux paires » devient `r2 & r2.wrapping_sub(1)`.
L'encodage est inchangé, ce qui compte puisque `HandRank` est sérialisé tel quel : le test
croisé contre l'évaluateur naïf compare désormais la **valeur exacte** (et plus seulement la
catégorie et l'ordre) sur 50 000 mains ; le test exhaustif des 133 M mains passe.

**Mesures.** `AhKh QsQd`, 1 fil : 597 M → 574 M instructions (−4 %) ; en temps, 44,3 →
43,9 ms sur 1 fil (bruit), 15,8 → 13,8 ms sur 4 cœurs ; Monte-Carlo 2 M tirages, 4 cœurs :
54,1 → 49,5 ms. Entraînement `--rapide` inchangé (3,8 s), modèles identiques octet pour
octet.

**Bilan des chemins chauds.** L'évaluateur fait ~60 instructions sur le chemin courant
(paire ou hauteur), sans appel ni boucle : on arrive aux rendements décroissants sans
changer d'algorithme (tables de rangs) ou de générateur aléatoire (ce qui changerait les
modèles entraînés). Cumul depuis le début de la routine : `AhKh QsQd` exact 78 → 14 ms
(4 cœurs), Monte-Carlo −25 à −40 %, entraînement −17 %.

**Pistes suivantes** (au-delà de la performance pure) :

- Robustesse des imports d'historiques (`poker/src/import.rs`, `database/src/text/`) :
  fichiers tronqués, encodages, formats inattendus — tests de fuzzing déterministes.
- Site : poids du bundle WebAssembly, temps de premier affichage de `/app`.
- `Sampler::new` (`mc.rs`) reconstruit les ranges à chaque équité de la simulation.
