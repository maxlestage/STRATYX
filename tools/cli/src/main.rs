//! `stratyx` — outil en ligne de commande pour le moteur poker.
//!
//! ```text
//! stratyx eval AsKsQsJsTs9h2c
//! stratyx equity "AhKh" "QQ+,AKo" --board Qs7h2h
//! stratyx range "22+,A2s+,KTo+"
//! stratyx icm --stacks 5000,3000,2000 --payouts 50,30,20
//! stratyx analyse historique.txt --joueur Hero
//! ```

use std::process::ExitCode;

use stratyx_poker::equity::{EquityMethod, EquityOptions, equity};
use stratyx_poker::eval::best_five;
use stratyx_poker::icm::icm_equities;
use stratyx_poker::{CardSet, Range};
use stratyx_shared::Chips;

const USAGE: &str = "\
Usage :
  stratyx eval <cartes>                       force de la meilleure main (5 à 7 cartes)
  stratyx equity <range> <range> [...]        équité entre 2 à 10 ranges
         [--board <cartes>] [--dead <cartes>] [--samples <n>] [--seed <n>]
  stratyx range <range>                       nombre de combinaisons d'une range
  stratyx icm --stacks <a,b,...> --payouts <x,y,...>
                                              équité ICM (Malmuth-Harville)
  stratyx analyse <fichier | -> [--joueur <nom>] [--format texte|json]
                                              analyse après partie d'historiques texte
                                              (PokerStars, Winamax) : chaque décision
                                              comparée à la référence des IA

Exemples de range : AA, AKs, AKo, TT+, ATs+, 22-55, A2s-A5s, AsKd, AKs:0.5, random";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("erreur : {e}\n\n{USAGE}");
            ExitCode::FAILURE
        }
    }
}

/// Options `--clé valeur` de la ligne de commande.
type Options<'a> = Vec<(&'a str, &'a str)>;

/// Sépare les arguments positionnels des options `--clé valeur`.
fn split_options(args: &[String]) -> Result<(Vec<&str>, Options<'_>), String> {
    let mut positional = Vec::new();
    let mut options = Vec::new();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        if let Some(key) = a.strip_prefix("--") {
            let value = it
                .next()
                .ok_or_else(|| format!("valeur manquante pour --{key}"))?;
            options.push((key, value.as_str()));
        } else {
            positional.push(a.as_str());
        }
    }
    Ok((positional, options))
}

fn option<'a>(options: &[(&str, &'a str)], key: &str) -> Option<&'a str> {
    options
        .iter()
        .rev()
        .find(|(k, _)| *k == key)
        .map(|(_, v)| *v)
}

fn parse_list<T: std::str::FromStr>(s: &str, what: &str) -> Result<Vec<T>, String> {
    s.split(',')
        .map(|x| {
            x.trim()
                .parse()
                .map_err(|_| format!("{what} illisible : « {x} »"))
        })
        .collect()
}

