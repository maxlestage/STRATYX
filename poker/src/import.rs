//! Import des historiques texte des salles : PokerStars (et les salles au même format) et
//! Winamax, en tournoi comme en cash game.
//!
//! Chaque main devient un [`HandRecord`] (provenance [`Source::Import`]), que
//! [`HandRecord::replay`] valide ensuite dans le moteur de règles. Le texte peut contenir
//! plusieurs mains à la suite : elles sont toutes lues, et une main illisible n'empêche
//! pas de lire les autres.
//!
//! Ce qui est lu : format (cash game, tournoi, Expresso), taille de table, bouton, sièges et
//! tapis, blindes et antes, cartes du joueur analysé (« Dealt to »), actions de chaque tour,
//! board, cartes montrées à l'abattage. Les blindes postées ne sont pas des actions : le
//! moteur les poste lui-même. Les montants à décimales (cash game) sont convertis en
//! centimes.

use stratyx_shared::{Chips, Observed, Source};
use thiserror::Error;

use crate::card::{Card, parse_cards};
use crate::format::{GameFormat, GameType};
use crate::game::{Action, Ante, BettingStructure, Blinds};
use crate::history::{HandEvent, HandRecord, SeatRecord};
use crate::range::HoleCards;

/// Salle d'origine d'un historique.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Room {
    PokerStars,
    Winamax,
    /// Format proche de PokerStars (GGPoker, PartyPoker, iPoker…), lu au mieux.
    Other,
}

impl Room {
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Room::PokerStars => "PokerStars",
            Room::Winamax => "Winamax",
            Room::Other => "autre salle",
        }
    }
}

/// Une main importée.
#[derive(Debug, Clone, PartialEq)]
pub struct ImportedHand {
    pub record: HandRecord,
    /// Le joueur dont les cartes sont connues (« Dealt to »), s'il y en a un.
    pub hero: Option<String>,
    pub room: Room,
    /// Facteur appliqué aux montants (100 quand le texte a des décimales).
    pub scale: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ImportError {
    #[error("aucune main reconnue dans le texte")]
    Empty,
    #[error("main {hand} : {reason}")]
    Hand { hand: String, reason: String },
}

/// Résultat de l'import d'un texte : les mains lues et les erreurs des autres.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ImportReport {
    pub hands: Vec<ImportedHand>,
    pub errors: Vec<ImportError>,
}

/// Lit toutes les mains d'un texte.
#[must_use]
pub fn import_text(text: &str) -> ImportReport {
    let mut report = ImportReport::default();
    let blocks = split_hands(text);
    if blocks.is_empty() {
        report.errors.push(ImportError::Empty);
    }
    for block in blocks {
        match parse_hand(&block) {
            Ok(h) => report.hands.push(h),
            Err(e) => report.errors.push(e),
        }
    }
    report
}

/// Lit une seule main (la première du texte).
pub fn import_hand(text: &str) -> Result<ImportedHand, ImportError> {
    let blocks = split_hands(text);
    let first = blocks.first().ok_or(ImportError::Empty)?;
    parse_hand(first)
}

/// Une ligne qui commence une main.
fn is_header(line: &str) -> bool {
    let l = line.trim_start_matches('\u{feff}').trim();
    (l.starts_with("PokerStars ") && (l.contains("Hand #") || l.contains("Game #")))
        || l.starts_with("Winamax Poker")
        || (l.starts_with("Poker Hand #") && l.contains(':'))
        || (l.contains(" Hand #") && l.contains("Hold'em"))
}

/// Découpe le texte en mains.
fn split_hands(text: &str) -> Vec<Vec<String>> {
    let mut out: Vec<Vec<String>> = Vec::new();
    for line in text.lines() {
        let line = line.trim_start_matches('\u{feff}').trim_end();
        if is_header(line) {
            out.push(vec![line.trim().to_owned()]);
        } else if let Some(cur) = out.last_mut()
            && !line.trim().is_empty()
        {
            cur.push(line.trim().to_owned());
        }
    }
    out
}

/// Montant lu dans un texte : `1,500`, `1 500`, `$0.25`, `0,92€`.
fn amount(s: &str) -> Option<f64> {
    let t: String = s
        .trim()
        .trim_matches(|c: char| !c.is_ascii_digit())
        .chars()
        .filter(|c| !matches!(c, ' ' | '\u{202f}' | '\u{a0}'))
        .collect();
    if t.is_empty() {
        return None;
    }
    // « 1,500 » : séparateur de milliers ; « 0,25 » : virgule décimale.
    let t = if let Some((a, b)) = t.split_once(',') {
        if b.len() == 3 && !b.contains(',') && !t.contains('.') {
            format!("{a}{b}")
        } else if b.len() == 3 && t.contains('.') {
            t.replace(',', "")
        } else {
            t.replace(',', ".")
        }
    } else {
        t
    };
    t.parse().ok()
}

