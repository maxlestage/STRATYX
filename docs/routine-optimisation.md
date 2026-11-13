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
  combinaisons par masques de bits.
- `strategy/src/preflop.rs` : construction de la matrice préflop (temps, parallélisme).
- Monte-Carlo : le tirage des cartes du board par rejet pourrait passer par un mélange
  partiel (Fisher-Yates) du paquet restant.
