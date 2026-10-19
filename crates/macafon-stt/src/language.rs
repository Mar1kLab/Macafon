//! Layout-aware selection among Whisper's supported languages.
use crate::protocol::{Language, LanguageInput, LanguagePolicy};
pub const CYRILLIC_OVERRIDE_PROBABILITY: f32 = 0.90;

pub fn choose(input: &LanguageInput, probs: Option<&[(Language, f32)]>) -> Option<Language> {
    if input.policy == LanguagePolicy::Fixed {
        return Some(input.fixed);
    }
    let allowed = |language: Language| {
        input.layout_languages.is_empty() || input.layout_languages.contains(&language)
    };
    let candidates: Vec<_> = probs
        .unwrap_or_default()
        .iter()
        .copied()
        .filter(|(language, probability)| {
            allowed(*language) && probability.is_finite() && *probability >= 0.0
        })
        .collect();
    let sum: f32 = candidates.iter().map(|(_, probability)| probability).sum();
    if sum <= 0.0 {
        return input
            .layout
            .filter(|language| allowed(*language))
            .or(input.last_cyrillic.filter(|language| allowed(*language)));
    }
    let best = candidates
        .iter()
        .max_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(language, _)| *language)?;
    if input.policy == LanguagePolicy::Auto {
        return Some(best);
    }
    // Other target layouts are unambiguous; preserve the existing EN/UK/RU speech heuristic.
    if let Some(layout) = input
        .layout
        .filter(|language| !matches!(language.code(), "en" | "uk" | "ru"))
    {
        return Some(layout);
    }
    if !matches!(best.code(), "en" | "uk" | "ru") {
        return Some(best);
    }
    let probability = |code| {
        candidates
            .iter()
            .find(|(language, _)| language.code() == code)
            .map_or(0.0, |(_, p)| p / sum)
    };
    let (en, uk, ru) = (probability("en"), probability("uk"), probability("ru"));
    if en > uk + ru {
        return Language::from_code("en");
    }
    let hint = input
        .layout
        .filter(|language| language.is_cyrillic() && allowed(*language))
        .or(input
            .last_cyrillic
            .filter(|language| language.is_cyrillic() && allowed(*language)));
    if let Some(hint) = hint {
        if (hint.code() == "uk" && ru < CYRILLIC_OVERRIDE_PROBABILITY)
            || (hint.code() == "ru" && uk < CYRILLIC_OVERRIDE_PROBABILITY)
        {
            return Some(hint);
        }
    }
    Language::from_code(if uk >= ru { "uk" } else { "ru" })
        .filter(|language| allowed(*language))
        .or(Some(best))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn policy_table() {
        use LanguagePolicy::{Auto, Fixed, Layout};
        let lang = |code| Language::from_code(code).expect("supported language");
        let cases = [
            (
                Fixed,
                Some("en"),
                None,
                Some([0.01, 0.01, 0.98]),
                Some("uk"),
            ),
            (
                Layout,
                Some("en"),
                None,
                Some([0.05, 0.15, 0.8]),
                Some("ru"),
            ),
            (Layout, Some("uk"), None, Some([0.1, 0.2, 0.7]), Some("uk")),
            (
                Layout,
                Some("uk"),
                None,
                Some([0.01, 0.09, 0.90]),
                Some("ru"),
            ),
            (
                Layout,
                Some("ru"),
                None,
                Some([0.01, 0.90, 0.09]),
                Some("uk"),
            ),
            (
                Layout,
                Some("en"),
                Some("uk"),
                Some([0.1, 0.2, 0.7]),
                Some("uk"),
            ),
            (
                Layout,
                None,
                Some("ru"),
                Some([0.01, 0.95, 0.04]),
                Some("uk"),
            ),
            (
                Layout,
                Some("uk"),
                Some("ru"),
                Some([0.6, 0.3, 0.1]),
                Some("en"),
            ),
            (Auto, Some("uk"), None, Some([0.4, 0.25, 0.35]), Some("en")),
            (Auto, Some("uk"), None, Some([0.1, 0.2, 0.7]), Some("ru")),
            (Layout, Some("ru"), None, None, Some("ru")),
            (Layout, None, Some("uk"), None, Some("uk")),
            (Auto, None, None, None, None),
        ];
        for (policy, layout, last, probs, expected) in cases {
            let input = LanguageInput {
                policy,
                fixed: lang("uk"),
                layout: layout.map(lang),
                last_cyrillic: last.map(lang),
                layout_languages: vec![],
            };
            let probs =
                probs.map(|p| vec![(lang("en"), p[0]), (lang("uk"), p[1]), (lang("ru"), p[2])]);
            assert_eq!(
                choose(&input, probs.as_deref()),
                expected.map(lang),
                "{input:?}"
            );
        }
        let mut input = LanguageInput {
            policy: Auto,
            fixed: lang("fr"),
            layout: Some(lang("de")),
            last_cyrillic: None,
            layout_languages: vec![lang("de"), lang("fr")],
        };
        let probs = [(lang("en"), 0.8), (lang("fr"), 0.15), (lang("de"), 0.05)];
        assert_eq!(choose(&input, Some(&probs)), Some(lang("fr")));
        input.policy = Layout;
        assert_eq!(choose(&input, Some(&probs)), Some(lang("de")));
    }
}
