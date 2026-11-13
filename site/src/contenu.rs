//! Les textes du site, tirés du cahier des charges (docs/cahier-des-charges.md) et des
//! décisions (docs/decisions.md). Rien d'inventé : STRATYX est en développement, le site
//! le dit et ouvre une liste d'attente.

pub const NOM: &str = "STRATYX";
pub const ANNEE: &str = "2026";
pub const DEPOT: &str = "https://github.com/maxlestage/stratyx";

/// Le menu principal.
pub const MENU: [(&str, &str); 6] = [
    ("/app", "Analyse"),
    ("/fonctionnalites", "Fonctionnalités"),
    ("/ia", "Les IA"),
    ("/laboratoire", "Laboratoire"),
    ("/technologie", "Technologie"),
    ("/feuille-de-route", "Feuille de route"),
];

/// Les pages du plan du site (et du sitemap.xml).
pub const PAGES: [(&str, &str); 12] = [
    ("/", "Accueil"),
    ("/app", "Analyser une partie"),
    ("/fonctionnalites", "Fonctionnalités"),
    ("/ia", "Les deux IA"),
    ("/laboratoire", "Laboratoire : le moteur en direct"),
    ("/technologie", "Technologie : 100 % Rust"),
    ("/feuille-de-route", "Feuille de route"),
    ("/fair-play", "Fair-play et jeu responsable"),
    ("/contact", "Contact"),
    ("/mentions-legales", "Mentions légales"),
    ("/confidentialite", "Politique de confidentialité"),
    ("/plan-du-site", "Plan du site"),
];

pub const PIED_PRODUIT: &[(&str, &str)] = &[
    ("/app", "Analyse de parties"),
    ("/fonctionnalites", "Fonctionnalités"),
    ("/ia", "IA Expresso 3-Max"),
    ("/ia#tournois", "IA Tournois 6-Max"),
    ("/feuille-de-route", "Feuille de route"),
];

pub const PIED_RESSOURCES: &[(&str, &str)] = &[
    ("/laboratoire#equite", "Calculateur d'équité"),
    ("/laboratoire#ranges", "Grille des ranges"),
    ("/laboratoire#icm", "Calculateur ICM"),
    ("/technologie", "Architecture"),
];

pub const PIED_PROJET: &[(&str, &str)] = &[
    ("/fair-play", "Fair-play"),
    ("/contact", "Contact"),
    ("/plan-du-site", "Plan du site"),
    ("/confidentialite", "Données personnelles"),
];

pub struct Fonction {
    pub icone: &'static str,
    pub titre: &'static str,
    pub texte: &'static str,
    pub tags: &'static [&'static str],
}

pub const FONCTIONS: [Fonction; 6] = [
    Fonction {
        icone: "camera",
        titre: "Capture et vision",
        texte: "Capture de l'écran ou import d'une vidéo. Cartes, positions, mises et actions reconnues, chaque détection avec son score de confiance.",
        tags: &["Capture d'écran", "Import vidéo", "Confiance"],
    },
    Fonction {
        icone: "liste",
        titre: "Retranscription fidèle",
        texte: "Chaque main est reconstruite, du préflop à l'abattage, puis validée par rejeu dans le moteur de règles. Aucune carte invisible n'est inventée.",
        tags: &["Validation par rejeu", "Export JSON"],
    },
    Fonction {
        icone: "cible",
        titre: "Analyse stratégique",
        texte: "GTO et jeu exploitant, équité, cotes du pot, fold equity, EV, sizings et ICM. La décision est jugée indépendamment du résultat.",
        tags: &["GTO", "EV", "ICM"],
    },
    Fonction {
        icone: "profil",
        titre: "Profils d'adversaires",
        texte: "VPIP, PFR, 3-bet, c-bet, agressivité, défense des blindes… Chaque statistique affiche sa fiabilité : taille d'échantillon et intervalle de confiance.",
        tags: &["VPIP / PFR", "Fiabilité"],
    },
    Fonction {
        icone: "courbe",
        titre: "Statistiques personnelles",
        texte: "Gains, ROI, ITM, volume, résultats par position, erreurs récurrentes et décisions coûteuses : vos forces et vos fuites, mesurées.",
        tags: &["ROI", "ITM", "Fuites"],
    },
    Fonction {
        icone: "diplome",
        titre: "Coaching et entraînement",
        texte: "Exercices préflop et postflop, push/fold, ICM, quiz et simulations, dans un parcours qui cible vos faiblesses et suit vos progrès.",
        tags: &["Exercices", "Quiz", "Parcours"],
    },
];

