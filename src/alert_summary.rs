//! Summarize vague weather alerts (e.g. "Special Weather Statement") into concrete,
//! actionable phrases by inspecting the alert description.

use crate::weather::WeatherAlert;

/// NWS event names that are too generic to be useful on their own — they
/// require the description to know what's actually being warned about.
pub fn is_vague_event(event: &str) -> bool {
    let normalized = event.trim().to_lowercase();
    matches!(
        normalized.as_str(),
        "special weather statement" | "hazardous weather outlook" | "weather advisory"
    )
}

/// True if there's at least one vague-event alert whose description doesn't
/// match any rule-based phenomenon, meaning we need the LLM fallback.
/// Callers use this to avoid initializing the Bedrock client (and paying
/// the AWS config/credential load) when no alert needs it.
pub fn needs_llm_fallback(alerts: &[WeatherAlert]) -> bool {
    alerts
        .iter()
        .any(|a| is_vague_event(&a.event) && extract_phenomenon(&a.description).is_none())
}

/// Scan a description for a concrete weather phenomenon. Returns a short
/// phrase suitable for use after "There will be ..." (e.g. "areas of fog").
///
/// Checks are ordered from most specific to most general so that e.g.
/// "dense fog" wins over "fog", and "heavy snow" wins over plain "snow".
pub fn extract_phenomenon(description: &str) -> Option<String> {
    let desc = description.to_lowercase();

    // Match in specificity order
    let phrase = if contains_word(&desc, "tornado") {
        "a tornado"
    } else if desc.contains("dense fog") {
        "dense fog"
    } else if contains_word(&desc, "fog") {
        "areas of fog"
    } else if desc.contains("freezing rain") {
        "freezing rain"
    } else if desc.contains("heavy snow") || contains_word(&desc, "blizzard") {
        "heavy snow"
    } else if contains_word(&desc, "snow") {
        "snow"
    } else if contains_word(&desc, "hail") {
        "hail"
    } else if contains_word(&desc, "thunderstorm") {
        "thunderstorms"
    } else if desc.contains("flash flood") {
        "flash flooding"
    } else if contains_word(&desc, "flood") {
        "flooding"
    } else if desc.contains("damaging wind") || desc.contains("high wind") {
        "strong winds"
    } else if desc.contains("wind gust")
        || desc.contains("gusty wind")
        || contains_word(&desc, "gusts")
    {
        "gusty winds"
    } else if desc.contains("heavy rain") || contains_word(&desc, "downpour") {
        "heavy rain"
    } else if contains_word(&desc, "freezing") || contains_word(&desc, "frost") {
        "freezing conditions"
    } else if contains_word(&desc, "ice") || contains_word(&desc, "icy") {
        "icy conditions"
    } else if desc.contains("excessive heat") || desc.contains("extreme heat") {
        "excessive heat"
    } else if contains_word(&desc, "heat") {
        "high heat"
    } else if desc.contains("wind chill") || desc.contains("extreme cold") {
        "dangerous cold"
    } else if contains_word(&desc, "cold") {
        "cold temperatures"
    } else {
        return None;
    };

    Some(phrase.to_string())
}