fn run(args: &[String]) -> Result<(), String> {
    let Some((command, rest)) = args.split_first() else {
        println!("{USAGE}");
        return Ok(());
    };
    let (positional, options) = split_options(rest)?;
    match command.as_str() {
        "eval" => {
            let cards = CardSet::parse(&positional.concat()).map_err(|e| e.to_string())?;
            let (five, rank) = best_five(cards).ok_or("il faut entre 5 et 7 cartes distinctes")?;
            let five: Vec<String> = five.iter().map(ToString::to_string).collect();
            println!("{rank}");
            println!("Meilleure combinaison : {}", five.join(" "));
        }
        "equity" => {
            let ranges = positional
                .iter()
                .map(|r| Range::parse(r).map_err(|e| e.to_string()))
                .collect::<Result<Vec<_>, _>>()?;
            let board = option(&options, "board").map(CardSet::parse).transpose();
            let dead = option(&options, "dead").map(CardSet::parse).transpose();
            let board = board.map_err(|e| e.to_string())?.unwrap_or_default();
            let dead = dead.map_err(|e| e.to_string())?.unwrap_or_default();
            let mut opts = EquityOptions::default();
            if let Some(n) = option(&options, "samples") {
                opts.samples = n.parse().map_err(|_| "--samples : entier attendu")?;
            }
            if let Some(s) = option(&options, "seed") {
                opts.seed = s.parse().map_err(|_| "--seed : entier attendu")?;
            }
            let res = equity(&ranges, board, dead, &opts).map_err(|e| e.to_string())?;
            let method = match res.method {
                EquityMethod::Exact => "énumération exacte".to_string(),
                EquityMethod::MonteCarlo => format!("Monte-Carlo, graine {}", opts.seed),
            };
            println!("Méthode : {method} — {} distributions", res.deals);
            for (r, p) in positional.iter().zip(&res.players) {
                let err = if p.std_error > 0.0 {
                    format!(" ± {:.2}", 1.96 * p.std_error * 100.0)
                } else {
                    String::new()
                };
                println!(
                    "{r:<24} équité {:>6.2} %{err}   victoire {:>6.2} %   partage {:>5.2} %",
                    p.equity * 100.0,
                    p.win * 100.0,
                    p.tie * 100.0
                );
            }
        }
        "range" => {
            let r = Range::parse(&positional.join(",")).map_err(|e| e.to_string())?;
            println!(
                "{:.1} combinaisons ({:.2} % des mains)",
                r.combo_count(),
                r.fraction() * 100.0
            );
            println!("{r}");
        }
        "icm" => {
            let stacks: Vec<u64> = parse_list(
                option(&options, "stacks").ok_or("--stacks requis")?,
                "tapis",
            )?;
            let payouts: Vec<f64> = parse_list(
                option(&options, "payouts").ok_or("--payouts requis")?,
                "prix",
            )?;
            let stacks: Vec<Chips> = stacks.into_iter().map(Chips).collect();
            let eq = icm_equities(&stacks, &payouts).map_err(|e| e.to_string())?;
            let total: u64 = stacks.iter().map(|s| s.get()).sum();
            for (i, (s, e)) in stacks.iter().zip(&eq).enumerate() {
                println!(
                    "Joueur {} : tapis {s:>8} ({:>5.1} % des jetons)   équité ICM {e:>10.4}",
                    i + 1,
                    s.get() as f64 / total as f64 * 100.0
                );
            }
        }
        "analyse" | "analyze" => {
            let path = positional.first().copied().unwrap_or("-");
            let text = if path == "-" {
                let mut s = String::new();
                std::io::Read::read_to_string(&mut std::io::stdin(), &mut s)
                    .map_err(|e| e.to_string())?;
                s
            } else {
                let bytes = std::fs::read(path).map_err(|e| format!("{path} : {e}"))?;
                String::from_utf8_lossy(&bytes).into_owned()
            };
            let hero = option(&options, "joueur");
            let (analyses, errors) = stratyx_analytics::analyse_text(
                &text,
                hero,
                &stratyx_analytics::Options::default(),
            );
            for e in &errors {
                eprintln!("⚠ {e}");
            }
            if option(&options, "format") == Some("json") {
                let ok: Vec<_> = analyses.iter().filter_map(|a| a.as_ref().ok()).collect();
                println!(
                    "{}",
                    serde_json::to_string_pretty(&ok).map_err(|e| e.to_string())?
                );
                return Ok(());
            }
            for a in &analyses {
                match a {
                    Ok(a) => print_analysis(a),
                    Err(e) => eprintln!("⚠ {e}"),
                }
            }
            if analyses.is_empty() {
                return Err("aucune main lisible".into());
            }
        }
        "help" | "--help" | "-h" => println!("{USAGE}"),
        other => return Err(format!("commande inconnue : {other}")),
    }
    Ok(())
}

/// Affiche une analyse de main dans le terminal.
fn print_analysis(a: &stratyx_analytics::HandAnalysis) {
    use stratyx_analytics::{bb, pct};
    println!("━━ Main {} · {} · {}", a.id, a.format, a.engine);
    println!(
        "{} ({}) · {} · tapis {}",
        a.hero,
        a.position,
        a.hero_cards,
        bb(a.stack_bb)
    );
    for l in &a.lines {
        if l.player.is_empty() {
            println!("  ── {}", l.text);
        } else {
            let mark = if l.hero { "▶" } else { " " };
            println!(
                "  {mark} {:<4} {:<14} {:<26} pot {}",
                l.position,
                l.player,
                l.text,
                bb(l.pot_bb)
            );
        }
    }
    for (i, d) in a.decisions.iter().enumerate() {
        let odds = if d.facing_bet {
            format!(" · cote {}", pct(d.pot_odds))
        } else {
            String::new()
        };
        println!(
            "\n  Décision {} · {:?} · {} · équité {}{odds}",
            i + 1,
            d.street,
            d.severity.label(),
            pct(d.equity),
        );
        println!("    joué : {} · référence : {}", d.played_text, d.best_text);
        println!("    {}", d.comment);
    }
    println!(
        "\n  Note {}/100 · {} · EV perdue {} · {}",
        a.summary.score,
        a.summary.verdict,
        bb(a.summary.ev_lost_bb),
        a.outcome
    );
    for t in &a.summary.tips {
        println!("  → {t}");
    }
    println!();
}
