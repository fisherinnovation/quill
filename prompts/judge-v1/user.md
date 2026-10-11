Score each candidate below on these criteria, each from 1 (very poor) to 10
(excellent):

- completeness: covers every important topic, decision and outcome in the
  transcript.
- accuracy: everything stated matches the transcript (names, numbers, dates,
  who said or owns what).
- action_items: every action item in the transcript is listed with the right
  owner and due date (when one was mentioned); nothing that wasn't agreed is
  listed as an action item.
- faithfulness: every claim, name, number and decision is supported by the
  transcript. 10 = nothing invented; deduct for each unsupported claim.
- formatting: follows the requested structure (title, TL;DR, Decisions,
  Action items, Discussion by topic, Open questions / risks), is easy to scan,
  and is concise.
- overall: your overall judgement of how useful and trustworthy these notes
  are for someone who missed the meeting. Not a plain average: a serious
  accuracy or faithfulness problem should pull it down a lot.

First read the transcript. Then evaluate each candidate independently against
it. Then reply with ONLY a JSON object, no other text, in exactly this shape:

{
  "A": {"completeness": 0, "accuracy": 0, "action_items": 0, "faithfulness": 0, "formatting": 0, "overall": 0, "notes": "one or two sentences on the main strengths and problems"},
  "B": { ... }
}

Include every candidate label: {{labels}}.

<transcript>
{{transcript}}
</transcript>

{{candidates}}
