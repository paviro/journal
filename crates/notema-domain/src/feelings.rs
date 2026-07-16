/// A single canonical feeling plus the alternate words that resolve to it.
/// Search aliases are never offered in the picker; they only let a typed or
/// `--feeling` value like "joyful" map onto the canonical "happy".
pub struct Feeling {
    pub name: &'static str,
    pub search_aliases: &'static [&'static str],
}

/// A named cluster of related feelings, following the Nonviolent Communication
/// "feelings wheel": a core emotion with its finer-grained sub-feelings.
pub struct FeelingGroup {
    pub name: &'static str,
    pub feelings: &'static [Feeling],
}

const fn f(name: &'static str, search_aliases: &'static [&'static str]) -> Feeling {
    Feeling {
        name,
        search_aliases,
    }
}

/// The canonical feelings the picker offers, clustered into core-emotion groups.
/// Grouping and order are display/navigation only; they carry no good/bad meaning
/// — a feeling's valence is never inferred from the word or its group. The only
/// signal for how an entry felt is the user's mood score.
pub const FEELING_GROUPS: &[FeelingGroup] = &[
    FeelingGroup {
        name: "Joy & Delight",
        feelings: &[
            f("happy", &["joyous", "cheerful"]),
            f("joyful", &[]),
            f("pleased", &["glad"]),
            f("delighted", &[]),
            f("elated", &[]),
            f("ecstatic", &["overjoyed", "jubilant"]),
            f("excited", &["exhilarated"]),
            f("thrilled", &[]),
            f("enthusiastic", &[]),
            f("passionate", &["ardent"]),
            f("playful", &[]),
            f("amused", &[]),
        ],
    },
    FeelingGroup {
        name: "Gratitude & Appreciation",
        feelings: &[
            f("grateful", &["thankful"]),
            f("appreciative", &["appreciation"]),
            f("admiring", &["admiration"]),
            f("reverent", &["reverence"]),
            f("moved", &["touched"]),
            f("dazzled", &["awe", "awed", "spellbound"]),
        ],
    },
    FeelingGroup {
        name: "Interest, Focus & Energy",
        feelings: &[
            f("interested", &[]),
            f("curious", &[]),
            f("intrigued", &[]),
            f("engaged", &["involved"]),
            f("absorbed", &["engrossed"]),
            f("fascinated", &[]),
            f("mesmerized", &["captivated", "entranced", "enchanted"]),
            f("focused", &["concentrated"]),
            f("attentive", &[]),
            f("present", &["mindful"]),
            f("alert", &[]),
            f("clear-headed", &["clear", "clear minded"]),
            f("receptive", &["open", "openhearted"]),
            f(
                "energetic",
                &["animated", "energized", "alive", "enlivened"],
            ),
            f("refreshed", &["renewed", "invigorated"]),
            f("inspired", &[]),
            f("eager", &["keen"]),
        ],
    },
    FeelingGroup {
        name: "Love & Connection",
        feelings: &[
            f("loving", &[]),
            f("affectionate", &[]),
            f("caring", &[]),
            f("compassionate", &["sympathetic"]),
            f("tender", &["tenderness"]),
            f("connected", &[]),
            f("close", &[]),
            f("intimate", &[]),
            f("friendly", &[]),
            f("fond", &["fondness"]),
            f("adoring", &["adoration"]),
            f("warm", &["warmhearted"]),
        ],
    },
    FeelingGroup {
        name: "Peace & Ease",
        feelings: &[
            f("peaceful", &["at peace"]),
            f("calm", &["composed"]),
            f("relaxed", &["at ease", "mellow"]),
            f("comfortable", &[]),
            f("content", &[]),
            f("satisfied", &[]),
            f("serene", &["tranquil"]),
            f("quiet", &["still"]),
            f(
                "grounded",
                &[
                    "harmonious",
                    "centered",
                    "balanced",
                    "aligned",
                    "congruous",
                    "congruent",
                ],
            ),
            f("relieved", &[]),
        ],
    },
    FeelingGroup {
        name: "Safety, Trust & Hope",
        feelings: &[
            f("safe", &[]),
            f("secure", &[]),
            f("trusting", &[]),
            f("assured", &["certain", "convinced"]),
            f("hopeful", &[]),
            f("optimistic", &["positive"]),
            f("expectant", &["looking forward"]),
            f("reassured", &["comforted"]),
        ],
    },
    FeelingGroup {
        name: "Confidence & Agency",
        feelings: &[
            f("confident", &["self-confident"]),
            f("self-assured", &[]),
            f("proud", &["self-esteem"]),
            f("encouraged", &[]),
            f("empowered", &[]),
            f("capable", &["effectual", "effective", "competent"]),
            f("powerful", &["strong"]),
            f("brave", &["courageous"]),
            f("poised", &[]),
            f("fulfilled", &[]),
            f("determined", &["motivated", "resolute"]),
        ],
    },
    FeelingGroup {
        name: "Anger & Frustration",
        feelings: &[
            f("angry", &["mad"]),
            f("annoyed", &["displeased"]),
            f("irritated", &["irked", "ick"]),
            f("frustrated", &[]),
            f("aggravated", &["exasperated"]),
            f("upset", &[]),
            f("resentful", &["bitter"]),
            f("furious", &["enraged", "irate"]),
            f("hostile", &["hateful", "animosity", "mean"]),
            f("jealous", &[]),
            f("envious", &[]),
        ],
    },
    FeelingGroup {
        name: "Disgust & Aversion",
        feelings: &[
            f("disgusted", &[]),
            f("averse", &["aversion"]),
            f("repulsed", &["repulsive", "revolted"]),
            f("nauseous", &["queasy"]),
            f("sickened", &[]),
            f("contemptuous", &["contempt"]),
            f("scornful", &["scorn"]),
            f("resistant", &[]),
            f("reluctant", &[]),
        ],
    },
    FeelingGroup {
        name: "Fear & Vulnerability",
        feelings: &[
            f("afraid", &["fearful"]),
            f("scared", &[]),
            f("frightened", &[]),
            f("terrified", &[]),
            f("panicked", &["panicky", "alarmed"]),
            f("anxious", &["nervous"]),
            f("worried", &[]),
            f("concerned", &[]),
            f("apprehensive", &[]),
            f("insecure", &[]),
            f("suspicious", &["wary", "skeptical"]),
            f("vulnerable", &["susceptible", "sensitive", "exposed"]),
            f("helpless", &["powerless"]),
        ],
    },
    FeelingGroup {
        name: "Surprise & Startle",
        feelings: &[
            f("surprised", &[]),
            f("amazed", &[]),
            f("astonished", &[]),
            f("shocked", &[]),
            f("startled", &[]),
            f("stunned", &[]),
            f("horrified", &["appalled"]),
            f("caught off guard", &["caught by surprise"]),
        ],
    },
    FeelingGroup {
        name: "Confusion & Overwhelm",
        feelings: &[
            f("confused", &["muddled", "foggy"]),
            f("puzzled", &["perplexed", "bewildered"]),
            f("distracted", &["scattered"]),
            f("preoccupied", &[]),
            f("hesitant", &["unsure"]),
            f("overwhelmed", &["overloaded"]),
            f("stressed", &["pressured", "strained"]),
            f("flustered", &["rattled"]),
        ],
    },
    FeelingGroup {
        name: "Sadness, Hurt & Grief",
        feelings: &[
            f("sad", &["down", "melancholy"]),
            f("unhappy", &[]),
            f("miserable", &[]),
            f("depressed", &["downcast", "gloomy"]),
            f("hurt", &["pained", "pain"]),
            f("grieving", &["grief", "mourning"]),
            f("sorrowful", &["sorrow"]),
            f("heartbroken", &["brokenhearted"]),
            f("devastated", &["crushed", "anguished"]),
            f("discouraged", &["disheartened", "defeated"]),
            f("disappointed", &[]),
            f("hopeless", &[]),
            f("despairing", &["despair", "despondent"]),
        ],
    },
    FeelingGroup {
        name: "Shame & Regret",
        feelings: &[
            f("ashamed", &[]),
            f("guilty", &[]),
            f("embarrassed", &["chagrined"]),
            f("humiliated", &["humiliation"]),
            f("self-conscious", &[]),
            f("awkward", &[]),
            f("uncomfortable", &[]),
            f("regretful", &[]),
            f("remorseful", &[]),
            f("mortified", &[]),
        ],
    },
    FeelingGroup {
        name: "Loneliness & Disconnection",
        feelings: &[
            f("lonely", &["alone", "isolated"]),
            f("disconnected", &["unconnected", "alienated"]),
            f("distant", &[]),
            f("withdrawn", &["closed off"]),
            f("detached", &["aloof"]),
            f("longing", &["wistful"]),
        ],
    },
    FeelingGroup {
        name: "Low Energy & Numbness",
        feelings: &[
            f("tired", &["fatigued", "sleepy", "weary"]),
            f("exhausted", &["drained", "beat", "worn out", "burned out"]),
            f("bored", &[]),
            f("uninterested", &["indifferent"]),
            f(
                "apathetic",
                &["passive", "listless", "lethargic", "resigned"],
            ),
            f("numb", &["shut down", "blank"]),
            f("empty", &["hollow"]),
        ],
    },
    FeelingGroup {
        name: "Restlessness & Tension",
        feelings: &[
            f("restless", &["fidgety"]),
            f("tense", &["tight"]),
            f("uneasy", &["unsettled", "troubled"]),
            f("on edge", &["edgy", "jittery", "keyed up"]),
            f("agitated", &[]),
            f("impatient", &[]),
            f("overstimulated", &["sensory overloaded"]),
        ],
    },
    FeelingGroup {
        name: "Neutral & Steady",
        feelings: &[f("neutral", &["okay", "fine"]), f("steady", &["stable"])],
    },
];

/// Every valid feeling word, flattened from [`FEELING_GROUPS`] in display order.
pub fn feelings() -> impl Iterator<Item = &'static str> {
    FEELING_GROUPS
        .iter()
        .flat_map(|group| group.feelings)
        .map(|feeling| feeling.name)
}

