//! Import des historiques texte de PokerStars vers des [`HandRecord`].

use std::collections::BTreeSet;

use stratyx_poker::card::parse_cards;
use stratyx_poker::game::{ActionKind, Ante, BettingStructure, Blinds, Phase};
use stratyx_poker::history::{Outcome, RecordError, Replay};
use stratyx_poker::pot::build_pots;
use stratyx_poker::{
    Action, Card, GameFormat, GameType, HandEvent, HandMeta, HandRecord, HoleCards, SeatRecord,
};
use stratyx_shared::{Chips, Observed, Source};

use super::amount::{currency_from_symbol, is_currency_symbol, parse_amount};
use super::datetime::parse_header_date;
use super::{
    BuyIn, ImportError, ImportErrorKind as K, ImportOptions, ImportedHand, POKERSTARS_SITE,
    TournamentInfo,
};

/// Lit toutes les mains d'un fichier d'historiques PokerStars. Chaque main est lue et
/// validée indépendamment : une main illisible n'empêche pas de lire les suivantes.
pub fn import_pokerstars(
    text: &str,
    options: &ImportOptions,
) -> Vec<Result<ImportedHand, ImportError>> {
    split_hands(text)
        .iter()
        .map(|chunk| parse_hand(chunk, options))
        .collect()
}

/// Lit un texte contenant exactement une main.
pub fn import_pokerstars_hand(
    text: &str,
    options: &ImportOptions,
) -> Result<ImportedHand, ImportError> {
    let mut all = import_pokerstars(text, options);
    match all.len() {
        1 => all.pop().expect("une main"),
        0 => Err(ImportError {
            hand_id: None,
            line: 0,
            kind: K::NoHand,
        }),
        n => Err(ImportError {
            hand_id: None,
            line: 0,
            kind: K::SeveralHands(n),
        }),
    }
}

/// Observation issue d'un historique importé : certaine.
fn ok<T>(value: T) -> Observed<T> {
    Observed::certain(value, Source::Import)
}

/// Lignes d'une main : (numéro de ligne à partir de 1, contenu sans espaces finaux).
type Chunk<'a> = Vec<(usize, &'a str)>;

fn is_header(line: &str) -> bool {
    line.starts_with("PokerStars ") && (line.contains(" Hand #") || line.contains(" Game #"))
}

fn split_hands(text: &str) -> Vec<Chunk<'_>> {
    let mut chunks: Vec<Chunk<'_>> = Vec::new();
    for (i, raw) in text.lines().enumerate() {
        let line = raw.trim_start_matches('\u{feff}').trim_end();
        if is_header(line) {
            chunks.push(vec![(i + 1, line)]);
        } else if !line.is_empty()
            && let Some(c) = chunks.last_mut()
        {
            c.push((i + 1, line));
        }
    }
    chunks
}

// --- En-tête ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Limit {
    No,
    Pot,
    Fixed,
}

struct Header {
    id: String,
    tournament: Option<TournamentInfo>,
    limit: Limit,
    /// Montants entre parenthèses : blindes (NL, PL) ou petite et grosse mise (FL).
    stakes: (Chips, Chips),
    money: bool,
    currency: Option<String>,
    played_at_ms: Option<u64>,
}

