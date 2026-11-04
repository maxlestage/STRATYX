//! Montants des historiques texte : jetons entiers, ou monnaie en centimes.

use stratyx_shared::Chips;

/// Symbole d'une devise (code ISO 4217) dans les historiques PokerStars.
pub(crate) fn currency_symbol(code: &str) -> &'static str {
    match code {
        "USD" => "$",
        "EUR" => "€",
        "GBP" => "£",
        _ => "",
    }
}

/// Devise d'un symbole monétaire.
pub(crate) fn currency_from_symbol(c: char) -> Option<&'static str> {
    match c {
        '$' => Some("USD"),
        '€' => Some("EUR"),
        '£' => Some("GBP"),
        _ => None,
    }
}

pub(crate) fn is_currency_symbol(c: char) -> bool {
    currency_from_symbol(c).is_some()
}

/// Formate un montant. `symbol` : `None` pour des jetons (entiers), `Some(symbole)` pour de
/// la monnaie exprimée en centimes (« $10 », « $0.05 », « €1.50 »).
pub(crate) fn format_amount(amount: Chips, symbol: Option<&str>) -> String {
    let v = amount.get();
    match symbol {
        None => v.to_string(),
        Some(s) if v.is_multiple_of(100) => format!("{s}{}", v / 100),
        Some(s) => format!("{s}{}.{:02}", v / 100, v % 100),
    }
}

/// Lit un montant. En monnaie (`money`), le résultat est en centimes (au plus deux
/// décimales) ; sinon, un entier de jetons. Les séparateurs de milliers (`,`) sont ignorés.
pub(crate) fn parse_amount(s: &str, money: bool) -> Option<Chips> {
    let t = s
        .trim()
        .trim_start_matches(is_currency_symbol)
        .replace(',', "");
    if t.is_empty() {
        return None;
    }
    let digits = |x: &str| !x.is_empty() && x.bytes().all(|b| b.is_ascii_digit());
    match t.split_once('.') {
        None if digits(&t) => {
            let v: u64 = t.parse().ok()?;
            Some(Chips(if money { v.checked_mul(100)? } else { v }))
        }
        Some((whole, frac)) if money && digits(whole) && digits(frac) && frac.len() <= 2 => {
            let w: u64 = whole.parse().ok()?;
            let f: u64 = frac.parse().ok()?;
            let f = if frac.len() == 1 { f * 10 } else { f };
            w.checked_mul(100)?.checked_add(f).map(Chips)
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn money_and_chips() {
        assert_eq!(format_amount(Chips(1000), Some("$")), "$10");
        assert_eq!(format_amount(Chips(5), Some("$")), "$0.05");
        assert_eq!(format_amount(Chips(1037), Some("€")), "€10.37");
        assert_eq!(format_amount(Chips(1500), None), "1500");
        assert_eq!(parse_amount("$10", true), Some(Chips(1000)));
        assert_eq!(parse_amount("$0.05", true), Some(Chips(5)));
        assert_eq!(parse_amount("€1.5", true), Some(Chips(150)));
        assert_eq!(parse_amount("1,500", false), Some(Chips(1500)));
        assert_eq!(parse_amount("1.5", false), None);
        assert_eq!(parse_amount("$0.055", true), None);
        assert_eq!(parse_amount("abc", false), None);
        assert_eq!(parse_amount("", true), None);
        for v in [0, 1, 5, 99, 100, 1037, 123_456] {
            let s = format_amount(Chips(v), Some("$"));
            assert_eq!(parse_amount(&s, true), Some(Chips(v)));
        }
    }
}
