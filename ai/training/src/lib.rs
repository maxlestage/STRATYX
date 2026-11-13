//! Entraînement des deux IA spécialisées (décision D5 : archives + simulations).
//!
//! Pour chaque IA (Expresso 3-Max, Tournois 6-Max), [`train_format`] :
//!
//! 1. résout l'**équilibre push/fold** du format par jeu fictif ([`stratyx_strategy::pushfold`]) ;
//! 2. génère des **archives simulées** par auto-jeu ([`sim`]) et lit les **archives réelles**
//!    du dossier fourni ([`archive`]) — nettoyées, dédupliquées, validées par rejeu ;
//! 3. apprend le **modèle de population** (ce que jouent les joueurs, régression softmax) ;
//! 4. étiquette chaque décision par la **référence** (équilibre ou EV, indépendante du
//!    résultat) et apprend le **modèle de décision** : un perceptron multicouche et, pour
//!    comparaison, une régression softmax ;
//! 5. mesure chaque modèle sur des mains jamais vues et écrit le tout dans un
//!    [`ModelBundle`] JSON.
//!
//! Tout est en Rust pur, sur CPU, reproductible à graine égale (décision T8).

pub mod archive;
pub mod dataset;
pub mod sim;
pub mod train;

use std::path::{Path, PathBuf};

use stratyx_poker::Specialist;
use stratyx_poker::history::HandRecord;
use stratyx_strategy::bundle::{Metric, ModelBundle, Provenance};
use stratyx_strategy::preflop::{CLASSES, class_at};
use stratyx_strategy::pushfold::{Chart, Node, SolverConfig, solve};

/// Une IA à entraîner.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Expresso3Max,
    Tournament6Max,
}

impl Format {
    pub const ALL: [Format; 2] = [Format::Expresso3Max, Format::Tournament6Max];

    #[must_use]
    pub const fn slug(self) -> &'static str {
        match self {
            Format::Expresso3Max => "expresso-3max",
            Format::Tournament6Max => "tournois-6max",
        }
    }

    #[must_use]
    pub const fn title(self) -> &'static str {
        match self {
            Format::Expresso3Max => "IA Expresso 3-Max",
            Format::Tournament6Max => "IA Tournois 6-Max",
        }
    }

    #[must_use]
    pub const fn specialist(self) -> Specialist {
        match self {
            Format::Expresso3Max => Specialist::Expresso3Max,
            Format::Tournament6Max => Specialist::Tournament6Max,
        }
    }

    /// Le fichier de modèles, relatif à la racine du dépôt.
    #[must_use]
    pub fn model_path(self) -> PathBuf {
        match self {
            Format::Expresso3Max => PathBuf::from("ai/expresso_3max/models/expresso-3max.json"),
            Format::Tournament6Max => PathBuf::from("ai/tournament_6max/models/tournois-6max.json"),
        }
    }

    fn solver(self, quick: bool, seed: u64) -> SolverConfig {
        let (iterations, samples) = if quick { (6, 200) } else { (28, 2000) };
        match self {
            Format::Expresso3Max => SolverConfig {
                players: 3,
                ante_bb: 0.0,
                depths: vec![
                    2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0, 11.0, 12.0, 14.0, 16.0, 18.0,
                    20.0,
                ],
                iterations,
                samples,
                seed,
            },
            Format::Tournament6Max => SolverConfig {
                players: 6,
                ante_bb: 0.125,
                depths: vec![3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 10.0, 12.0, 14.0, 17.0, 20.0],
                iterations,
                samples,
                seed,
            },
        }
    }

    fn simulate(self, hands: usize, seed: u64, chart: &Chart) -> Vec<HandRecord> {
        match self {
            Format::Expresso3Max => sim::expresso(hands, seed, Some(chart)),
            Format::Tournament6Max => sim::tournament_6max(hands, seed, Some(chart)),
        }
    }
}

/// Réglages d'un entraînement.
#[derive(Debug, Clone)]
pub struct Options {
    pub simulated_hands: usize,
    /// Dossier des archives réelles (sous-dossiers `expresso-3max/` et `tournois-6max/`).
    pub archives: Option<PathBuf>,
    /// Où écrire les archives simulées (JSON Lines), si demandé.
    pub write_archives: Option<PathBuf>,
    /// Tirages Monte-Carlo par calcul d'équité de la référence.
    pub reference_samples: usize,
    pub quick: bool,
    pub seed: u64,
    pub date: String,
}

/// Le résultat d'un entraînement : les modèles et un rapport Markdown.
#[derive(Debug, Clone)]
pub struct Outcome {
    pub bundle: ModelBundle,
    pub report: String,
}