fn parse_header(line: &str) -> Result<Header, K> {
    let bad = |m: &str| K::Header(m.to_string());
    let hash = line.find('#').ok_or_else(|| bad("numéro de main absent"))?;
    let (id, rest) = line[hash + 1..]
        .split_once(':')
        .ok_or_else(|| bad("« : » attendu après le numéro de main"))?;
    let id = id.trim().to_string();
    if id.is_empty() {
        return Err(bad("numéro de main vide"));
    }
    let rest = rest.trim();
    let segments: Vec<&str> = rest.split(" - ").collect();
    let game = segments[0];

    let holdem = game
        .find("Hold'em")
        .ok_or_else(|| K::UnsupportedGame(game.to_string()))?;
    if [
        "Omaha",
        "Stud",
        "Razz",
        "Draw",
        "HORSE",
        "Mixed",
        "Courchevel",
    ]
    .iter()
    .any(|g| game.contains(g))
    {
        return Err(K::UnsupportedGame(game.to_string()));
    }
    let after = game[holdem + "Hold'em".len()..].trim_start();
    let limit = if after.starts_with("No Limit") {
        Limit::No
    } else if after.starts_with("Pot Limit") {
        Limit::Pot
    } else if after.starts_with("Limit") {
        Limit::Fixed
    } else {
        return Err(K::UnsupportedGame(game.to_string()));
    };

    let tournament = if let Some(t) = game.strip_prefix("Tournament #") {
        let (tid, buy_in_text) = match t.split_once(',') {
            Some((a, b)) => (a.trim(), b[..b.find("Hold'em").unwrap_or(b.len())].trim()),
            None => (t[..t.find(' ').unwrap_or(t.len())].trim(), ""),
        };
        let level_segment = segments.iter().skip(1).find(|s| s.contains("Level "));
        let level = level_segment.and_then(|s| {
            let l = &s[s.find("Level ")? + "Level ".len()..];
            Some(l[..l.find(" (").unwrap_or(l.len())].trim().to_string())
        });
        Some(TournamentInfo {
            id: tid.to_string(),
            buy_in: parse_buy_in(buy_in_text)?,
            level,
        })
    } else {
        None
    };

    // Montants entre parenthèses : dans le segment du jeu (cash) ou du niveau (tournoi).
    let stakes_segment = if tournament.is_some() {
        segments
            .iter()
            .skip(1)
            .find(|s| s.contains("Level "))
            .copied()
            .ok_or_else(|| bad("niveau de blindes absent"))?
    } else {
        game
    };
    let open = stakes_segment
        .rfind('(')
        .ok_or_else(|| bad("blindes absentes"))?;
    let close = stakes_segment[open..]
        .find(')')
        .ok_or_else(|| bad("blindes absentes"))?;
    let inside = &stakes_segment[open + 1..open + close];
    let mut parts = inside.split_whitespace();
    let amounts = parts.next().ok_or_else(|| bad("blindes absentes"))?;
    let code = parts
        .next()
        .filter(|c| c.len() == 3 && c.bytes().all(|b| b.is_ascii_uppercase()));
    let symbol = amounts.chars().find(|c| is_currency_symbol(*c));
    let currency = code
        .map(str::to_string)
        .or_else(|| symbol.and_then(currency_from_symbol).map(str::to_string));
    let money = tournament.is_none() && currency.is_some();
    let (a, b) = amounts
        .split_once('/')
        .ok_or_else(|| bad("blindes : « a/b » attendu"))?;
    let amount = |s: &str| parse_amount(s, money).ok_or_else(|| K::InvalidAmount(s.to_string()));
    let stakes = (amount(a)?, amount(b)?);

    let played_at_ms = segments
        .iter()
        .skip(1)
        .rev()
        .find_map(|s| parse_header_date(s));

    Ok(Header {
        id,
        tournament,
        limit,
        stakes,
        money,
        currency: if money { currency } else { None },
        played_at_ms,
    })
}

/// « $1.40+$0.10 USD », « Freeroll », « 10+0 » (jetons fictifs).
fn parse_buy_in(text: &str) -> Result<Option<BuyIn>, K> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(None);
    }
    if text.eq_ignore_ascii_case("freeroll") {
        return Ok(Some(BuyIn {
            total: 0,
            fee: 0,
            currency: None,
        }));
    }
    let mut parts = text.split_whitespace();
    let amounts = parts.next().unwrap_or("");
    let symbol = amounts.chars().find(|c| is_currency_symbol(*c));
    let currency = parts
        .next()
        .map(str::to_string)
        .or_else(|| symbol.and_then(currency_from_symbol).map(str::to_string));
    let money = currency.is_some() || amounts.contains('.');
    let values: Option<Vec<u64>> = amounts
        .split('+')
        .map(|a| parse_amount(a, money).map(Chips::get))
        .collect();
    let values = values.ok_or_else(|| K::InvalidAmount(text.to_string()))?;
    let fee = if values.len() > 1 {
        values.last().copied().unwrap_or(0)
    } else {
        0
    };
    Ok(Some(BuyIn {
        total: values.iter().sum(),
        fee,
        currency,
    }))
}

struct TableLine {
    name: Option<String>,
    size: Option<u8>,
    button: u8,
}