/// Every `(alias, canonical)` pair, so an alias can resolve to its feeling.
fn feeling_aliases() -> impl Iterator<Item = (&'static str, &'static str)> {
    FEELING_GROUPS
        .iter()
        .flat_map(|group| group.feelings)
        .flat_map(|feeling| {
            feeling
                .search_aliases
                .iter()
                .map(move |alias| (*alias, feeling.name))
        })
}

pub fn normalize_feeling(feeling: &str) -> Option<String> {
    let feeling = feeling.trim().to_lowercase();
    if feelings().any(|f| f == feeling) {
        return Some(feeling);
    }
    // Fall back to a search alias so words like "joyful" resolve to "happy".
    feeling_aliases()
        .find(|(alias, _)| *alias == feeling)
        .map(|(_, canonical)| canonical.to_string())
}

/// Whether a canonical `feeling` stored on an entry should match the partial search
/// `query`: the (trimmed, lowercased) query is a substring of the feeling name or of one
/// of that feeling's search aliases. Mirrors the feelings picker filter (see
/// `EditFeelingState::visible_rows`) so `feelings:` search and the picker agree. An empty
/// query matches every feeling, like an empty `tags:` filter matches any tagged entry.
pub fn feeling_matches_search(feeling: &str, query: &str) -> bool {
    let query = query.trim().to_lowercase();
    feeling.contains(&query)
        || feeling_aliases()
            .any(|(alias, canonical)| canonical == feeling && alias.contains(&query))
}