fn has_decimals(s: &str) -> bool {
    // « 0.25 » ou « 0,25 » (mais pas « 1,500 »).
    let b = s.as_bytes();
    for i in 1..b.len().saturating_sub(1) {
        if (b[i] == b'.' || b[i] == b',') && b[i - 1].is_ascii_digit() && b[i + 1].is_ascii_digit()
        {
            let digits = b[i + 1..].iter().take_while(|c| c.is_ascii_digit()).count();
            if b[i] == b'.' || digits != 3 {
                return true;
            }
        }
    }
    false
}

/// Les cartes entre crochets : `[Ah Kd]`. Avec plusieurs crochets, le dernier.
fn bracket_cards(line: &str) -> Option<Vec<Card>> {
    let start = line.rfind('[')?;
    let end = start + line[start..].find(']')?;
    parse_cards(&line[start + 1..end]).ok()
}

/// Les blindes de l'en-tête : `(10/20)`, `($0.01/$0.02 USD)`, Winamax `(5/10/20)` (ante en
/// premier), GGPoker `(400/800(100))`.
fn header_blinds(header: &str) -> Option<(f64, f64, f64)> {
    let mut rest = header;
    while let Some(i) = rest.find('(') {
        let after = &rest[i + 1..];
        let end = after.find(')').unwrap_or(after.len());
        let inner = &after[..end];
        let parts: Vec<&str> = inner.split('/').collect();
        if parts.len() >= 2 && parts.iter().all(|p| p.chars().any(|c| c.is_ascii_digit())) {
            // GGPoker : « 400/800(100 » (l'ante entre parenthèses).
            if let Some((bb, ante)) = parts[parts.len() - 1].split_once('(') {
                let sb = amount(parts[parts.len() - 2])?;
                return Some((amount(ante).unwrap_or(0.0), sb, amount(bb)?));
            }
            let nums: Vec<f64> = parts.iter().filter_map(|p| amount(p)).collect();
            return match nums.as_slice() {
                [sb, bb] => Some((0.0, *sb, *bb)),
                [ante, sb, bb] => Some((*ante, *sb, *bb)),
                _ => None,
            };
        }
        rest = &after[end.min(after.len())..];
    }
    None
}

#[derive(Debug, Clone)]
struct RawSeat {
    seat: u8,
    name: String,
    stack: f64,
}

/// `Seat 3: Nom (1500 in chips)` ou `Seat 3: Nom (500, 0.92€ bounty)`.
fn parse_seat(line: &str) -> Option<RawSeat> {
    let rest = line.strip_prefix("Seat ")?;
    let (num, rest) = rest.split_once(':')?;
    let seat: u8 = num.trim().parse().ok()?;
    let rest = rest.trim();
    if rest.contains("is sitting out") || rest.contains("out of hand") {
        return None;
    }
    // La dernière parenthèse ouvrante suivie d'un montant.
    let mut cut = None;
    for (i, _) in rest.match_indices(" (") {
        let after = &rest[i + 2..];
        if after
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_digit() || "$€£¥".contains(c))
        {
            cut = Some(i);
        }
    }
    let i = cut?;
    let name = rest[..i].trim().to_owned();
    let inside = &rest[i + 2..];
    let inside = inside.split([')', ',']).next()?;
    let inside = inside.split(" in chips").next()?;
    let stack = amount(inside)?;
    (!name.is_empty() && stack > 0.0).then_some(RawSeat { seat, name, stack })
}

/// Sépare le nom du joueur du reste de la ligne, en essayant les noms connus (le plus long
/// d'abord, les noms peuvent contenir des espaces).
fn split_actor<'a>(line: &'a str, names: &[String]) -> Option<(usize, &'a str)> {
    let mut best: Option<(usize, usize)> = None;
    for (i, name) in names.iter().enumerate() {
        if line.starts_with(name.as_str()) {
            let rest = &line[name.len()..];
            if (rest.starts_with(':') || rest.starts_with(' '))
                && best.is_none_or(|(_, len)| name.len() > len)
            {
                best = Some((i, name.len()));
            }
        }
    }
    let (i, len) = best?;
    let rest = line[len..].trim_start_matches(':').trim();
    Some((i, rest))
}