/// « Table 'Aase III' 6-max Seat #4 is the button ».
fn parse_table(line: &str) -> Result<TableLine, K> {
    let syntax = || K::Syntax(line.to_string());
    let rest = line.strip_prefix("Table '").ok_or_else(syntax)?;
    let seat_pos = rest.rfind(" Seat #").ok_or_else(syntax)?;
    let before = &rest[..seat_pos];
    let quote = before.rfind('\'').ok_or_else(syntax)?;
    let name = &before[..quote];
    let size = before[quote + 1..]
        .split_whitespace()
        .find_map(|t| t.strip_suffix("-max"))
        .map(|n| n.parse::<u8>().map_err(|_| syntax()))
        .transpose()?;
    let button = rest[seat_pos + " Seat #".len()..]
        .split_whitespace()
        .next()
        .and_then(|b| b.parse().ok())
        .ok_or_else(syntax)?;
    Ok(TableLine {
        name: (!name.is_empty()).then(|| name.to_string()),
        size,
        button,
    })
}

struct SeatLine {
    seat: u8,
    name: String,
    stack: Chips,
    /// Assis sans être servi (« is sitting out », « out of hand »).
    out: bool,
}

/// « Seat 3: Hero ($10 in chips) », « Seat 2: X (5400 in chips, $0.50 bounty) is sitting out ».
fn parse_seat(line: &str, money: bool) -> Result<SeatLine, K> {
    let syntax = || K::Syntax(line.to_string());
    let rest = line.strip_prefix("Seat ").ok_or_else(syntax)?;
    let (n, rest) = rest.split_once(": ").ok_or_else(syntax)?;
    let seat: u8 = n.trim().parse().map_err(|_| syntax())?;
    let chips = rest.find(" in chips").ok_or_else(syntax)?;
    let head = &rest[..chips];
    let open = head.rfind(" (").ok_or_else(syntax)?;
    let name = head[..open].to_string();
    let amount = &head[open + 2..];
    let stack = parse_amount(amount, money).ok_or_else(|| K::InvalidAmount(amount.to_string()))?;
    let tail = &rest[chips..];
    let out = tail.contains("is sitting out") || tail.contains("out of hand");
    if name.is_empty() {
        return Err(syntax());
    }
    Ok(SeatLine {
        seat,
        name,
        stack,
        out,
    })
}

fn bracket_cards(s: &str) -> Result<Vec<Card>, K> {
    let open = s.find('[').ok_or_else(|| K::InvalidCards(s.to_string()))?;
    let close = s[open..]
        .find(']')
        .ok_or_else(|| K::InvalidCards(s.to_string()))?;
    parse_cards(&s[open + 1..open + close]).map_err(|_| K::InvalidCards(s.to_string()))
}

fn last_bracket_cards(s: &str) -> Result<Vec<Card>, K> {
    let open = s.rfind('[').ok_or_else(|| K::InvalidCards(s.to_string()))?;
    bracket_cards(&s[open..])
}

fn hole_cards(s: &str) -> Result<HoleCards, K> {
    match bracket_cards(s)?.as_slice() {
        [a, b] => HoleCards::new(*a, *b).ok_or_else(|| K::InvalidCards(s.to_string())),
        _ => Err(K::InvalidCards(s.to_string())),
    }
}

// --- Corps de la main ---------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PostKind {
    Ante,
    Small,
    Big,
}

struct Post {
    line: usize,
    seat: u8,
    kind: PostKind,
    amount: Chips,
    all_in: bool,
}

#[derive(Debug, Clone, Copy)]
enum Verb {
    Fold,
    Check,
    Call(Chips),
    Bet(Chips),
    Raise(Chips),
}

struct TextAction {
    line: usize,
    verb: Verb,
    all_in: bool,
}

/// Pseudonymes connus, du plus long au plus court (un nom peut en prolonger un autre).
struct Names(Vec<(String, u8)>);

impl Names {
    /// `line` commence par un pseudonyme suivi de `sep` : (siège, suite).
    fn prefix<'l>(&self, line: &'l str, sep: &str) -> Option<(u8, &'l str)> {
        self.0.iter().find_map(|(name, seat)| {
            line.strip_prefix(name.as_str())
                .and_then(|r| r.strip_prefix(sep))
                .map(|r| (*seat, r))
        })
    }

    fn exact(&self, name: &str) -> Option<u8> {
        self.0.iter().find(|(n, _)| n == name).map(|(_, s)| *s)
    }
}

