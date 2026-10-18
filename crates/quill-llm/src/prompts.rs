//! Versioned prompt templates.
//!
//! Templates live as Markdown in the repo's `prompts/` directory and are
//! compiled into the binary with `include_str!`, so a build always carries
//! exactly the prompts it was tested with. Placeholders are `{{name}}`; single
//! braces like `{topic}` are instructions to the model and left alone.
//!
//! To change a prompt, copy the directory to the next version
//! (`summary-v3/`), edit it, add it to [`SUMMARY_VERSIONS`] and point
//! [`SUMMARY`] at it, so summaries made
//! with the old version stay attributable and comparable in evals.

/// One version of the summary prompts.
#[derive(Debug, Clone, Copy)]
pub struct SummaryPrompts {
    /// Recorded with every summary, e.g. `summary-v1`.
    pub version: &'static str,
    pub system: &'static str,
    /// Whole transcript in one request. Vars: date, duration, transcript.
    pub single: &'static str,
    /// One part of a long transcript. Vars: part, parts, range, transcript.
    pub chunk: &'static str,
    /// Combine part notes. Vars: date, duration, parts, notes.
    pub reduce: &'static str,
}

// `concat!(env!("CARGO_MANIFEST_DIR"), ...)` builds an absolute path at
// compile time, so the include works no matter where cargo is run from.
macro_rules! prompt {
    ($file:literal) => {
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../prompts/",
            $file
        ))
    };
}

pub const SUMMARY_V1: SummaryPrompts = SummaryPrompts {
    version: "summary-v1",
    system: prompt!("summary-v1/system.md"),
    single: prompt!("summary-v1/single.md"),
    chunk: prompt!("summary-v1/chunk.md"),
    reduce: prompt!("summary-v1/reduce.md"),
};

/// v2: resolves "I/you/we" relative to each speaker (v1 once reversed who
/// was on the speakers vs. the mic) and calls the recorder "you" rather
/// than "Me". Templates otherwise unchanged.
pub const SUMMARY_V2: SummaryPrompts = SummaryPrompts {
    version: "summary-v2",
    system: prompt!("summary-v2/system.md"),
    single: prompt!("summary-v2/single.md"),
    chunk: prompt!("summary-v2/chunk.md"),
    reduce: prompt!("summary-v2/reduce.md"),
};

/// v3: understands voice-separated speakers ("Speaker 1", …) and names the
/// user gave them, as well as "Me" / "Others". Templates otherwise unchanged.
pub const SUMMARY_V3: SummaryPrompts = SummaryPrompts {
    version: "summary-v3",
    system: prompt!("summary-v3/system.md"),
    single: prompt!("summary-v3/single.md"),
    chunk: prompt!("summary-v3/chunk.md"),
    reduce: prompt!("summary-v3/reduce.md"),
};

/// The version used by default.
pub const SUMMARY: SummaryPrompts = SUMMARY_V3;

/// Every summary prompt version this build knows, oldest first. Add new
/// versions here (keep old ones so evals can compare against them).
pub const SUMMARY_VERSIONS: &[SummaryPrompts] = &[SUMMARY_V1, SUMMARY_V2, SUMMARY_V3];

/// Looks up a summary prompt version by name, e.g. `summary-v1`.
pub fn find_summary(version: &str) -> Option<&'static SummaryPrompts> {
    SUMMARY_VERSIONS.iter().find(|p| p.version == version)
}

/// Prompts for the LLM judge that scores summaries in evals.
#[derive(Debug, Clone, Copy)]
pub struct JudgePrompts {
    pub version: &'static str,
    pub system: &'static str,
    /// Vars: labels, transcript, candidates.
    pub user: &'static str,
}

pub const JUDGE: JudgePrompts = JudgePrompts {
    version: "judge-v1",
    system: prompt!("judge-v1/system.md"),
    user: prompt!("judge-v1/user.md"),
};

/// Prompts that clean up dictated text before it's typed.
#[derive(Debug, Clone, Copy)]
pub struct DictationPrompts {
    pub version: &'static str,
    /// Vars: vocabulary, destination (where the text is going and how it
    /// should read there; may be empty), and from v3 about (what the user
    /// wrote about themselves; may be empty).
    pub system: &'static str,
    /// Vars: transcript.
    pub user: &'static str,
}

/// v2: adds the destination app and its style (v1, in prompts/dictation-v1,
/// had no destination).
pub const DICTATION_V2: DictationPrompts = DictationPrompts {
    version: "dictation-v2",
    system: prompt!("dictation-v2/system.md"),
    user: prompt!("dictation-v2/user.md"),
};

/// v3: adds what the user wrote about themselves (Dictation → About you):
/// role, names, spelling preferences. Otherwise unchanged.
pub const DICTATION: DictationPrompts = DictationPrompts {
    version: "dictation-v3",
    system: prompt!("dictation-v3/system.md"),
    user: prompt!("dictation-v3/user.md"),
};

/// Prompts that keep live notes during a meeting (quill_core::live_meeting).
#[derive(Debug, Clone, Copy)]
pub struct LivePrompts {
    pub version: &'static str,
    pub system: &'static str,
    /// Vars: minutes, notes (JSON), dismissed (item ids), transcript.
    pub user: &'static str,
}

pub const LIVE: LivePrompts = LivePrompts {
    version: "live-v1",
    system: prompt!("live-v1/system.md"),
    user: prompt!("live-v1/user.md"),
};

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum RenderError {
    #[error("template uses {{{{{0}}}}} but no value was given")]
    Missing(String),
    #[error("value given for {{{{{0}}}}} but the template doesn't use it")]
    Unused(String),
}