/// True if any whitespace/punctuation-delimited word in `haystack` begins
/// with `prefix`. Avoids substring false positives like "preheat" matching
/// "heat" or "permafrost" matching "frost", while still allowing inflections
/// like "flooding" to match "flood".
fn contains_word(haystack: &str, prefix: &str) -> bool {
    haystack
        .split(|c: char| !c.is_alphanumeric())
        .any(|w| w.starts_with(prefix))
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::weather::WeatherAlert;
    use chrono::Utc;
    use chrono_tz::Tz;

    #[test]
    fn vague_events_detected() {
        assert!(is_vague_event("Special Weather Statement"));
        assert!(is_vague_event("special weather statement"));
        assert!(is_vague_event("Hazardous Weather Outlook"));
        assert!(is_vague_event("Weather Advisory"));
    }

    #[test]
    fn specific_events_not_vague() {
        assert!(!is_vague_event("Winter Storm Warning"));
        assert!(!is_vague_event("Flood Watch"));
        assert!(!is_vague_event("Tornado Warning"));
        assert!(!is_vague_event("Small Craft Advisory"));
    }

    #[test]
    fn extracts_fog_from_nws_example() {
        let desc = "Areas of fog continue early this morning, with visibilities in \
                    the fog ranging between one and one- quarter mile. Improvement is \
                    expected to be slow and will continue to impact travelers through \
                    late this morning.";
        assert_eq!(extract_phenomenon(desc), Some("areas of fog".to_string()));
    }

    #[test]
    fn dense_fog_wins_over_fog() {
        assert_eq!(
            extract_phenomenon("Dense fog is expected overnight"),
            Some("dense fog".to_string())
        );
    }

    #[test]
    fn extracts_thunderstorms() {
        assert_eq!(
            extract_phenomenon("Scattered thunderstorms will develop this afternoon"),
            Some("thunderstorms".to_string())
        );
    }

    #[test]
    fn extracts_strong_winds() {
        assert_eq!(
            extract_phenomenon("High wind warning: damaging winds up to 60 mph"),
            Some("strong winds".to_string())
        );
    }

    #[test]
    fn extracts_heavy_snow_over_snow() {
        assert_eq!(
            extract_phenomenon("Heavy snow is expected, with snowfall totals of 8-12 inches"),
            Some("heavy snow".to_string())
        );
    }

    #[test]
    fn extracts_plain_snow() {
        assert_eq!(
            extract_phenomenon("Light snow developing this evening"),
            Some("snow".to_string())
        );
    }

    #[test]
    fn extracts_freezing_rain() {
        assert_eq!(
            extract_phenomenon("Freezing rain will cause icy roads"),
            Some("freezing rain".to_string())
        );
    }

    #[test]
    fn extracts_flash_flooding_over_flooding() {
        assert_eq!(
            extract_phenomenon("Flash flood warning in effect for low lying areas"),
            Some("flash flooding".to_string())
        );
    }

    #[test]
    fn extracts_tornado() {
        assert_eq!(
            extract_phenomenon("A tornado has been spotted on the ground"),
            Some("a tornado".to_string())
        );
    }

    #[test]
    fn extracts_heat() {
        assert_eq!(
            extract_phenomenon("Excessive heat warning with indexes above 105"),
            Some("excessive heat".to_string())
        );
    }

    #[test]
    fn extracts_heavy_rain() {
        assert_eq!(
            extract_phenomenon("Heavy rain and downpours expected"),
            Some("heavy rain".to_string())
        );
    }

    #[test]
    fn no_match_returns_none() {
        assert_eq!(
            extract_phenomenon("A routine weather outlook for the region"),
            None
        );
    }

    #[test]
    fn case_insensitive() {
        assert_eq!(
            extract_phenomenon("AREAS OF FOG CONTINUE"),
            Some("areas of fog".to_string())
        );
    }

    #[test]
    fn word_boundary_avoids_heat_false_positives() {
        // "preheat" / "heating" — wait, "heating" starts with "heat" so it
        // would match. We accept that; reject true false positives instead.
        assert_eq!(
            extract_phenomenon("The oven preheat cycle is unrelated"),
            None
        );
    }

    #[test]
    fn word_boundary_avoids_frost_false_positives() {
        assert_eq!(
            extract_phenomenon("Permafrost layers are stable in this region"),
            None
        );
    }

    #[test]
    fn word_boundary_avoids_cold_false_positives() {
        // "scolded" and "could" would match a naive .contains("cold")
        assert_eq!(
            extract_phenomenon("They scolded the crowd; nothing could stop them"),
            None
        );
    }

    #[test]
    fn word_boundary_avoids_ice_false_positives() {
        // "slice", "dice", "nice" would all match a naive .contains("ice")
        assert_eq!(
            extract_phenomenon("A nice slice of advice for the day"),
            None
        );
    }

    #[test]
    fn flood_still_matches_flooding_inflection() {
        assert_eq!(
            extract_phenomenon("Flooding is expected in low-lying areas"),
            Some("flooding".to_string())
        );
    }

    fn test_alert(event: &str, description: &str) -> WeatherAlert {
        let now = Utc::now().with_timezone(&Tz::UTC);
        WeatherAlert {
            event: event.to_string(),
            sender_name: "NWS".to_string(),
            start: now,
            end: now,
            description: description.to_string(),
        }
    }

    #[test]
    fn needs_llm_fallback_false_when_no_alerts() {
        assert!(!needs_llm_fallback(&[]));
    }

    #[test]
    fn needs_llm_fallback_false_when_all_specific() {
        let alerts = vec![
            test_alert("Winter Storm Warning", "lots of snow"),
            test_alert("Flood Watch", "lots of water"),
        ];
        assert!(!needs_llm_fallback(&alerts));
    }

    #[test]
    fn needs_llm_fallback_false_when_vague_but_rule_matches() {
        let alerts = vec![test_alert("Special Weather Statement", "Areas of fog")];
        assert!(!needs_llm_fallback(&alerts));
    }

    #[test]
    fn needs_llm_fallback_true_when_vague_and_no_rule_match() {
        let alerts = vec![test_alert(
            "Special Weather Statement",
            "Routine outlook with no specific phenomenon",
        )];
        assert!(needs_llm_fallback(&alerts));
    }
}
