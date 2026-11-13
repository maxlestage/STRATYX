# Entraînement des IA

Rapport écrit par `stratyx-train` (`ai/training`). Les modèles sont embarqués par `ai/expresso_3max` et `ai/tournament_6max`, et utilisés par l'analyse des mains (`analytics`), l'outil `stratyx analyse` et la page `/app` du site.

Relancer : `cargo run --release -p stratyx-ai-training --bin stratyx-train` (ajoutez `--archives <dossier>` pour vos propres historiques).

## IA Expresso 3-Max

Fichier : `ai/expresso_3max/models/expresso-3max.json` · entraîné le 2026-11-13 · graine `0x5354524154595821`.

**Données.** 40000 mains simulées (auto-jeu, six profils de joueurs) et 0 mains d'archives réelles (0 fichiers lus, 0 doublons et 0 mains invalides écartés). 117813 décisions étiquetées.

**Équilibre push/fold** (3 joueurs, ante 0 BB, 28 itérations de jeu fictif, 2000 tirages par position et par classe à chaque itération) :

| Tapis | Tapis BTN | Tapis SB | BB suit le tapis de la SB |
|---|---|---|---|
| 3 BB | 48,4 % | 82,0 % | 89,6 % |
| 5 BB | 42,5 % | 73,4 % | 67,3 % |
| 8 BB | 34,9 % | 60,5 % | 48,4 % |
| 10 BB | 31,6 % | 54,4 % | 42,0 % |
| 12 BB | 29,8 % | 49,8 % | 36,7 % |
| 16 BB | 26,5 % | 42,4 % | 30,4 % |
| 20 BB | 22,1 % | 37,5 % | 25,6 % |

Petite blinde à 10 BB, mains poussées (≥ 50 %) : 97 classes sur 169.

**Modèles** (mesurés sur environ 20 % des mains, jamais vues à l'entraînement) :

| Modèle | Paramètres | Entraînement | Validation | Précision | Perte (log) | Classe majoritaire |
|---|---|---|---|---|---|---|
| `population-softmax` | 104 | 94117 | 23696 | 68,4 % | 0.765 | 42,4 % |
| `decision-mlp` | 5476 | 94117 | 23696 | 90,7 % | 0.227 | 32,5 % |
| `decision-softmax` | 204 | 94117 | 23696 | 85,0 % | 0.403 | 32,5 % |

Étiquettes de la population (actions jouées) : se coucher 42,3 % · parler / suivre 32,3 % · miser / relancer 11,8 % · faire tapis 13,6 %.

Étiquettes de la décision (référence) : se coucher 30,4 % · parler / suivre 17,0 % · miser / relancer 19,9 % · faire tapis 32,6 %.


## IA Tournois 6-Max

Fichier : `ai/tournament_6max/models/tournois-6max.json` · entraîné le 2026-11-13 · graine `0x5354524154595821`.

**Données.** 40000 mains simulées (auto-jeu, six profils de joueurs) et 0 mains d'archives réelles (0 fichiers lus, 0 doublons et 0 mains invalides écartés). 290068 décisions étiquetées.

**Équilibre push/fold** (6 joueurs, ante 0.125 BB, 28 itérations de jeu fictif, 2000 tirages par position et par classe à chaque itération) :

| Tapis | Tapis UTG | Tapis HJ | Tapis CO | Tapis BTN | Tapis SB | BB suit le tapis de la SB |
|---|---|---|---|---|---|---|
| 3 BB | 40,6 % | 44,3 % | 49,5 % | 58,4 % | 88,5 % | 92,7 % |
| 5 BB | 32,3 % | 37,2 % | 43,3 % | 50,7 % | 80,5 % | 73,1 % |
| 8 BB | 25,0 % | 28,7 % | 34,3 % | 43,0 % | 70,7 % | 57,7 % |
| 10 BB | 21,4 % | 26,5 % | 31,4 % | 39,9 % | 65,8 % | 50,7 % |
| 12 BB | 18,9 % | 23,7 % | 28,3 % | 36,3 % | 61,7 % | 45,9 % |
| 20 BB | 11,9 % | 15,5 % | 21,6 % | 29,0 % | 48,9 % | 35,0 % |

Petite blinde à 10 BB, mains poussées (≥ 50 %) : 117 classes sur 169.

**Modèles** (mesurés sur environ 20 % des mains, jamais vues à l'entraînement) :

| Modèle | Paramètres | Entraînement | Validation | Précision | Perte (log) | Classe majoritaire |
|---|---|---|---|---|---|---|
| `population-softmax` | 104 | 231630 | 58438 | 73,2 % | 0.683 | 55,3 % |
| `decision-mlp` | 5476 | 231630 | 58438 | 89,4 % | 0.251 | 36,3 % |
| `decision-softmax` | 204 | 231630 | 58438 | 76,2 % | 0.605 | 36,3 % |

Étiquettes de la population (actions jouées) : se coucher 55,8 % · parler / suivre 26,7 % · miser / relancer 7,5 % · faire tapis 9,9 %.

Étiquettes de la décision (référence) : se coucher 36,3 % · parler / suivre 13,6 % · miser / relancer 20,3 % · faire tapis 29,7 %.