/// Fills `{{name}}` placeholders. Strict both ways: a placeholder without a
/// value or a value without a placeholder is an error, so a typo in a
/// template or a caller can't silently send a broken prompt.
pub fn render(template: &str, vars: &[(&str, &str)]) -> Result<String, RenderError> {
    let mut out =
        String::with_capacity(template.len() + vars.iter().map(|(_, v)| v.len()).sum::<usize>());
    let mut used = vec![false; vars.len()];
    let mut rest = template;
    while let Some(open) = rest.find("{{") {
        out.push_str(&rest[..open]);
        let after = &rest[open + 2..];
        let Some(close) = after.find("}}") else {
            // An unmatched `{{` is just text.
            out.push_str(&rest[open..]);
            rest = "";
            break;
        };
        let name = after[..close].trim();
        let index = vars
            .iter()
            .position(|(k, _)| *k == name)
            .ok_or_else(|| RenderError::Missing(name.to_string()))?;
        used[index] = true;
        out.push_str(vars[index].1);
        rest = &after[close + 2..];
    }
    out.push_str(rest);

    if let Some(index) = used.iter().position(|u| !u) {
        return Err(RenderError::Unused(vars[index].0.to_string()));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fills_placeholders_and_leaves_single_braces() {
        let out = render(
            "# {title} — {{date}}\n{{ body }}",
            &[("date", "2026-10-11"), ("body", "x")],
        );
        assert_eq!(out.unwrap(), "# {title} — 2026-10-11\nx");
    }

    #[test]
    fn values_are_not_re_expanded() {
        // A transcript that happens to contain "{{date}}" stays literal.
        let out = render("{{transcript}}", &[("transcript", "said {{date}}")]);
        assert_eq!(out.unwrap(), "said {{date}}");
    }

    #[test]
    fn rejects_missing_and_unused_values() {
        assert_eq!(render("{{a}}", &[]), Err(RenderError::Missing("a".into())));
        assert_eq!(
            render("{{a}}", &[("a", "1"), ("b", "2")]),
            Err(RenderError::Unused("b".into()))
        );
    }

    /// Every shipped template renders with exactly the variables its doc
    /// comment promises (catches template typos at test time).
    #[test]
    fn shipped_templates_render() {
        for p in SUMMARY_VERSIONS {
            check_summary_templates(p);
        }
        render(JUDGE.system, &[]).unwrap();
        render(
            JUDGE.user,
            &[("labels", "A, B"), ("transcript", "t"), ("candidates", "c")],
        )
        .unwrap();
        render(
            DICTATION.system,
            &[
                ("vocabulary", "Quill"),
                ("destination", "It's going into Slack."),
                ("about", "I build Quill."),
            ],
        )
        .unwrap();
        render(DICTATION.user, &[("transcript", "t")]).unwrap();
    }

    fn check_summary_templates(p: &SummaryPrompts) {
        render(p.system, &[]).unwrap();
        render(
            p.single,
            &[("date", "d"), ("duration", "1 min"), ("transcript", "t")],
        )
        .unwrap();
        render(
            p.chunk,
            &[
                ("part", "1"),
                ("parts", "2"),
                ("range", "00:00–30:00"),
                ("transcript", "t"),
            ],
        )
        .unwrap();
        render(
            p.reduce,
            &[
                ("date", "d"),
                ("duration", "2 h"),
                ("parts", "2"),
                ("notes", "n"),
            ],
        )
        .unwrap();
    }

    /// Golden file for the live-notes prompt; regenerate like the others.
    #[test]
    fn live_prompt_golden() {
        let rendered = render(
            LIVE.user,
            &[
                ("minutes", "12"),
                (
                    "notes",
                    r#"{"gist": "Planning the launch.", "items": [{"id": "a1", "kind": "ask", "text": "Ask who owns the rollout plan", "who": null, "done": false}]}"#,
                ),
                ("dismissed", "f2"),
                (
                    "transcript",
                    "[11:02] Me: Priya, can you take the rollout plan?\n[11:05] Others: Sure, by Wednesday.",
                ),
            ],
        )
        .unwrap();
        let golden = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join(format!("tests/golden/{}-user.md", LIVE.version));
        if std::env::var_os("UPDATE_GOLDEN").is_some() {
            std::fs::write(&golden, &rendered).unwrap();
        }
        assert_eq!(rendered, std::fs::read_to_string(&golden).unwrap());
    }

    /// Golden file: the fully rendered single-pass prompt for a tiny
    /// transcript. If a template change is intended, regenerate with
    /// `UPDATE_GOLDEN=1 cargo test -p quill-llm`, then review the diff.
    #[test]
    fn single_prompt_golden() {
        let rendered = render(
            SUMMARY.single,
            &[
                ("date", "2026-10-11"),
                ("duration", "2 min"),
                (
                    "transcript",
                    "[00:00] Me: Shall we ship Friday?\n[00:04] Others: Yes, Friday works.",
                ),
            ],
        )
        .unwrap();
        let golden = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join(format!("tests/golden/{}-single.md", SUMMARY.version));
        if std::env::var_os("UPDATE_GOLDEN").is_some() {
            std::fs::create_dir_all(golden.parent().unwrap()).unwrap();
            std::fs::write(&golden, &rendered).unwrap();
        }
        assert_eq!(rendered, std::fs::read_to_string(&golden).unwrap());
    }
}
