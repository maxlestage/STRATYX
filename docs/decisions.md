# Registre des décisions — Stratyx

Chaque décision est numérotée, datée et justifiée. Une décision ne se modifie pas : on en
ajoute une nouvelle qui la remplace (« remplace Dx »).

- **D1 à D8** : décisions de cadrage du cahier des charges, **validées le 2026-11-04**
  (D8 remplace D4).
- **T1 à T14** : décisions techniques qui en découlent. « Acceptée » : déjà appliquée dans
  le code. « Proposée » : à confirmer au début de la phase indiquée.

---

## Décisions de cadrage (validées)

### D1 — Politique Rust : Rust partout, API natives des systèmes autorisées

**Choix.** Tout le code applicatif est écrit en Rust. Seules exceptions : les **API des systèmes
d'exploitation** et les **pilotes** (graphiques notamment), appelés depuis Rust.

**Pourquoi.** L'option « aucun composant natif, y compris les interfaces système » est
techniquement irréalisable. Capturer l'écran, ouvrir une fenêtre, utiliser le GPU ou lire un
fichier passe forcément par l'OS : Win32/DXGI, ScreenCaptureKit/CoreGraphics, X11/Wayland/PipeWire,
MediaProjection (Android), ReplayKit (iOS). La bibliothèque standard de Rust elle-même s'appuie
sur la libc ou sur ntdll.

**Périmètre de l'exception.**

| Autorisé | Interdit |
|---|---|
| Liaisons Rust vers les API système (`windows`, `objc2`, `x11rb`, `zbus`…) | Compiler du C/C++ tiers (`cc`, `cmake`) |
| Pilotes GPU via `wgpu` (Vulkan, Metal, DirectX 12) | Embarquer OpenCV, FFmpeg, SQLite, OpenSSL, libtorch, ONNX Runtime, Skia, Tesseract |
| Bibliothèques fournies par l'OS, au cas par cas (ex. PipeWire sous Wayland) | Python, Swift ou Kotlin dans le produit |

**Application.** `deny.toml` + `cargo deny check` en intégration continue (voir T2).

### D2 — Plateformes : ordinateur et mobile, priorité à l'ordinateur

**Choix.** Windows, macOS et Linux en premier : capture, retranscription, analyse, coaching.
iOS et Android en phase 7, pour la revue des mains, les statistiques et le coaching.

**Pourquoi.** La capture d'écran d'une autre application est très contrainte sur mobile (extension
ReplayKit sur iOS, MediaProjection via JNI sur Android), et plusieurs salles l'interdisent. La
valeur principale (retranscription et analyse) se joue sur ordinateur. Le moteur commun en Rust
sera réutilisé tel quel sur mobile.

### D3 — Calcul IA : CPU + GPU selon le matériel

**Choix.** Les modèles tournent sur CPU partout. Le GPU est utilisé automatiquement s'il est
disponible.

**Pourquoi.** Un ordinateur portable sans GPU dédié doit pouvoir analyser une session.
L'entraînement, lui, profite du GPU. Le framework **Burn** offre un backend CPU en Rust pur
(`ndarray`) et un backend GPU (`wgpu`), conforme à D1 puisqu'il passe par Vulkan, Metal ou
DirectX 12. L'audit du 2026-11-04 confirme : ni Burn (`ndarray` + `wgpu`) ni Candle (CPU) ne
compilent de C. CUDA est écarté : ses noyaux sont compilés par `nvcc`.

### D4 — Variantes : Texas Hold'em No-Limit uniquement au lancement

**Remplacée par D8 (2026-11-04).**

**Choix.** Les deux formats visés (Expresso 3-Max et tournois 6-Max) se jouent en NLHE.

**Pourquoi.** Concentrer l'effort sur la qualité : reconnaissance, règles, IA. Les types du moteur
(`Street`, `Action`, `HandState`) restent isolés dans `poker/`. Une variante (Omaha) fera l'objet
d'une décision dédiée.

### D5 — Entraînement IA : archives + simulations