/// Montant suivi éventuellement de « and is all-in ».
fn amount_all_in(s: &str, money: bool) -> Result<(Chips, bool), K> {
    let (a, all_in) = match s.strip_suffix(" and is all-in") {
        Some(a) => (a, true),
        None => (s, false),
    };
    let a = a.trim();
    parse_amount(a, money)
        .map(|c| (c, all_in))
        .ok_or_else(|| K::InvalidAmount(a.to_string()))
}

/// Déclarations sans effet sur le jeu.
const NOISE: &[&str] = &[
    "doesn't show hand",
    "sits out",
    "is sitting out",
    "sitting out",
    "has timed out",
    "has timed out while disconnected",
    "has timed out while being disconnected",
    "is disconnected",
    "is connected",
    "has returned",
    "leaves the table",
    "is away",
];

struct Parsed {
    posts: Vec<Post>,
    actions: Vec<TextAction>,
    events: Vec<HandEvent>,
    event_lines: Vec<usize>,
    dealt: Vec<(u8, Option<HoleCards>)>,
    uncalled: Option<(usize, u8, Chips)>,
    collected: Vec<(u8, Chips)>,
    rake: Option<Chips>,
    participants: BTreeSet<u8>,
}

fn parse_hand(chunk: &Chunk<'_>, options: &ImportOptions) -> Result<ImportedHand, ImportError> {
    let (header_line, header_text) = chunk[0];
    let header = parse_header(header_text).map_err(|kind| ImportError {
        hand_id: None,
        line: header_line,
        kind,
    })?;
    let id = header.id.clone();
    let fail = |line: usize, kind: K| ImportError {
        hand_id: Some(id.clone()),
        line,
        kind,
    };

    let mut lines = chunk[1..].iter().copied().peekable();
    let (table_line, table_text) = lines
        .next()
        .ok_or_else(|| fail(header_line, K::Missing("ligne « Table »")))?;
    let table = parse_table(table_text).map_err(|k| fail(table_line, k))?;

    // PokerStars liste les sièges dans l'ordre, de 1 à la taille de la table : un numéro hors
    // de la table, en double ou dans le désordre trahit un fichier abîmé (« Seat 91 » à une
    // table 6-max était accepté, et l'ordre des sièges changeait à l'aller-retour).
    let last_seat = table.size.unwrap_or(10);
    let mut seats: Vec<SeatLine> = Vec::new();
    while let Some(&(n, l)) = lines.peek() {
        if !(l.starts_with("Seat ") && l.contains(" in chips")) {
            break;
        }
        let seat = parse_seat(l, header.money).map_err(|k| fail(n, k))?;
        if !(1..=last_seat).contains(&seat.seat) {
            return Err(fail(
                n,
                K::Inconsistent(format!(
                    "siège {} hors de la table (1 à {last_seat})",
                    seat.seat
                )),
            ));
        }
        if let Some(prev) = seats.last()
            && seat.seat <= prev.seat
        {
            return Err(fail(
                n,
                K::Inconsistent(format!(
                    "siège {} après le siège {} (sièges en double ou dans le désordre)",
                    seat.seat, prev.seat
                )),
            ));
        }
        seats.push(seat);
        lines.next();
    }
    if seats.is_empty() {
        return Err(fail(table_line, K::Missing("liste des sièges")));
    }
    let mut names: Vec<(String, u8)> = seats.iter().map(|s| (s.name.clone(), s.seat)).collect();
    names.sort_by(|a, b| b.0.len().cmp(&a.0.len()).then(a.0.cmp(&b.0)));
    let names = Names(names);

    let parsed = parse_body(lines, &names, header.money).map_err(|(n, k)| fail(n, k))?;

    // Joueurs servis : ceux qui ne sont pas absents, et les absents qui ont misé ou agi
    // (en tournoi, un joueur absent paie ses blindes et se couche automatiquement).
    let dealt_in: Vec<&SeatLine> = seats
        .iter()
        .filter(|s| !s.out || parsed.participants.contains(&s.seat))
        .collect();

    let hero = parsed
        .dealt
        .iter()
        .find(|(_, c)| c.is_some())
        .or_else(|| (parsed.dealt.len() == 1).then(|| &parsed.dealt[0]))
        .map(|(s, _)| *s);

    let seat_records: Vec<SeatRecord> = dealt_in
        .iter()
        .map(|s| SeatRecord {
            seat: s.seat,
            player: s.name.clone(),
            stack: ok(s.stack),
            hole_cards: parsed
                .dealt
                .iter()
                .find(|(seat, c)| *seat == s.seat && c.is_some())
                .and_then(|(_, c)| *c)
                .map(ok),
        })
        .collect();

    // Blindes et antes.
    let big_post = parsed.posts.iter().find(|p| p.kind == PostKind::Big);
    let small_post = parsed.posts.iter().find(|p| p.kind == PostKind::Small);
    let (small_blind, big_blind) = match header.limit {
        Limit::Fixed => {
            let sb = small_post
                .map(|p| p.amount)
                .ok_or_else(|| fail(header_line, K::Missing("petite blinde")))?;
            (sb, header.stakes.0)
        }
        Limit::No | Limit::Pot => header.stakes,
    };
    let antes: Vec<&Post> = parsed
        .posts
        .iter()
        .filter(|p| p.kind == PostKind::Ante)
        .collect();
    let ante = match antes.iter().map(|p| p.amount).max() {
        None => Ante::None,
        Some(a) => {
            let only_big_blind = antes.len() == 1
                && dealt_in.len() > 1
                && big_post.is_some_and(|b| b.seat == antes[0].seat);
            if only_big_blind {
                Ante::BigBlind(a)
            } else {
                Ante::Classic(a)
            }
        }
    };
    let blinds = Blinds {
        small_blind,
        big_blind,
        ante,
    };

    let structure = match (header.limit, header.tournament.is_some()) {
        (Limit::No, _) => BettingStructure::NoLimit,
        (Limit::Pot, _) => BettingStructure::PotLimit,
        (Limit::Fixed, false) => BettingStructure::FIXED_LIMIT_CASH,
        (Limit::Fixed, true) => BettingStructure::FIXED_LIMIT_TOURNAMENT,
    };
    let game_type = match (&header.tournament, options.tournament_type) {
        (None, _) => GameType::CashGame,
        (Some(_), GameType::CashGame) => GameType::Tournament,
        (Some(_), t) => t,
    };
    let max_seat = seats.iter().map(|s| s.seat).max().unwrap_or(0);
    let table_size = table
        .size
        .unwrap_or_else(|| max_seat.max(u8::try_from(seats.len()).unwrap_or(u8::MAX)));
    let format = GameFormat::new(game_type, table_size, structure)
        .map_err(|e| fail(table_line, K::Header(e.to_string())))?;

    let record = HandRecord {
        id: header.id.clone(),
        format,
        blinds: ok(blinds),
        button_seat: table.button,
        seats: seat_records,
        events: parsed.events.iter().cloned().map(ok).collect(),
        meta: HandMeta {
            hero_seat: hero,
            played_at_ms: header.played_at_ms,
            session_id: options.session_id.clone(),
            site: Some(POKERSTARS_SITE.to_string()),
            table: table.name.clone(),
            provenance: options.provenance.clone(),
        },
    };

    let replay = record.replay().map_err(|e| {
        let line = match &e {
            RecordError::Rule { index, .. }
            | RecordError::UnknownSeat { index, .. }
            | RecordError::DuplicateCard { index, .. } => parsed
                .event_lines
                .get(*index)
                .copied()
                .unwrap_or(header_line),
            RecordError::ButtonNotSeated(_) => {
                return fail(
                    table_line,
                    K::Unsupported("bouton sur un siège sans joueur servi (bouton mort)".into()),
                );
            }
            _ => header_line,
        };
        fail(line, K::Replay(e))
    })?;

    check_posts(&replay, &parsed, &names).map_err(|(n, k)| fail(n.max(header_line), k))?;
    check_actions(&replay, &parsed).map_err(|(n, k)| fail(n, k))?;
    check_uncalled(&replay, &parsed).map_err(|(n, k)| fail(n, k))?;
    let warnings = collected_warnings(&replay, &parsed, &names);

    Ok(ImportedHand {
        record,
        currency: header.currency,
        tournament: header.tournament,
        rake: parsed.rake,
        warnings,
    })
}

