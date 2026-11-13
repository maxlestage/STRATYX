//! Description anglaise des mains à l'abattage, comme dans les historiques PokerStars
//! (« a pair of Kings », « two pair, Aces and Sevens »…). Purement informative : l'import
//! l'ignore et recalcule la force des mains.

use stratyx_poker::{HandCategory, HandRank, Rank};

fn name(r: Rank) -> &'static str {
    match r {
        Rank::Two => "Deuce",
        Rank::Three => "Three",
        Rank::Four => "Four",
        Rank::Five => "Five",
        Rank::Six => "Six",
        Rank::Seven => "Seven",
        Rank::Eight => "Eight",
        Rank::Nine => "Nine",
        Rank::Ten => "Ten",
        Rank::Jack => "Jack",
        Rank::Queen => "Queen",
        Rank::King => "King",
        Rank::Ace => "Ace",
    }
}

fn plural(r: Rank) -> String {
    match r {
        Rank::Six => "Sixes".to_string(),
        r => format!("{}s", name(r)),
    }
}

/// Carte basse d'une quinte dont la carte haute est `high` (As pour la quinte blanche).
fn straight_low(high: Rank) -> Rank {
    if high == Rank::Five {
        Rank::Ace
    } else {
        Rank::from_index(high.index() - 4).unwrap_or(Rank::Two)
    }
}

pub(crate) fn describe(rank: HandRank) -> String {
    let r = rank.ranks();
    let at = |i: usize| r.get(i).copied().unwrap_or(Rank::Two);
    match rank.category() {
        HandCategory::HighCard => format!("high card {}", name(at(0))),
        HandCategory::OnePair => format!("a pair of {}", plural(at(0))),
        HandCategory::TwoPair => format!("two pair, {} and {}", plural(at(0)), plural(at(1))),
        HandCategory::ThreeOfAKind => format!("three of a kind, {}", plural(at(0))),
        HandCategory::Straight => {
            format!(
                "a straight, {} to {}",
                name(straight_low(at(0))),
                name(at(0))
            )
        }
        HandCategory::Flush => format!("a flush, {} high", name(at(0))),
        HandCategory::FullHouse => {
            format!("a full house, {} full of {}", plural(at(0)), plural(at(1)))
        }
        HandCategory::FourOfAKind => format!("four of a kind, {}", plural(at(0))),
        HandCategory::StraightFlush if at(0) == Rank::Ace => "a Royal Flush".to_string(),
        HandCategory::StraightFlush => format!(
            "a straight flush, {} to {}",
            name(straight_low(at(0))),
            name(at(0))
        ),
    }
}

#[cfg(test)]
mod tests {
    use stratyx_poker::{CardSet, evaluate};

    use super::*;

    fn d(cards: &str) -> String {
        describe(evaluate(CardSet::parse(cards).unwrap()))
    }

    #[test]
    fn descriptions() {
        assert_eq!(d("AhKd9c7s2h"), "high card Ace");
        assert_eq!(d("KhKd9c7s2h"), "a pair of Kings");
        assert_eq!(d("AhAd7c7s2h"), "two pair, Aces and Sevens");
        assert_eq!(d("6h6d6c7s2h"), "three of a kind, Sixes");
        assert_eq!(d("Ah2d3c4s5h"), "a straight, Ace to Five");
        assert_eq!(d("9hTdJcQsKh"), "a straight, Nine to King");
        assert_eq!(d("Ah9h7h4h2h"), "a flush, Ace high");
        assert_eq!(d("KhKdKc7s7h"), "a full house, Kings full of Sevens");
        assert_eq!(d("2h2d2c2s7h"), "four of a kind, Deuces");
        assert_eq!(d("9hThJhQhKh"), "a straight flush, Nine to King");
        assert_eq!(d("AhKhQhJhTh"), "a Royal Flush");
    }
}