**Choix.** Les **simulations** (auto-jeu, solveurs CFR, équilibres push/fold) fournissent la
stratégie de référence, la GTO. Les **archives** (mains propres de l'utilisateur et données sous
licence) apportent les tendances réelles de la population : jeu exploitant et profils d'adversaires.

**Pourquoi.** Les simulations ne posent aucun problème de licence et donnent une référence
indépendante du résultat des mains. Les archives seules reproduiraient les erreurs des joueurs.
Toute archive est **validée par rejeu** dans le moteur de règles (`HandRecord::replay`), puis
nettoyée et dédupliquée, avant tout entraînement.

### D6 — Utilisation : analyse après partie et entraînement, en modes distincts

**Choix.** Deux modes :

- **Analyse** : capture ou import vidéo, retranscription, revue, analyse stratégique, statistiques.
- **Entraînement** : exercices, quiz, simulations, parcours personnalisé.

**Garde-fou.** Pendant une capture en cours, aucune information stratégique n'est affichée. Une
session n'est analysable qu'une fois close. C'est une règle d'architecture, pas une simple option
d'interface. Elle découle de l'exclusion de l'assistance en temps réel, interdite par les salles
(cahier des charges, section 11).

### D7 — Ambition : plateforme complète avec API et services professionnels

**Choix.** La cible est l'écosystème complet : logiciel, serveur, API, services. La livraison
reste **incrémentale** : le logiciel de bureau professionnel (phases 1 à 6) précède le serveur et
l'API publique (phase 7).

**Pourquoi.** C'est la vision exprimée. L'architecture en crates (moteur commun, `apps/server`)
la prépare dès maintenant sans retarder le premier livrable utile.

### D8 — Couverture complète du Texas Hold'em (remplace D4)

**Choix.** La plateforme couvre le Texas Hold'em sous toutes ses formes :

| Dimension | Couverture |
|---|---|
| Structures de mise | No-limit, pot-limit, limite fixe |
| Tailles de table | 2 à 10 joueurs (tête-à-tête, 3-max, 6-max, 8-max, table pleine 9 et 10) |
| Types de partie | Cash game, tournoi multi-tables, sit & go, Expresso |

Les autres variantes (Omaha, Short Deck, Stud…) restent hors périmètre.

**Rôle des IA.** Les deux IA restent spécialisées : Expresso 3-Max (NL) et Tournois 6-Max
(NL). Tous les autres formats bénéficient de la capture, de la retranscription, des
statistiques et de l'analyse par le moteur commun : équité, EV, cotes, ICM en tournoi.
`GameFormat::specialist()` indique quelle IA est compétente pour une main.

**Règles appliquées.** Pot-limit : relance maximale égale au pot après le call. Limite fixe :
petite mise au préflop et au flop, grosse mise (double) au turn et à la river ; une mise et
trois relances au plus par tour. Le plafond est levé en tête-à-tête en cash game, et en tournoi
seulement quand deux joueurs sont servis. Un tapis d'au moins une demi-mise compte comme une
mise complète ; en dessous, il ne rouvre pas les enchères et peut être complété (Robert's Rules
of Poker, règles TDA).

**Hors moteur pour l'instant.** Straddles, blindes mortes et blindes postées à l'arrivée en cash
game : à ajouter avec l'import des historiques des salles (phase 3).

---

## Décisions techniques

### T1 — Organisation : workspace Cargo, édition 2024 — *acceptée*

Un crate par module du cahier des charges (`shared`, `poker`, `capture`, `video`, `vision`,
`ai/*`, `strategy`, `analytics`, `coaching`, `database`, `security`, `apps/*`, `tools/cli`).
`unsafe_code = "forbid"` dans tout le workspace. Seul le crate `capture` pourra déroger, pour les
appels système, avec une justification écrite.

### T2 — Audit des dépendances : cargo-deny bloquant — *acceptée*

