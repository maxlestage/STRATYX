//! Empreinte SHA-256 du contenu de jeu d'une main, pour la déduplication.
//!
//! L'empreinte couvre le **contenu de jeu canonique** : format, blindes et antes, bouton,
//! sièges (numéro, joueur, tapis, cartes connues), événements. Elle ignore l'identifiant,
//! les métadonnées ([`HandMeta`](stratyx_poker::HandMeta) : date, session, salle, provenance)
//! ainsi que la provenance et la confiance de chaque observation. Une même main capturée par
//! la vision puis importée d'un historique texte a donc la même empreinte.
//!
//! Canonicalisation :
//! - sièges triés par numéro ;
//! - cartes de chaque tirage du board triées (l'ordre d'affichage du flop varie selon la
//!   source) ;
//! - si la main se rejoue, chaque action est décrite par ce que le moteur a appliqué (nature
//!   et montant « à ») : un tapis noté `AllIn` et la relance explicite équivalente donnent la
//!   même empreinte. Sinon, l'action brute est utilisée.
//!
//! Le codage est explicite (champs étiquetés et préfixés par leur longueur), indépendant de la
//! sérialisation JSON : il ne change pas si le format de stockage évolue.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use sha2::{Digest, Sha256};
use stratyx_poker::game::{ActionKind, Ante, AppliedAction, BettingStructure, Blinds};
use stratyx_poker::history::Replay;
use stratyx_poker::{Action, Card, GameFormat, GameType, HandEvent, HandRecord};
use thiserror::Error;

/// Version du codage canonique, incluse dans le condensat.
const CANON_VERSION: &str = "stratyx-empreinte-v1";

/// Empreinte SHA-256 du contenu de jeu d'une main.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Fingerprint(pub [u8; 32]);

/// Texte qui n'est pas une empreinte hexadécimale valide.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("empreinte invalide « {0} » : 64 chiffres hexadécimaux attendus")]
pub struct ParseFingerprintError(pub String);

impl Fingerprint {
    /// Calcule l'empreinte d'une main (la rejoue pour canonicaliser les actions).
    #[must_use]
    pub fn of(record: &HandRecord) -> Fingerprint {
        Fingerprint::with_replay(record, record.replay().ok().as_ref())
    }

    /// Calcule l'empreinte en réutilisant un rejeu déjà effectué.
    #[must_use]
    pub(crate) fn with_replay(record: &HandRecord, replay: Option<&Replay>) -> Fingerprint {
        let mut c = Canon(Sha256::new());
        c.str(CANON_VERSION);
        format(&mut c, record.format);
        blinds(&mut c, &record.blinds.value);
        c.str("button");
        c.u64(u64::from(record.button_seat));

        let mut seats: Vec<_> = record.seats.iter().collect();
        seats.sort_by_key(|s| s.seat);
        c.str("seats");
        c.u64(seats.len() as u64);
        for s in seats {
            c.u64(u64::from(s.seat));
            c.str(&s.player);
            c.u64(s.stack.value.get());
            match &s.hole_cards {
                Some(h) => cards(&mut c, &h.value.cards()),
                None => c.str("?"),
            }
        }

        let applied = replay.map(|r| {
            let voluntary: Vec<&AppliedAction> = r
                .state
                .actions()
                .iter()
                .filter(|a| a.kind.is_voluntary())
                .collect();
            (voluntary, r)
        });
        let mut next_action = 0usize;
        c.str("events");
        c.u64(record.events.len() as u64);
        for ev in &record.events {
            match &ev.value {
                HandEvent::Action { seat, action } => {
                    let canonical = applied.as_ref().and_then(|(list, _)| list.get(next_action));
                    next_action += 1;
                    match canonical {
                        Some(a) => {
                            c.str("action");
                            c.u64(u64::from(*seat));
                            applied_action(&mut c, a);
                        }
                        None => {
                            c.str("raw-action");
                            c.u64(u64::from(*seat));
                            raw_action(&mut c, *action);
                        }
                    }
                }
                HandEvent::Board { cards: board } => {
                    c.str("board");
                    let mut sorted = board.clone();
                    sorted.sort();
                    cards(&mut c, &sorted);
                }
                HandEvent::Show { seat, cards: hole } => {
                    c.str("show");
                    c.u64(u64::from(*seat));
                    cards(&mut c, &hole.cards());
                }
                HandEvent::Muck { seat } => {
                    c.str("muck");
                    c.u64(u64::from(*seat));
                }
            }
        }
        let digest = c.0.finalize();
        let mut out = [0u8; 32];
        out.copy_from_slice(&digest);
        Fingerprint(out)
    }