/// Entraîne tous les modèles d'une IA.
pub fn train_format(
    format: Format,
    options: &Options,
    mut log: impl FnMut(&str),
) -> std::io::Result<Outcome> {
    let seed = options.seed ^ (format as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15);
    log(&format!("■ {} — équilibre push/fold", format.title()));
    let solver = format.solver(options.quick, seed);
    let mut last = -1.0f32;
    let chart = solve(&solver, |depth, it| {
        if depth != last && it == 0 {
            last = depth;
            log(&format!("    tapis de {depth} BB…"));
        }
    });

    log(&format!(
        "■ {} — {} mains simulées",
        format.title(),
        options.simulated_hands
    ));
    let simulated = format.simulate(options.simulated_hands, seed ^ 0x51, &chart);
    if let Some(dir) = &options.write_archives {
        let path = dir
            .join(format.slug())
            .join(format!("simulation-{seed:016x}.jsonl"));
        archive::write_jsonl(&path, &simulated)?;
        log(&format!(
            "    archives simulées écrites dans {}",
            path.display()
        ));
    }
    let (real, load) = match &options.archives {
        Some(dir) => archive::load(&dir.join(format.slug()), format.specialist())?,
        None => (Vec::new(), archive::LoadReport::default()),
    };
    if options.archives.is_some() {
        log(&format!(
            "    archives réelles : {} mains valides ({} fichiers, {} doublons, {} invalides, {} d'un autre format)",
            real.len(),
            load.files,
            load.duplicates,
            load.invalid,
            load.other_format
        ));
    }
    let mut all = simulated.clone();
    all.extend(real.iter().cloned());

    // Population : toutes les décisions, archives réelles comprises.
    log(&format!("■ {} — modèle de population", format.title()));
    let (pop_train, pop_val) = dataset::split(dataset::population(&all));
    let epochs = if options.quick { 3 } else { 10 };
    let (population, pop_metric) = train::train(
        &train::Config {
            name: "population-softmax".into(),
            hidden: vec![],
            classes: 4,
            epochs,
            batch: 128,
            learning_rate: 0.01,
            l2: 1e-5,
            seed: seed ^ 2,
        },
        &pop_train,
        &pop_val,
        &mut log,
    );
    let pop_labels = dataset::label_counts(&pop_train);

    // Décision : référence (équilibre / EV) avec le modèle de population pour la fold equity.
    log(&format!(
        "■ {} — étiquetage par la référence ({} tirages par équité)",
        format.title(),
        options.reference_samples
    ));
    let decisions = dataset::decisions(&all, &chart, Some(&population), options.reference_samples);
    let decisions_count = decisions.len();
    let (dec_train, dec_val) = dataset::split(decisions);
    let dec_labels = dataset::label_counts(&dec_train);
    log(&format!(
        "    {} décisions ({} entraînement, {} validation)",
        decisions_count,
        dec_train.len(),
        dec_val.len()
    ));
    let epochs = if options.quick { 4 } else { 24 };
    let (decision, dec_metric) = train::train(
        &train::Config {
            name: "decision-mlp".into(),
            hidden: vec![64, 32],
            classes: 4,
            epochs,
            batch: 64,
            learning_rate: 0.003,
            l2: 1e-5,
            seed: seed ^ 3,
        },
        &dec_train,
        &dec_val,
        &mut log,
    );
    let (decision_linear, lin_metric) = train::train(
        &train::Config {
            name: "decision-softmax".into(),
            hidden: vec![],
            classes: 4,
            epochs: epochs / 2 + 1,
            batch: 128,
            learning_rate: 0.01,
            l2: 1e-5,
            seed: seed ^ 4,
        },
        &dec_train,
        &dec_val,
        &mut log,
    );

    let metrics = vec![pop_metric, dec_metric, lin_metric];
    let bundle = ModelBundle {
        format: format.slug().into(),
        provenance: Provenance {
            simulated_hands: simulated.len(),
            archive_hands: real.len(),
            decisions: decisions_count,
            seed: options.seed,
            date: options.date.clone(),
        },
        pushfold: chart,
        decision,
        decision_linear,
        population,
        metrics,
    };
    let report = report(format, &bundle, &solver, pop_labels, dec_labels, &load);
    Ok(Outcome { bundle, report })
}

fn pct(x: f32) -> String {
    format!("{:.1} %", x * 100.0).replace('.', ",")
}