`deny.toml` interdit les crates qui compilent du C/C++ (`cc`, `cmake`, `bindgen`) et celles qui
embarquent une bibliothèque native : OpenCV, FFmpeg, SQLite, OpenSSL, `ring`, `aws-lc`, libtorch,
ONNX Runtime, Skia, Tesseract, zstd/zlib en C, allocateurs en C. Les licences doivent être
permissives. Les vulnérabilités connues (RustSec) bloquent la CI. **Tout script de build doit
être approuvé nominativement** après relecture. Les 14 scripts actuels (serde, rayon, libc…) ne
font que détecter la version de `rustc`.

Constat d'audit du 2026-11-04 sur la pile candidate du cahier des charges :

| Crate candidate | Tire du code natif ? | Conséquence |
|---|---|---|
| Burn (`ndarray`, `wgpu`) | Non | Retenu (D3) |
| Candle (CPU) | Non | Alternative possible |
| Tokio, Axum, Serde, Rayon | Non | Retenus |
| `rustls` (par défaut) | **Oui** : `aws-lc-sys` (C) | Fournisseur crypto à remplacer (T4) |
| `sqlx` avec SQLite | **Oui** : `libsqlite3-sys` (C) | SQLite écarté (T3) |
| `sqlx` avec PostgreSQL | Non (pilote en Rust) | Retenu côté serveur |
| Slint (par défaut) | **Oui** : `skia-bindings` (C++) | Rendu femtovg ou logiciel (T5) |
| `xcap` (capture) | **Oui** sous Linux : `pipewire-sys`, `libspa-sys` | Capture écrite sur mesure (T6) |

### T3 — Bases de données : PostgreSQL (serveur) et redb (local) — *proposée, phase 3*

- Serveur : PostgreSQL via SQLx. Le pilote est en Rust pur. PostgreSQL est un service externe,
  pas une bibliothèque liée au binaire.
- Poste de travail et mobile : **redb**, base embarquée transactionnelle en Rust pur. SQLite est
  exclu par D1. *Remplacé par Nuage DB embarqué (T14) ; redb ne sert plus qu'à importer une
  ancienne base.*
- Les quatre bases logiques (Expresso, Tournois, Stratégie, Utilisateurs) sont des espaces
  séparés. Le format pivot est `HandRecord` (JSON versionné).

### T4 — Chiffrement et TLS : RustCrypto — *proposée, phase 8*

Chiffrement au repos (AES-GCM ou ChaCha20-Poly1305), dérivation de clés (Argon2) et signatures
(Ed25519) avec les crates RustCrypto. Pour le TLS : `rustls` avec un fournisseur RustCrypto, à la
place d'`aws-lc-rs` (C) ou de `ring` (C et assembleur). La maturité de ce fournisseur reste à
vérifier avant la mise en production du serveur.

### T5 — Interfaces : Slint pour le produit, egui pour les outils — *proposée, phase 7*

Slint couvre le bureau et le mobile avec un seul langage d'interface, rendu femtovg ou logiciel
(Skia exclu). egui sert aux outils internes : annotation des images pour la vision, visualiseurs
de débogage. **Point à trancher :** la licence de Slint (gratuite avec mention « Made with Slint »,
ou commerciale).

### T6 — Capture et vidéo sans FFmpeg — *proposée, phase 2*

- **Capture** : crate `capture` écrit sur mesure au-dessus des API système. Windows Graphics
  Capture, ScreenCaptureKit, X11 via `x11rb` (protocole en Rust pur), Wayland via le portail XDG
  (`zbus`, Rust pur) et PipeWire (bibliothèque système, exception à confirmer).
- **Enregistrements Stratyx** : format interne adapté aux tables de poker. Images conservées
  seulement quand la table change, compression sans perte en Rust (QOI/PNG + `lz4_flex`), ou AV1
  via `rav1e` (assembleur désactivé en mode strict).
- **Import de vidéos externes** : démultiplexage MP4 en Rust. Décodage H.264/HEVC par les API
  système (Media Foundation, VideoToolbox, VA-API). AV1 via `rav1d`.

### T7 — Vision : modèles légers en Burn, validés par les règles — *proposée, phase 2*

