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

## 2026-11-13 — Itération 7 : robustesse de l'import, débordement trouvé par fuzzing

**Démarche.** Nouveau test `poker/tests/import_robustesse.rs` : fuzzing déterministe de
`import_text` suivi du rejeu de chaque main lue (ce que fait l'analyse), à partir des
historiques d'exemple PokerStars et Winamax — troncature à chaque position, chaque ligne
supprimée ou doublée, et 4 000 corruptions aléatoires (caractères multi-octets, BOM, `\r`,
montants géants, mots-clés déplacés). Le test vérifie l'absence de panique.

**Trouvé** (200 000 corruptions en local, 8 cas) : des tapis proches de 2⁶⁴ dans un
historique abîmé faisaient **déborder l'addition des pots** dans `HandState::settle`
(`attempt to add with overflow` en debug ; en release, l'addition bouclait et donnait des
gains faux sans erreur).

**Correction** (`poker/src/game.rs`) : `MAX_TOTAL_CHIPS = 2⁶²`. `HandState::new` refuse une
main dont la somme des tapis, la grosse blinde ou l'ante dépassent ce plafond (bien au-delà
de toute partie réelle). Tous les montants en jeu étant bornés par la somme des tapis, pots,
relances et résultats nets (`i64`) ne peuvent plus déborder. Tests : cas limites dans
`invalid_setups`, et un test de régression explicite (tapis à 2⁶⁴ − 1 dans l'historique
PokerStars d'exemple) qui échoue sans le correctif. Après correction, 200 000 corruptions
passent sans panique.

**Pistes suivantes.**

- Même fuzzing pour l'importeur PokerStars de la base locale
  (`database/src/text/pokerstars_import.rs`, 978 lignes), avec ses trois fichiers réels de
  `database/tests/data/`.
- Site : poids du bundle WebAssembly, temps de premier affichage de `/app`.

## 2026-11-13 — Itération 8 : import PokerStars de la base, sièges incohérents acceptés

**Démarche.** Nouveau test `database/tests/import_robustesse.rs`, sur le modèle de
l'itération 7, pour l'importeur PokerStars de la base locale
(`database/src/text/pokerstars_import.rs`) à partir des trois fichiers réels de
`database/tests/data/` : troncatures, lignes supprimées ou doublées, 3 000 corruptions
aléatoires. En plus de l'absence de panique, il vérifie un **invariant d'aller-retour** :
toute main acceptée malgré la corruption, exportée au format PokerStars puis relue, redonne
exactement la même main.

**Trouvé.** Aucune panique (200 000 corruptions en local), mais une incohérence : une
corruption de `Seat 1:` en `Seat 91:` à une table 6-max était **acceptée** ; l'export
réécrivant les sièges dans l'ordre, la main relue n'était plus la même.

**Correction.** À la lecture des sièges, le numéro doit être compris entre 1 et la taille de
la table (`6-max`, 10 sinon) et strictement croissant (PokerStars liste toujours les sièges
dans l'ordre) : un siège hors de la table, en double ou dans le désordre donne une erreur
localisée (`Inconsistent`, avec la ligne). Test de régression (sièges 91, 0 et 7 à une table
6-max, deux sièges inversés), qui échoue sans le correctif. Après correction, 200 000
corruptions passent, aller-retour compris.

**Pistes suivantes.**

- Même invariant d'aller-retour pour `poker::import` (Winamax) si un export Winamax existe.
- Site : poids du bundle WebAssembly, temps de premier affichage de `/app`.

## 2026-11-13 — Itération 9 : site, réponses compressées (WebAssembly −70 %)

**Constat.** Le serveur du site envoyait tout sans compression, et le routeur d'Heroku ne
compresse pas : le WebAssembly des îlots (`pkg/stratyx_site_bg.wasm`, 2,32 Mo en
`wasm-release`) et chaque page HTML partaient tels quels, ce qui pèse surtout sur mobile au
premier affichage.

**Changement** (`site/`) : fonctionnalité `compress` de Paquets (brotli et flate2 avec
miniz_oxide, en Rust pur) et `app.middleware(middleware::compress())`. Brotli si le
navigateur l'accepte, gzip sinon ; les fichiers statiques sont compressés une seule fois au
niveau maximal puis gardés en cache, les pages au niveau rapide sur un fil bloquant ;
`Vary: Accept-Encoding` est posé. Test `les_reponses_sont_compressees` (Brotli, gzip, et rien
sans `Accept-Encoding`), qui échoue sans le middleware.

**Mesures** (serveur release local, octets transférés) :

| Ressource                  | Sans     | gzip    | Brotli  |
| -------------------------- | -------- | ------- | ------- |
| `stratyx_site_bg.wasm`     | 2 316 572 | 783 966 | 686 095 (−70 %) |
| `/` (accueil)              | 35 212   | 10 751  | 9 724 (−72 %)   |
| `/app`                     | 26 374   | 8 839   | 8 083 (−69 %)   |

Décompression vérifiée identique au bit près (gzip et Brotli). Le WebAssembly compressé est
servi en ~23 ms depuis le cache.

> **Correction (itération 10).** Faute de `wasm-bindgen` en local, ces mesures ont été
> faites sur le `.wasm` **brut** de `cargo build`, et non sur le fichier réellement servi
> (`pkg/stratyx_site_bg.wasm`, produit par `wasm-bindgen`). Celui-ci est plus petit :
> `wasm-bindgen` retire les 4 313 exports `__wbindgen_describe_*` (section export de
> 250 Ko, et le code associé) et la section `__wasm_bindgen_unstable` (586 Ko). Le taux de
> compression (~70 %) reste représentatif ; les octets absolus, non.

**Pistes suivantes.**

- Cache navigateur du bundle : `pkg/` est servi avec `max-age=0` (revalidation à chaque
  visite) ; un nom de fichier versionné permettrait un cache long.
- Taille du WebAssembly lui-même (2,3 Mo) : voir ce qui pèse (`twiggy` n'est pas installé ;
  sections par crate via `cargo bloat --target wasm32`).

## 2026-11-13 — Itération 10 : la page `/app` face aux historiques abîmés

**Constat.** La page `/app` importe et analyse dans le navigateur les historiques que
l'utilisateur dépose, en WebAssembly compilé avec `panic = "abort"` : la moindre panique dans
l'import ou l'analyse ferait planter la page entière, sans message.

**Changement.** Nouveau test `analytics/tests/robustesse.rs` qui suit exactement le parcours
de la page : import, analyse de chaque main pour le héros saisi (vide, joueur de la table,
inconnu, bruit), puis bilan de session (`SessionBuilder`). Sur les historiques intacts, il
vérifie d'abord que l'analyse aboutit vraiment (décisions évaluées) ; puis 400 corruptions
aléatoires par exécution (50 000 en local : aucune panique). Pas de correction nécessaire :
les garde-fous des itérations 7 et 8 couvrent ce chemin.

**Pistes suivantes.**

- Mesurer le vrai `pkg/stratyx_site_bg.wasm` (après `wasm-bindgen`) : la CI du site pourrait
  publier sa taille, compressée et non compressée, à chaque build.
- Cache navigateur du bundle : `pkg/` est servi avec `max-age=0` (revalidation à chaque
  visite).

## 2026-11-13 — Itération 11 : analyse de spot face aux lectures d'image absurdes

**Constat.** L'analyse de spot de `/app` (`analytics::spot::analyse_spot`) reçoit des
montants lus sur des captures d'écran ou des vidéos (`f32`) et des cartes reconnues par la
vision : `NaN`, infinis, négatifs, énormes, cartes en double ou déjà dans la main sont
possibles, et une panique ferait planter la page (WebAssembly, `panic = "abort"`).

**Changement.** Test `absurd_spots_never_panic` (dans `analytics/tests/robustesse.rs`) :
1 500 spots au hasard par exécution — 0 à 12 joueurs, toutes les positions, board de 0 à 7
cartes avec doublons, montants parmi `NaN`, ±∞, −1, 0, 1e30, `f32::MAX`…, les quatre
situations, avec ou sans IA spécialisée. Garde-fou : une part réelle des spots doit aller au
bout de l'analyse.

**Résultat** (100 000 spots en local) : aucune panique ; 14 % analysés jusqu'au bout, le
reste refusé avec un message clair (position impossible à la table, carte en double,
relance illégale, « personne ne parle avant cette position »). 7 404 spots aux montants
infinis ou énormes sont arrêtés par le plafond de jetons de l'itération 7
(`MAX_TOTAL_CHIPS`) : ce correctif protégeait aussi ce chemin. Aucune correction de code
nécessaire.

**Pistes suivantes.**

- Mesurer le vrai `pkg/stratyx_site_bg.wasm` (après `wasm-bindgen`) dans la CI du site.
- Les messages « pas de HJ à 2 joueurs » apparaissent quand la vision lit 0 ou 1 joueur
  (ramené à 2) : un message dédié (« nombre de joueurs illisible ») serait plus clair.

## 2026-11-13 — Itération 12 : montants illisibles d'un spot, messages clairs

**Constat.** Les montants d'un spot de `/app` viennent de champs saisis ou lus sur une
image, convertis par `nombre()` (`site/src/capture.rs`) avec `str::parse::<f32>` : « inf »,
« NaN », « 1e30 » ou « -5 » y passent. Ils finissaient en messages incompréhensibles, comme
« situation de départ invalide : montants démesurés (au-delà de 2^62 jetons) » (le plafond de
l'itération 7), ou étaient ramenés silencieusement à 0.

**Changement** (`analytics/src/spot.rs`) : `analyse_spot` vérifie d'abord les montants qui
comptent pour le spot (tapis effectif ; relance au préflop ; pot et montant à suivre après le
flop) et répond « tapis effectif illisible », « montant à suivre négatif (-1 BB) » ou « pot
démesuré (…, au plus 100000 BB) » (`MAX_SPOT_BB`). Test unitaire
`unreadable_amounts_get_a_clear_message`. Le fuzzing de l'itération 11 tire désormais les
montants absurdes une fois sur quatre seulement, pour que les spots plausibles restent
représentés (garde-fou : au moins 100 spots analysés sur 1 500) ; 100 000 spots en local,
aucune panique.

**Piste abandonnée.** Le message « pas de HJ à 2 joueurs » quand 0 ou 1 joueur est lu : le
site fixe le nombre de joueurs (3 en Expresso, 6 sinon), le cas est inatteignable.

**Pistes suivantes.**

- Mesurer le vrai `pkg/stratyx_site_bg.wasm` (après `wasm-bindgen`) dans la CI du site.

## 2026-11-13 — Itération 13 : vision face aux images quelconques ; confiance `NaN` jamais certaine

**Démarche.** Dernier maillon de `/app` : la vision (`vision/`), qui lit dans le navigateur
des captures et des vidéos de taille et de contenu arbitraires (WebAssembly,
`panic = "abort"`). Nouveau test `vision/tests/robustesse.rs` :

- 300 images au hasard par exécution (10 000 en local) : tailles dégénérées (1×1, 1×300,
  1 500×2…), aplats, bruit, rectangles clairs pris pour des cartes (parfois coupés par le
  bord), taille qui change d'une trame à l'autre pour `Keyframer` ; parcours complet de la
  page (images clés, `detect_cards`, `arrange`, `recognize`) et `crop` / `read_corner` /
  `downscale` sur des rectangles quelconques (0, hors image, `u32::MAX`). Garde-fou : des
  cartes sont bien détectées. **Aucune panique.**
- Modèles appris relus depuis le stockage du navigateur mais incohérents (grilles de
  mauvaise taille, rang 200, couleur 9, aspects démesurés) : aucune panique, confiance
  toujours dans `[0, 1]`.

**Trouvé en lisant le code.** `Recognized::confidence()` bornait la confiance brute puis,
si elle restait invalide (`NaN`, seul cas possible après le bornage), renvoyait
`Confidence::CERTAIN` : une lecture douteuse aurait été déclarée certaine, et aurait échappé
au seuil de vérification (0,95) sous lequel l'utilisateur est invité à confirmer la carte.
Elle vaut désormais 0. Test `an_unreadable_confidence_is_never_certain`, qui échoue sans le
correctif. (Pas de `NaN` observé depuis de vraies images : correctif défensif.)

**Bilan de la robustesse de `/app`** (itérations 7 à 13) : import texte, analyse des mains,
bilan de session, analyse de spot et vision sont couverts par des tests de fuzzing
déterministes ; trois défauts corrigés (débordement des pots, sièges incohérents, confiance
`NaN`), un message d'erreur clarifié.

**Pistes suivantes.**

- Mesurer le vrai `pkg/stratyx_site_bg.wasm` (après `wasm-bindgen`) dans la CI du site.
- Temps de la vision sur une capture 4K (3 840 × 2 160) sur mobile : `detect_cards`
  réduit l'image d'abord, mais `Keyframer` et `read_corner` travaillent en pleine
  résolution.

## 2026-11-13 — Itération 14 : vision, réduction d'image en une passe par ligne

**Constat** (release, table synthétique, parcours de `/app`) : la reconnaissance est
négligeable (0,05 ms), mais `detect_cards` passe **80 % de son temps** dans
`Frame::downscale` (réduction à 640 px de large avant la détection) : 2,8 ms en 1 280 × 720,
4,0 ms en 1 920 × 1 080, 10,7 ms en 3 840 × 2 160. L'ancien calcul, pixel de sortie par
pixel de sortie, recalculait un index et bouclait octet par octet.

**Changement** (`vision/src/frame.rs`) : la colonne de sortie de chaque colonne source est
calculée une fois (mêmes bornes, donc même résultat), puis chaque ligne source est parcourue
une seule fois, linéairement, en ajoutant ses pixels aux sommes de la ligne de sortie. Si un
arrondi faisait se chevaucher deux boîtes (cas théorique), repli sur l'ancien calcul, gardé
comme `downscale_per_pixel`. Test `downscale_matches_the_per_pixel_reference` : résultat
identique octet pour octet sur 200 images et largeurs au hasard.

**Mesures** (`downscale(640)`, meilleur de 5 × 20, A/B dans le même binaire) :

| Image             | Avant   | Après   |
| ----------------- | ------- | ------- |
| 1 280 × 720       | 2,95 ms | 2,51 ms (−15 %) |
| 1 920 × 1 080     | 4,09 ms | 3,32 ms (−19 %) |
| 3 840 × 2 160     | 9,62 ms | 9,47 ms (≈) |

Une variante par tranches de colonnes gagnait 28 % en 4K mais perdait 10 % en 720p ; les
captures d'écran et vidéos d'écran courantes sont en 720p–1080p, d'où ce choix. En 4K, la
lecture des 33 Mo de l'image domine probablement (bande passante mémoire).

**Pistes suivantes.**

- 4K : réduire par 2 pendant la capture côté navigateur (`drawImage` à taille réduite) avant
  de passer l'image au WebAssembly, plutôt qu'en Rust.
- Mesurer le vrai `pkg/stratyx_site_bg.wasm` (après `wasm-bindgen`) dans la CI du site.

## 2026-11-13 — Itération 15 : la CI mesure le vrai bundle WebAssembly

**Piste abandonnée.** Réduire les images 4K dans le navigateur avant la vision : c'est déjà
le cas, `media::lire` réduit chaque image à la capture (`image_video(&video, 1280)`), une
image 4K n'atteint jamais le Rust. L'ingestion dans la base locale, autre piste examinée, se
fait déjà en une seule transaction par lot.

**Changement** (`.github/workflows/ci.yml`) : nouveau job `bundle` qui construit le bundle
exactement comme Heroku (`cargo build --profile wasm-release` puis `wasm-bindgen` à la
version de `Cargo.lock`, comme `heroku-build.sh`) et publie dans le résumé du run la taille
de `pkg/stratyx_site_bg.wasm` et `pkg/stratyx_site.js` : brute, gzip -9 et Brotli -q 11.
Il comble l'angle mort de l'itération 9 (tailles mesurées sur le `.wasm` brut, faute de
`wasm-bindgen` en local) et vérifie à chaque push que le bundle se construit, ce que la CI
ne faisait pas (seul le lint WebAssembly tournait). Job informatif : `continue-on-error`,
hors des dépendances de `merge`, il ne retarde ni ne bloque la fusion.

Vérifié en local : YAML, dépendances de `merge` inchangées, extraction de la version
(0.2.129) et script du rapport (sur un `.wasm` de substitution). Le téléchargement de
`wasm-bindgen` n'a pas été exécuté ici : premier vrai passage en CI.

**Pistes suivantes.**

- Lire le résumé du premier run du job `bundle` et reporter ici les vraies tailles servies.

## 2026-11-13 — Itération 16 : CI rouge depuis l'itération 14 (clippy de Rust 1.100), corrigée

**Incident.** La CI des itérations 14 et 15 a échoué au lint sur les trois systèmes :
le clippy de Rust 1.100 (celui de la CI, `dtolnay/rust-toolchain@stable`) refuse
`chunks_exact(4)` à taille constante (`clippy::chunks_exact_to_as_chunks`), utilisé par la
nouvelle réduction d'image de l'itération 14. Localement, la toolchain était restée en Rust
1.97, qui ne connaît pas ce lint : les vérifications locales étaient vertes. Conséquence :
ni l'itération 14 ni la 15 n'ont été fusionnées dans `master` (la fusion automatique exige une
CI verte), rien de cassé n'a été déployé.

**Correction.** `vision/src/frame.rs` : `as_chunks::<4>()` / `as_chunks_mut::<4>()`, et
sommes en `[u32; 4]` par colonne (plus lisible). Toolchain locale mise à jour
(`rustup update stable` → Rust 1.100, comme la CI) : fmt, clippy (workspace, site natif et
WebAssembly), tests (296 + 16) et évaluateur exhaustif verts ; test d'identité de la
réduction toujours vert.

**Leçon pour la routine.** Mettre la toolchain à jour (`rustup update stable`) avant de
vérifier, et contrôler le résultat de la CI de l'itération précédente en début d'itération.

**Premier rapport du job `bundle`** (itération 15, Rust 1.100) — tailles réellement servies :

| Fichier                | Brut      | gzip -9 | Brotli -q 11     |
| ---------------------- | --------- | ------- | ---------------- |
| `stratyx_site_bg.wasm` | 1 246 069 | 417 920 | 333 794 (−73 %)  |
| `stratyx_site.js`      | 70 330    | 10 770  | 9 105 (−87 %)    |

Le WebAssembly servi fait donc 1,25 Mo (et non 2,3 Mo, taille du `.wasm` brut avant
`wasm-bindgen`), soit ~334 Ko transférés avec la compression de l'itération 9.

**Pistes suivantes.**

- Ce qui pèse dans les 1,25 Mo : code (~1 Mo) et données (~430 Ko) ; les modèles des IA
  embarqués dans le WebAssembly (JSON) sont un candidat.

## 2026-11-14 — Itération 17 : poids du bundle (mesures, statu quo) et avertissement de cargo 1.100

**CI.** Celle de l'itération 16 est verte : les itérations 14 à 16 sont fusionnées dans
`master`.

**Ce qui pèse dans les 1,25 Mo du WebAssembly.**

- Données (~430 Ko) : les deux modèles des IA en JSON (`expresso-3max.json` 125 Ko,
  `tournois-6max.json` 219 Ko : push/fold encodé en lettres, poids des réseaux en
  flottants) et la matrice préflop (114 Ko). Le JSON se compresse très bien en Brotli ; un
  format binaire gagnerait peu sur le réseau, et arrondir les flottants changerait les
  décisions des IA. Statu quo.
- Code (~1 Mo) : `opt-level = "z"` au lieu de `"s"` dans `wasm-release` réduit la section de
  code de 10,6 % (1 030 485 → 920 867 octets ; 294 195 → 271 121 en gzip -9, ~23 Ko de moins
  à transférer). Mais le même réglage en natif ralentit l'équité de 10 à 16 %
  (`AhKh QsQd` 35,2 → 38,7 ms ; Monte-Carlo 80 000 tirages 8,6 → 9,7 ms et 24,1 → 27,9 ms,
  1 fil). Le site étant une PWA dont le service worker garde le bundle en cache, les 23 Ko ne
  se paient qu'une fois, le ralentissement à chaque calcul : **`"s"` est gardé**.

**Changement.** Cargo 1.100 avertissait à chaque compilation : « unused field
`workspace.package.repository` ». Les 19 crates du workspace héritent désormais du champ
(`repository.workspace = true`, comme `version`, `edition` et `publish`) : métadonnées
complètes, compilation sans avertissement.

**Pistes suivantes.**

- Temps de démarrage de `/app` : analyse des deux modèles JSON au premier calcul
  (`serde_json` en WebAssembly) — à mesurer.

## 2026-11-14 — Itération 18 : mesures, aucune piste rentable (pas de changement de code)

**CI.** Celle de l'itération 17 est verte et fusionnée (run complet : ~4 min).

Mesures (release natif, 1 fil), pour ne pas les refaire :

| Piste                                             | Mesure                           | Décision |
| ------------------------------------------------- | -------------------------------- | -------- |
| Lecture des modèles JSON des IA au premier calcul | 0,16–0,30 ms par modèle          | négligeable, statu quo |
| Analyse d'une main (`/app`, 900 tirages)          | 0,21 ms (PokerStars), 0,76 ms (Winamax) | déjà rapide, statu quo |
| Test `invalid_parameters_are_rejected` (2,7 s seul) | coût de la table de test `small_table()`, calculée une fois (`OnceLock`) et partagée par toute la suite | fausse alerte |
| `ThreeWayEquities::compute` (≈ 2,7 s en test, ≈ 11 s hors ligne) | utilisé seulement par la table de test et l'exemple `pushfold_charts` (génération hors ligne) ; ~3 s sur ~4 min de CI | pas rentable |

**Bilan.** Les chemins chauds côté utilisateur (équité, évaluateur, Monte-Carlo,
entraînement, analyse, vision, réponses du site) ont été optimisés aux itérations 1 à 6,
9 et 14 ; la robustesse de `/app` est couverte (itérations 7 à 13). Les gains restants sont
marginaux : les prochaines itérations gagneraient à viser des fonctionnalités ou de la
dette repérée plutôt que la micro-performance.

**Pistes suivantes** (hors performance) :

- Crates encore squelettes (`apps/desktop`, `apps/server`, `apps/mobile`, `coaching`,
  `transcription`, `video`, `ai/inference`) : voir avec le cahier des charges lesquels
  méritent une première brique testée.
- Documentation : le README (section « État ») ne mentionne ni la base PostgreSQL du site,
  ni la compression, ni les tests de robustesse.

## 2026-11-14 — Itération 19 : README à jour

**CI.** Celle de l'itération 18 est verte et fusionnée.

**Changement** (`README.md`, section « État » et « Démarrer ») :

- la durée annoncée des tests était fausse : « `cargo test --workspace` (≈ 1 s) » ; mesurée,
  la suite prend **≈ 25 s** sur 4 cœurs une fois compilée (solveur push/fold et table des
  confrontations à trois, entraînement, capture) ;
- trois points ajoutés à l'état du projet : tests de robustesse de tout le parcours de `/app`
  (`*/tests/*robustesse.rs`), compression et base PostgreSQL gratuite du site, lien vers ce
  journal.

**Pistes suivantes.** En attente d'une décision de l'utilisateur sur la réorientation de la
routine (fonctionnalités dans les crates squelettes, ou fréquence réduite).

## 2026-11-14 — Itération 20 : sauvegardes chiffrées, toute altération refusée

**CI.** Celle de l'itération 19 est verte et fusionnée.

**Constat.** Les tests unitaires de `security/src/backup.rs` vérifiaient l'altération de
4 octets et 3 coupures d'une archive de sauvegarde ; l'en-tête (identifiant de clé, nonce,
paramètres) n'était pas couvert octet par octet.

**Changement.** Nouveau test `security/tests/backup_robustesse.rs`, sur une archive de trois
fichiers (dont un vide et un de ~150 Ko, sur plusieurs blocs), déverrouillée par clé (pas
d'Argon2 à chaque essai) :

- chaque octet des 4 premiers Kio (en-tête compris) puis un sur 97, modifié de deux façons
  (`^0x01`, `^0x80`), et le dernier octet ;
- une coupure à chaque position des 4 premiers Kio, puis une sur 61, et les 64 dernières ;
  des octets ajoutés après la fin ;
- une mauvaise clé.

Chaque cas doit être refusé par `verify_backup` **et** par `restore_to_memory` (qui ne
renvoie rien en cas d'échec), sans panique. Version exhaustive lancée une fois en local
(chaque octet ×2 et chaque coupure, ~450 000 essais, 48 s en release) : tout est refusé.
Aucune correction nécessaire.

**Pistes suivantes.** En attente de la décision de l'utilisateur sur la réorientation de la
routine (fonctionnalités dans les crates squelettes, ou fréquence réduite).

## 2026-11-14 — Itération 21 : CLI `stratyx` testée ; deux défauts ICM corrigés

**CI.** Celle de l'itération 20 est verte et fusionnée.

**Constat.** `tools/cli` (la commande `stratyx` du README : `eval`, `equity`, `range`, `icm`,
`analyse`) n'avait aucun test.

**Changement.** Tests passant par `run()` : chaque exemple du README réussit (dont `analyse`
d'un historique, en texte et en JSON) ; 18 arguments invalides (commande inconnue, cartes en
double ou illisibles, board de 2 cartes, `--samples` sans valeur ou non numérique, range
illisible, tapis nuls, négatifs ou manquants, prix `NaN,inf`, fichier absent) donnent une
erreur, jamais une panique. Ils ont trouvé deux défauts :

- **`stratyx icm` avec des tapis géants** (`--stacks 18446744073709551615,…`) : la somme
  entière des tapis, qui ne sert qu'à afficher la part des jetons, débordait (panique en debug,
  pourcentages faux en release). Elle est calculée en `f64` ; l'équité ICM elle-même était
  juste (vérifié : deux tapis égaux à 2⁶⁴ − 1 → 40 chacun pour des prix 50 et 30).
- **Prix `NaN`, infinis ou négatifs** : `poker::icm::icm_equities` les acceptait et renvoyait
  des équités `NaN` sans erreur — y compris pour le calculateur ICM du site, qui lit les prix
  saisis. Nouvelle erreur `IcmError::Payout` (« le prix 1 est invalide (NaN) : un nombre
  positif ou nul est attendu »), test `invalid_payouts_are_rejected`. Le solveur push/fold
  validait déjà ses prix en amont : son `expect` reste sûr.

**Pistes suivantes.** `capture` (873 lignes, 3 tests) et `shared` (4 tests) sont les crates
livrés les moins testés ; en attente de la décision de l'utilisateur sur la réorientation de
la routine.

## 2026-11-14 — Itération 22 : capture d'écran, tampon trop court refusé au lieu de paniquer

**CI.** Celle de l'itération 21 est verte et fusionnée.

**Constat.** `shared` (284 lignes) est petit et ses points sensibles sont couverts. Dans
`capture`, `bgrx_to_rgba` découpait le tampon source sans vérifier sa longueur ; le moteur
X11 déduit le pas des lignes de la taille de la réponse du serveur
(`data.len() / hauteur`) : une réponse incomplète (serveur X, XWayland, fenêtre
redimensionnée pendant la capture) faisait paniquer la capture au lieu de renvoyer une
erreur.

**Changement** (`capture/src/lib.rs`, `x11.rs`, `windows.rs`) : `bgrx_to_rgba` renvoie
`Result` et répond `CaptureError::PixelFormat` (« tampon de N octets trop court pour W × H
pixels ») si le pas est plus court qu'une ligne ou le tampon trop court (calcul vérifié
contre le débordement) ; la dernière ligne peut rester sans remplissage. Les deux moteurs
propagent l'erreur (`?`), le moteur Windows lit aussi son décalage avec `get`. Test
`short_buffers_are_errors`.

Le moteur Windows ne se compile pas sous Linux : vérifié avec `cargo clippy --target
x86_64-pc-windows-gnu`, et macOS avec `--target aarch64-apple-darwin` (toolchains ajoutées
localement), en plus de Linux.

**Pistes suivantes.** En attente de la décision de l'utilisateur sur la réorientation de la
routine (fonctionnalités dans les crates squelettes, ou fréquence réduite).

## 2026-11-14 — Itération 23 : archives d'entraînement, un fichier abîmé n'arrête plus tout

**CI.** Celle de l'itération 22 est verte et fusionnée.

**Constat** (`ai/training/src/archive.rs`, lecture de `--archives DOSSIER`). Les `.txt`
étaient lus de façon tolérante (`from_utf8_lossy`), mais pas les `.json`
(`read_to_string(…)?`) ni les `.jsonl` (`line?`) : un seul `.json` non UTF-8, une ligne
abîmée dans un `.jsonl`, ou un sous-dossier nommé `x.json` faisaient échouer **tout
l'entraînement** sur une erreur d'entrée-sortie.

**Changement.** Chaque fichier est lu en octets ; un fichier illisible compte comme invalide
et la lecture continue ; les `.json` sont analysés avec `from_slice` et les `.jsonl` ligne
par ligne en octets (une ligne illisible compte comme invalide, sans écarter les autres).
Test `unreadable_files_do_not_stop_the_load` : un dossier mêlant historique valide, doublon
JSON Lines, JSON non UTF-8, ligne abîmée, sous-dossier `piege.json` et fichier sans rapport
— 1 main gardée, 2 doublons, 3 invalides comptés. Entraînement `--rapide` : modèles
identiques (hors date).

**Pistes suivantes.** En attente de la décision de l'utilisateur sur la réorientation de la
routine (fonctionnalités dans les crates squelettes, ou fréquence réduite).

## 2026-11-14 — Itération 24 : bilan de session et accessibilité vérifiés (pas de changement)

**CI.** Celle de l'itération 23 est verte et fusionnée.

- **Bilan de session sans main analysée** (`analytics/src/session.rs`) : tous les ratios et
  moyennes sont protégés (`b == 0`, `max(1)`), et `skip()` incrémente aussi `hands`, donc
  `hands - skipped` ne peut pas déborder. Rien à corriger.
- **Accessibilité du HTML servi** (15 pages du serveur, analyse des boutons et liens sans
  nom accessible, champs sans libellé, images sans `alt`) : deux signalements, tous deux
  des faux positifs de l'audit — le champ piège anti-robots `site_web` a déjà
  `tabindex="-1"`, `aria-hidden="true"` et `autocomplete="off"` ; les champs de fichier de
  `/app` sont dans leur `<label>` (libellé implicite). L'unique image a son `alt`.

**Pistes suivantes.** En attente de la décision de l'utilisateur sur la réorientation de la
routine (fonctionnalités dans les crates squelettes, ou fréquence réduite) : les dernières
itérations trouvent de moins en moins de défauts.

## 2026-11-14 — Itération 25 : base locale abîmée, limite de redb mise au jour (pas de correctif)

**CI.** Celle de l'itération 24 est verte et fusionnée.

**Démarche.** Nouveau test `database/tests/fichier_abime.rs` : une vraie base (mains de
l'historique d'exemple), puis le fichier vide, remplacé par du texte ou du bruit, tronqué à
9 positions, un octet inversé tous les 509 octets (~300 cas) et des blocs de 64 octets mis à
zéro ; à chaque fois `Database::open` puis lecture de toutes les mains, sous `catch_unwind`.

**Trouvé.** Les fichiers vides, de texte, de bruit ou tronqués sont refusés proprement. Mais
**redb 4.4.0 panique sur 6 fichiers dont un seul octet est inversé** (index hors limites
dans l'allocateur de pages, `unwrap` d'un nom de table non UTF-8), à l'ouverture et même à
la libération de la base. Essais :

- redb **4.6.0** (dernière version, compatible) : toujours 9 paniques (« Malformed allocator
  state », `unwrap` sur `None`…) — mise à jour non retenue, elle ne corrige rien ;
- `redb::Database::check_integrity()` à l'ouverture : panique lui-même sur 6 fichiers sur
  306, et déclare « intègres » 247 fichiers abîmés (octet dans une page libre) — pas une
  parade.

C'est un défaut de redb, pas du code de STRATYX. Le contourner demanderait d'envelopper
chaque opération de la base dans `catch_unwind` (décision d'architecture, et les paniques à
la libération ne se rattrapent pas proprement) : non fait. Le test est commité **ignoré**,
avec la limite documentée en en-tête, pour revérifier une future version de redb
(`cargo test -p stratyx-database --test fichier_abime -- --ignored`).

**Pistes suivantes.**

- Signaler le défaut en amont (dépôt de redb) — action externe, à décider par l'utilisateur.
- Parade côté application : la base pourrait être sauvegardée régulièrement avec
  `stratyx_security::backup` (archives chiffrées vérifiées, robustes : itération 20).

## 2026-11-14 — Itération 27 : dépendances à jour (versions compatibles)

**CI.** Celle de l'itération 25 est verte et fusionnée (rien de poussé à l'itération 26).

**Changement** (`Cargo.lock` du workspace) : `cargo update` — errno 0.3.15, inout 0.2.6,
itoa 1.0.19, **redb 4.6.0** (au lieu de 4.4.0), **sha2 0.11.1**, syn 3.0.10, wasip2 1.0.4,
wit-bindgen 0.57.1 ; toutes compatibles semver, en Rust pur. Le site (Paquets en Git) n'a pas
de mise à jour.

**Vérifications.** fmt, clippy et 304 tests verts, évaluateur exhaustif vert ; entraînement
`--rapide` : modèles identiques (hors date), les flux aléatoires ne changent pas. **Fichiers
de base existants** : une base écrite avec redb 4.4 se relit avec 4.6 (2 mains présentes) et
accepte une écriture. `cargo deny` n'a pas pu tourner en local : c'est le job d'audit de la
CI qui vérifie la règle 100 % Rust et les licences.

redb 4.6 ne corrige pas les paniques de l'itération 25 sur fichiers abîmés (test toujours
ignoré).

**Pistes suivantes.** En attente de la décision de l'utilisateur sur la suite de la routine
(espacer, réorienter vers des fonctionnalités, ou arrêter).

## 2026-11-14 — Demande de l'utilisateur : Nuage DB remplace PostgreSQL (Neon) pour le site

**Choix** (de l'utilisateur) : Nuage DB **embarqué dans le serveur du site**, à la place de
la base PostgreSQL gratuite (Neon) de la veille.

**Mise en œuvre** (`site/`) :

- dépendance `nuage` (sans son site web), côté serveur seulement ; Paquets compilé sans
  `postgres` ;
- module `base` (`src/main.rs`) : Nuage ouvert au démarrage avec la configuration de
  `nuage::config::from_vars` (clé `NUAGE_ENCRYPTION_KEY`, sauvegardes `NUAGE_BACKUP_S3_*`
  ou `NUAGE_BACKUP_DIR`), tables `inscrit` (adresse `UNIQUE`) et `message` créées en SQL ;
  les formulaires écrivent en SQL paramétré (`ON CONFLICT (email) DO NOTHING`) ; à l'arrêt
  (`SIGTERM`), point de contrôle, dernière sauvegarde et fermeture ;
- Nuage a sa propre version de Memoire (`fd1fdd5`), distincte de celle de Paquets
  (`504e331`) : le site lui parle en SQL, sans modèles partagés ; la base interne de Paquets
  est fixée en mémoire (`app.database("memory")`), ce qui évite aussi qu'un ancien
  `DATABASE_URL=postgres://…` empêche le démarrage ;
- messages d'erreur dédiés si la base est indisponible (« Inscription impossible »,
  « Votre message n'a pas pu être enregistré ») ;
- déploiement : les secrets GitHub `NUAGE_*` sont transmis à Heroku et `DATABASE_URL` est
  retiré ; documentation (site/README.md, README.md) réécrite.

**Vérifié.** Test `les_formulaires_enregistrent_dans_nuage` (une inscription en double
ignorée, un message enregistré). En local, avec sauvegarde vers un dossier (même mécanisme
que S3) et chiffrement : trois cycles démarrage → inscription → `SIGTERM` → **effacement du
dossier de données** → redémarrage : 3 inscriptions restaurées. Mauvaise clé : refus clair au
démarrage, pages toujours servies. Lint natif et WebAssembly, 17 tests du site, bundle
`wasm-release` inchangé (Nuage n'y entre pas).

**Reste à faire (utilisateur).** Créer un seau S3 gratuit et ajouter les secrets `NUAGE_*`
(site/README.md, « Base de données »), sinon la base est perdue à chaque redémarrage.
L'ancien secret GitHub `DATABASE_URL` (Neon) peut être supprimé.

## Chantier Nuage — la base locale passe dans Nuage DB (décision T14)

Demande de l'utilisateur : « Déplace tout … sert t'en de base de données ». Plan accepté, en
trois chantiers :

1. **Memoire** : fonctionnalité `nuage-wire` (format d'échange seul) — *fait*, branche
   `claude/nuage-wire` de Memoire (`074fbde`), fusionnée.
2. **Nuage** : mode embarqué en Rust pur (RustCrypto à la place de `ring`, réseau et S3
   optionnels) — *fait*, branche `claude/nuage-100-rust` de Nuage (`5fd8056`), compatible
   avec les données chiffrées et les sauvegardes de la version précédente ; fusionnée.
3. **STRATYX** : le crate `database` migre de redb vers Nuage, en étapes :
   - **a** — dépendance optionnelle et audit — *fait (2026-11-14)* ;
   - **b** — stockage Nuage complet (mains, requêtes, utilisateurs, stratégie, import,
     confidentialité, migrations) — *fait (2026-11-14)* ;
   - **c** — Nuage par défaut, import automatique des anciennes bases redb, redb réduit à
     cette lecture — *fait (2026-11-14)*.

### Étape a (2026-11-14)

- `database/Cargo.toml` : `nuage` (révision `5fd8056`, `default-features = false`) optionnel,
  derrière la fonctionnalité `nuage`, désactivée par défaut. Arbre : 74 crates, ni `ring`, ni
  `cc`, ni `rustls`, ni `hyper`, ni `tokio`.
- `deny.toml` : sources Git `maxlestage/Nuage` et `maxlestage/Memoire` autorisées ; licence
  propriétaire de `nuage`, `memoire-orm` et `memoire-orm-derive` clarifiée par l'empreinte de
  leur `LICENSE` et autorisée pour eux seuls ; script de build de `num-traits` (sonde de
  `rustc`) approuvé après relecture. Décision T14 dans `docs/decisions.md`.
- Première pièce de l'étape b : `database/src/nuage_store.rs` ouvre le moteur embarqué et y
  garde la version du schéma (table `meta`) ; test de réouverture.
- CI : lint et tests du crate `database` avec `--features nuage` (job Linux).

**Vérifié.** `cargo deny check` : advisories, bans, licences et sources ok. Lint et tests du
workspace ; lint et tests de `stratyx-database --features nuage` (dont le nouveau test).

### Étape b, 1re partie : les mains (2026-11-14)

- `nuage_store.rs` : table SQL `main` (une ligne par main : espace, empreinte, date de jeu,
  date d'enregistrement, main enregistrée et résumé en JSON ; index sur l'espace et
  l'empreinte), qui remplace à elle seule quatre tables redb (`hands`, `summaries`,
  `hand_locator`, `fingerprints`). Valeurs toujours en paramètres `?`.
- Opérations : `put_hand` (insertion, remplacement, doublon d'une autre main, identifiant
  vide refusé), `get_hand`, `hand_summary`, `find_by_fingerprint`, `delete_hand`,
  `count_hands`. Les écritures (lecture du doublon puis écriture) sont sérialisées par un
  verrou ; chaque requête SQL est une transaction.
- Erreur `DatabaseError::Nuage` (présente seulement avec la fonctionnalité `nuage`).

**Vérifié.** Test `les_mains_se_comportent_comme_avec_redb` : les historiques d'exemple (cash
et tournoi) enregistrés côte à côte dans redb et dans Nuage, chaque résultat comparé
(écriture, lecture, résumé, empreinte, comptage par espace, remplacement, doublon,
suppression). Test de réouverture : les mains relues après fermeture. Lint et tests du
workspace, `cargo deny check`, lint et tests de `stratyx-database --features nuage`.

### Demande de l'utilisateur : tout fusionner d'un coup (2026-11-14)

« Tu peux merge stratyx vers nuage db tu peux importer memoire et paquets » (choix :
« Tout, d'un coup »). Fait en une fois, sans attendre la routine :

- **Memoire** : `claude/nuage-wire` fusionnée dans master (avance rapide, `074fbde`).
- **Nuage** : `claude/nuage-100-rust` fusionnée avec le master du jour (`5c057b9` : textes
  rangés en nombres, `INTERVAL`, `extract`), lint et tests (toutes fonctionnalités et moteur
  seul) verts, poussée sur master (`5953e23`). Puis **`EmbeddedTransaction::sql`** (`23eb6fc`,
  testé) : du SQL dans une transaction, pour lire, décider et écrire d'un bloc.
- **Paquets** : ajouté à la session ; `Cargo.lock` passé au master de Memoire, sur la branche
  `claude/memoire-nuage-wire` (sa CI fusionne et déploie si tout est vert).
- **STRATYX, base locale** : tout le crate `database` est sur Nuage DB (étapes b et c
  ensemble). Un dossier Nuage au lieu d'un fichier redb ; tables SQL `main` (main et résumé
  en JSON, colonnes indexées : espace, session, format, validation, à revoir, date),
  `main_joueur`, `empreinte`, `session`, `profil`, `progression`, `strategie`, `ingestion`,
  `meta`, `migration`. Chaque opération est une transaction Nuage (écritures sérialisées) ;
  migrations dans une transaction (une migration ratée n'a rien créé). API publique
  inchangée ; `DatabaseError::Storage` porte désormais un texte.
- **Anciennes bases** : `Database::open` d'un fichier redb l'importe (mains avec dates, lots
  et statuts, empreintes, sessions, profils, progression, stratégie, rapports, compteur de
  lots), le garde renommé `<nom>.ancienne-redb` ; en cas d'échec, rien n'est changé. redb ne
  sert plus qu'à cette lecture (fonctionnalité `import-redb`, par défaut).
- **Site** : Nuage du master (`23eb6fc`) avec `features = ["s3"]` et
  `config::engine_from_vars` (configuration du moteur seule).

**Vérifié.** Les 14 tests d'intégration de la base (`tests/store.rs`), l'import PokerStars et
sa robustesse, les tests de migration réécrits, deux tests d'import d'une ancienne base
(tout relu, réouverture sans réimport, compteur de lots qui continue ; fichier illisible
laissé intact) ; aussi sans `import-redb`. Lint et tests du workspace, `cargo deny check`
(aucun crate interdit dans l'arbre de la base). Site : formatage, lint natif et
WebAssembly, tests. Test des fichiers abîmés réécrit pour un dossier Nuage : il trouve
encore des paniques de redb 4.6 (utilisé par Nuage pour ses pages), il reste ignoré.

**Chantier Nuage : terminé.** Reste (facultatif) : rapporter le défaut de redb en amont ;
créer le seau S3 du site (secrets `NUAGE_*`).

## 2026-11-14 — Demande de l'utilisateur : configuration de Heroku

L'app Heroku `stratyx` (stack heroku-26, Europe, dyno eco) existait déjà et se déploie par
l'intégration GitHub de Heroku (chaque fusion dans master est compilée) ; le workflow
`heroku.yml` est sauté faute de secret `HEROKU_API_KEY` et de variable `HEROKU_APP_NAME`
côté GitHub. L'app n'avait **aucune variable de configuration**. Réglé par l'API Heroku :

- `SITE_URL` = `https://stratyx-075b7d703eed.herokuapp.com` (liens canoniques, plan du site
  et robots.txt) ;
- `NUAGE_NAME` = `stratyx` et `NUAGE_ENCRYPTION_KEY` (clé aléatoire de 256 bits, générée
  sur place, jamais écrite ailleurs que dans la configuration de l'app) : la base Nuage DB
  du site est chiffrée.

**Vérifié.** Redémarrage du dyno, `/health` 200, `sitemap.xml` sur l'adresse publique, base
ouverte sans erreur avec la clé (journaux de l'app).

**Reste (utilisateur).** Les sauvegardes S3 (`NUAGE_BACKUP_S3_*`) demandent un seau à son
nom : sans elles, la liste d'attente et les messages sont perdus à chaque redémarrage du
dyno. Si les secrets GitHub `NUAGE_*` sont ajoutés un jour pour le workflow, y reprendre la
même `NUAGE_ENCRYPTION_KEY` que l'app (sinon la base sauvegardée serait illisible).

## 2026-11-14 — Itération 28 : recherche et lecture de la base locale 2 à 2,5 fois plus rapides

**Mesuré** (nouvel exemple `cargo run --release -p stratyx-database --example mesure_base --
5000` : mains distinctes, moitié écrites une à une, moitié ingérées en lot). Sur Nuage DB,
le coût d'un parcours est celui des lignes lues : la main complète (plusieurs Ko de JSON)
était dans la même ligne que les colonnes de recherche, et une recherche paginée relisait et
désérialisait le résumé de chaque candidat. Banc SQL isolé, 5 000 lignes : un parcours
passe de 4,6 ms à 48,8 ms quand chaque ligne porte 8 Ko de plus.

**Fait.**
- Schéma : la main complète va dans `main_donnees` (identifiant → JSON) ; `main` ne garde
  que les colonnes indexées et le résumé. Le schéma 1 de Nuage n'a encore été écrit par
  aucune application (le crate n'est utilisé que par ses tests) : modifié sans migration.
- Recherche : quand tous les filtres sont des colonnes indexées (espace, joueur, session,
  format, type de partie, statut, période), un seul passage lit l'identifiant et la date des
  candidats, le tri et la page se font dessus, puis seuls les 20 résumés de la page sont
  relus. Les filtres sur le résumé (salle, résultat du héros, abattage) gardent le parcours.
- Écriture d'une main : une seule requête (ligne, données, empreinte, joueurs en une
  insertion de plusieurs lignes) au lieu de 3 + un par joueur.

| 5 000 mains | Avant | Après |
|---|---:|---:|
| recherche par joueur (page de 20) | 43,4 ms | 21,5 ms |
| recherche sans filtre (page de 20) | 21,8 ms | 9,6 ms |
| recherche par salle (parcours) | 18,9 ms | 18,9 ms |
| lecture d'une main | 87 µs | 35 à 48 µs |
| ingestion de 2 500 mains | 1,45 s | 1,18 s |

L'écriture une à une reste vers 0,65 ms par main : c'est le rejeu de validation et la
validation de la transaction, pas le SQL.

**Vérifié.** Nouveau test `search_in_base_matches_the_scan` : pour 72 combinaisons (9
filtres indexés × 2 ordres × 4 pages, avec dates en double et mains sans date), la recherche
par la base donne exactement le résultat du parcours un à un. Lint et tests du workspace,
base sans `import-redb`, `cargo deny check`.

## 2026-11-14 — Demande de l'utilisateur : optimiser Nuage DB (itération 29)

**Mesuré** (nouvel exemple de Nuage `sql_bench`, 5 000 lignes, puis callgrind) : 88 % du
temps d'une lecture par clé partait dans `serde_json` — chaque instruction relisait le JSON
du catalogue de ses tables (`Ctx::meta`), puis le resérialisait pour retrouver le schéma
(`SchemaCache::schema`).

**Fait dans Nuage** (master `997927b`) :
- cache des entrées du catalogue analysées, par leurs octets stockés (mêmes octets, même
  table : exact sur toutes les branches et dans toute transaction), avec le schéma ;
- `COUNT(*)` dont le filtre fixe exactement les colonnes d'un index : compté sur l'index ;
- test : le compte par l'index égale le nombre de lignes (entiers, texte, NULL, littéral d'un
  autre type, index de deux colonnes, écritures non validées) ; exemple `sql_bench`.
- Essayé puis écarté : ne copier qu'une fois les tables d'une sous-requête (aucun gain
  mesuré, les tables de Memoire sont déjà partagées).

| Nuage, 5 000 lignes | Avant | Après |
|---|---:|---:|
| insertion (par ligne, en transaction) | 73 µs | 32 µs |
| lecture par clé primaire | 37 µs | 8,5 µs |
| `count(*)` | 29,6 µs | 3,6 µs |
| `count(*)` sur colonne indexée (1/3) | 2,73 ms | 0,43 ms |
| page de 20 clés (`IN`) | 69 µs | 42 µs |

**STRATYX** passe à ce Nuage (base locale et site). `mesure_base -- 5000` :

| Base locale, 5 000 mains | Avant | Après |
|---|---:|---:|
| écriture d'une main | 619 µs | 323 µs |
| ingestion de 2 500 mains | 1,09 s | 0,63 s |
| lecture d'une main | 34 µs | 21 µs |
| revalidation | 4,5 s | 1,9 s |
| recherche (pages de 20) | inchangée (à ±10 % près) | |

**Vérifié.** Nuage : formatage, lint (toutes fonctionnalités, moteur seul, site en
WebAssembly), tests, exemples des clients contre un serveur local (tous sauf C, faute
d'en-têtes libcurl ici ; la CI de Nuage les installe). STRATYX : lint et tests du workspace,
base sans `import-redb`, `cargo deny check` ; site : formatage, lint natif et WebAssembly,
tests.

## 2026-11-14 — Demande de l'utilisateur : STRATYX dans la console du serveur Nuage DB

La console du serveur Nuage DB (`nuage-f48a8945f31c.herokuapp.com`) liste ses applications
(`nuage`, `seriously`) : STRATYX n'y figurait pas, car le site gardait sa base **embarquée**
dans son propre dyno (perdue à chaque redémarrage, faute de seau S3).

**Fait (site).** Avec `NUAGE_URL` et `NUAGE_TOKEN`, la base du site est une application du
serveur Nuage DB, interrogée en SQL sur HTTPS (`POST /v1/sql`, jeton `Bearer`, valeurs en
paramètres) ; les tables sont créées à la première requête. Sans ces variables, la base
reste embarquée (tests, développement). Client HTTPS : hyper + rustls + webpki-roots, les
crates déjà présentes pour les sauvegardes S3 de Nuage (aucune de plus dans `Cargo.lock`),
délai de 10 s. `site/README.md`, « Base de données », décrit les deux modes.

**Vérifié.** Trois tests du client contre un faux serveur (requête, jeton, corps JSON,
lecture des lignes ; erreur 401 rapportée ; adresses invalides refusées). De bout en bout,
avec un vrai serveur Nuage DB local (master `997927b`) : inscription, inscription en double
ignorée et message de contact écrits dans l'application du serveur, relus par `/v1/sql` ;
mauvais jeton → page « indisponible » et cause dans le journal ; aucun dossier embarqué créé.
HTTPS vers le vrai serveur : non vérifiable d'ici (la passerelle de sortie du bac à sable
re-signe les certificats) ; le certificat du serveur (Amazon RSA 2048 M01) est couvert par
webpki-roots, comme pour les sauvegardes S3 de Nuage.

**Reste (utilisateur).** Créer l'application `stratyx` dans la console du serveur et mettre
son jeton dans `NUAGE_TOKEN` de l'app Heroku `stratyx` (lecture des secrets du serveur
refusée à l'assistant : la création passe par la console).

## 2026-11-14 — Itération 30 : le jeton du serveur Nuage DB ne part jamais en clair

**CI.** Celle du dernier push (`b752d5d`) est verte et fusionnée dans master ; branche
repartie de master.

**Trouvé.** Le client du serveur Nuage DB (`site/src/main.rs`, `base::distant`) acceptait
une `NUAGE_URL` en `http://` vers n'importe quel hôte (utile aux tests, contre un faux
serveur local), alors que son message d'erreur exigeait `https://`. Une adresse mal saisie
dans les Config Vars d'Heroku aurait envoyé le jeton (`Authorization: Bearer`) en clair à
chaque inscription, sans aucun signe : Heroku redirige vers HTTPS, mais après avoir reçu
l'en-tête.

**Fait.** `http://` n'est admis que vers la machine elle-même (`localhost`, adresses de
bouclage) ; ailleurs, `Serveur::new` refuse l'adresse au démarrage, avec l'adresse `https://`
à utiliser (le journal du site dit pourquoi, les formulaires affichent « indisponible »).
`site/README.md`, « Base de données », le précise.

**Mesuré.** Adresses `http://` distantes acceptées : 4 sur 4 avant, 0 sur 4 après (nom de
domaine, port explicite, adresse privée `192.168.…`, `localhost.example.com`) ; les 4
adresses locales restent admises (`localhost`, `LOCALHOST`, `127.0.0.1`, `127.8.9.1`).
La production n'est pas touchée : sa `NUAGE_URL` est déjà en `https` (schéma seul relu
par l'API d'Heroku, sans lire le jeton).

**Vérifié.** Nouveau test `le_jeton_ne_part_jamais_en_clair` ; les trois tests existants du
client (faux serveur local en `http://127.0.0.1`) passent toujours. Toutes les commandes de
la CI : workspace (formatage, lint, tests, base sans `import-redb`, `cargo deny check`) et
site (formatage, lint natif et WebAssembly, tests).

**Pistes suivantes.** Borner la taille de la réponse lue du serveur (aujourd'hui seul le
délai de 10 s la limite) ; adresses IPv6 entre crochets (`http://[::1]:8080`) non gérées par
le découpage hôte/port.

## 2026-11-14 — Itération 31 : réponse du serveur Nuage DB bornée à 1 Mio

**CI.** Celle de l'itération 30 (`4fbef8d`) est verte et fusionnée dans master.

**Trouvé.** Le client du serveur Nuage DB (`base::distant`) gardait en mémoire toute la
réponse, sans limite : seul le délai de 10 s la bornait. Un serveur défaillant (ou une
page d'erreur d'un intermédiaire) pouvait faire allouer au dyno du site (512 Mo) tout ce
qu'il envoyait en 10 s, à chaque formulaire.

**Fait.** Lecture par `http_body_util::Limited` (crate déjà présente), limite
`REPONSE_MAX` = 1 Mio : le site ne lit que des comptes et des accusés d'écriture (quelques
dizaines d'octets). Au-delà, la lecture s'arrête et l'erreur va au journal du site, comme
les autres (« indisponible » pour le visiteur), que le statut soit 200 ou non.

**Mesuré.** Réponse d'exactement 1 Mio : lue en entier ; 1 Mio + 1 octet : refusée
(avant : acceptée sans limite), message d'erreur de moins de 200 caractères.

**Vérifié.** Nouveau test `une_reponse_demesuree_est_abandonnee` (limite exacte, un octet
de plus, statuts 200 et 500) ; les quatre autres tests du client passent. Toutes les
commandes de la CI, workspace et site.

**Pistes suivantes.** Adresses IPv6 entre crochets (`http://[::1]:8080`) non gérées par le
découpage hôte/port de `NUAGE_URL`.

## 2026-11-14 — Itération 32 : CI rouge (réouverture de la base), corrigée

**CI.** Celle de l'itération 31 (`09453ac`) était **rouge**, donc non fusionnée : sous Linux,
`file_database_persists_and_migrates` (`database/tests/store.rs`) a échoué à rouvrir la
base qu'il venait de fermer (« already open. Cannot acquire lock ») ; macOS et Windows
verts. Sans lien avec le changement de l'itération 31 (site seul).

**Cause.** Une base fermée ne rend pas toujours son dossier à l'instant : le fil d'entretien
de Nuage (un passage toutes les 200 ms) tient le moteur le temps de son passage, et le verrou
du fichier n'est levé qu'ensuite. Rouvrir la base dans cette fenêtre échoue. Les tests de
Nuage contournent eux-mêmes ce cas (`open_when_free`). Le défaut est intermittent : 0 échec
sur 500 exécutions ici, 1 sur la CI. Entre-temps master est passé au magasin de pages
propre à Nuage (`7ce16d2`) : la fenêtre y existe toujours (même fil d'entretien).

**Fait.** Branche rebasée sur master (l'itération 31 y est rejouée telle quelle).
`Database::open` attend jusqu'à 2 s qu'un dossier « already open » soit rendu
(`open_when_free`, `database/src/store.rs`), par essais de 10 ms ; toute autre erreur est
rendue aussitôt, et une base réellement tenue ailleurs reste refusée, après ce délai.

**Mesuré.** Base fermée 300 ms après la demande d'ouverture (cas rendu déterministe) :
échec immédiat avant, ouverture réussie après (test rouge sans le correctif, vert avec).
Base tenue pour de bon : toujours refusée, en 2 s au lieu de 0.

**Vérifié.** Deux tests : `reopening_waits_for_a_database_being_closed`,
`a_database_held_elsewhere_is_refused`. Toutes les commandes de la CI, workspace et site.

**Pistes suivantes.** En amont (Nuage) : que `Engine::close` attende la fin du passage du
fil d'entretien rendrait l'attente inutile — à décider par l'utilisateur. Adresses IPv6
entre crochets dans `NUAGE_URL`.

## 2026-11-14 — Itération 33 : `NUAGE_URL` en IPv6 (adresse entre crochets)

**CI.** Celle de l'itération 32 (`fa35047`) est verte et fusionnée dans master (les
itérations 31 et 32 sont donc déployées).

**Trouvé.** Le découpage hôte/port de `NUAGE_URL` (`base::distant`, site) coupait au dernier
`:` : `https://[2001:db8::1]` donnait « port invalide », et `http://[::1]:8080` (serveur
local) était pris pour une adresse distante, donc refusé depuis l'itération 30.

**Fait.** Une adresse entre crochets est lue comme IPv6 (vérifiée, port facultatif après le
crochet), gardée sans crochets pour la connexion et le nom TLS, remise entre crochets dans
l'en-tête `Host`. Une IPv6 sans crochets est refusée avec la marche à suivre.
`site/README.md` le précise.

**Mesuré.** Sur 10 adresses IPv6 de test : 3 valides acceptées (0 avant), 7 invalides
refusées avec un message qui dit quoi corriger (crochet non fermé, port, IPv6 nue, nom entre
crochets, `http://` vers une IPv6 distante).

**Vérifié.** Tests `les_adresses_ipv6_sont_entre_crochets` et `un_serveur_en_ipv6_est_joint`
(requête réelle à un faux serveur sur `[::1]`). **Limite :** ce bac à sable n'a pas d'IPv6 ;
le second test s'y déclare sans objet et ne s'exécute vraiment que là où `[::1]` existe (CI).
Toutes les commandes de la CI, workspace et site.

**Pistes suivantes.** Le client du site n'a plus de piste ouverte. En amont (Nuage) :
fermeture qui attend le fil d'entretien (itération 32).

## 2026-11-14 — Itération 34 : polices servies par le site (plus aucune requête vers Google)

**CI.** Celle de l'itération 33 (`f7177e2`) est verte et fusionnée dans master (non
vérifié : si le test `un_serveur_en_ipv6_est_joint` s'y est exécuté ou déclaré sans objet).

**Trouvé.** Chaque page chargeait ses polices (Inter, Space Grotesk) depuis
`fonts.googleapis.com` puis `fonts.gstatic.com` : l'adresse IP de chaque visiteur partait
chez Google, alors que la page « Confidentialité » annonce « aucun traceur tiers ». C'était
aussi une feuille de style bloquante sur un autre site (deux connexions de plus avant le
premier texte) et deux origines ouvertes dans la politique de sécurité.

**Fait.**
- `site/public/fonts/` : les quatre fichiers woff2 (polices variables, latin et latin
  étendu, tels que distribués par Google Fonts) et `LICENCE.txt` (SIL Open Font License 1.1,
  auteurs). `@font-face` en tête de `style.css`, mêmes plages de caractères
  (`unicode-range`) : le latin étendu n'est téléchargé que si une page en a besoin.
- Pages : plus de feuille de style externe ; les deux polices latines sont préchargées.
- Service worker : les deux polices latines sont gardées dès l'installation (hors ligne dès
  la première visite) ; le cache séparé des polices Google et son code disparaissent
  (l'ancien cache est supprimé à l'activation).
- Aucun texte du site à changer : la page de confidentialité dit maintenant vrai.

**Mesuré** (Chromium, pages `/` et `/app` servies en local) :

| | Avant | Après |
|---|---:|---:|
| origines contactées | 3 (site, googleapis, gstatic) | 1 |
| feuilles de style bloquantes hors du site | 1 | 0 |
| origines tierces dans la politique de sécurité (pages, `sw.js`) | 2 | 0 |
| polices téléchargées par page | 2 (chez Google) | 2 (70 Ko, sur le site) |

Les deux polices sont bien chargées et appliquées (`document.fonts`, familles calculées du
corps et des titres), sans erreur de console. Poids du dépôt : +175 Ko. Le temps gagné sur
un vrai réseau n'est pas mesurable d'ici (pas d'accès direct à Google depuis le bac à sable).

**Vérifié.** Nouveau test `aucune_ressource_ne_vient_d_un_tiers` : chaque page (plan du
site, `/app`, `/hors-ligne`), `style.css`, `sw.js` et les politiques de sécurité ne nomment
aucun autre site pour une ressource ; les quatre polices répondent en `font/woff2`. Toutes
les commandes de la CI, workspace et site.

**Pistes suivantes.** Fichiers statiques servis avec `cache-control: max-age=0` (une
revalidation par fichier à chaque visite sans service worker) : une durée plus longue pour
les polices et les icônes, qui ne changent pas, est à étudier (réglage de Paquets ?).

## 2026-11-14 — Itération 35 : cache long des polices, mesuré puis écarté (pas de changement de code)

**CI.** Celle de l'itération 34 (`4fb82cf`) est verte, fusionnée et déployée : en production,
`/fonts/inter-latin.woff2` répond en `font/woff2` (48 256 octets) et la page d'accueil ne
nomme plus `googleapis`.

**Essayé.** La piste de l'itération 34 : servir les polices avec `cache-control: public,
max-age=2592000` (30 jours) au lieu du `max-age=0` des fichiers statiques, par un middleware
du site (réponses 200 et 304), pour supprimer leur revalidation à chaque visite.

**Mesuré** (Chromium, profil gardé entre les visites, requêtes comptées dans le journal du
serveur, avant et après le changement) :

| Requêtes reçues par le serveur | Polices avant | Polices après | `style.css` |
|---|---:|---:|---:|
| 3 visites de l'accueil, sans service worker | 2 (200), 0 revalidation | identique | 1 (200) + 2 (304) |
| 3 visites de 2 pages, service worker actif | 2 (200), 0 revalidation | identique | 1 (200) + 5 (304) |

Chromium ne revalide déjà pas les polices d'une visite à l'autre, même avec `max-age=0` :
le changement n'y gagne aucune requête. **Écarté** (annulé, rien de poussé côté code) : un
middleware de plus sans gain mesurable. Non mesuré : Firefox et Safari, absents d'ici.

**Constat utile.** Ce qui est revalidé à chaque page, c'est `style.css` (une requête 304
par page vue, service worker ou non).

**Pistes suivantes.** Donner à `style.css` une adresse qui porte son empreinte
(`/style.css?v=…`) et un cache long supprimerait cette requête ; il faut alors que le
service worker (pré-cache et correspondance des adresses) et l'export statique suivent :
changement plus large, à mesurer de la même façon.

## 2026-11-14 — Itération 36 : adresses e-mail des formulaires, validation resserrée

**CI.** Celle de l'itération 35 (`1b0c37c`) est verte et fusionnée dans master.

**Trouvé.** `email_valide` (`site/src/main.rs`, formulaires « liste d'attente » et
« contact ») ne demandait qu'un `@`, un point dans la suite et aucun espace. Elle acceptait
donc `a@exemple.fr,victime@ailleurs.fr` (deux destinataires pour un futur envoi à la liste),
`a@b@exemple.fr`, `<script>@exemple.fr`, des caractères de contrôle ou invisibles, des
domaines impossibles (`a@exemple..fr`, `a@-exemple.fr`, `a@192.168.1.1`). Ces valeurs
étaient enregistrées telles quelles dans la base (toujours en paramètres SQL : pas
d'injection, mais des adresses inutilisables ou dangereuses à réutiliser).

**Fait.** Un seul `@` ; partie locale (64 octets au plus) en lettres, chiffres et
`.!#$%&'*+/=?^_{|}~-` et accent grave, sans point au bord ni doublé ; domaine d'au moins
deux étiquettes (63 octets au plus) en lettres, chiffres et tirets, sans tiret au bord,
dernière étiquette d'au moins deux caractères et pas toute en chiffres. Les lettres
accentuées restent admises (`zoé@exemple.fr`, `camille@école.fr`). Les réponses des
formulaires ne changent pas (mêmes pages d'erreur).

**Mesuré.** Table du test `emails` : 20 adresses invalides, toutes acceptées par l'ancienne
règle (20 sur 20, recalculé à part), toutes refusées par la nouvelle ; 8 adresses valides
(sous-domaines, `+`, apostrophe, accents, espaces autour) acceptées avant comme après.

**Vérifié.** Test `emails` étendu (33 cas : 8 valides, 5 déjà refusées avant, 20 nouvellement
refusées). Toutes les
commandes de la CI, workspace et site.

**Pistes suivantes.** Les adresses déjà enregistrées avec l'ancienne règle ne sont pas
revues (la base de production n'est pas lue par la routine). `style.css` revalidé à chaque
page (itération 35).

## 2026-11-14 — Itération 37 : formulaires du site, les refus sont testés

**CI.** Celle de l'itération 36 (`3611969`) est verte et fusionnée dans master.

**Constat.** Le seul test des formulaires (`les_formulaires_enregistrent_dans_nuage`)
n'envoyait que des demandes valides : aucun des refus des deux formulaires (liste d'attente,
contact) n'était exercé. Retirer un de ces contrôles ne faisait échouer aucun test.

**Fait.** Le test envoie aussi neuf demandes refusées et vérifie pour chacune la réponse et
qu'aucune ligne n'est écrite en base :
- liste d'attente : adresse invalide, deux adresses séparées par une virgule (itération 36),
  champ piège rempli (même réponse qu'à un visiteur, rien d'enregistré), requête venue d'un
  autre site (`sec-fetch-site: cross-site`, refusée par Paquets) ;
- contact : nom vide, adresse invalide, message trop court, champ piège, autre site.

Aucun défaut trouvé : le code des formulaires n'est pas modifié.

**Mesuré.** Cinq mutations du code des formulaires (champ piège ignoré dans chacun des deux,
adresse non vérifiée à l'inscription, longueur minimale du message, nom vide accepté) : 5
détectées sur 5 par le test étendu ; l'ancien n'envoyait aucune de ces demandes.

**Vérifié.** Toutes les commandes de la CI, workspace et site.

**Pistes suivantes.** `style.css` revalidé à chaque page (itération 35).

## 2026-11-14 — Itération 38 : polices de la copie GitHub Pages (régression de l'itération 34), corrigée

**CI.** Celle de l'itération 37 (`66b05e1`) est verte et fusionnée dans master.

**Trouvé.** Régression introduite par l'itération 34 : `style.css` appelait les polices par
une adresse absolue (`url("/fonts/…")`). Sur Heroku (site servi à la racine) tout va bien,
mais l'export statique (`cargo run -- export`, copie GitHub Pages servie sous `/STRATYX/`)
copie `style.css` tel quel, sans préfixer ses adresses : les polices y étaient cherchées à
la racine du domaine (404) et le texte retombait sur les polices du système. Les tests ne le
voyaient pas : ils ne rendent le site qu'à la racine.

Portée : la copie GitHub Pages ne se déploie plus qu'à la main (`pages.yml`,
`workflow_dispatch`) ; rien n'indique qu'elle ait été relancée depuis l'itération 34 (non
vérifié). Le site de production (Heroku) n'a jamais été touché.

**Fait.** Adresses relatives à la feuille de style (`url("fonts/…")`) : justes à la racine
comme sous un préfixe. Le reste (préchargement dans les pages, pré-cache du service worker)
était déjà préfixé par l'export.

**Mesuré** (export avec `BASE_PATH=/stratyx`, servi sous ce préfixe, Chromium) :

| Page d'accueil de la copie statique | Avant | Après |
|---|---:|---:|
| polices chargées | 0 | 2 (Inter, Space Grotesk) |
| réponses 404 | 2 (`/fonts/*.woff2`) | 0 |

Site servi par le serveur (racine) : 2 polices chargées, aucune erreur, avant comme après.

**Vérifié.** Le test `aucune_ressource_ne_vient_d_un_tiers` exige maintenant des adresses
relatives pour les polices et refuse toute adresse absolue (`url("/…")`) dans `style.css`.
Toutes les commandes de la CI, workspace et site.

**Pistes suivantes.** Un test qui exporte le site sous un préfixe et vérifie chaque adresse
(l'export bascule un état global, `STATIQUE` : à isoler dans son propre binaire de test).
`style.css` revalidé à chaque page (itération 35).

## 2026-11-14 — Itération 39 : l'export statique sous un préfixe est testé de bout en bout

**CI.** Celle de l'itération 38 (`b2ec992`) est verte et fusionnée dans master.

**Constat.** La régression de l'itération 34 (polices introuvables sur la copie servie sous
`/STRATYX/`) n'avait été vue par aucun test : seul `prefixer` était testé, sur cinq chaînes.

**Fait.** Nouveau test `site/tests/export.rs` : il lance le binaire comme `pages.yml`
(`stratyx-site export <dossier>`, `BASE_PATH=/prefixe`, dans son propre processus puisque
l'export bascule l'état global `STATIQUE`), puis relit tout ce qui est exporté :
- pages (`href`, `src`, `poster`, `content`, et `action` des formulaires) ;
- feuilles de style (`url(…)`, absolues ou relatives à la feuille) ;
- manifeste et service worker (toute chaîne qui commence par `/`).

Chaque adresse du site doit porter le préfixe et désigner un fichier exporté (les `action`
et `/health`, qui n'existent que sur le serveur : préfixe seulement).

**Mesuré.** 549 adresses vérifiées dans l'export (pages, `style.css`, manifeste, `sw.js`),
0 cassée. La régression de l'itération 34, remise à la main (`url("/fonts/…")`),
fait échouer le test en nommant les 4 polices ; avant ce test, rien ne la signalait.
Durée : 1 s une fois le binaire construit.

**Vérifié.** Toutes les commandes de la CI, workspace et site.

**Pistes suivantes.** Le test ne voit pas les adresses que le WebAssembly des îlots
construirait à l'exécution (pas de `pkg/` ici ni dans le job de tests). `style.css` revalidé
à chaque page (itération 35).

## 2026-11-15 — Itération 40 : identifiants HTML en double ; le champ « Message » retrouve son libellé

**CI.** Celle de l'itération 39 (`abf0e8b`) est verte et fusionnée dans master.

**Trouvé.** Deux identifiants servaient deux fois dans une même page :
- `contenu`, sur `/contact` : à la fois `<main id="contenu">` (lien « Aller au contenu ») et
  la zone de texte du message. Le libellé « Message » (`for="contenu"`) désignait donc
  `<main>` : cliquer dessus ne plaçait pas le curseur dans le champ, et le champ n'avait
  **aucun nom accessible** pour un lecteur d'écran. L'audit de l'itération 24 ne l'avait pas
  vu (le champ a bien un `<label for>`, mais vers le mauvais élément) ;
- `lg`, sur toutes les pages : le dégradé du logo, dessiné dans l'en-tête et dans le pied.

**Fait.** La zone de texte s'appelle `message` (son nom de formulaire reste `contenu` : rien
ne change pour le serveur) ; le retour après une erreur de saisie pointe sur elle
(`/contact?erreur=…#message`). Le logo reçoit un identifiant de dégradé par exemplaire
(`lg-entete`, `lg-pied`).

**Mesuré** (Chromium, page `/contact`) :

| | Avant | Après |
|---|---:|---:|
| libellés associés à la zone de texte | 0 | 1 (« Message ») |
| champ trouvé par son nom accessible « Message » | non | oui |
| clic sur le libellé | rien (`body`) | curseur dans le champ |
| pages avec un identifiant en double | toutes | 0 |

**Vérifié.** `chaque_page_repond` vérifie maintenant, sur chaque page du plan du site,
`/app`, `/hors-ligne`, la page 404 et `/merci`, qu'aucun identifiant ne sert deux fois.
Toutes les commandes de la CI, workspace et site.

**Pistes suivantes.** `style.css` revalidé à chaque page (itération 35).

## 2026-11-15 — Itération 41 : hiérarchie des titres, pied de page corrigé

**CI.** Celle de l'itération 40 (`c40d862`) est verte et fusionnée dans master.

**Démarche.** Audit d'accessibilité dans Chromium sur les 16 pages (champs, liens et boutons
sans nom, images sans `alt`, libellés et attributs `aria-*` vers un identifiant absent ou
non étiquetable, langue, titre, un seul `h1`, ordre des titres). Depuis l'itération 40, plus
aucun champ, lien, bouton ni libellé en défaut.

**Trouvé.** Neuf pages passaient d'un `h1` à un `h3` sans `h2`. Sur toutes, le pied de page
(commun) ouvrait par un `h3` (« Soyez parmi les premiers… ») suivi de colonnes en `h4` :
pour un lecteur d'écran qui navigue par titres, le pied apparaissait comme une sous-partie de
la dernière section de chaque page.

**Fait.** Pied de page : `h2` pour l'inscription, `h3` pour les quatre colonnes (feuille de
style ajustée pour un rendu identique).

**Mesuré.**

| | Avant | Après |
|---|---:|---:|
| pages avec un saut de niveau de titre (sur 15 pages HTML) | 9 | 5 |
| rendu de `/`, `/contact`, `/plan-du-site` (captures pleine page, 1280 px) | — | 0 pixel de différence |

La comparaison de captures a servi : un premier essai laissait le titre « Engagements » en
`h4`, sans style, ce que seul le rendu montrait.

**Vérifié.** Nouveau test `les_titres_ne_sautent_pas_de_niveau` : un seul `h1` par page, pied
en `h2` puis `h3`, aucun saut sur les pages corrigées. Toutes les commandes de la CI,
workspace et site.

**Reste (5 pages).** `/fonctionnalites`, `/ia`, `/feuille-de-route`, `/fair-play` et
`/contact` ont des cartes en `h3` directement sous le `h1` (pas de titre de section) : les
corriger demande un titre de section ou un changement de niveau visible, donc un choix de
mise en page. Elles sont listées dans le test (`a_reprendre`).

## 2026-11-15 — Itération 42 : contraste, plus de texte clair sur le vert du site

**CI.** Celle de l'itération 41 (`0d3e82d`) est verte et fusionnée dans master.

**Démarche.** Mesure du contraste des textes dans Chromium (couleur calculée du texte contre
son fond, seuils WCAG AA : 4,5:1, ou 3:1 pour un grand texte), puis relecture de toutes les
règles de `style.css` qui peignent le vert du site (`--violet`, #16c784).

**Trouvé.** Sept éléments écrivaient en clair sur ce vert, alors que les boutons et les
pastilles actives du site y écrivent déjà en sombre (#04140c) :
- le lien d'évitement « Aller au contenu » (premier élément atteint au clavier) et le
  bouton rond « Laboratoire » du pied de page : texte blanc, **2,2:1** ;
- les pictogrammes des réseaux au survol, la flèche « retour en haut » au survol, le signe
  d'une question ouverte de la FAQ, le numéro d'une étape active, le texte sélectionné :
  blanc ou presque, environ 2:1 (3:1 demandé pour un pictogramme).

**Fait.** Ces sept règles écrivent en #04140c (le lien d'évitement passe aussi en gras).

**Mesuré.** Contraste sur #16c784 : 2,2:1 (blanc) → 8,6:1 (#04140c), calcul WCAG. Règles à
fond vert avec un texte ou un pictogramme clair : 7 → 0 (sur 14 règles à fond vert, dont 3
décors sans contenu). Rendu vérifié sur captures (lien d'évitement, bouton rond, réseau au
survol, flèche, FAQ).

**Vérifié.** Nouveau test `pas_de_texte_clair_sur_le_vert` : toute règle de `style.css` à
fond vert fixe la couleur sombre, hors décors listés. Toutes les commandes de la CI,
workspace et site.

**Limites et pistes.** La mesure automatique ne couvre pas les textes posés sur un dégradé
ou une image (fond non calculable simplement) : 249 textes mesurés sur 15 pages. La couleur
`--faint` (#66806f) est à **4,49:1** sur le fond principal et à 3,9:1 sur le fond le plus
clair (`--bg-3`), juste sous 4,5:1 : elle sert à 18 règles (mentions, libellés, texte
d'exemple des champs). L'éclaircir est un choix de charte, laissé à l'utilisateur. Titres :
5 pages à reprendre (itération 41).

## 2026-11-15 — Itération 43 : audit mobile ; liens du pied de page, zone tactile de 23 → 35 px

**CI.** Celle de l'itération 42 (`a504453`) est verte et fusionnée dans master.

**Démarche.** Audit dans Chromium des 15 pages à 320, 375 et 768 px de large (écran tactile
pour les deux premières) : page plus large que l'écran, texte coupé par un conteneur, zones
tactiles de moins de 24 px (critère WCAG 2.2 « taille de la cible », hors liens dans une
phrase ou une liste).

**Trouvé.** Aucun débordement horizontal ni texte coupé, sur aucune page et à aucune
largeur. Seul défaut : cinq liens isolés du pied de page (« Mentions légales »,
« Confidentialité », « Plan du site », « sitemap.xml », et « Fair-play » dans le bandeau du jeu
responsable) n'offraient que 23 px de haut, sur toutes les pages.

**Fait.** Ces liens reçoivent 6 px de marge intérieure en haut et en bas ; leur soulignement
au survol reste sous le texte, et l'écart entre les lignes de liens est repris par cette
marge.

**Mesuré.**

| | Avant | Après |
|---|---:|---:|
| signalements de l'audit (15 pages × 3 largeurs) | 225 | 0 |
| hauteur des cinq liens | 23 px | 35 px |
| hauteur du bas de page à 1280 px | 105 px | 105 px |
| hauteur du bas de page à 320 px | 207 px | 223 px |

**Vérifié.** Test `les_liens_du_pied_ont_une_zone_tactile` (règle présente, liens bien dans
ces blocs) ; rendu contrôlé sur capture. Toutes les commandes de la CI, workspace et site.

**Pistes suivantes.** Couleur `--faint` (itération 42) et titres de 5 pages (itération 41),
laissés au choix de l'utilisateur.

## 2026-11-15 — Itération 44 : formulaire d'inscription, focus de la liste visible et texte entier

**CI.** Celle de l'itération 43 (`daa8dd4`) est verte et fusionnée dans master.

**Démarche.** Audit de la navigation au clavier dans Chromium (touche Tab sur 5 pages à
1280 px et 2 à 375 px, 398 éléments atteints) : indicateur de focus présent, focus jamais
dans un bloc caché ni hors de l'écran.

**Trouvé.** Un seul défaut de focus, puis un second vu sur la capture :
- formulaire d'inscription (pied de chaque page, accueil) : le champ e-mail et la liste des
  profils n'ont pas d'indicateur propre, seulement le halo commun du formulaire. Dans le
  champ, le curseur de texte suffit ; sur la liste, rien ne disait qu'elle avait le focus ;
- à partir de 641 px de large (formulaire en ligne), cette liste était réduite à 98 px
  pour un texte qui en demande 149 : « Joueur loisir » s'affichait « Joueu », flèche masquée.

**Fait.** La liste montre un contour au focus clavier (`:focus-visible`). En ligne, elle
garde la largeur de son texte (`flex: none`), le champ e-mail prend le reste, et le
formulaire peut aller jusqu'à 580 px (520 avant).

**Mesuré** (Chromium) :

| | Avant | Après |
|---|---:|---:|
| contour de focus de la liste au clavier | aucun | 2 px |
| largeur de la liste, formulaire en ligne (besoin : 149 px, 172 pour la plus longue) | 98 px | 173 px |
| largeur du champ e-mail, en ligne | 218 px | 203 px (139 px à 1024 px) |
| formulaires qui débordent (24 mesurés, 6 largeurs de 641 à 1440 px, 4 pages) | 0 | 0 |

En colonne (jusqu'à 640 px) rien ne change. Audit mobile de l'itération 43 : toujours 0.

**Vérifié.** Test `la_liste_des_profils_reste_lisible_et_son_focus_visible` (règles
présentes) ; rendu contrôlé sur captures avant et après. Toutes les commandes de la CI,
workspace et site.

**Pistes suivantes.** Couleur `--faint` (itération 42) et titres de 5 pages (itération 41),
laissés au choix de l'utilisateur.

## 2026-11-15 — Itération 45 : première visite, le service worker ne retélécharge plus tout (−42 %)

**CI.** Celle de l'itération 44 (`8060c08`) est verte et fusionnée dans master.

**Démarche.** Bundle WebAssembly construit ici pour la première fois (comme
`heroku-build.sh` : `wasm-bindgen` 0.2.129), puis site parcouru dans Chromium avec ses îlots :
15 pages à 1280 et 375 px (défilement complet), les trois exemples de `/app`, quatre
saisies invalides (vide, bruit, main tronquée, caractères de contrôle), et sur
`/laboratoire` les exemples et des saisies absurdes de l'équité, de la grille, de l'ICM et
des jetons. **Aucune exception, aucun `NaN`, aucun débordement** ; chaque saisie invalide
reçoit un message clair.

**Trouvé.** À l'installation, le service worker demandait chaque fichier de son pré-cache
avec `cache: "reload"`, qui ignore le cache du navigateur : tout ce que la page venait de
charger (WebAssembly de 1,2 Mo, son JavaScript, `style.css`, les deux polices) était
téléchargé une seconde fois, en entier, dès la première visite.

**Fait.** `cache: "no-cache"` : chaque fichier est toujours revalidé auprès du serveur
(jamais de copie périmée à une mise à jour), mais un fichier inchangé répond 304, sans corps.

**Mesuré** (première visite de l'accueil, Chromium, requêtes lues dans le journal du
serveur, tailles en Brotli) :

| | Avant | Après |
|---|---:|---:|
| octets envoyés par le serveur | 1 302 916 | 756 668 (−42 %) |
| réponses complètes (200) | 30 | 25 |
| réponses 304 | 0 | 5 |
| WebAssembly téléchargé | 2 fois | 1 fois |
| entrées du cache du worker | 23 | 23 |

Les entrées du cache sont entières (WebAssembly 1 245 732 octets, police 48 256,
`style.css` 86 506, comme sur le disque) et, réseau coupé, `/laboratoire` se charge et
calcule une équité.

**Vérifié.** `chaque_page_repond` exige `no-cache` et refuse `reload` dans `sw.js`. Toutes
les commandes de la CI, workspace et site (tests du site passés avec et sans `pkg/`).

**Pistes suivantes.** La page d'accueil (HTML, sans `etag`) reste demandée deux fois à la
première visite. Avertissement de Chromium « preload … not used » pour le bundle quand le
worker est actif : à regarder côté Paquets. Couleur `--faint` et titres de 5 pages : choix
de l'utilisateur.

## 2026-11-15 — Itération 46 : calculateur ICM du laboratoire, saisies absurdes refusées clairement

**CI.** Celle de l'itération 45 (`7fb17da`) est verte et fusionnée dans master.

**Démarche.** Lecture, dans Chromium et avec le WebAssembly, de ce qu'affichent le
calculateur ICM et la grille des ranges pour quinze saisies limites chacun (l'itération 45
n'avait vérifié que l'absence d'erreur, pas le texte affiché).

**Trouvé** (`calculer_icm`, `site/src/lib.rs`) :
- paiements vides ou tous nuls : le tableau s'affichait avec « -0,00 » et « -50,0 pt » pour
  chaque joueur, sans dire qu'il manquait les paiements ;
- `1e308, 1e308` (ou `inf`, `nan`, acceptés par `parse::<f64>`) : une valeur de plus de
  300 chiffres, ou `NaN`, à l'écran ;
- messages qui numérotaient les joueurs à partir de 0 (« le joueur 0 a un tapis nul ») alors
  que l'écran les nomme J1, J2… ;
- un tapis illisible était recopié en entier dans le message, quelle que soit sa longueur.

**Fait.** Avant le calcul : un paiement doit être un nombre fini entre 0 et mille
milliards ; il en faut au moins un supérieur à zéro ; un tapis nul est signalé avec le
numéro affiché du joueur ; le texte recopié dans un message est limité à 20 caractères. Le
moteur (`stratyx-poker`) n'est pas modifié.

**Mesuré.** Les 13 saisies absurdes du test sont refusées avec un message de moins de
120 caractères. Observé avant, dans le navigateur : tableau faux pour des paiements vides
ou nuls, valeur de plus de 300 chiffres pour `1e308`, « joueur 0 » pour un tapis nul (`inf`
et `nan` n'ont pas été essayés avant le correctif). Les 6 saisies sensées du test (montants,
espaces, plus de prix que de joueurs) restent acceptées. Après, à 375 px : messages
affichés, pas de débordement, aucune exception.

**Vérifié.** Test `l_icm_refuse_les_saisies_absurdes` (`site/tests/iles.rs`) ; bundle
reconstruit et relu dans Chromium. Toutes les commandes de la CI, workspace et site.

**Pistes suivantes.** Même numérotation à partir de 0 dans un message du calculateur
d'équité (« la range du joueur 0 est vide… ») : le texte vient du moteur, partagé avec la
CLI ; à décider. La grille des ranges n'affiche aucun message pour une saisie invalide
(`ZZ+`) : à vérifier. Couleur `--faint`, titres de 5 pages : choix de l'utilisateur.

## 2026-11-15 — Itération 47 : calculateur d'équité, les joueurs gardent le numéro de leur champ

**CI.** Celle de l'itération 46 (`39e99f3`) est verte et fusionnée dans master.

**Écarté.** La piste « la grille des ranges n'affiche aucun message » était une erreur de
ma lecture (le script de l'itération 46 retirait le bloc avant de le lire) : relue, la
grille affiche bien un message pour chaque saisie invalide (15 essais). Rien à corriger.

**Trouvé** (`calculer_equite`, `site/src/lib.rs`). Les champs vides étaient retirés avant
le calcul, puis les joueurs renumérotés à la suite :
- champ « Joueur 1 » vide, champs 2 et 3 remplis : les résultats s'affichaient sous
  « Joueur 1 » et « Joueur 2 » ;
- les messages comptaient à partir de 0 et dans cette liste réduite (« la range du joueur 0
  est vide… ») ;
- une range illisible ne disait pas de quel joueur elle venait ;
- sans aucune main : « il faut entre 2 et 10 joueurs (reçu : 0) », alors que l'écran n'a
  que trois champs.

**Fait.** Chaque joueur garde le numéro de son champ, dans les lignes de résultat et dans
les messages (« joueur 2 : range invalide… », « la range du joueur 3 est vide… »,
« indiquez les mains d'au moins deux joueurs »). Le moteur (`stratyx-poker`) et ses messages,
partagés avec la CLI, ne sont pas modifiés : le site traduit les deux cas concernés.

**Mesuré.** Test `l_equite_nomme_les_joueurs_comme_a_l_ecran` : 2 dispositions avec un
champ vide et 6 saisies en erreur, toutes numérotées comme à l'écran, aucune « joueur 0 »
(avant : 2 dispositions mal étiquetées, 2 messages décalés, 2 sans numéro). Relu dans
Chromium avec le WebAssembly reconstruit : mêmes libellés, aucune exception.

**Vérifié.** Toutes les commandes de la CI, workspace et site.

**Pistes suivantes.** Couleur `--faint` (itération 42) et titres de 5 pages (itération 41) :
choix de l'utilisateur. Page d'accueil demandée deux fois à la première visite, préchargement
du bundle signalé inutilisé par Chromium (itération 45).

## 2026-11-15 — Itération 48 : page `/app` relue et temps de calcul mesurés (pas de changement)

**CI.** Celle de l'itération 47 (`663022d`) est verte et fusionnée dans master.

**Démarche.** Avec le WebAssembly construit ici, lecture du résultat d'une analyse sur
`/app` (exemple « Expresso Winamax », 375 px : texte complet et capture), puis mesure du
temps entre le clic et la dernière mise à jour de la page, dans Chromium.

**Constaté.** Rien à corriger :
- l'analyse s'affiche en entier sur mobile (note, cartes, déroulé, trois décisions avec
  leurs valeurs), sans débordement ni valeur aberrante ;
- temps de calcul, bundle `wasm-release`, serveur local :

| Action | Temps jusqu'à l'affichage |
|---|---:|
| analyse « Expresso Winamax » (3 essais) | 39 à 41 ms |
| analyse « Tournoi 6-max PokerStars » (3 essais) | 34 à 36 ms |
| analyse « Call de tapis léger » (3 essais) | 33 à 34 ms |
| équité « Coinflip » (calcul exact, 1 712 304 distributions) | 83 ms |
| équité « Tirage couleur » | 33 ms |
| équité « Multiway » (Monte-Carlo, 80 000 tirages) | 67 ms |

Une seule tâche de plus de 50 ms (52 ms, « Coinflip »). Mesures sur la machine du bac à
sable, un seul navigateur : un téléphone sera plus lent, non mesuré.

**Bilan des itérations 40 à 47 (site dans un vrai navigateur).** Les audits automatiques —
noms accessibles, identifiants, titres, contraste sur le vert, zones tactiles, focus au
clavier, débordement mobile, erreurs de console, saisies invalides des îlots — ne signalent
plus rien. Restent des choix qui reviennent à l'utilisateur : couleur `--faint` à 4,49:1
(itération 42), titres de section sur 5 pages (itération 41), numérotation à partir de 0
dans les messages du moteur côté CLI (itération 47).

**Pistes suivantes.** Page d'accueil demandée deux fois à la première visite et
préchargement du bundle signalé inutilisé quand le service worker est actif (itération 45,
côté Paquets). Hors du site : les crates squelettes (`coaching`, `transcription`, `video`,
`apps/*`) attendent une décision de fonctionnalités (itération 24).

## 2026-11-15 — Itération 49 : analyse d'une session entière 6 fois plus rapide dans le navigateur

**CI.** Celle de l'itération 48 (`a8ef919`) est verte et fusionnée dans master.

**Démarche.** Test de charge de `/app` dans Chromium avec le WebAssembly : 1 à 1 000 mains
collées (les trois exemples répétés, 837 Ko pour 1 000), lecture puis « Analyser toute la
session ». La lecture seule est rapide (143 ms pour 1 000 mains, aucune erreur).

**Trouvé** (`analyser_session`, `site/src/analyse.rs`). Pour laisser la page fluide,
l'analyse avançait par paquets de 4 mains séparés par une attente de 30 ms. Or une main se
calcule en 1,5 ms environ : sur 1 000 mains, 250 attentes de 30 ms, soit 7,5 s d'attente
pour 1,5 s de calcul.

**Fait.** Les paquets sont bornés par le temps et non par un nombre : le calcul continue
tant que 40 ms ne sont pas écoulées (au moins une main), puis rend la main au navigateur et
reprend aussitôt (`ensuite`, attente nulle). Hors du navigateur (tests), rien n'est découpé.

**Mesuré** (Chromium, bundle `wasm-release`, du clic au bilan affiché) :

| Mains | Avant | Après |
|---:|---:|---:|
| 10 | 154 ms | 105 ms |
| 50 | 495 ms | 164 ms |
| 200 | 1 812 ms | 385 ms |
| 1 000 | 8 987 ms | 1 472 ms (6,1 fois plus vite) |

L'en-tête du bilan affiché (260 premiers caractères : mains analysées, note, EV perdue,
décomptes des décisions) est identique avant et après pour chaque taille. Fluidité pendant l'analyse de 1 000 mains, après : 46 images
en 1,45 s, écart médian de 33 ms entre deux images, 50 ms au plus, 29 étapes de progression
affichées (« 31 / 1000 mains »…) ; aucune tâche de plus de 50 ms relevée, avant comme après.

**Vérifié.** Lint natif et WebAssembly, tests du site (l'analyse de session n'a pas de
test propre : son découpage dépend de l'horloge du navigateur, mesuré ci-dessus). Toutes
les commandes de la CI, workspace et site.

**Pistes suivantes.** Mesures sur la machine du bac à sable ; sur un téléphone, un morceau
de 40 ms contiendra moins de mains mais gardera la même fluidité. Choix laissés à
l'utilisateur : voir l'itération 48.

## 2026-11-15 — Itération 50 : production hors ligne (quota d'heures Eco d'Heroku épuisé), pas de changement de code

**CI.** Celle de l'itération 49 (`b32ee75`) est verte, fusionnée et déployée (version 62
d'Heroku, 09:44 UTC, « succeeded »).

**Constaté** (10:32 UTC). En vérifiant le déploiement, toutes les adresses de la
production (`/`, `/app`, `/sw.js`, `/health`, fichiers statiques) répondent **503** en
0,4 s, avec la page d'Heroku « Application's dyno hours exhausted ». Le dyno web de l'app
`stratyx` est de type Eco, à l'état `idle` ; l'app n'est pas en maintenance. Le serveur
Nuage DB (autre app) répond.

**Cause.** Le quota mensuel d'heures Eco du compte Heroku est épuisé : Heroku n'allume plus
aucun dyno Eco du compte jusqu'au mois suivant. Ce n'est pas un défaut du code ni du
déploiement.

**Non fait, volontairement.** Changer le type de dyno ou acheter des heures engage une
dépense : laissé à l'utilisateur (prévenu par notification). Rien n'a été modifié sur
Heroku ; la routine n'y a fait que des lectures.

**À savoir.** Un dyno Eco s'endort après 30 minutes sans trafic, et chaque déploiement le
redémarre : à un déploiement par heure (un par itération fusionnée, y compris celles qui ne
changent que ce journal), il reste éveillé environ la moitié du temps, ce qui consomme des
heures Eco en continu. Pistes, à décider par l'utilisateur : dyno Basic pour `stratyx`,
fréquence réduite de la routine, ou ne déployer (`heroku.yml`) que si `site/` ou une crate
dont il dépend a changé.

**Pistes suivantes.** Tant que la production est hors ligne, les vérifications « en
production » des itérations sont impossibles ; les mesures locales restent valables.

## 2026-11-15 — Itération 51 : pages HTML revalidées par `ETag` (304 sans corps)

**CI.** Celle de l'itération 50 (`62433fe`) est verte et fusionnée dans master. La
production reste hors ligne (itération 50) : mesures locales seulement.

**Trouvé** (piste de l'itération 45). Les pages HTML (et le manifeste, `sw.js`, la page
hors ligne) partaient sans `ETag` : quand le service worker revalide son pré-cache
(`cache: "no-cache"`) ou qu'un navigateur redemande une page déjà vue, le serveur ne peut
pas répondre `304` et renvoie la page entière. Les fichiers statiques avaient déjà le leur.
Le HTML d'une page est identique d'une requête à l'autre (vérifié : même empreinte pour
`/`, `/app`, `/contact`), donc une étiquette du corps est stable.

**Fait.** `app.middleware(middleware::etag())` (Vitesse, via Paquets), ajouté après
`compress()` : l'étiquette faible est calculée sur le corps non compressé, la réponse `304`
part sans corps, et une page `200` est compressée comme avant. Aucune dépendance ajoutée.

**Mesuré** (serveur local, 13 pages HTML du pré-cache lues dans `sw.js`, Brotli,
revalidation avec l'étiquette reçue) :

| | Avant | Après |
|---|---:|---:|
| octets au premier chargement | 92 493 | 92 493 |
| octets à la revalidation | 92 493 | 0 |
| réponses `304` | 0 / 13 | 13 / 13 |

**Vérifié.** Nouveau test `pages_revalidees_par_etag` (`/`, `/app`, `/contact`,
`/hors-ligne`, manifeste : étiquette faible stable, `304` à corps vide avec Brotli, `200`
pour une autre étiquette). Toutes les commandes de la CI, workspace et site (y compris
l'export statique et le lint WebAssembly).

**Pistes suivantes.** Avertissement « preload … not used » du bundle quand le worker est
actif (côté Paquets, itération 45). Choix laissés à l'utilisateur : itérations 48 et 50.

## 2026-11-15 — Itération 52 : documentation sans avertissement, vérifiée par la CI

**CI.** Celle de l'itération 51 (`a7e2f0a`) est verte et fusionnée dans master
(`adabf7a`). La production répond toujours `503` (quota Eco, itération 50).

**Écarté, mesuré.** Mettre les pages en cache côté serveur (`middleware::cache`) : en
`--release`, une page se rend en 0,7 à 2 ms (`/`, `/app`, `/laboratoire`,
`/fonctionnalites`, `/contact`, 6 requêtes chacune) ; l'`ETag` de l'itération 51 évite
déjà le transfert. Gain négligeable, statu quo.

**Trouvé.** `cargo doc --workspace --no-deps` signalait un lien ambigu dans
`strategy/src/lib.rs` : `` [`reference`] `` désigne à la fois le module et la fonction
réexportée ; rustdoc ne savait pas lequel ouvrir. Rien ne vérifiait la documentation dans
la CI. Le site (`site/`) n'a aucun avertissement.

**Fait.** Le lien vise le module (`mod@reference`). Nouvelle étape de la CI (Ubuntu
seulement) : `cargo doc --workspace --no-deps` avec `RUSTDOCFLAGS=-D warnings`, pour qu'un
lien cassé ou ambigu bloque la fusion.

**Mesuré.** Avertissements de rustdoc dans le workspace : 1 → 0. Coût de l'étape : 13 s
sans cache (4 cœurs), 4 s avec le cache de compilation.

**Vérifié.** La commande de la nouvelle étape passe ; toutes les autres commandes de la CI
aussi, workspace et site.

**Pistes suivantes.** Avertissement « preload … not used » du bundle (côté Paquets,
itération 45). Choix laissés à l'utilisateur : itérations 48 et 50.

## 2026-11-15 — Itération 53 : descriptions des pages, ni coupées ni trop courtes

**CI.** Celle de l'itération 52 (`3f4b7c1`) est verte, y compris la nouvelle étape
`cargo doc`, et fusionnée dans master (`e3c6ec8`). La production répond toujours `503`
(quota Eco, itération 50).

**Audit** (12 pages du plan du site, servies localement). Chaque page a un titre propre,
une adresse canonique et ses balises Open Graph. Quatre descriptions (`<meta
name="description">`) sortaient de la plage de 50 à 160 caractères que les moteurs de
recherche affichent sans les couper :

| Page | Avant | Après |
|---|---:|---:|
| `/` | 162 | 149 |
| `/app` | 199 | 157 |
| `/mentions-legales` | 49 | 87 |
| `/plan-du-site` | 33 | 132 |

**Fait.** Les quatre textes sont réécrits sans changer ce qu'ils disent (accueil :
« spécialisées » retiré ; `/app` : « ses IA » au lieu des deux noms, cités dans la page ;
mentions et plan : contenu de la page nommé).

**Vérifié.** Nouveau test `descriptions_des_pages` : chaque page du plan a une
description de 50 à 160 caractères (entités décodées), toutes différentes. Il échouait
avant le changement (`/`, 162 caractères). Toutes les commandes de la CI, workspace et
site, plus `cargo doc` (itération 52).

**Pistes suivantes.** Avertissement « preload … not used » du bundle (côté Paquets,
itération 45). Choix laissés à l'utilisateur : itérations 48 et 50.

## 2026-11-15 — Itération 54 : progression, une mesure `NaN` ne rend plus le profil illisible

**CI.** Celle de l'itération 53 (`d9e2e60`) est verte (3 min 40 s) et fusionnée dans
master (`a64f48f`). La production répond toujours `503` (quota Eco, itération 50).

**Audits sans suite** (pour ne pas les refaire) : site servi localement, aucune balise
`<img>` (dessins en SVG), donc ni dimension ni `alt` manquants ; les 20 adresses internes
des pages (liens, scripts, styles, formulaires) répondent `200`, sauf `/liste-attente`
(`405` à un `GET`, normal : formulaire en `POST`) ; ancres de `/`, `/app`, `/laboratoire`
présentes. Documentation : plus aucune mention de PostgreSQL, Neon ou redb hors historique.

**Trouvé** (`Database::add_progress`, `database/src/users.rs`). La progression d'un
utilisateur est un tableau JSON. `serde_json` écrit un `f64` non fini (`NaN`, infini) sous
la forme `null`, qu'il ne sait pas relire en `f64`. Reproduit : l'ajout d'une mesure `NaN`
réussit, puis `progress("u1")` **et tout ajout suivant** échouent (« invalid type: null,
expected f64 ») ; la progression entière de l'utilisateur devient inaccessible.

**Fait.** `add_progress` refuse une valeur non finie avant d'écrire, avec une nouvelle
erreur `DatabaseError::InvalidValue` qui nomme la mesure. `details` ne risque rien
(`serde_json::Value` ne contient jamais de nombre non fini), et aucun autre type enregistré
par la base n'a de champ flottant.

**Mesuré.** Avant : 1 mesure `NaN` → 0 lecture possible, 0 ajout possible. Après : `NaN`,
`+∞` et `−∞` refusés, les mesures déjà enregistrées restent lisibles et les ajouts
suivants passent.

**Vérifié.** Nouveau test `progression_valeur_non_finie_refusee` (`database/tests/store.rs`).
Toutes les commandes de la CI, workspace (avec et sans fonctionnalités par défaut pour
`stratyx-database`) et site, plus `cargo doc`.

**Pistes suivantes.** Avertissement « preload … not used » du bundle (côté Paquets,
itération 45). Choix laissés à l'utilisateur : itérations 48 et 50.

## 2026-11-15 — Itération 55 : entraînement, un modèle illisible n'écrase plus le précédent

**CI.** Celle de l'itération 54 (`4a2a642`) est verte et fusionnée dans master
(`055a4ab`). La production répond toujours `503` (quota Eco, itération 50).

**Trouvé** (même famille que l'itération 54, `write_bundle`, `ai/training/src/lib.rs`).
L'entraînement écrit les modèles des IA en JSON (`ai/*/models/*.json`, 125 et 219 Ko),
sans les relire. Un poids ou une métrique non fini (`NaN` après un entraînement qui
diverge) s'écrit `null`, que `ModelBundle::from_json` refuse. Or les deux IA chargent ce
fichier à la compilation (`include_str!`) puis `expect(...)` : le fichier commité aurait
été remplacé par un fichier illisible, et l'analyse (site, `/app`, CLI) aurait paniqué au
premier calcul. L'écriture directe (`fs::write`) pouvait aussi laisser un fichier tronqué
en cas d'interruption.

**Fait.** `write_bundle` relit le JSON produit avant d'écrire ; s'il est illisible, il
renvoie une erreur `InvalidData` qui nomme le fichier et **garde le fichier précédent**.
L'écriture passe par `….json.tmp`, renommé ensuite (remplacement atomique).

**Mesuré.** Contre-épreuve : la vérification retirée, le nouveau test échoue (le modèle
abîmé est écrit par-dessus l'ancien) ; rétablie, il passe. Coût : le test entier (deux
écritures, deux relectures du modèle Expresso) prend moins de 10 ms, négligeable devant un
entraînement.

**Vérifié.** Nouveau test `un_modele_illisible_n_ecrase_pas_le_precedent` (modèle Expresso
réel, une métrique mise à `NaN` : erreur, fichier inchangé, pas de fichier temporaire
restant). Toutes les commandes de la CI, workspace et site, plus `cargo doc`.

**Pistes suivantes.** Avertissement « preload … not used » du bundle (côté Paquets,
itération 45). Choix laissés à l'utilisateur : itérations 48 et 50.

## 2026-11-15 — Itération 56 : revalidation de la base locale 6 fois plus rapide

**CI.** Celle de l'itération 55 (`7aa903d`) est verte et fusionnée dans master
(`cbd187c`). La production répond toujours `503` (quota Eco, itération 50).

**Audit sans suite.** Serveur du site : aucun `unwrap`/`expect` hors des tests sur le
chemin des requêtes.

**Trouvé** (`Database::revalidate`, `database/src/store.rs`, avec `mesure_base -- 5000`).
Revalider 5 000 mains sans aucun changement prenait 1,5 s, plus que les écrire : chaque
main était supprimée puis réinsérée avec tous ses index (ligne de recherche, données,
empreinte, joueurs), même quand le moteur rendait exactement le même résultat.

**Fait.** Avant de réécrire une main, `hand_is_current` vérifie qu'elle est au format
d'enregistrement actuel, que son résumé recalculé est identique, au JSON près, à celui de
sa ligne de recherche (tous les index en dérivent, avec la main elle-même, inchangée), et
que son empreinte figure dans la table des empreintes (une réécriture l'y remettrait).
Sinon, elle est réécrite comme avant. Seule différence visible : une main inchangée garde
sa date d'enregistrement (`stored_at_ms`).

**Mesuré** (`mesure_base -- 5000`, release, 3 exécutions de chaque version) :

| | Avant | Après |
|---|---:|---:|
| revalidation, 0 main changée | 1,46 à 1,77 s | 0,25 à 0,29 s (≈ 6 fois plus vite) |
| écriture d'une main (témoin) | 190 à 240 µs | 206 à 236 µs |

**Vérifié.** Le test `revalidation_applies_a_new_review_threshold` contrôle en plus que
seule la main dont le statut change est réécrite (sa date avance, celle de l'autre ne
bouge pas) ; il échoue sur l'ancien code. Toutes les commandes de la CI, workspace (avec et
sans fonctionnalités par défaut pour `stratyx-database`) et site, plus `cargo doc`.

**Pistes suivantes.** Avertissement « preload … not used » du bundle (côté Paquets,
itération 45). Choix laissés à l'utilisateur : itérations 48 et 50.

## 2026-11-15 — Itération 57 : recherche par joueur, mesurée sur un cas réaliste (pas de changement de la recherche)

**CI.** Celle de l'itération 56 (`d8d7097`) est verte et fusionnée dans master
(`f0be5c9`). La production répond toujours `503` (quota Eco, itération 50).

**Piste.** `mesure_base -- 5000` donnait 17 ms par page pour la recherche par joueur,
plus que la lecture de toute la table (3,6 ms). Or le joueur choisi par l'exemple était
assis à **toutes** les mains : c'est le pire cas, pas l'usage courant (un adversaire
régulier).

**Mesuré** (5 000 mains, release, requête SQL seule, 20 répétitions, test temporaire
retiré ensuite) :

| Requête | Joueur à 5 000 mains | à 250 mains | à 1 main |
|---|---:|---:|---:|
| `id IN (SELECT main FROM main_joueur WHERE joueur = ?)` (actuelle) | 16,3 ms | 0,67 ms | 0,010 ms |
| jointure `main JOIN main_joueur` | 12,5 ms | 4,0 ms | 4,5 ms |
| index `main_joueur` seul (borne basse) | 6,3 ms | 0,38 ms | 0,003 ms |
| toute la table `main` (témoin) | 3,0 ms | 3,0 ms | 3,0 ms |

**Écarté.** La jointure ne gagne qu'au pire cas et est 6 à 450 fois plus lente quand le
joueur est rare : la sous-requête actuelle est le bon choix. Coût de la sous-requête
proportionnel aux mains trouvées (≈ 3 µs par main), sans gain simple à attendre.

**Fait.** `mesure_base` mesure aussi le cas réaliste : un adversaire assis à une main sur
20 (« joueur (1 main sur 20) » : 0,84 ms par page), à côté du pire cas renommé « joueur
(toutes les mains) » (16,8 ms). Les autres mesures de l'exemple ne bougent pas
(revalidation 0,18 s, écriture 190 µs par main sur cette exécution).

**Vérifié.** Toutes les commandes de la CI, workspace et site, plus `cargo doc`.

**Pistes suivantes.** Avertissement « preload … not used » du bundle (côté Paquets,
itération 45). Choix laissés à l'utilisateur : itérations 48 et 50.

## 2026-11-15 — Itération 58 : recherche par salle (filtres non indexés) 2,3 fois plus rapide

**CI.** Celle de l'itération 57 (`9c8031e`) est verte et fusionnée dans master
(`6eda933`). La production répond toujours `503` (quota Eco, itération 50).

**Trouvé** (`Database::search`, `database/src/query.rs`). Trois filtres ne sont pas des
colonnes indexées : salle, résultat du héros, abattage. Avec l'un d'eux, la recherche
désérialisait **chaque résumé candidat en entier** (joueurs, statut et ses détails), puis
triait des résumés complets pour n'en garder qu'une page de 20.

**Écarté.** Indexer la salle : il faudrait un schéma 2, et une base migrée ne s'ouvrirait
plus avec une version antérieure du logiciel. Trop lourd pour ce gain.

**Fait.** `search_filtered` : la base applique toujours les conditions indexées ; de chaque
candidat, seuls trois champs du résumé (`site`, `hero_net`, `showdown`, sans copie de la
salle) sont lus pour les filtres non indexés ; le tri et la page se font sur l'identifiant
et la date, et seuls les 20 résumés de la page sont relus — comme la recherche indexée
(`page_of`, partagé). Les trois filtres sont écrits une seule fois
(`HandQuery::matches_unindexed`, utilisé aussi par `matches`). L'ancien parcours reste,
pour les tests seulement, comme référence.

**Mesuré** (`mesure_base -- 5000`, release, binaires avant/après lancés en alternance,
5 fois chacun ; toutes les mains sont PokerStars, donc la salle ne filtre rien : pire cas) :

| Recherche, page de 20 | Avant | Après |
|---|---:|---:|
| par salle (filtre non indexé) | 13,6 à 20,4 ms | 6,4 à 7,8 ms (≈ 2,3 fois plus vite) |
| sans filtre (témoin) | 3,7 à 5,9 ms | 3,4 à 5,2 ms |

**Vérifié.** `search_in_base_matches_the_scan` compare en plus, pour les 72 requêtes
indexées de ce test combinées à 7 filtres non indexés (salle présente ou absente, trois
résultats du héros, abattage avec et sans, seul ou avec la salle), la nouvelle recherche au
parcours complet : résultats identiques, et non vides. Toutes les commandes de la CI,
workspace et site, plus `cargo doc`.

**Pistes suivantes.** Avertissement « preload … not used » du bundle (côté Paquets,
itération 45). Choix laissés à l'utilisateur : itérations 48 et 50.

## 2026-11-15 — Itération 59 : profil de la base locale (pas de piste côté STRATYX), documentation de la recherche à jour

**CI.** Celle de l'itération 58 (`a0e805c`) est verte et fusionnée dans master
(`982725b`). La production répond toujours `503` (quota Eco, itération 50).

**Mesuré** (callgrind sur `mesure_base -- 600`, release avec tables de lignes ; 1,25 G
instructions), pour ne pas le refaire :

| Part du total | Où |
|---:|---|
| 51 % | exécution SQL de Nuage DB (dont 25 % à filtrer les lignes, 19 % à insérer) |
| 13 % | allocations (`malloc`, `free`, `realloc`), surtout dans Nuage |
| 8 % | relecture JSON des mains (`HandRecord`), nécessaire au rejeu de la revalidation |
| 7 % | SHA-256 des empreintes (déduplication) |

Aucune fonction de STRATYX ne coûte par elle-même : le reste du temps est dans Nuage DB
(dépôt à part, optimisé à la demande de l'utilisateur à l'itération 29) ou dans un travail
nécessaire. Pas de piste rentable côté STRATYX pour la base. Écarté aussi : les divisions
des statistiques (`analytics`) qui pourraient donner `NaN` sont déjà gardées (`max(1)`,
`then`).

**Fait.** La documentation de `Database::search` décrivait encore l'ancien parcours pour
les filtres non indexés (« chaque résumé candidat est relu ») : elle décrit la lecture des
trois champs de l'itération 58. Dans `mesure_base`, la mesure « salle (parcours) »
s'appelle « salle (filtre non indexé) » (6,5 ms par page, 5 000 mains).

**Vérifié.** Toutes les commandes de la CI, workspace et site, plus `cargo doc`.

**Pistes suivantes.** Avertissement « preload … not used » du bundle (côté Paquets,
itération 45). Choix laissés à l'utilisateur : itérations 48 et 50.

## 2026-11-15 — Itération 60 : site et solveurs hors ligne mesurés, rien de rentable (README à jour)

**CI.** Celle de l'itération 59 (`af32f31`) est verte et fusionnée dans master
(`7bddd50`). La production répond toujours `503` (quota Eco, itération 50).

**Audits sans suite** (pour ne pas les refaire) :
- Site servi localement : `robots.txt` annonce le plan du site ; le plan liste les 12 pages ;
  `/nulle-part` répond `404`, et `/merci`, `/hors-ligne` et la page 404 portent
  `noindex,follow`. L'adresse canonique vient de `SITE_URL`, que le déploiement remplit
  (`app.json`).
- Tests du workspace, une fois compilés : 17,6 s (le README annonçait ≈ 25 s ; corrigé en
  ≈ 20 s). Le binaire de tests le plus long est `stratyx-strategy` (6,7 s), déjà limité par
  le calcul (≈ 24 s de calcul sur 4 fils) : paralléliser ses tests n'y gagnerait rien.
- Solveur push/fold à trois (callgrind, `three_max_chip_ev_equilibrium` en release, un fil,
  56 G instructions) : 30 % pour la table de test des confrontations à trois (tirages
  aléatoires compris), 18 % dans `ThreeWayEquities::lookup_idx`, appelée trois fois par
  triplet de classes (une fois par joueur) à la construction du jeu. Ne la calculer qu'une
  fois gagnerait ≈ 12 %, mais `solve_three_max` et le solveur de river ne servent qu'hors
  ligne (exemples `pushfold_charts`, `river_convergence`) et aux tests : écarté.

**Fait.** README : durée des tests mise à jour.

**Vérifié.** Toutes les commandes de la CI, workspace et site, plus `cargo doc`.

**Pistes suivantes.** Avertissement « preload … not used » du bundle (côté Paquets,
itération 45). Choix laissés à l'utilisateur : itérations 48 et 50 (dont la fréquence de
la routine : les chemins chauds côté utilisateur sont tous mesurés et optimisés).