pub fn normalize_feelings<'a>(feelings: impl IntoIterator<Item = &'a str>) -> Vec<String> {
    let mut normalized = Vec::new();
    for feeling in feelings {
        let Some(feeling) = normalize_feeling(feeling) else {
            continue;
        };
        if !normalized.contains(&feeling) {
            normalized.push(feeling);
        }
    }
    normalized
}

pub fn validate_feelings<'a>(
    values: impl IntoIterator<Item = &'a str>,
) -> Result<Vec<String>, String> {
    let mut normalized = Vec::new();
    for feeling in values {
        let trimmed = feeling.trim();
        let Some(feeling) = normalize_feeling(trimmed) else {
            return Err(format!(
                "unknown feeling '{trimmed}'; valid feelings: {}",
                feelings().collect::<Vec<_>>().join(", ")
            ));
        };
        if !normalized.contains(&feeling) {
            normalized.push(feeling);
        }
    }
    Ok(normalized)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn feeling_groups_have_no_duplicate_leaves() {
        let mut seen = HashSet::new();
        for feeling in feelings() {
            assert!(seen.insert(feeling), "duplicate feeling: {feeling}");
        }
    }

    #[test]
    fn aliases_are_unique_and_never_shadow_a_canonical_feeling() {
        let canonical: HashSet<&str> = feelings().collect();
        let mut seen = HashSet::new();
        for (alias, _) in feeling_aliases() {
            assert!(
                !canonical.contains(alias),
                "alias '{alias}' collides with a canonical feeling"
            );
            assert!(seen.insert(alias), "duplicate alias: {alias}");
        }
    }

    #[test]
    fn normalize_accepts_grouped_feelings_case_insensitively() {
        assert_eq!(normalize_feeling(" Serene "), Some("serene".to_string()));
        assert_eq!(normalize_feeling("nope"), None);
    }

    #[test]
    fn normalize_resolves_search_aliases_to_canonical_feeling() {
        assert_eq!(normalize_feeling("Joyous"), Some("happy".to_string()));
        assert_eq!(normalize_feeling("thankful"), Some("grateful".to_string()));
        assert_eq!(normalize_feeling("Worn Out"), Some("exhausted".to_string()));
    }

    #[test]
    fn feeling_matches_search_handles_partial_name_alias_and_empty() {
        // Partial canonical name: `relaxe` still finds `relaxed`.
        assert!(feeling_matches_search("relaxed", "relaxe"));
        assert!(feeling_matches_search("relaxed", "relax"));
        // Partial alias resolves onto its canonical feeling: `thank` -> `grateful`.
        assert!(feeling_matches_search("grateful", "thank"));
        assert!(feeling_matches_search("grateful", "thankful"));
        // Case-insensitive and trimmed.
        assert!(feeling_matches_search("relaxed", "  RELAX  "));
        // A non-matching query matches nothing.
        assert!(!feeling_matches_search("relaxed", "zzz"));
        // An alias of another feeling doesn't leak across canonicals.
        assert!(!feeling_matches_search("relaxed", "thank"));
        // Empty query matches any feeling.
        assert!(feeling_matches_search("relaxed", ""));
    }

    #[test]
    fn validate_feelings_resolves_aliases() {
        assert_eq!(
            validate_feelings(["joyous", "glad"]),
            Ok(vec!["happy".to_string(), "pleased".to_string()])
        );
    }
}