enum Line {
    Action(Action),
    PostSmall(f64),
    PostBig(f64),
    PostAnte(f64),
    Show(Vec<Card>),
    Muck,
    Ignore,
}

fn parse_action(rest: &str, chips: impl Fn(f64) -> u64) -> Line {
    let all_in = rest.contains("all-in") || rest.contains("all in");
    let first_amount = |s: &str| s.split_whitespace().find_map(amount);
    if rest.starts_with("folds") {
        Line::Action(Action::Fold)
    } else if rest.starts_with("checks") {
        Line::Action(Action::Check)
    } else if let Some(r) = rest.strip_prefix("calls") {
        let _ = r;
        Line::Action(if all_in { Action::AllIn } else { Action::Call })
    } else if let Some(r) = rest.strip_prefix("bets") {
        match first_amount(r) {
            _ if all_in => Line::Action(Action::AllIn),
            Some(a) => Line::Action(Action::Bet(Chips(chips(a)))),
            None => Line::Ignore,
        }
    } else if let Some(r) = rest.strip_prefix("raises") {
        let to = r.split(" to ").nth(1).and_then(first_amount);
        match to.or_else(|| first_amount(r)) {
            _ if all_in => Line::Action(Action::AllIn),
            Some(a) => Line::Action(Action::Raise(Chips(chips(a)))),
            None => Line::Ignore,
        }
    } else if let Some(r) = rest.strip_prefix("posts small blind") {
        first_amount(r).map_or(Line::Ignore, Line::PostSmall)
    } else if let Some(r) = rest.strip_prefix("posts big blind") {
        first_amount(r).map_or(Line::Ignore, Line::PostBig)
    } else if let Some(r) = rest
        .strip_prefix("posts the ante")
        .or_else(|| rest.strip_prefix("posts ante"))
    {
        first_amount(r).map_or(Line::Ignore, Line::PostAnte)
    } else if rest.starts_with("shows") || rest.starts_with("showed") {
        bracket_cards(rest).map_or(Line::Ignore, Line::Show)
    } else if rest.starts_with("mucks") || rest.starts_with("doesn't show") {
        Line::Muck
    } else {
        Line::Ignore
    }
}

