//! Dates des en-têtes d'historiques : « 2024/12/01 21:15:03 CET [2024/12/01 15:15:03 ET] ».
//!
//! Les fuseaux à décalage fixe usuels sont reconnus, ainsi que l'heure de l'Est américain
//! (`ET`, avec l'heure d'été en vigueur depuis 2007). Un fuseau inconnu laisse la date absente
//! plutôt que de la deviner.

/// Jours écoulés depuis le 1970-01-01 (calendrier grégorien proleptique).
fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = i64::from((m + 9) % 12);
    let doy = (153 * mp + 2) / 5 + i64::from(d) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Date civile d'un nombre de jours depuis le 1970-01-01.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// Jour du mois du `n`-ième dimanche (1 = premier) du mois.
fn nth_sunday(y: i64, m: u32, n: u32) -> u32 {
    // 1970-01-01 était un jeudi : (jours + 4) mod 7 donne 0 pour dimanche.
    let weekday = (days_from_civil(y, m, 1) + 4).rem_euclid(7) as u32;
    1 + (7 - weekday) % 7 + 7 * (n - 1)
}

/// Heure d'été américaine pour une heure locale de l'Est (règle de 2007). Pendant l'heure
/// répétée de novembre, la première occurrence (heure d'été) est retenue.
fn us_eastern_dst(y: i64, m: u32, d: u32, h: u32) -> bool {
    match m {
        4..=10 => true,
        3 => {
            let s = nth_sunday(y, 3, 2);
            d > s || (d == s && h >= 2)
        }
        11 => {
            let s = nth_sunday(y, 11, 1);
            d < s || (d == s && h < 2)
        }
        _ => false,
    }
}

/// Décalage UTC (minutes) d'un fuseau à décalage fixe.
fn fixed_offset(zone: &str) -> Option<i64> {
    Some(match zone {
        "UTC" | "GMT" | "WET" => 0,
        "WEST" | "BST" | "CET" => 60,
        "CEST" | "EET" => 120,
        "EEST" | "MSK" => 180,
        "EST" => -300,
        "EDT" | "AST" => -240,
        "CST" => -360,
        "CDT" => -300,
        "MST" => -420,
        "MDT" => -360,
        "PST" => -480,
        "PDT" => -420,
        "BRT" | "ART" => -180,
        "JST" => 540,
        _ => return None,
    })
}

struct Local {
    y: i64,
    mo: u32,
    d: u32,
    h: u32,
    mi: u32,
    s: u32,
}

impl Local {
    fn parse(date: &str, time: &str) -> Option<Local> {
        let mut dp = date.split('/');
        let y: i64 = dp.next()?.parse().ok()?;
        let mo: u32 = dp.next()?.parse().ok()?;
        let d: u32 = dp.next()?.parse().ok()?;
        let mut tp = time.split(':');
        let h: u32 = tp.next()?.parse().ok()?;
        let mi: u32 = tp.next()?.parse().ok()?;
        let s: u32 = tp.next()?.parse().ok()?;
        if dp.next().is_some()
            || tp.next().is_some()
            || !(1..=12).contains(&mo)
            || !(1..=31).contains(&d)
            || h > 23
            || mi > 59
            || s > 60
        {
            return None;
        }
        Some(Local { y, mo, d, h, mi, s })
    }

    /// Millisecondes UTC pour un décalage donné (minutes).
    fn to_utc_ms(&self, offset_min: i64) -> Option<u64> {
        let secs = days_from_civil(self.y, self.mo, self.d) * 86_400
            + i64::from(self.h) * 3600
            + i64::from(self.mi) * 60
            + i64::from(self.s)
            - offset_min * 60;
        u64::try_from(secs).ok()?.checked_mul(1000)
    }

    fn to_utc_in(&self, zone: &str) -> Option<u64> {
        let offset = match zone {
            "ET" => {
                if us_eastern_dst(self.y, self.mo, self.d, self.h) {
                    -240
                } else {
                    -300
                }
            }
            z => fixed_offset(z)?,
        };
        self.to_utc_ms(offset)
    }
}