    /// Les 32 octets du condensat.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// Représentation hexadécimale (64 caractères minuscules).
    #[must_use]
    pub fn to_hex(&self) -> String {
        crate::hex::encode(&self.0)
    }
}

impl fmt::Display for Fingerprint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_hex())
    }
}

impl fmt::Debug for Fingerprint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Fingerprint({})", self.to_hex())
    }
}

impl FromStr for Fingerprint {
    type Err = ParseFingerprintError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let bytes = crate::hex::decode(s).ok_or_else(|| ParseFingerprintError(s.to_string()))?;
        let arr: [u8; 32] = bytes
            .try_into()
            .map_err(|_| ParseFingerprintError(s.to_string()))?;
        Ok(Fingerprint(arr))
    }
}

impl Serialize for Fingerprint {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Fingerprint {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

/// Alimente le condensat avec des champs non ambigus.
struct Canon(Sha256);

impl Canon {
    fn u64(&mut self, v: u64) {
        self.0.update(v.to_le_bytes());
    }

    fn str(&mut self, s: &str) {
        self.u64(s.len() as u64);
        self.0.update(s.as_bytes());
    }
}

fn cards(c: &mut Canon, list: &[Card]) {
    c.u64(list.len() as u64);
    for card in list {
        c.u64(u64::from(card.id()));
    }
}

fn format(c: &mut Canon, f: GameFormat) {
    c.str("format");
    c.str(match f.game_type {
        GameType::CashGame => "cash_game",
        GameType::Tournament => "tournament",
        GameType::SitAndGo => "sit_and_go",
        GameType::Expresso => "expresso",
    });
    c.u64(u64::from(f.table_size));
    match f.structure {
        BettingStructure::NoLimit => c.str("no_limit"),
        BettingStructure::PotLimit => c.str("pot_limit"),
        BettingStructure::FixedLimit {
            max_bets,
            uncapped_heads_up,
        } => {
            c.str("fixed_limit");
            c.u64(max_bets.map_or(0, |m| u64::from(m) + 1));
            c.u64(u64::from(uncapped_heads_up));
        }
    }
}

fn blinds(c: &mut Canon, b: &Blinds) {
    c.str("blinds");
    c.u64(b.small_blind.get());
    c.u64(b.big_blind.get());
    match b.ante {
        Ante::None => c.str("no_ante"),
        Ante::Classic(a) => {
            c.str("classic_ante");
            c.u64(a.get());
        }
        Ante::BigBlind(a) => {
            c.str("big_blind_ante");
            c.u64(a.get());
        }
    }
}

fn applied_action(c: &mut Canon, a: &AppliedAction) {
    let kind = match a.kind {
        ActionKind::Fold => "fold",
        ActionKind::Check => "check",
        ActionKind::Call => "call",
        ActionKind::Bet => "bet",
        ActionKind::Raise => "raise",
        // Jamais rencontré : les mises obligatoires sont filtrées en amont.
        ActionKind::PostAnte | ActionKind::PostSmallBlind | ActionKind::PostBigBlind => "post",
    };
    c.str(kind);
    if matches!(
        a.kind,
        ActionKind::Call | ActionKind::Bet | ActionKind::Raise
    ) {
        c.u64(a.to.get());
    }
}

fn raw_action(c: &mut Canon, a: Action) {
    match a {
        Action::Fold => c.str("fold"),
        Action::Check => c.str("check"),
        Action::Call => c.str("call"),
        Action::AllIn => c.str("all_in"),
        Action::Bet(to) => {
            c.str("bet");
            c.u64(to.get());
        }
        Action::Raise(to) => {
            c.str("raise");
            c.u64(to.get());
        }
    }
}

#[cfg(test)]
mod tests {
    use stratyx_poker::{Blinds, HandMeta, SeatRecord};
    use stratyx_shared::{Chips, Confidence, Observed, Source};

