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
