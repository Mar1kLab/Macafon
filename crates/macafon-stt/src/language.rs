//! Restricted English/Ukrainian/Russian language selection.
use crate::protocol::{Language, LanguageInput, LanguagePolicy};

pub const CYRILLIC_OVERRIDE_PROBABILITY: f32 = 0.90;

pub fn choose(input: &LanguageInput, probs: Option<[f32; 3]>) -> Option<Language> {
    use Language::{En, Ru, Uk};
    if input.policy == LanguagePolicy::Fixed {
        return Some(input.fixed);
    }
    let Some(p) = probs
        .filter(|p| p.iter().all(|v| v.is_finite() && *v >= 0.0) && p.iter().sum::<f32>() > 0.0)
    else {
        return input
            .layout
            .or(input.last_cyrillic.filter(|l| l.is_cyrillic()));
    };
    let sum: f32 = p.iter().sum();
    let [en, uk, ru] = p.map(|v| v / sum);
    if input.policy == LanguagePolicy::Auto {
        return Some(if en >= uk && en >= ru {
            En
        } else if uk >= ru {
            Uk
        } else {
            Ru
        });
    }
    if en > uk + ru {
        return Some(En);
    }
    let hint = input
        .layout
        .filter(|l| l.is_cyrillic())
        .or(input.last_cyrillic.filter(|l| l.is_cyrillic()));
    Some(match hint {
        Some(Uk) if ru < CYRILLIC_OVERRIDE_PROBABILITY => Uk,
        Some(Ru) if uk < CYRILLIC_OVERRIDE_PROBABILITY => Ru,
        _ if uk >= ru => Uk,
        _ => Ru,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn policy_table() {
        use Language::{En, Ru, Uk};
        use LanguagePolicy::{Auto, Fixed, Layout};
        let cases = [
            (Fixed, Some(En), None, Some([0.01, 0.01, 0.98]), Some(Uk)),
            (Layout, Some(En), None, Some([0.05, 0.15, 0.8]), Some(Ru)),
            (Layout, Some(Uk), None, Some([0.1, 0.2, 0.7]), Some(Uk)),
            (Layout, Some(Uk), None, Some([0.01, 0.09, 0.90]), Some(Ru)),
            (Layout, Some(Ru), None, Some([0.01, 0.90, 0.09]), Some(Uk)),
            (Layout, Some(En), Some(Uk), Some([0.1, 0.2, 0.7]), Some(Uk)),
            (Layout, None, Some(Ru), Some([0.01, 0.95, 0.04]), Some(Uk)),
            (Layout, Some(Uk), Some(Ru), Some([0.6, 0.3, 0.1]), Some(En)),
            (Auto, Some(Uk), None, Some([0.4, 0.25, 0.35]), Some(En)),
            (Auto, Some(Uk), None, Some([0.1, 0.2, 0.7]), Some(Ru)),
            (Layout, Some(Ru), None, None, Some(Ru)),
            (Layout, None, Some(Uk), None, Some(Uk)),
            (Auto, None, None, None, None),
        ];
        for (policy, layout, last_cyrillic, probs, expected) in cases {
            let input = LanguageInput {
                policy,
                fixed: Uk,
                layout,
                last_cyrillic,
            };
            assert_eq!(choose(&input, probs), expected, "{input:?}, {probs:?}");
        }
    }
}
