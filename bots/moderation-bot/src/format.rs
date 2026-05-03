// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Sascha Daemgen, IT and More Systems

//! User-sichtbare Format-Helfer für deutsche Bot-Antworten.
//!
//! Alle Funktionen sind pure und unit-getestet. Sprache: Deutsch mit
//! korrekten Umlauten. Zeitzonen-Auflösung erfolgt über `chrono::Local`,
//! also nach der System-Zeitzone des Servers (in Produktion `Europe/Berlin`).

use chrono::{DateTime, Local, TimeZone};

/// Wandelt eine Dauer in Sekunden in eine kurze deutsche Beschreibung um.
///
/// Schwellen:
///
/// - unter 60 Sekunden: `"X Sekunden"` (Singular bei 1)
/// - unter 60 Minuten: `"X Minuten"` (Sekunden werden auf Minuten abgerundet)
/// - unter 24 Stunden: `"X Stunden"`
/// - sonst: `"X Tage"`
///
/// Mischformen wie "2 Stunden 30 Minuten" werden in dieser Phase nicht
/// erzeugt; der Bot nennt nur die größte natürliche Einheit.
#[must_use]
pub fn format_duration_de(seconds: u64) -> String {
    if seconds < 60 {
        return format_with_unit(seconds, "Sekunde", "Sekunden");
    }
    if seconds < 3_600 {
        return format_with_unit(seconds / 60, "Minute", "Minuten");
    }
    if seconds < 86_400 {
        return format_with_unit(seconds / 3_600, "Stunde", "Stunden");
    }
    format_with_unit(seconds / 86_400, "Tag", "Tage")
}

fn format_with_unit(value: u64, singular: &str, plural: &str) -> String {
    if value == 1 {
        format!("1 {singular}")
    } else {
        format!("{value} {plural}")
    }
}

/// Formatiert eine Unix-Sekunde als lokale Uhrzeit `HH:MM:SS Uhr`.
///
/// Bei ungültigen oder nicht repräsentierbaren Zeitstempeln liefert die
/// Funktion einen Fallback-Text mit dem rohen Wert, damit die Bot-Antwort
/// nicht leer bleibt.
#[must_use]
pub fn format_unix_time_de(unix: i64) -> String {
    match Local.timestamp_opt(unix, 0) {
        chrono::LocalResult::Single(dt) | chrono::LocalResult::Ambiguous(dt, _) => {
            format_local_time(&dt)
        }
        chrono::LocalResult::None => format!("Unix-Sekunde {unix}"),
    }
}

fn format_local_time(dt: &DateTime<Local>) -> String {
    format!("{} Uhr", dt.format("%H:%M:%S"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duration_seconds_singular_and_plural() {
        assert_eq!(format_duration_de(0), "0 Sekunden");
        assert_eq!(format_duration_de(1), "1 Sekunde");
        assert_eq!(format_duration_de(30), "30 Sekunden");
        assert_eq!(format_duration_de(59), "59 Sekunden");
    }

    #[test]
    fn duration_rolls_over_into_minutes() {
        assert_eq!(format_duration_de(60), "1 Minute");
        assert_eq!(format_duration_de(120), "2 Minuten");
        assert_eq!(format_duration_de(300), "5 Minuten");
        // Floor-Division: 89 Sekunden bleiben "1 Minute".
        assert_eq!(format_duration_de(89), "1 Minute");
    }

    #[test]
    fn duration_rolls_over_into_hours() {
        assert_eq!(format_duration_de(3_600), "1 Stunde");
        assert_eq!(format_duration_de(7_200), "2 Stunden");
        assert_eq!(format_duration_de(3_600 * 23), "23 Stunden");
    }

    #[test]
    fn duration_rolls_over_into_days() {
        assert_eq!(format_duration_de(86_400), "1 Tag");
        assert_eq!(format_duration_de(86_400 * 7), "7 Tage");
    }

    #[test]
    fn unix_time_format_has_uhr_suffix_and_correct_pattern() {
        // We do not assert a specific clock value (depends on system tz),
        // only the surface shape: HH:MM:SS Uhr.
        let s = format_unix_time_de(1_700_000_000);
        assert!(s.ends_with(" Uhr"), "expected suffix ' Uhr' in {s}");
        let prefix = s.trim_end_matches(" Uhr");
        let parts: Vec<&str> = prefix.split(':').collect();
        assert_eq!(
            parts.len(),
            3,
            "expected three colon-separated parts in {s}"
        );
        for p in parts {
            assert_eq!(p.len(), 2, "expected two-digit segment in {s}");
            assert!(
                p.chars().all(|c| c.is_ascii_digit()),
                "expected digits in {s}"
            );
        }
    }

    #[test]
    fn unix_time_handles_zero_timestamp() {
        // 0 = 1970-01-01 00:00:00 UTC; in Local this is some valid time.
        let s = format_unix_time_de(0);
        assert!(s.ends_with(" Uhr"));
    }
}