/// Étapes du fonctionnement : titre, texte.
pub const ETAPES: [(&str, &str); 4] = [
    (
        "Capturer",
        "Enregistrez votre session ou importez une vidéo existante. Rien ne s'affiche pendant la partie.",
    ),
    (
        "Retranscrire",
        "Les mains sont reconnues, reconstruites, puis validées par rejeu dans le moteur de règles.",
    ),
    (
        "Analyser",
        "Chaque décision est comparée à la décision de référence, avec l'équité, l'EV et l'ICM.",
    ),
    (
        "Progresser",
        "Des exercices ciblés sur vos fuites, et un suivi de progression d'une session à l'autre.",
    ),
];

/// Les chiffres du moteur : nombre, suffixe, libellé (tous vérifiables dans le dépôt).
pub const CHIFFRES: [(u32, &str, &str); 4] = [
    (133, " M", "de mains à 7 cartes vérifiées par l'évaluateur"),
    (10, " joueurs", "de 2 à 10 par table, de UTG à BB"),
    (3, " structures", "no-limit, pot-limit et limite fixe"),
    (100, " %", "Rust, contrôlé à chaque push"),
];

/// Les formats couverts.
pub const FORMATS_COUVERTS: [(&str, &str); 4] = [
    (
        "Cash game",
        "Tables de 2 à 10 joueurs, toutes structures de mises.",
    ),
    (
        "Tournois",
        "Structures, antes classiques et de grosse blinde, ICM, bulle et table finale.",
    ),
    ("Sit & go", "Paiements, pression ICM, tapis courts."),
    (
        "Expresso",
        "Trois joueurs, tapis courts, multiplicateurs et structures de paiement.",
    ),
];

pub struct Ia {
    pub id: &'static str,
    pub nom: &'static str,
    pub accroche: &'static str,
    pub points: &'static [&'static str],
    pub crate_rs: &'static str,
}

pub const IAS: [Ia; 2] = [
    Ia {
        id: "expresso",
        nom: "IA Expresso 3-Max",
        accroche: "Spécialiste des formats rapides à trois joueurs, où chaque jeton compte.",
        points: &[
            "Push/fold et jeu à tapis courts",
            "Ranges par position",
            "Heads-up et jeu à trois",
            "Stratégie préflop et postflop",
            "Multiplicateurs et structures de paiement",
        ],
        crate_rs: "ai/expresso_3max",
    },
    Ia {
        id: "tournois",
        nom: "IA Tournois 6-Max",
        accroche: "Spécialiste des tournois à six joueurs, de l'ouverture à la table finale.",
        points: &[
            "Open, 3-bet et 4-bet",
            "Ranges par position et jeu postflop",
            "Gestion des tapis",
            "ICM, bulle et table finale",
            "Adaptation aux adversaires",
        ],
        crate_rs: "ai/tournament_6max",
    },
];