Pas d'OpenCV ni de Tesseract. Chaque salle a son gabarit de table (zones des cartes, montants et
boutons). La reconnaissance combine correspondance de motifs et petits réseaux convolutifs (Burn)
pour les cartes, les chiffres et les actions. Chaque lecture produit un `Observed<T>` avec sa
confiance. La cohérence temporelle est vérifiée par le moteur de règles : une transition illégale
est signalée, jamais corrigée en silence. Une carte non visible reste inconnue.

### T8 — Reproductibilité des calculs — *acceptée*

Les simulations utilisent ChaCha8 avec une graine explicite, un flux par bloc de tirages. À graine
égale, le résultat est identique quel que soit le nombre de cœurs : les évaluations des IA sont
comparables d'une version à l'autre.

### T9 — Conventions de code — *acceptée*

Identifiants en anglais, conformément à l'écosystème Rust. Documentation, messages d'erreur et
textes affichés en français. Montants en entiers (`Chips`), jamais en flottants. Toute donnée
issue de la vision porte sa provenance et sa confiance.

### T10 — Premiers modèles des IA en Rust pur, sans Burn — *acceptée*

**Choix.** Les modèles de la première version des deux IA (régression softmax et perceptron
multicouche de quelques milliers de paramètres) sont écrits, entraînés et exécutés en Rust
pur (`strategy::model`, `ai/training`), sur CPU. Burn reste retenu (D3) pour les modèles
lourds : vision (phase 2) et modèles postflop profonds.

**Pourquoi.** Ces modèles doivent tourner partout, y compris dans le navigateur (page `/app`
du site, en WebAssembly, hors ligne) : quelques milliers de multiplications ne justifient ni
la taille ni le temps de compilation de Burn. Le format des poids est un JSON simple et
versionné, embarqué dans les crates `ai/expresso_3max` et `ai/tournament_6max`. Aucune
dépendance nouvelle : l'audit `cargo deny` est inchangé.

### T11 — Données d'entraînement : simulations d'abord, archives réelles en option — *acceptée*

**Choix.** Faute d'archives sous licence vérifiée, les modèles sont entraînés sur des
**archives simulées** : auto-jeu dans le moteur de règles entre six profils de joueurs (serré,
régulier, large agressif, calling station, maniaque, équilibre), sit & go complets à 3 pour
l'Expresso, mains de table de tournoi 6-max (tapis de 4 à 80 BB, ante de 12,5 %). Les mains
simulées portent la provenance `Source::Simulation` et ne sont jamais mélangées aux mains
d'un utilisateur. Les archives réelles de l'utilisateur (historiques texte PokerStars et
Winamax, JSON `HandRecord`) s'ajoutent avec `stratyx-train --archives <dossier>` : nettoyées,
dédupliquées, validées par rejeu (D5).