type LineError = (usize, K);

fn parse_body<'a>(
    lines: impl Iterator<Item = (usize, &'a str)>,
    names: &Names,
    money: bool,
) -> Result<Parsed, LineError> {
    let mut p = Parsed {
        posts: Vec::new(),
        actions: Vec::new(),
        events: Vec::new(),
        event_lines: Vec::new(),
        dealt: Vec::new(),
        uncalled: None,
        collected: Vec::new(),
        rake: None,
        participants: BTreeSet::new(),
    };
    let mut in_summary = false;
    for (n, l) in lines {
        let at = |k: K| (n, k);
        if in_summary {
            if let Some(rake) = l.strip_prefix("Total pot ").and_then(|t| {
                let r = &t[t.find("| Rake ")? + "| Rake ".len()..];
                Some(r.split_whitespace().next().unwrap_or(""))
            }) {
                p.rake = Some(
                    parse_amount(rake, money).ok_or_else(|| at(K::InvalidAmount(rake.into())))?,
                );
            }
            continue;
        }
        if l.starts_with("*** ") {
            let board_len = |e: &[HandEvent]| {
                e.iter()
                    .map(|x| match x {
                        HandEvent::Board { cards } => cards.len(),
                        _ => 0,
                    })
                    .sum::<usize>()
            };
            let street = |expected_before: usize, count: usize, p: &mut Parsed| {
                let cards = last_bracket_cards(l).map_err(at)?;
                if cards.len() != count || board_len(&p.events) != expected_before {
                    return Err(at(K::Syntax(l.to_string())));
                }
                p.events.push(HandEvent::Board { cards });
                p.event_lines.push(n);
                Ok(())
            };
            if l == "*** HOLE CARDS ***" || l == "*** SHOW DOWN ***" {
            } else if l.starts_with("*** FLOP ***") {
                street(0, 3, &mut p)?;
            } else if l.starts_with("*** TURN ***") {
                street(3, 1, &mut p)?;
            } else if l.starts_with("*** RIVER ***") {
                street(4, 1, &mut p)?;
            } else if l == "*** SUMMARY ***" {
                in_summary = true;
            } else if l.starts_with("*** FIRST") || l.starts_with("*** SECOND") {
                return Err(at(K::Unsupported(
                    "main jouée deux fois (run it twice)".into(),
                )));
            } else {
                return Err(at(K::Syntax(l.to_string())));
            }
            continue;
        }
        if let Some(rest) = l.strip_prefix("Dealt to ") {
            let found = names.0.iter().find_map(|(name, seat)| {
                let r = rest.strip_prefix(name.as_str())?;
                if r.is_empty() {
                    Some((*seat, None))
                } else {
                    r.strip_prefix(' ')
                        .filter(|c| c.starts_with('['))
                        .map(|c| (*seat, Some(c)))
                }
            });
            let (seat, cards) = found.ok_or_else(|| at(K::UnknownPlayer(rest.to_string())))?;
            let cards = cards.map(hole_cards).transpose().map_err(at)?;
            p.dealt.push((seat, cards));
            continue;
        }
        if let Some(rest) = l.strip_prefix("Uncalled bet (") {
            let (amount, name) = rest
                .split_once(") returned to ")
                .ok_or_else(|| at(K::Syntax(l.to_string())))?;
            let seat = names
                .exact(name)
                .ok_or_else(|| at(K::UnknownPlayer(name.to_string())))?;
            let amount =
                parse_amount(amount, money).ok_or_else(|| at(K::InvalidAmount(amount.into())))?;
            p.uncalled = Some((n, seat, amount));
            continue;
        }
        if let Some((seat, rest)) = names.prefix(l, ": ") {
            statement(&mut p, n, seat, rest, money)?;
            continue;
        }
        if let Some((seat, rest)) = names.prefix(l, " collected ") {
            let amount = rest.split(" from ").next().unwrap_or(rest);
            let amount =
                parse_amount(amount, money).ok_or_else(|| at(K::InvalidAmount(amount.into())))?;
            p.collected.push((seat, amount));
            continue;
        }
        if names.prefix(l, " cashed out").is_some() {
            return Err(at(K::Unsupported("cash-out de la main".into())));
        }
        // Autres lignes (discussion, arrivées, classements de tournoi…) : sans effet.
    }
    Ok(p)
}