/// La part des mains poussées à chaque position, et quelques mains limites.
fn chart_summary(chart: &Chart) -> String {
    let n = chart.players;
    let names: Vec<&str> = match n {
        2 => vec!["BTN/SB", "BB"],
        3 => vec!["BTN", "SB", "BB"],
        6 => vec!["UTG", "HJ", "CO", "BTN", "SB", "BB"],
        _ => vec!["?"; 10],
    };
    let mut s = String::from("| Tapis | ");
    s.push_str(
        &names[..n - 1]
            .iter()
            .map(|p| format!("Tapis {p}"))
            .collect::<Vec<_>>()
            .join(" | "),
    );
    s.push_str(" | BB suit le tapis de la SB |\n|---|");
    s.push_str(&"---|".repeat(n));
    s.push('\n');
    for &d in &chart.depths {
        if ![3.0, 5.0, 8.0, 10.0, 12.0, 15.0, 16.0, 20.0].contains(&d) {
            continue;
        }
        s.push_str(&format!("| {d} BB |"));
        for o in 0..n - 1 {
            s.push_str(&format!(
                " {} |",
                pct(chart.range_fraction(Node::FirstIn, d, o, n))
            ));
        }
        s.push_str(&format!(
            " {} |\n",
            pct(chart.range_fraction(Node::VsShove { shover: n - 2 }, d, n - 1, n))
        ));
    }
    // Les mains poussées par la petite blinde à 10 BB.
    let d = 10.0;
    let mut pushed: Vec<String> = (0..CLASSES)
        .filter(|c| chart.probability(Node::FirstIn, d, n - 2, n, *c) >= 0.5)
        .map(|c| class_at(c).to_string())
        .collect();
    pushed.truncate(400);
    s.push_str(&format!(
        "\nPetite blinde à 10 BB, mains poussées (≥ 50 %) : {} classes sur 169.\n",
        pushed.len()
    ));
    s
}

fn metrics_table(metrics: &[Metric]) -> String {
    let mut s = String::from(
        "| Modèle | Paramètres | Entraînement | Validation | Précision | Perte (log) | Classe majoritaire |\n|---|---|---|---|---|---|---|\n",
    );
    for m in metrics {
        s.push_str(&format!(
            "| `{}` | {} | {} | {} | {} | {:.3} | {} |\n",
            m.model,
            m.parameters,
            m.train,
            m.validation,
            pct(m.accuracy),
            m.log_loss,
            pct(m.baseline)
        ));
    }
    s
}

fn labels_line(counts: [usize; 4]) -> String {
    let total: usize = counts.iter().sum::<usize>().max(1);
    dataset::label_names()
        .iter()
        .zip(counts)
        .map(|(n, c)| format!("{n} {}", pct(c as f32 / total as f32)))
        .collect::<Vec<_>>()
        .join(" · ")
}

fn report(
    format: Format,
    b: &ModelBundle,
    solver: &SolverConfig,
    pop_labels: [usize; 4],
    dec_labels: [usize; 4],
    load: &archive::LoadReport,
) -> String {
    format!(
        "## {title}\n\n\
         Fichier : `{path}` · entraîné le {date} · graine `{seed:#x}`.\n\n\
         **Données.** {sim} mains simulées (auto-jeu, six profils de joueurs) et {real} mains \
         d'archives réelles ({files} fichiers lus, {dup} doublons et {inv} mains invalides \
         écartés). {dec} décisions étiquetées.\n\n\
         **Équilibre push/fold** ({players} joueurs, ante {ante} BB, {it} itérations de jeu \
         fictif, {samples} tirages par position et par classe à chaque itération) :\n\n{chart}\n\
         **Modèles** (mesurés sur environ 20 % des mains, jamais vues à l'entraînement) :\n\n{metrics}\n\
         Étiquettes de la population (actions jouées) : {pop}.\n\n\
         Étiquettes de la décision (référence) : {decl}.\n\n",
        title = format.title(),
        path = format.model_path().display(),
        date = b.provenance.date,
        seed = b.provenance.seed,
        sim = b.provenance.simulated_hands,
        real = b.provenance.archive_hands,
        files = load.files,
        dup = load.duplicates,
        inv = load.invalid,
        dec = b.provenance.decisions,
        players = solver.players,
        ante = solver.ante_bb,
        it = solver.iterations,
        samples = solver.samples,
        chart = chart_summary(&b.pushfold),
        metrics = metrics_table(&b.metrics),
        pop = labels_line(pop_labels),
        decl = labels_line(dec_labels),
    )
}

/// Écrit un fichier de modèles (JSON compact).
pub fn write_bundle(root: &Path, format: Format, bundle: &ModelBundle) -> std::io::Result<PathBuf> {
    let path = root.join(format.model_path());
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string(bundle).map_err(std::io::Error::other)?;
    std::fs::write(&path, json)?;
    Ok(path)
}