**Étiquettes.** Le modèle de population apprend l'action jouée (tendances réelles des joueurs,
jeu exploitant). Le modèle de décision apprend la **décision de référence** : équilibre
push/fold (jeu fictif, chip EV) pour les tapis courts au préflop, sinon une référence EV
(équité contre les ranges déduites des actions, cotes du pot, fold equity tirée du modèle de
population, réalisation de l'équité selon la position, valeur des tours suivants). Elle est
indépendante du résultat des mains (cahier des charges, section 6). Rapport :
[`entrainement.md`](entrainement.md).

### T12 — Historiques partagés : ils restent sur l'appareil — *acceptée*

**Choix.** La page `/app` analyse les mains dans le navigateur (WebAssembly). Un historique
partagé vers l'application installée (`share_target` du manifeste, `/app?texte=…`) ou passé
dans un lien (`/app#texte=…`) n'est jamais envoyé au serveur : le service worker sert la page
`/app` sans son adresse complète, et l'îlot lit le texte localement.

**Pourquoi.** Les pseudonymes des adversaires sont des données personnelles (point de
vigilance 3) ; le pare-feu applicatif du serveur refuserait aussi ces adresses. Le site
reste utilisable hors ligne.

### T13 — Capture et vision dans le navigateur, après la partie — *acceptée*

**Choix.** La page `/app` filme l'écran (`getDisplayMedia`) ou la caméra, ou lit une vidéo et
des captures d'écran choisies sur l'appareil. Pendant l'enregistrement, **rien n'est reconnu à
l'écran et aucun conseil n'est affiché** (D6) : seules des images clés (quand la table change)
sont gardées en mémoire. À l'arrêt, le crate `vision` (Rust pur, en WebAssembly) détecte les
cartes face visible, sépare la main du joueur et le board, et reconnaît rang et couleur.

**Calibration.** Chaque salle dessine ses cartes à sa façon : l'utilisateur indique les cartes
douteuses, et chaque carte indiquée apprend son rang et sa couleur (plus proches voisins sur
des coins normalisés). Les modèles appris restent dans le navigateur. Une lecture sous 60 %
de confiance est toujours signalée, jamais utilisée en silence (T7).

**Autorisations.** Seule la page `/app` ouvre `camera`, `display-capture` et `autoplay`
(Permissions-Policy) et accepte les médias `blob:` (CSP) ; le reste du site garde la
politique la plus stricte. Les images ne quittent pas l'appareil.

**Limites.** Les tapis, mises, pot, bouton et actions ne sont pas encore lus : l'utilisateur
les indique pour analyser un spot. L'historique texte de la salle reste la source la plus
fiable pour une session complète. La lecture des montants demande des captures réelles de
chaque salle visée.

### T14 — Base locale dans Nuage DB, dépôts Git de l'auteur autorisés — *acceptée*

**Choix.** La base locale (crate `database`, auparavant redb, T3) est une base **Nuage DB**
en mode embarqué, la base de l'ORM Memoire, toutes deux écrites par l'auteur de STRATYX. Le
moteur embarqué est compilé sans ses fonctionnalités par défaut : Rust pur, sans réseau, sans
`ring` ni `cc` (chiffrement RustCrypto, T4). La base est un dossier, interrogé en SQL (valeurs
toujours en paramètres) ; chaque opération est une transaction (`EmbeddedTransaction::sql`,
ajouté à Nuage pour cela), les écritures sont sérialisées.

**Anciennes bases.** Une base redb (un fichier) est importée automatiquement à sa première
ouverture : le fichier est renommé `<nom>.ancienne-redb` et gardé en secours, tout son contenu
est recopié dans une base Nuage créée à sa place, en une transaction ; en cas d'échec, rien
n'est changé. redb ne sert plus qu'à cette lecture (fonctionnalité `import-redb`, par défaut).

**Exceptions à `deny.toml` (T2), limitées à ces crates.** Les sources Git
`github.com/maxlestage/Nuage` et `github.com/maxlestage/Memoire` sont autorisées (épinglées
par révision dans `Cargo.lock`). Leur licence propriétaire (même titulaire des droits que
STRATYX) est clarifiée par l'empreinte du fichier `LICENSE` et autorisée pour `nuage`,
`memoire-orm` et `memoire-orm-derive` uniquement : un changement de ce fichier fait échouer
l'audit jusqu'à relecture. Le script de build de `num-traits` (sonde de `rustc` via autocfg,
tiré par chrono) est approuvé.

---

## Points de vigilance (conformité)

1. **Conditions des salles** : vérifier, salle par salle, ce qui est autorisé : enregistrement,
   outils d'analyse après partie, collecte de statistiques sur les adversaires. Tenir la liste à
   jour dans `security/`.
2. **Archives tierces** : les historiques achetés ou collectés en masse enfreignent souvent les
   conditions des salles et le RGPD. N'utiliser que les mains de l'utilisateur, des données sous
   licence vérifiée et les simulations (D5).
3. **RGPD** : les pseudonymes des adversaires sont des données personnelles, et leurs profils un
   traitement. Il faut une base légale (intérêt légitime à documenter), la minimisation, des
   durées de conservation, le stockage local par défaut, l'export et la suppression.
4. **Droit à l'image** : les vidéos peuvent contenir des éléments personnels (webcam, chat). Le
   masquage des zones hors table doit exister dès la capture.