/// Les phases de la feuille de route : numéro, titre, statut, détail, avancement (0 à 1).
pub const PHASES: [(u8, &str, &str, &str, f32); 8] = [
    (
        1,
        "Fondations et moteur poker",
        "En cours",
        "Moteur livré : cartes, évaluateur 5 à 7 cartes, ranges, équité exacte et Monte-Carlo, règles NL/PL/FL, pots, ICM, import des historiques texte, CLI.",
        0.92,
    ),
    (
        2,
        "Capture vidéo et reconnaissance",
        "Commencée",
        "Dans le navigateur : capture de l'écran ou caméra, import de vidéos et de captures, détection et reconnaissance des cartes avec calibration et confiance. À venir : lecture des tapis, mises et actions, capture native.",
        0.3,
    ),
    (
        3,
        "Retranscription automatique",
        "Commencée",
        "Format pivot, validation par rejeu et import des historiques PokerStars et Winamax livrés.",
        0.45,
    ),
    (
        4,
        "IA Expresso et IA Tournois",
        "En cours",
        "Livrés : équilibres push/fold à 3 et à 6, modèles de décision et de population entraînés sur archives simulées. À venir : archives sous licence, postflop approfondi, GPU.",
        0.45,
    ),
    (
        5,
        "Analyses et statistiques",
        "Commencée",
        "Analyse des mains livrée (page Analyse et stratyx analyse). À venir : profils d'adversaires, statistiques personnelles.",
        0.25,
    ),
    (
        6,
        "Coaching et simulations",
        "À venir",
        "Exercices, quiz, parcours personnalisé, suivi de progression.",
        0.0,
    ),
    (
        7,
        "Interfaces et API",
        "À venir",
        "Applications bureau et mobile, serveur et API professionnelle.",
        0.0,
    ),
    (
        8,
        "Sécurité et optimisation",
        "Commencée",
        "Audit des dépendances et intégration continue déjà en place.",
        0.2,
    ),
];

/// Les modules du workspace : chemin, rôle.
pub const MODULES: [(&str, &str); 12] = [
    (
        "poker/",
        "Moteur commun : cartes, évaluation, ranges, équité, règles, pots, ICM",
    ),
    (
        "shared/",
        "Types transverses : jetons, confiance, provenance",
    ),
    ("capture/", "Capture d'écran par les API système"),
    ("video/", "Import, décodage, horodatage"),
    ("vision/", "Reconnaissance avec score de confiance"),
    ("strategy/", "Ranges de référence, push/fold, solveurs"),
    (
        "ai/",
        "Entraînement, inférence, Expresso 3-Max, Tournois 6-Max",
    ),
    ("analytics/", "Analyse des mains, profils, statistiques"),
    ("coaching/", "Exercices, quiz, parcours"),
    ("database/", "Persistance des bases"),
    ("security/", "Chiffrement, authentification, RGPD"),
    ("apps/", "Bureau, mobile, serveur et API"),
];

/// Les principes techniques : icône, titre, texte.
pub const PRINCIPES: [(&str, &str, &str); 4] = [
    (
        "rouage",
        "Rust partout",
        "Moteurs, IA, interfaces, serveur et outils. Seules les API des systèmes et les pilotes sont appelées depuis Rust.",
    ),
    (
        "bouclier",
        "Audit bloquant",
        "cargo deny refuse toute dépendance qui compile du C/C++ ou embarque une bibliothèque native, et chaque script de build est approuvé nommément.",
    ),
    (
        "eclair",
        "Parallèle et reproductible",
        "L'équité Monte-Carlo est parallèle et déterministe : à graine égale, le même résultat, quel que soit le nombre de cœurs.",
    ),
    (
        "puce",
        "CPU et GPU",
        "Les IA s'appuient sur Burn, en ndarray ou wgpu selon le matériel.",
    ),
];