/// Lit la date d'un en-tête. Le premier horodatage est retenu si son fuseau est connu, sinon
/// celui entre crochets (heure de l'Est chez PokerStars).
pub(crate) fn parse_header_date(s: &str) -> Option<u64> {
    let (main, bracket) = match s.split_once('[') {
        Some((a, b)) => (a, Some(b.trim_end_matches(']'))),
        None => (s, None),
    };
    let parse = |part: &str| {
        let mut it = part.split_whitespace();
        let local = Local::parse(it.next()?, it.next()?)?;
        let zone = it.next().unwrap_or("ET");
        local.to_utc_in(zone)
    };
    parse(main).or_else(|| bracket.and_then(parse))
}

fn format_local(ms: u64, offset_min: i64) -> String {
    let secs = (ms / 1000) as i64 + offset_min * 60;
    let (y, m, d) = civil_from_days(secs.div_euclid(86_400));
    let rem = secs.rem_euclid(86_400);
    format!(
        "{y:04}/{m:02}/{d:02} {:02}:{:02}:{:02}",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

/// Décalage (minutes) de l'heure de l'Est américain à un instant UTC donné.
fn us_eastern_offset_at(secs_utc: i64) -> i64 {
    let (y, _, _) = civil_from_days(secs_utc.div_euclid(86_400));
    // Passage à l'heure d'été à 2 h locales (7 h UTC), retour à 2 h locales (6 h UTC).
    let start = days_from_civil(y, 3, nth_sunday(y, 3, 2)) * 86_400 + 7 * 3600;
    let end = days_from_civil(y, 11, nth_sunday(y, 11, 1)) * 86_400 + 6 * 3600;
    if (start..end).contains(&secs_utc) {
        -240
    } else {
        -300
    }
}

/// Date d'en-tête : « 2024/12/01 20:15:03 UTC [2024/12/01 15:15:03 ET] » (à la seconde).
pub(crate) fn format_header_date(ms: u64) -> String {
    let offset = us_eastern_offset_at((ms / 1000) as i64);
    format!(
        "{} UTC [{} ET]",
        format_local(ms, 0),
        format_local(ms, offset)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn civil_roundtrip() {
        for days in [-1_000_000, -1, 0, 1, 19_000, 20_000, 2_000_000] {
            let (y, m, d) = civil_from_days(days);
            assert_eq!(days_from_civil(y, m, d), days);
        }
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(days_from_civil(2000, 3, 1), 11_017);
    }

    #[test]
    fn us_dst_boundaries() {
        // 2024 : du 10 mars au 3 novembre.
        assert_eq!(nth_sunday(2024, 3, 2), 10);
        assert_eq!(nth_sunday(2024, 11, 1), 3);
        assert!(!us_eastern_dst(2024, 3, 10, 1));
        assert!(us_eastern_dst(2024, 3, 10, 3));
        assert!(us_eastern_dst(2024, 11, 3, 1));
        assert!(!us_eastern_dst(2024, 11, 3, 2));
        assert!(!us_eastern_dst(2024, 12, 1, 12));
    }

    #[test]
    fn header_dates() {
        // 2024-12-01 20:15:03 UTC.
        let utc = 1_733_084_103_000;
        assert_eq!(
            parse_header_date("2024/12/01 21:15:03 CET [2024/12/01 15:15:03 ET]"),
            Some(utc)
        );
        assert_eq!(parse_header_date("2024/12/01 15:15:03 ET"), Some(utc));
        // Fuseau inconnu : repli sur l'heure de l'Est entre crochets.
        assert_eq!(
            parse_header_date("2024/12/01 21:15:03 XYZ [2024/12/01 15:15:03 ET]"),
            Some(utc)
        );
        assert_eq!(parse_header_date("2024/12/01 21:15:03 XYZ"), None);
        // Heures sans zéro initial (anciens historiques).
        assert_eq!(
            parse_header_date("2024/12/01 5:02:41 UTC"),
            Some(1_733_029_361_000)
        );
        assert_eq!(parse_header_date("n'importe quoi"), None);

        assert_eq!(
            format_header_date(utc),
            "2024/12/01 20:15:03 UTC [2024/12/01 15:15:03 ET]"
        );
        // Été : décalage de quatre heures.
        let summer = 1_719_835_200_000; // 2024-07-01 12:00:00 UTC
        assert_eq!(
            format_header_date(summer),
            "2024/07/01 12:00:00 UTC [2024/07/01 08:00:00 ET]"
        );
        for ms in [utc, summer, 0, 1_700_000_000_000] {
            assert_eq!(parse_header_date(&format_header_date(ms)), Some(ms));
        }
    }
}
