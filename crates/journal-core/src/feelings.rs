pub const FEELINGS: &[&str] = &[
    "calm",
    "content",
    "grateful",
    "hopeful",
    "joyful",
    "excited",
    "energized",
    "focused",
    "proud",
    "relieved",
    "curious",
    "okay",
    "mixed",
    "tired",
    "bored",
    "sad",
    "lonely",
    "anxious",
    "stressed",
    "overwhelmed",
    "frustrated",
    "angry",
    "guilty",
    "numb",
];

pub fn normalize_feeling(feeling: &str) -> Option<String> {
    let feeling = feeling.trim().to_lowercase();
    FEELINGS.contains(&feeling.as_str()).then_some(feeling)
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
    feelings: impl IntoIterator<Item = &'a str>,
) -> Result<Vec<String>, String> {
    let mut normalized = Vec::new();
    for feeling in feelings {
        let trimmed = feeling.trim();
        let Some(feeling) = normalize_feeling(trimmed) else {
            return Err(format!(
                "unknown feeling '{trimmed}'; valid feelings: {}",
                FEELINGS.join(", ")
            ));
        };
        if !normalized.contains(&feeling) {
            normalized.push(feeling);
        }
    }
    Ok(normalized)
}