pub const FAQ: [(&str, &str); 6] = [
    (
        "STRATYX m'aide-t-il pendant une partie ?",
        "Non, jamais. STRATYX sert à analyser après la partie et à s'entraîner, dans des modes distincts. Aucune assistance stratégique n'est affichée pendant une partie réelle.",
    ),
    (
        "Quelles variantes sont couvertes ?",
        "Le Texas Hold'em sous toutes ses formes : no-limit, pot-limit et limite fixe, tables de 2 à 10 joueurs, cash game, tournois, sit & go et Expresso.",
    ),
    (
        "Sur quelles plateformes ?",
        "Ordinateur d'abord (Windows, macOS, Linux), puis mobile (iOS, Android).",
    ),
    (
        "Que se passe-t-il si une carte n'est pas visible ?",
        "Elle n'est pas inventée. Si une carte manque pour désigner un gagnant, la main est déclarée « non résolue », et les détections incertaines sont signalées.",
    ),
    (
        "Quand STRATYX sera-t-il disponible ?",
        "Le moteur poker est livré, les autres modules arrivent par étapes (voir la feuille de route). Inscrivez-vous à la liste d'attente pour l'accès anticipé.",
    ),
    (
        "Les démos du site sont-elles réelles ?",
        "Oui : le calculateur d'équité, la grille des ranges, l'ICM et la table de démonstration utilisent le vrai moteur stratyx-poker, compilé en WebAssembly et exécuté dans votre navigateur.",
    ),
];

pub const PROFILS: [&str; 4] = ["Joueur loisir", "Joueur régulier", "Professionnel", "Coach"];

pub const FAIR_PLAY: [(&str, &str); 4] = [
    (
        "Aucune aide en direct",
        "STRATYX n'affiche rien pendant une partie réelle. L'analyse se fait après coup, l'entraînement dans un mode séparé. C'est une règle du cahier des charges, pas une option.",
    ),
    (
        "Le respect des salles",
        "Les conditions d'utilisation des salles de poker sont respectées, et la licence de chaque source d'archives est vérifiée avant tout entraînement.",
    ),
    (
        "Vos données vous appartiennent",
        "Vidéos et historiques sont protégés, exportables et supprimables. RGPD et droit à l'image sont respectés.",
    ),
    (
        "Le jeu reste un jeu",
        "Le poker d'argent est interdit aux mineurs. Jouer comporte des risques : endettement, dépendance… Fixez-vous des limites. Besoin d'aide ? Joueurs Info Service : 09 74 75 13 13 (appel non surtaxé).",
    ),
];

pub const MENTIONS: [(&str, &str); 4] = [
    (
        "Éditeur du site",
        "STRATYX est un projet en développement. Les informations légales de l'éditeur (raison sociale, adresse, directeur de la publication) seront publiées ici avant l'ouverture commerciale.\n\nContact : via la page Contact de ce site.",
    ),
    (
        "Hébergement",
        "Les informations de l'hébergeur seront précisées à la mise en ligne.",
    ),
    (
        "Propriété intellectuelle",
        "Le nom STRATYX, les textes, visuels, dessins 2D et le code de ce site sont protégés. Toute reproduction sans autorisation est interdite.",
    ),
    (
        "Crédits",
        "Site écrit entièrement en Rust avec Paquets ; dessins en Canvas 2D ; calculs par le moteur stratyx-poker compilé en WebAssembly.",
    ),
];

pub const CONFIDENTIALITE: [(&str, &str); 5] = [
    (
        "Données collectées",
        "Uniquement ce que vous saisissez : l'adresse e-mail et le profil de la liste d'attente, et les champs du formulaire de contact. Les démos du laboratoire s'exécutent dans votre navigateur : rien n'est envoyé.",
    ),
    (
        "Finalités",
        "Vous prévenir de l'ouverture de l'accès anticipé et répondre à vos messages. Vos données ne sont ni vendues ni cédées.",
    ),
    (
        "Durée de conservation",
        "Jusqu'à votre désinscription pour la liste d'attente, et 3 ans après le dernier échange pour les messages.",
    ),
    (
        "Cookies",
        "Aucun cookie publicitaire ni traceur tiers. Le mode hors ligne repose sur le cache de votre navigateur.",
    ),
    (
        "Vos droits",
        "Accès, rectification, effacement, portabilité et opposition (RGPD), via la page Contact. Vous pouvez aussi saisir la CNIL.",
    ),
];