/// Déclaration « <joueur>: <texte> ».
fn statement(p: &mut Parsed, n: usize, seat: u8, rest: &str, money: bool) -> Result<(), LineError> {
    let at = |k: K| (n, k);
    let unsupported = |what: &str| at(K::Unsupported(what.to_string()));
    let action = |p: &mut Parsed, verb: Verb, all_in: bool, action: Action| {
        p.events.push(HandEvent::Action { seat, action });
        p.event_lines.push(n);
        p.actions.push(TextAction {
            line: n,
            verb,
            all_in,
        });
        p.participants.insert(seat);
    };
    if let Some(r) = rest.strip_prefix("posts ") {
        let (kind, amount) = if let Some(a) = r.strip_prefix("small blind ") {
            (PostKind::Small, a)
        } else if let Some(a) = r.strip_prefix("big blind ") {
            (PostKind::Big, a)
        } else if let Some(a) = r.strip_prefix("the ante ") {
            (PostKind::Ante, a)
        } else if r.starts_with("small & big blinds") {
            return Err(unsupported(
                "blindes mortes postées (« small & big blinds »)",
            ));
        } else if r.starts_with("straddle") || r.contains("straddle") {
            return Err(unsupported("straddle"));
        } else if r.starts_with("button blind") {
            return Err(unsupported("blinde du bouton"));
        } else {
            return Err(at(K::Syntax(rest.to_string())));
        };
        let (amount, all_in) = amount_all_in(amount, money).map_err(at)?;
        p.posts.push(Post {
            line: n,
            seat,
            kind,
            amount,
            all_in,
        });
        p.participants.insert(seat);
    } else if rest == "folds" || rest.starts_with("folds [") {
        action(p, Verb::Fold, false, Action::Fold);
    } else if rest == "checks" {
        action(p, Verb::Check, false, Action::Check);
    } else if let Some(a) = rest.strip_prefix("calls ") {
        let (amount, all_in) = amount_all_in(a, money).map_err(at)?;
        action(p, Verb::Call(amount), all_in, Action::Call);
    } else if let Some(a) = rest.strip_prefix("bets ") {
        let (amount, all_in) = amount_all_in(a, money).map_err(at)?;
        action(p, Verb::Bet(amount), all_in, Action::Bet(amount));
    } else if let Some(a) = rest.strip_prefix("raises ") {
        let (_, to) = a
            .split_once(" to ")
            .ok_or_else(|| at(K::Syntax(rest.to_string())))?;
        let (to, all_in) = amount_all_in(to, money).map_err(at)?;
        action(p, Verb::Raise(to), all_in, Action::Raise(to));
    } else if let Some(c) = rest.strip_prefix("shows ") {
        // Une seule carte montrée : information partielle, sans effet sur le rejeu.
        if let Ok(cards) = hole_cards(c) {
            p.events.push(HandEvent::Show { seat, cards });
            p.event_lines.push(n);
            p.participants.insert(seat);
        }
    } else if rest.starts_with("mucks hand") {
        p.events.push(HandEvent::Muck { seat });
        p.event_lines.push(n);
        p.participants.insert(seat);
    } else if NOISE.iter().any(|x| rest.starts_with(x)) {
    } else {
        return Err(at(K::Syntax(rest.to_string())));
    }
    Ok(())
}