fn parse_hand(lines: &[String]) -> Result<ImportedHand, ImportError> {
    let header = lines.first().cloned().unwrap_or_default();
    let id = hand_id(&header);
    let fail = |reason: &str| ImportError::Hand {
        hand: id.clone(),
        reason: reason.to_owned(),
    };

    let room = if header.starts_with("PokerStars") {
        Room::PokerStars
    } else if header.starts_with("Winamax") {
        Room::Winamax
    } else {
        Room::Other
    };
    let lower = header.to_lowercase();
    if lower.contains("omaha") || lower.contains("stud") || lower.contains("razz") {
        return Err(fail("variante autre que le Texas Hold'em"));
    }
    let structure = if lower.contains("pot limit") || lower.contains("pot-limit") {
        BettingStructure::PotLimit
    } else if lower.contains("no limit") || lower.contains("no-limit") {
        BettingStructure::NoLimit
    } else if lower.contains("limit") {
        BettingStructure::FIXED_LIMIT_TOURNAMENT
    } else {
        BettingStructure::NoLimit
    };
    let tournament = lower.contains("tournament") || lower.contains("tournoi");
    let expresso = lower.contains("expresso") || lower.contains("spin");

    // Table, bouton, taille.
    let mut table_size: Option<u8> = None;
    let mut button: Option<u8> = None;
    let mut seats: Vec<RawSeat> = Vec::new();
    let mut body_start = lines.len();
    for (i, line) in lines.iter().enumerate().skip(1) {
        if line.starts_with("Table") {
            if let Some(m) = line.find("-max") {
                let digits: String = line[..m]
                    .chars()
                    .rev()
                    .take_while(char::is_ascii_digit)
                    .collect::<Vec<_>>()
                    .into_iter()
                    .rev()
                    .collect();
                table_size = digits.parse().ok();
            }
            if let Some(b) = line.split("Seat #").nth(1) {
                button = b.split_whitespace().next().and_then(|n| n.parse().ok());
            }
        } else if line.starts_with("Seat ") {
            if let Some(s) = parse_seat(line) {
                seats.push(s);
            }
        } else if line.starts_with("***") || !seats.is_empty() {
            body_start = i;
            break;
        }
    }
    if seats.len() < 2 {
        return Err(fail("moins de deux joueurs servis"));
    }
    seats.sort_by_key(|s| s.seat);
    let names: Vec<String> = seats.iter().map(|s| s.name.clone()).collect();

    let from_header = header_blinds(&header);
    let mut decimals =
        from_header.is_some_and(|(a, s, b)| [a, s, b].iter().any(|x| x.fract() != 0.0));
    for l in &lines[1..body_start.min(lines.len())] {
        if l.starts_with("Seat ") && has_decimals(l) {
            decimals = true;
        }
    }
    let scale: u64 = if decimals { 100 } else { 1 };
    let chips = |x: f64| (x * scale as f64).round() as u64;

    // Le corps : actions, cartes, abattage.
    let mut hero: Option<(usize, HoleCards)> = None;
    let mut events: Vec<HandEvent> = Vec::new();
    let mut small: Option<(usize, f64)> = None;
    let mut big: Option<(usize, f64)> = None;
    let mut antes: Vec<(usize, f64)> = Vec::new();
    for line in &lines[body_start.min(lines.len())..] {
        if line.starts_with("*** SUMMARY") || line.starts_with("*** RÉSUMÉ") {
            break;
        }
        if line.starts_with("***") {
            let l = line.to_uppercase();
            let street = (l.contains("FLOP") && !l.contains("PRE"))
                || l.contains("TURN")
                || l.contains("RIVER");
            if street {
                if l.contains("FIRST") || l.contains("SECOND") {
                    return Err(fail(
                        "main jouée deux fois (run it twice) : non prise en charge",
                    ));
                }
                let cards = bracket_cards(line).ok_or_else(|| fail("board illisible"))?;
                events.push(HandEvent::Board { cards });
            }
            continue;
        }
        if let Some(rest) = line.strip_prefix("Dealt to ") {
            if let Some((i, after)) = split_actor(rest, &names)
                && let Some(c) = bracket_cards(after)
                && c.len() == 2
                && let Some(h) = HoleCards::new(c[0], c[1])
            {
                hero = Some((i, h));
            }
            continue;
        }
        let Some((i, rest)) = split_actor(line, &names) else {
            continue;
        };
        let seat = seats[i].seat;
        match parse_action(rest, chips) {
            Line::Action(action) => events.push(HandEvent::Action { seat, action }),
            Line::PostSmall(a) => small = Some((i, a)),
            Line::PostBig(a) => big = Some((i, a)),
            Line::PostAnte(a) => antes.push((i, a)),
            Line::Show(c) => {
                if c.len() == 2
                    && let Some(h) = HoleCards::new(c[0], c[1])
                {
                    events.push(HandEvent::Show { seat, cards: h });
                }
            }
            Line::Muck => events.push(HandEvent::Muck { seat }),
            Line::Ignore => {}
        }
    }

    // Blindes : celles postées, sinon celles de l'en-tête.
    let sb = small
        .map(|s| s.1)
        .or(from_header.map(|h| h.1))
        .ok_or_else(|| fail("petite blinde introuvable"))?;
    let bb = big
        .map(|b| b.1)
        .or(from_header.map(|h| h.2))
        .ok_or_else(|| fail("grosse blinde introuvable"))?;
    let mut blinds = Blinds::new(chips(sb), chips(bb));
    if !antes.is_empty() {
        let max = antes.iter().map(|a| a.1).fold(0.0, f64::max);
        let only_bb = antes.len() == 1 && big.is_some_and(|b| b.0 == antes[0].0);
        blinds = blinds.with_ante(if only_bb {
            Ante::BigBlind(Chips(chips(max)))
        } else {
            Ante::Classic(Chips(chips(max)))
        });
    } else if let Some((ante, _, _)) = from_header
        && ante > 0.0
    {
        blinds = blinds.with_ante(Ante::Classic(Chips(chips(ante))));
    }

    // Bouton : celui de la table s'il est servi, sinon déduit de la petite blinde.
    let seated = |s: u8| seats.iter().any(|r| r.seat == s);
    let button_seat = match button {
        Some(b) if seated(b) => b,
        _ => {
            let sb_idx = small
                .map(|s| s.0)
                .ok_or_else(|| fail("bouton introuvable"))?;
            if seats.len() == 2 {
                seats[sb_idx].seat
            } else {
                seats[(sb_idx + seats.len() - 1) % seats.len()].seat
            }
        }
    };

    let n = seats.len() as u8;
    let size = table_size.unwrap_or(n).max(n).clamp(2, 10);
    // Les sit & go à 3 de Winamax sont des Expresso, même quand l'en-tête ne le dit pas.
    let game_type = if size <= 3 && (expresso || (tournament && room == Room::Winamax)) {
        GameType::Expresso
    } else if tournament {
        GameType::Tournament
    } else {
        GameType::CashGame
    };
    let structure = if game_type == GameType::CashGame
        && matches!(structure, BettingStructure::FixedLimit { .. })
    {
        BettingStructure::FIXED_LIMIT_CASH
    } else {
        structure
    };
    let format = GameFormat::new(game_type, size, structure).map_err(|e| fail(&e.to_string()))?;

    let record = HandRecord {
        id: id.clone(),
        format,
        blinds: imported(blinds),
        button_seat,
        seats: seats
            .iter()
            .enumerate()
            .map(|(i, s)| SeatRecord {
                seat: s.seat,
                player: s.name.clone(),
                stack: imported(Chips(chips(s.stack))),
                hole_cards: hero
                    .filter(|(h, _)| *h == i)
                    .map(|(_, cards)| imported(cards)),
            })
            .collect(),
        events: events.into_iter().map(imported).collect(),
    };
    Ok(ImportedHand {
        record,
        hero: hero.map(|(i, _)| names[i].clone()),
        room,
        scale,
    })
}

