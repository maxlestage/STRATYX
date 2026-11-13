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
- `strategy/src/preflop.rs` : construction de la matrice préflop (temps, parallélisme).
- Monte-Carlo : tirage du board par mélange partiel (Fisher-Yates) au lieu du rejet.