// --- Contrôles de cohérence avec le moteur -----------------------------------------------

fn kind_name(kind: PostKind) -> &'static str {
    match kind {
        PostKind::Ante => "ante",
        PostKind::Small => "petite blinde",
        PostKind::Big => "grosse blinde",
    }
}

fn check_posts(replay: &Replay, parsed: &Parsed, names: &Names) -> Result<(), LineError> {
    let player_name = |seat: u8| {
        names
            .0
            .iter()
            .find(|(_, s)| *s == seat)
            .map_or_else(String::new, |(n, _)| n.clone())
    };
    let mut matched = vec![false; parsed.posts.len()];
    for a in replay.state.actions() {
        let kind = match a.kind {
            ActionKind::PostAnte => PostKind::Ante,
            ActionKind::PostSmallBlind => PostKind::Small,
            ActionKind::PostBigBlind => PostKind::Big,
            _ => continue,
        };
        let seat = replay.seats[a.player];
        let found = parsed
            .posts
            .iter()
            .enumerate()
            .find(|(i, p)| !matched[*i] && p.seat == seat && p.kind == kind);
        let Some((i, post)) = found else {
            let what = format!("{} de {}", kind_name(kind), player_name(seat));
            return Err((
                0,
                if kind == PostKind::Small {
                    K::Unsupported(format!("{what} absente (bouton mort ou blinde manquante)"))
                } else {
                    K::Inconsistent(format!("{what} absente de l'historique"))
                },
            ));
        };
        matched[i] = true;
        if post.amount != a.amount {
            return Err((
                post.line,
                K::AmountMismatch {
                    what: format!("{} de {}", kind_name(kind), player_name(seat)),
                    expected: a.amount,
                    found: post.amount,
                },
            ));
        }
        if post.all_in != a.all_in {
            return Err((
                post.line,
                K::Inconsistent("indication de tapis (« and is all-in ») erronée".into()),
            ));
        }
    }
    if let Some((i, _)) = matched.iter().enumerate().find(|(_, m)| !**m) {
        return Err((
            parsed.posts[i].line,
            K::Unsupported(
                "mise obligatoire hors du schéma standard (blinde morte ou postée à l'arrivée)"
                    .into(),
            ),
        ));
    }
    Ok(())
}

