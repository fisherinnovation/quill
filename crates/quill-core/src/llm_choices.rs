//! The models Settings recommends for summaries and dictation clean-up.
//!
//! Clean-up recommendations come from the measured comparison in
//! `eval/dictation/README.md` (latency and correctness on the eval clips),
//! leaving out models that failed there. Summary models are described rather
//! than ranked; `quill-cli eval` compares them on your own transcripts.

/// A recommended model, by OpenRouter id (or `ollama:<model>`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Recommended {
    pub id: &'static str,
    pub label: &'static str,
    pub note: &'static str,
}

/// For meeting notes. The first is the default.
pub const SUMMARY: &[Recommended] = &[
    Recommended {
        id: crate::settings::DEFAULT_SUMMARY_MODEL,
        label: "Claude Haiku 5.5",
        note: "Fast and inexpensive (the default)",
    },
    Recommended {
        id: "anthropic/claude-sonnet-5.5",
        label: "Claude Sonnet 5.5",
        note: "More thorough notes; costs more",
    },
    Recommended {
        id: "openai/gpt-6-luna",
        label: "GPT-6 Luna",
        note: "Fast and inexpensive",
    },
    Recommended {
        id: "google/gemini-3.8-flash",
        label: "Gemini 3.8 Flash",
        note: "Fast",
    },
    Recommended {
        id: "deepseek/deepseek-v4.1-flash",
        label: "DeepSeek V4.1 Flash",
        note: "Least expensive",
    },
];

/// For dictation clean-up, where speed matters most. The first is the
/// default.
pub const CLEANUP: &[Recommended] = &[
    Recommended {
        id: crate::settings::DEFAULT_POLISH_MODEL,
        label: "Gemini 3.5 Flash-Lite",
        note: "Fastest reliable: about 0.7 s (the default)",
    },
    Recommended {
        id: "deepseek/deepseek-v4.1-flash",
        label: "DeepSeek V4.1 Flash",
        note: "About as fast, but sometimes slow to answer",
    },
    Recommended {
        id: "anthropic/claude-haiku-5.5",
        label: "Claude Haiku 5.5",
        note: "Accurate, but about 1.1 s",
    },
];

/// Rough size of an hour-long meeting's summary request, to show prices as
/// "per hour of meeting": about 9,000 spoken words plus the prompt in, and
/// the notes out.
pub const HOUR_INPUT_TOKENS: f64 = 13_000.0;
pub const HOUR_OUTPUT_TOKENS: f64 = 1_500.0;

/// Dollars for summarizing an hour-long meeting at these prices (per
/// million tokens).
pub fn cost_per_hour(input_per_million: f64, output_per_million: f64) -> f64 {
    (input_per_million * HOUR_INPUT_TOKENS + output_per_million * HOUR_OUTPUT_TOKENS) / 1_000_000.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_come_first() {
        assert_eq!(SUMMARY[0].id, crate::settings::DEFAULT_SUMMARY_MODEL);
        assert_eq!(CLEANUP[0].id, crate::settings::DEFAULT_POLISH_MODEL);
    }

    #[test]
    fn hour_cost() {
        // Claude Haiku 5.5: $0.10 in, $0.50 out per million tokens.
        assert!((cost_per_hour(0.10, 0.50) - 0.00205).abs() < 1e-9);
    }
}
