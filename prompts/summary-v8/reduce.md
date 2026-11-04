A long meeting ({{duration}}) was split into {{parts}} consecutive parts, and
detailed notes were taken for each part. Combine them into one set of meeting
notes.

Merge duplicates (a topic or action item may appear in several parts), keep
the chronological order of topics, and keep every decision and action item
with its [mm:ss] timestamp (the earliest, when merging).
If a later part changes or reverses something from an earlier part, report
the final outcome.
Keep the specifics the parts kept: the cause a speaker stressed, one
concrete example per claim, and whether something is past or current.

Use exactly this Markdown structure. Omit a section only if the meeting truly
has nothing for it.

# {short descriptive meeting title} — {{date}}

## TL;DR
3–5 bullets: the most important outcomes, in order of importance.

## Decisions
### Decided
- What was agreed, with who decided if clear. [mm:ss]
### Direction given
- **Who** — an instruction or steer given without discussion. [mm:ss]
### Proposed, not agreed
- An idea put forward that wasn't settled. [mm:ss]
Leave out a group with nothing in it, and the whole section if all three are
empty. Never write "no decisions were made" and then list some.

## Action items
- **Owner** — task — due date [mm:ss]
- **Owner** (suggested) — a task someone suggested but nobody took on [mm:ss]
An action item is committed when its owner said they'd do it or agreed when
asked; mark the rest "(suggested)". Give a due date only if one was
mentioned, in the speaker's words; otherwise end at the task.

## Discussion by topic
### {topic}

## Open questions / risks

## Notable quotes
- Up to 3 short verbatim quotes, from the other people rather than the
  recorder, including any of them saying plainly how they feel about the
  work or how the meeting went. Optional.

{{you}}{{vocabulary}}Notes from each part:

{{notes}}