fn check_actions(replay: &Replay, parsed: &Parsed) -> Result<(), LineError> {
    let applied = replay
        .state
        .actions()
        .iter()
        .filter(|a| a.kind.is_voluntary());
    for (a, t) in applied.zip(&parsed.actions) {
        let mismatch = |what: &str, expected: Chips, found: Chips| {
            Err((
                t.line,
                K::AmountMismatch {
                    what: what.to_string(),
                    expected,
                    found,
                },
            ))
        };
        match t.verb {
            Verb::Call(x) if x != a.amount => return mismatch("suivi", a.amount, x),
            Verb::Bet(x) | Verb::Raise(x) if x != a.to => return mismatch("mise", a.to, x),
            Verb::Fold | Verb::Check | Verb::Call(_) | Verb::Bet(_) | Verb::Raise(_) => {}
        }
        if t.all_in != a.all_in {
            return Err((
                t.line,
                K::Inconsistent("indication de tapis (« and is all-in ») erronée".into()),
            ));
        }
    }
    Ok(())
}

fn check_uncalled(replay: &Replay, parsed: &Parsed) -> Result<(), LineError> {
    if !matches!(replay.state.phase(), Phase::Showdown | Phase::Finished) {
        return Ok(());
    }
    let computed = build_pots(&replay.state.contributions())
        .uncalled
        .map(|(p, amount)| (replay.seats[p], amount));
    let text = parsed.uncalled.map(|(_, seat, amount)| (seat, amount));
    if computed == text {
        return Ok(());
    }
    let line = parsed.uncalled.map_or(0, |(n, _, _)| n);
    let describe = |u: Option<(u8, Chips)>| {
        u.map_or_else(
            || "aucune".to_string(),
            |(seat, amount)| format!("{amount} au siège {seat}"),
        )
    };
    Err((
        line,
        K::Inconsistent(format!(
            "mise non suivie : {} dans l'historique, {} selon le moteur",
            describe(text),
            describe(computed)
        )),
    ))
}

fn collected_warnings(replay: &Replay, parsed: &Parsed, names: &Names) -> Vec<String> {
    let mut warnings = Vec::new();
    let Outcome::Settled(s) = &replay.outcome else {
        if !parsed.collected.is_empty() {
            warnings.push(
                "gagnant indiqué par la salle mais indéterminable sans les cartes : résultat non \
                 résolu"
                    .to_string(),
            );
        }
        return warnings;
    };
    let won: Vec<Chips> = (0..replay.seats.len())
        .map(|p| {
            let uncalled = s
                .pots
                .uncalled
                .filter(|(q, _)| *q == p)
                .map_or(Chips::ZERO, |(_, a)| a);
            s.collected[p].saturating_sub(uncalled)
        })
        .collect();
    let text: Vec<Chips> = replay
        .seats
        .iter()
        .map(|seat| {
            parsed
                .collected
                .iter()
                .filter(|(s, _)| s == seat)
                .map(|(_, a)| *a)
                .sum()
        })
        .collect();
    let rake = parsed.rake.unwrap_or(Chips::ZERO);
    if rake.is_zero() {
        for (p, seat) in replay.seats.iter().enumerate() {
            if won[p] != text[p] {
                let name = names
                    .0
                    .iter()
                    .find(|(_, s)| s == seat)
                    .map_or("?", |(n, _)| n.as_str());
                warnings.push(format!(
                    "{name} : gain de {} annoncé, {} selon le moteur",
                    text[p], won[p]
                ));
            }
        }
    } else {
        let announced: Chips = text.iter().sum();
        let pot = s.pots.total();
        if announced + rake != pot {
            warnings.push(format!(
                "gains annoncés ({announced}) + râteau ({rake}) ≠ pot calculé ({pot})"
            ));
        }
    }
    warnings
}
