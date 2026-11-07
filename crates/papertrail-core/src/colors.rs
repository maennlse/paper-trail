// SPDX-License-Identifier: GPL-3.0-or-later
pub const PRESETS: [&str; 12] = [
    "#d49b16", "#7a9a01", "#1ea672", "#148085", "#0f81b8", "#2563c9", "#6f5bd6", "#a44ab8",
    "#d6457b", "#c13f3f", "#d86a1f", "#8d5a3b",
];

pub fn normalize(value: &serde_json::Value) -> String {
    if let Some(n) = value.as_i64() {
        return format!("preset-{}", n.clamp(0, 11));
    }
    let text = value.as_str().unwrap_or_default().trim().to_lowercase();
    if text
        .strip_prefix("preset-")
        .and_then(|s| s.parse::<usize>().ok())
        .is_some_and(|i| i < 12)
    {
        return text;
    }
    if let Some(hex) = text.strip_prefix('#') {
        if hex.chars().all(|c| c.is_ascii_hexdigit()) {
            if hex.len() == 6 {
                return text;
            }
            if hex.len() == 3 {
                return format!("#{}", hex.chars().flat_map(|c| [c, c]).collect::<String>());
            }
        }
    }
    "preset-0".into()
}

pub fn hex(token: &str) -> String {
    let normalized = normalize(&token.into());
    normalized
        .strip_prefix("preset-")
        .and_then(|n| n.parse::<usize>().ok())
        .map(|n| PRESETS[n].into())
        .unwrap_or(normalized)
}

pub fn css_class(token: &str) -> String {
    format!("folder-color-{}", hex(token).trim_start_matches('#'))
}

pub fn badge(title: &str) -> String {
    let title = title.replace(['_', '-'], " ");
    let words: Vec<_> = title.split_whitespace().collect();
    let value: String = if words.len() >= 2 {
        words
            .iter()
            .take(2)
            .filter_map(|w| w.chars().next())
            .collect()
    } else {
        title
            .chars()
            .filter(|c| c.is_alphanumeric())
            .take(2)
            .collect()
    };
    let value: String = value.to_uppercase().chars().take(2).collect();
    if value.is_empty() {
        "F".into()
    } else {
        value
    }
}