fn imported<T>(v: T) -> Observed<T> {
    Observed::certain(v, Source::Import)
}

/// Un exemple d'historique PokerStars (tournoi 6-max), pour les démos et les tests.
pub const SAMPLE_POKERSTARS: &str = "PokerStars Hand #250000000001: Tournament #3500000001, $4.75+$0.25 USD Hold'em No Limit - Level IV (50/100) - 2026/10/02 21:14:05 ET
Table '3500000001 1' 6-max Seat #2 is the button
Seat 1: Villain One (2350 in chips)
Seat 2: Bouton (1800 in chips)
Seat 3: Petite (3000 in chips)
Seat 4: Hero (1500 in chips)
Seat 5: Joueur 5 (2200 in chips)
Seat 6: Joueur 6 (900 in chips) is sitting out
Petite: posts small blind 50
Hero: posts big blind 100
*** HOLE CARDS ***
Dealt to Hero [Ah Qd]
Joueur 5: folds
Villain One: raises 150 to 250
Bouton: folds
Petite: folds
Hero: raises 1250 to 1500 and is all-in
Villain One: calls 1250
*** FLOP *** [Qs 7h 2c]
*** TURN *** [Qs 7h 2c] [Kd]
*** RIVER *** [Qs 7h 2c Kd] [3s]
*** SHOW DOWN ***
Hero: shows [Ah Qd] (a pair of Queens)
Villain One: shows [Kh Kc] (three of a kind, Kings)
Villain One collected 3050 from pot
*** SUMMARY ***
Total pot 3050 | Rake 0
Board [Qs 7h 2c Kd 3s]
Seat 1: Villain One showed [Kh Kc] and won (3050) with three of a kind, Kings
Seat 4: Hero (big blind) showed [Ah Qd] and lost with a pair of Queens
";

/// Un exemple d'historique Winamax (Expresso), pour les démos et les tests.
pub const SAMPLE_WINAMAX: &str = "Winamax Poker - Tournament \"Expresso\" buyIn: 4.60€ + 0.40€ level: 2 - HandId: #2938475-12-1790000000 - Holdem no limit (15/30) - 2026/10/02 19:30:12 UTC
Table: 'Expresso(912345678)#0' 3-max (real money) Seat #1 is the button
Seat 1: Hero (480)
Seat 2: Rival A (510)
Seat 3: Rival B (510)
*** ANTE/BLINDS ***
Rival A posts small blind 15
Rival B posts big blind 30
Dealt to Hero [7h 7c]
*** PRE-FLOP ***
Hero raises 30 to 60
Rival A folds
Rival B calls 30
*** FLOP *** [Kd 7s 2h]
Rival B checks
Hero bets 60
Rival B raises 120 to 180
Hero raises 240 to 420 and is all-in
Rival B calls 240
*** TURN *** [Kd 7s 2h][4c]
*** RIVER *** [Kd 7s 2h 4c][Js]
*** SHOW DOWN ***
Rival B shows [Kh Qs] (One pair : Kings)
Hero shows [7h 7c] (Trips of 7)
Hero collected 975 from pot
*** SUMMARY ***
Total pot 975 | No rake
Board: [Kd 7s 2h 4c Js]
Seat 1: Hero (button) showed [7h 7c] and won 975 with Trips of 7
";