    use super::*;

    fn ok<T>(v: T) -> Observed<T> {
        Observed::certain(v, Source::Import)
    }

    fn hand() -> HandRecord {
        HandRecord {
            id: "h1".into(),
            format: GameFormat::EXPRESSO,
            blinds: ok(Blinds::new(10, 20)),
            button_seat: 1,
            seats: vec![
                SeatRecord {
                    seat: 1,
                    player: "Hero".into(),
                    stack: ok(Chips(500)),
                    hole_cards: Some(ok("AhKh".parse().unwrap())),
                },
                SeatRecord {
                    seat: 2,
                    player: "V1".into(),
                    stack: ok(Chips(500)),
                    hole_cards: None,
                },
                SeatRecord {
                    seat: 3,
                    player: "V2".into(),
                    stack: ok(Chips(400)),
                    hole_cards: None,
                },
            ],
            events: vec![
                ok(HandEvent::Action {
                    seat: 1,
                    action: Action::AllIn,
                }),
                ok(HandEvent::Action {
                    seat: 2,
                    action: Action::Fold,
                }),
                ok(HandEvent::Action {
                    seat: 3,
                    action: Action::Call,
                }),
                ok(HandEvent::Board {
                    cards: stratyx_poker::card::parse_cards("Kd7c2s").unwrap(),
                }),
            ],
            meta: HandMeta::default(),
        }
    }

    #[test]
    fn ignores_metadata_and_confidence() {
        let a = hand();
        let mut b = a.clone();
        b.id = "autre".into();
        b.meta.site = Some("winamax".into());
        b.meta.played_at_ms = Some(42);
        b.events[1] = Observed::vision(
            b.events[1].value.clone(),
            Confidence::new(0.8).unwrap(),
            1_000,
        );
        b.seats.reverse();
        assert_eq!(Fingerprint::of(&a), Fingerprint::of(&b));
    }

    #[test]
    fn canonical_board_and_all_in() {
        let a = hand();
        let mut b = a.clone();
        b.events[3] = ok(HandEvent::Board {
            cards: stratyx_poker::card::parse_cards("2sKd7c").unwrap(),
        });
        // Le tapis du héros (500) équivaut à une relance explicite à 500.
        b.events[0] = ok(HandEvent::Action {
            seat: 1,
            action: Action::Raise(Chips(500)),
        });
        assert_eq!(Fingerprint::of(&a), Fingerprint::of(&b));
    }

    #[test]
    fn game_content_changes_the_fingerprint() {
        let a = hand();
        let mut b = a.clone();
        b.seats[2].stack = ok(Chips(401));
        assert_ne!(Fingerprint::of(&a), Fingerprint::of(&b));
        let mut c = a.clone();
        c.seats[0].hole_cards = None;
        assert_ne!(Fingerprint::of(&a), Fingerprint::of(&c));
    }

    #[test]
    fn hex_roundtrip() {
        let f = Fingerprint::of(&hand());
        let hex = f.to_hex();
        assert_eq!(hex.len(), 64);
        assert_eq!(hex.parse::<Fingerprint>().unwrap(), f);
        assert!("zz".parse::<Fingerprint>().is_err());
        let json = serde_json::to_string(&f).unwrap();
        assert_eq!(serde_json::from_str::<Fingerprint>(&json).unwrap(), f);
    }
}