/// L'identifiant de la main dans l'en-tête : `#123456789`.
fn hand_id(header: &str) -> String {
    header
        .split('#')
        .nth(1)
        .map(|s| {
            s.chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '-')
                .collect::<String>()
        })
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "sans-identifiant".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::history::Outcome;

    const CASH: &str = "PokerStars Hand #250000000777:  Hold'em No Limit ($0.05/$0.10 USD) - 2026/10/03 10:00:00 ET
Table 'Ariadne II' 6-max Seat #6 is the button
Seat 1: A1 ($10.00 in chips)
Seat 2: B2 ($8.35 in chips)
Seat 6: Hero ($12.40 in chips)
A1: posts small blind $0.05
B2: posts big blind $0.10
*** HOLE CARDS ***
Dealt to Hero [Td Th]
Hero: raises $0.20 to $0.30
A1: folds
B2: calls $0.20
*** FLOP *** [9c 4d 2s]
B2: checks
Hero: bets $0.40
B2: folds
Uncalled bet ($0.40) returned to Hero
Hero collected $0.65 from pot
*** SUMMARY ***
";

    #[test]
    fn pokerstars_tournament() {
        let h = import_hand(SAMPLE_POKERSTARS).unwrap();
        assert_eq!(h.room, Room::PokerStars);
        assert_eq!(h.hero.as_deref(), Some("Hero"));
        assert_eq!(h.record.id, "250000000001");
        assert_eq!(h.record.format, GameFormat::TOURNAMENT_6MAX);
        assert_eq!(h.record.seats.len(), 5, "le joueur absent est ignoré");
        assert_eq!(h.record.button_seat, 2);
        let r = h.record.replay().unwrap();
        let Outcome::Settled(s) = r.outcome else {
            panic!("{:?}", r.outcome)
        };
        // Héros siège 4 (index 3) : perd son tapis.
        assert_eq!(s.net[3], -1500);
        assert_eq!(s.net[0], 1550);
    }

    #[test]
    fn winamax_expresso() {
        let h = import_hand(SAMPLE_WINAMAX).unwrap();
        assert_eq!(h.room, Room::Winamax);
        assert_eq!(h.record.format, GameFormat::EXPRESSO);
        assert_eq!(
            h.record.format.specialist(),
            Some(crate::Specialist::Expresso3Max)
        );
        let r = h.record.replay().unwrap();
        let Outcome::Settled(s) = r.outcome else {
            panic!("{:?}", r.outcome)
        };
        assert_eq!(s.net[0], 495);
    }

    #[test]
    fn cash_game_in_cents() {
        let h = import_hand(CASH).unwrap();
        assert_eq!(h.scale, 100);
        assert_eq!(h.record.format.game_type, GameType::CashGame);
        assert_eq!(h.record.blinds.value, Blinds::new(5, 10));
        assert_eq!(h.record.seats[2].stack.value, Chips(1240));
        let r = h.record.replay().unwrap();
        let Outcome::Settled(s) = r.outcome else {
            panic!("{:?}", r.outcome)
        };
        assert_eq!(s.net[2], 35);
    }

    #[test]
    fn several_hands_and_errors() {
        let text = format!(
            "{SAMPLE_POKERSTARS}\n\n{SAMPLE_WINAMAX}\n\nPokerStars Hand #9: Omaha Pot Limit (1/2)\n"
        );
        let r = import_text(&text);
        assert_eq!(r.hands.len(), 2);
        assert_eq!(r.errors.len(), 1);
        assert!(import_text("bonjour").errors.contains(&ImportError::Empty));
    }

    #[test]
    fn amounts() {
        assert_eq!(amount("1,500"), Some(1500.0));
        assert_eq!(amount("$0.25"), Some(0.25));
        assert_eq!(amount("0,92€"), Some(0.92));
        assert_eq!(amount("(3050)"), Some(3050.0));
        assert!(has_decimals("($0.05/$0.10 USD)"));
        assert!(!has_decimals("(1,500 in chips)"));
        assert_eq!(
            header_blinds("Holdem no limit (5/10/20)"),
            Some((5.0, 10.0, 20.0))
        );
        assert_eq!(
            header_blinds("Level10(400/800(100))"),
            Some((100.0, 400.0, 800.0))
        );
    }
}
