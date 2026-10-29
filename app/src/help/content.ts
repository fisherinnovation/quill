// The Help page's articles: how Quill works and how to get the most from
// it. Bodies are Markdown; a key cap is written as inline code starting
// with "key:" (`key:Esc`). Wording that differs by platform comes from the
// constants below, and sections only one platform has say so.
import { isMac } from "../api";

export interface HelpSection {
  /** Stable, for links (`#live-notes`) and the table of contents. */
  id: string;
  title: string;
  /** Extra words people might search with that the text doesn't use. */
  keywords?: string;
  body: string;
  only?: "mac" | "windows";
}

export interface HelpTopic {
  id: string;
  title: string;
  /** One line under the topic's heading. */
  intro: string;
  sections: HelpSection[];
}

const MOD = isMac ? "⌘" : "Ctrl";
const ALT = isMac ? "⌥ Option" : "Alt";
const TRAY = isMac ? "menu-bar icon" : "tray icon (by the clock)";
const FILES = isMac ? "Finder" : "File Explorer";
const KEYS = isMac ? "the macOS Keychain" : "Windows Credential Manager";
const DATA = isMac
  ? "~/Library/Application Support/com.fisherinnovation.Quill"
  : "%APPDATA%\\fisherinnovation\\Quill\\data";

const ALL: HelpTopic[] = [
  {
    id: "start",
    title: "Getting started",
    intro: "What Quill does, and the few minutes of set-up it needs.",
    sections: [
      {
        id: "what-quill-does",
        title: "What Quill does",
        keywords: "overview introduction about features",
        body: `Quill does two things, both on your computer:

- **Meeting notes.** Record any call (Zoom, Google Meet, Teams, a browser tab, a phone app) and Quill writes a transcript that says who said what, then notes: a TL;DR, decisions, action items with owners, the discussion by topic and open questions. Nothing joins your call as a bot.
- **Dictation.** Hold a shortcut in any app, speak, and let go: your words are typed where your cursor is, with the "um"s and false starts tidied away.

Audio never leaves your computer. Speech is turned into text locally by Whisper. Only text goes to the model that writes notes or tidies dictation, and you choose that model: one through OpenRouter, or a local one with Ollama so nothing leaves your machine at all. See [What leaves your computer](#what-leaves).`,
      },
      {
        id: "first-run",
        title: "First run: permissions and models",
        keywords: "setup onboarding permission microphone system audio accessibility download install",
        body: `The welcome screen walks you through three things.

1. **Permissions.** An access check records a few seconds and plays a short chime, which also brings up ${isMac ? "macOS's" : "Windows'"} prompts. Allow **Microphone**${isMac ? " and **System Audio Recording**" : ""}. Quill needs both your microphone and the sound of the call to record a meeting.${isMac ? `
2. **Accessibility** (for dictation). Quill types by pasting for you, which macOS only allows with Accessibility access. Allow it from the **Dictations** page. Without it, dictated text is left on your clipboard for you to paste.` : ""}
${isMac ? "3" : "2"}. **Models.** Speech recognition, silence detection and telling speakers apart use models that run on your computer, about 1.7 GB in all. They download from Hugging Face and each is checked against a known fingerprint before use. Skip this and they download the first time they're needed.

You can run the access check again any time in **Settings → General → Audio access**.`,
      },
      {
        id: "connect-model",
        title: "Choosing who writes your notes",
        keywords: "openrouter api key ollama local model setup summary clean-up",
        body: `Meeting notes and dictation clean-up are written by a language model. You have two options:

- **OpenRouter** (the default): one API key gives access to hundreds of models. Add it in **Settings → General → OpenRouter** with **Add key…**; **Get a key** opens the page to create one. The key is kept in ${KEYS}, never in a file. A one-hour meeting typically costs well under a cent to summarize.
- **Ollama**: run models on your own computer, so no text leaves it. Install Ollama, pull a model, and it appears in Quill's model pickers.

Choose models in **Settings → Models**. See [Summary and clean-up models](#llm-models) for the details.`,
      },
      {
        id: "getting-around",
        title: "Finding your way around",
        keywords: "navigation sidebar collapse pages menu",
        body: `The sidebar leads to:

- **Search** (or \`key:${MOD}\` \`key:K\` from anywhere): everything you've recorded and dictated. See [Searching everything](#search).
- **Home**: your day at a glance, the notes from your latest meeting, recent meetings and your usage numbers.
- **Meetings**: every meeting, by day.
- **Action items**: what your meetings agreed needs doing, gathered from their notes to tick off. The number beside it is how many of yours are open.
- **Dictations**: whether dictation is ready, your dictation numbers, and what you've dictated.
- **Help**: this page.
- **Settings**: tabs for General, Meetings, Dictation, Models and Storage.

Collapse the sidebar to icons with the button at its foot or \`key:${MOD}\` \`key:\\\`; hover an icon to see its name. **Record meeting** stays at the top either way.

Closing the window doesn't quit Quill: it keeps running from the ${TRAY}, so a recording is never cut off and the dictation shortcut keeps working. To bring the window back, ${isMac ? "click Quill in the Dock, press `key:⌘` `key:0`, or choose" : "choose"} **Open Quill** from that icon's menu. Quit from the same menu.`,
      },
      {
        id: "search",
        title: "Searching everything",
        keywords: "search find look up meaning semantic related similar command palette ctrl k cmd k transcripts notes dictations",
        body: `**Search** at the top of the sidebar, or \`key:${MOD}\` \`key:K\` from anywhere, looks through everything at once:

- **Meetings**: their titles, and what was said in every transcript. A moment opens the meeting there.
- **Notes**: each meeting's latest notes, point by point (the TL;DR, each decision, topic and open question).
- **Action items**: by task or by who owns them.
- **Dictations**: what you've dictated, as corrected, while **Settings → Dictation** keeps a history.

A few of each kind show at first; **Show all** or the chips at the top narrow it to one kind. \`key:↑\` \`key:↓\` move through the results, \`key:Enter\` opens one and \`key:Esc\` closes search. The search box on the Meetings page opens it narrowed to meetings.

**Search by meaning** finds things that say what you searched for in other words, too: "when did we talk about hiring" finds "we're bringing on two more engineers". Those results are marked **Related** and come after the ones with your exact words. It works in about 100 languages and uses a 488 MB model that runs on this computer, so what you search and what's searched never leave it. The model downloads only when you ask: the line at the foot of search offers it, as does **Settings → Models**. Quill then prepares your library in the background (seconds per hour of meetings), waiting while you record or dictate; new meetings, notes and dictations are added as they arrive. Turn it off in **Settings → Meetings → Search by meaning**.`,
      },
    ],
  },
  {
    id: "recording",
    title: "Recording meetings",
    intro: "Recording a call, live notes while it happens, and what Quill does afterwards.",
    sections: [
      {
        id: "record-meeting",
        title: "Recording a meeting",
        keywords: "start stop record call zoom meet teams",
        body: `Click **Record meeting** at the top of the sidebar when the call starts, or choose **Start Recording** from the ${TRAY}. Click it again (it reads **Stop recording** while recording) when the call ends.

Quill records two tracks: your microphone, labelled **You**, and the computer's sound, which is everyone else. Because they're separate, your words are never mixed up with anyone else's, even when you talk over each other.

While recording, Home shows the meeting at the top with its timer, and the sidebar's Meetings item shows **Recording**.`,
      },
      {
        id: "pause",
        title: "Pausing and resuming",
        keywords: "pause resume break",
        body: `Pause from the ${TRAY} (**Pause Recording**) or with **Pause** in the live notes panel. Nothing is recorded while paused and the timer stands still. Resume the same way. Use it for a private aside or a break in a long session.`,
      },
      {
        id: "call-prompt",
        title: "When Quill offers to record",
        keywords: "call detection automatic prompt notification zoom teams facetime slack discord browser",
        only: "mac",
        body: `When a call app has been using your microphone for about two seconds, a small prompt rises at the bottom of the screen asking whether to record. When the call lets go of the microphone while Quill is recording, it asks whether to stop.

It recognises Zoom, Teams, Webex, FaceTime, Slack, Discord, and browsers for Google Meet and other web calls. Quill sees only *which app* is using the microphone, never its audio, windows or tabs, so a browser call shows as "Google Chrome is using your microphone".

Clicking the prompt doesn't pull you out of your call. Unanswered, it goes away by itself. Turn it off in **Settings → Meetings → Offer to record calls**.`,
      },
      {
        id: "live-notes",
        title: "Live notes during a meeting",
        keywords: "live panel side panel realtime real-time notes questions follow-ups decisions",
        body: `While you record, a panel at the right edge of your screen keeps notes as the meeting goes:

- **Ask before the end**: questions left open, and decisions without an owner or a date.
- **Follow-ups**: things someone said they'd do.
- **Decisions**: what was agreed.

A line at the top says where the conversation is. Every minute Quill updates the notes from what's been said; items the conversation deals with tick themselves off.

- **Tick** an item off yourself, or **dismiss** one with ✕ (it won't come back).
- **Update now** refreshes the notes straight away.
- **Collapse** the panel to a small tab on the screen edge; the tab shows how many questions are still open.
- **Pause** and **Stop** are in its header. Stop asks once, so a stray click doesn't end your meeting.

The panel is hidden from screen sharing and recordings, so others on the call don't see it, and clicking it doesn't bring Quill's window over your call.

Live notes are a rough draft made as you go. The full transcript and notes are still written after the recording; when you stop, the panel stays to follow that write-up (see [When your notes are ready](#notes-ready)). Each update sends the last 15 minutes of the conversation, as text, to your notes model; use an Ollama model to keep that on your computer. Turn live notes off in **Settings → Meetings → Live notes during meetings**.`,
      },
      {
        id: "after-recording",
        title: "What happens after you stop",
        keywords: "processing transcribing summarizing progress pipeline working",
        body: `When you stop, Quill works through the recording on your computer:

1. **Removes echo** of the call from your microphone, in case you used speakers rather than headphones.
2. **Transcribes** both tracks with Whisper, skipping silence so it doesn't invent words in quiet stretches.
3. **Tells the other people apart** by voice, as Speaker 1, Speaker 2 and so on.
4. **Writes the notes**, if **Summarize automatically** is on.
5. **Compacts the audio** to about 70 MB per hour.

You can watch it happen. The meeting shows each step as it goes, with how far it's got, roughly how long is left, and the lines it's hearing as they're transcribed. Home shows it at the top, the meeting's tile has a progress bar, and the sidebar says **Writing up**. You can keep using your computer, and dictate, while it runs. Telling speakers apart adds about two minutes per hour of audio.

When it's done, the meeting says what came out (people, action items, decisions, words, and how long it took) and the notes rise into view. See [When your notes are ready](#notes-ready).`,
      },
      {
        id: "notes-ready",
        title: "When your notes are ready",
        keywords: "notification toast alert finished done ready new badge write-up",
        body: `Quill tells you where you are:

- **On the meeting**, the write-up turns into **Your notes are ready**, with what came out.
- **Elsewhere in Quill**, a card at the bottom right says the notes are ready, with **Open**, and the meeting is marked **New** until you open it.
- **In your call**, if live notes were showing, the panel stays after you stop and follows the write-up, then offers **Open notes**. It goes away by itself a little later.
- **In another app**, a notification says the notes are ready (or couldn't be written). Click it to open the meeting. ${isMac ? "macOS asks whether Quill may send notifications the first time. " : ""}Turn these off in **Settings → Meetings → Tell me when notes are ready**. Notifications show the meeting's title.

If something stops the write-up, the step that failed is marked and the reason given, with **Try again**.`,
      },
      {
        id: "recording-tips",
        title: "Tips for better recordings",
        keywords: "quality headphones echo room accuracy",
        body: `- **Headphones help.** Echo removal handles speakers well, but headphones keep your track cleanest.
- **Set the meeting language** in **Settings → Meetings → Meeting language** if your meetings aren't in your computer's language.
- **Say how many people were there.** On a big call, people who say only a sentence or two may be merged into someone else. Setting **People on the call** on the transcript fixes most of it. See [Naming speakers](#speakers).
- **One microphone in a room** records everyone in it as **You**. Quill separates the people on the call, not the people around your microphone.
- **Keep Quill running.** Closing the window is fine; quitting stops the recording.`,
      },
    ],
  },
  {
    id: "meetings",
    title: "Your meetings",
    intro: "Finding meetings, reading their notes and transcripts, and naming who spoke.",
    sections: [
      {
        id: "meetings-page",
        title: "The Meetings page and search",
        keywords: "list library find search transcripts filters daily summary day digest",
        body: `At the top, **Meetings** shows what's happening (ready to record, recording, or writing up) over a weave of your recent conversations, each a ribbon of who was talking. Below it are this week's numbers: meetings, time on calls (with the last seven days), how much of the talking was you, and action items.

**Meetings** lists every meeting by day. Each shows the first point of its notes, who was there, its length, its action items and how much you spoke, over a faint picture of the conversation's back and forth.

Filters above the list show only meetings with action items, new ones, ones that need attention (not transcribed, nothing said, or failed), or meetings with the people you meet most.

The search box opens [search](#search), narrowed to meetings: their titles and every transcript. A result shows the matching line and who said it; open one to go to that moment in the meeting.

**Daily summary.** On a day with two or more meetings that have notes, **Summarize this day** under the day's heading writes one summary from those notes, with your summary model: what the day came to, the decisions as they ended up (if a later meeting changed one), your action items and other people's, threads that ran across meetings, and what's still open. The TL;DR shows first; **Decisions, action items and more** shows the rest. It's written only when you ask, so it costs nothing until then. If a meeting's notes are rewritten, or another meeting that day gets notes, it's marked **Notes changed**; **Rewrite** writes it again. Deleting any of that day's meetings deletes the day's summary too, since it holds what they said.`,
      },
      {
        id: "summary-tab",
        title: "Notes: the Summary tab",
        keywords: "summary notes tldr action items decisions export markdown copy re-summarize",
        body: `Open a meeting to see its notes: a TL;DR, decisions, action items with owners, the discussion by topic and open questions. Long meetings are summarized in parts and combined. Action items can be ticked off right in the notes; see [Action items](#action-items).

- **Re-summarize** writes the notes again, for example after naming speakers or choosing another model.
- **Copy** puts the notes on your clipboard as Markdown, ready for a doc or a message.
- **Export** saves them as a \`.md\` file.
- Click the title to rename the meeting.

If notes weren't written (no model set up, or **Summarize automatically** is off), the Summary tab has **Summarize** to write them.`,
      },
      {
        id: "action-items",
        title: "Action items",
        keywords: "action items tasks to-do todo follow up due overdue done tick checklist waiting on",
        body: `**Action items** in the sidebar gathers the action items from every meeting's notes in one list, so they don't stay buried in the notes.

- **Yours** come first, most pressing at the top: overdue, then due soonest, then those with no date.
- **Waiting on others** lists what other people agreed to do, by person, for following up.
- Click the circle to tick one off. It's ticked in the meeting's notes and on Home too, and moves to **Done**, where you can tick it back.
- Click the meeting under an item to open it.

Due dates are worked out from what was said and the day of the meeting: "by Thursday" in a Tuesday meeting means that Thursday, and "end of next week" the Friday after. Phrases that aren't a date, like "after the launch", show as written. The numbers at the top count your open items, those overdue, those due in the next three days, and what was done this week.

Items are read from the notes when they're written. If you re-summarize, items still there (even reworded) keep their ticks; ones the new notes drop go.`,
      },
      {
        id: "speakers",
        title: "Naming speakers: the Transcript tab",
        keywords: "rename speaker merge split people on the call diarization who said",
        body: `The **Transcript** tab shows the conversation turn by turn: **You**, and the people on the call as Speaker 1, Speaker 2 and so on.

- **Click a speaker's name** to rename them. The notes use the names once you re-summarize.
- **Type an existing name** to merge two speakers who are really the same person.
- **People on the call**: if Quill found too few or too many people, choose how many there were and it splits the voices again, instantly.

Quill reliably separates everyone who talks for more than a minute or so. Similar voices, and people who barely speak, are the usual mix-ups. Once you name someone, Quill can recognize their voice in later meetings: see [Known voices](#known-voices).`,
      },
      {
        id: "stats-tab",
        title: "How it went: the Stats tab",
        keywords: "statistics talk time pace turns questions timeline moments words",
        body: `The **Stats** tab shows how the conversation went:

- **Who talked**, for how long, at what pace, with how many turns and questions.
- **Who spoke when**: a timeline. Click a stretch to read it in the transcript.
- **Moments** such as the longest monologue, the longest silence, and the two people who went back and forth most.
- **What came up**: the words and phrases used most.

It's worked out from the transcript on your computer. Word lists are English-first.`,
      },
      {
        id: "nothing-said",
        title: "Recordings with nothing said",
        keywords: "empty silent blank recording delete",
        body: `A recording with no speech in it is marked **Nothing said**. Opening it offers **Delete recording** or **Keep**. **Settings → Storage** counts these and can delete them all at once after you confirm. Quill never deletes them by itself.`,
      },
      {
        id: "delete-meeting",
        title: "Deleting a meeting",
        keywords: "remove delete meeting audio",
        body: `Use the delete button on a meeting and confirm. Its transcript, notes, speaker fingerprints and audio are all removed from your computer. Voices you named in it stay known until you forget them (see [Known voices](#known-voices)).`,
      },
      {
        id: "known-voices",
        title: "Known voices: recognizing people again",
        keywords: "recognize remember voice name automatically same person speaker identification fingerprint forget",
        body: `When you name someone on the call, Quill remembers their voice, and in later meetings gives their name to a speaker whose voice matches. It only does so when it's confident. Otherwise they stay Speaker 1, 2…, since a wrong name is worse than none.

- **Recognized names** are marked with a sparkle and a dashed outline on the **Transcript** tab. Click **✓** if it's right: Quill learns their voice from this meeting too. If it's someone else, click the name and type the right one; this meeting's voice is taken away from the wrong person.
- **Only what you say teaches Quill.** Naming, merging or confirming someone adds this meeting's voice to theirs; a recognized name you haven't confirmed doesn't. People who said less than about 30 seconds aren't remembered, and you're never remembered: your microphone is a track of its own. For a meeting written up before Quill could remember voices, choose **People on the call** first; naming people there then teaches Quill too.
- **Settings → Meetings** lists the known voices. Remove one to forget it, or **Forget all voices**. Transcripts keep the names. Turn off **Recognize voices** to stop both recognizing and remembering.

Voices are stored as fingerprints (lists of numbers, not audio) in your library on this computer, and never sent anywhere. Recognition was tested on recorded meetings where the same people met several times; voices can sound different on a different microphone or a poor connection, so a familiar person may stay unnamed.`,
      },
      {
        id: "ai-assistants",
        title: "Asking Claude about your meetings",
        keywords: "mcp model context protocol claude desktop claude code chatgpt assistant ai agent connector integration localhost url address token port mcp-remote",
        body: `Turn on **Settings → Meetings → Let AI assistants read your meetings**, confirm, and an AI assistant such as Claude Code can look through your meetings to answer what you ask it: "what did we decide about pricing last week?", "what do I owe Priya?". It can search your meetings, list them by day, read their notes and transcripts, see your action items and read daily summaries. It can't change anything, and it never sees your dictations.

**What's shared.** Whatever the assistant reads goes to that assistant's provider (for Claude, Anthropic), like anything else you show it, so turn this on only if you're happy for your meetings to go there. It reads only when you ask it something. Turning the setting off stops it at once, even mid-conversation.

**Connecting Claude Code.** While the setting is on and Quill is open, Quill serves assistants at an address on this computer, shown in Settings with a token. **Copy command** there copies what to run once in a terminal; Claude Code then connects whenever it starts. No restart of Quill is needed.

**Other assistants.** Ones that connect by URL take the address and the token, sent as an \`Authorization: Bearer <token>\` header. Claude Desktop can't reach an address on your computer through its own connectors; bridge it with \`mcp-remote\` in its configuration file (**Settings → Developer → Edit Config**; needs Node.js): \`command\` \`npx\`, \`args\` \`["mcp-remote", "<address>", "--header", "Authorization: Bearer <token>"]\`.

**Keeping it private.** Only this computer can reach the address, only with the token, and web pages can't use it. If the token gets out, **New token…** locks out everything set up with the old one. If another app uses the port, change it in the same section; assistants then need the new address.`,
      },
    ],
  },
  {
    id: "dictation",
    title: "Dictation",
    intro: "Typing with your voice in any app, and teaching Quill your words.",
    sections: [
      {
        id: "dictating",
        title: "Dictating",
        keywords: "dictate speak voice typing shortcut hold tap cancel sound beep chime",
        body: `Put your cursor in any text field, then either:

- **Hold** \`key:${ALT}\` \`key:Space\` while you speak, and let go to type, or
- **Tap** it, speak, and tap it again.

Press \`key:Esc\` to cancel. A small pill near the bottom of the screen shows your microphone level while it listens and "Typing…" while it works. Text appears about a third of a second after you let go.

Quill types by pasting. Whatever was on your clipboard is put back afterwards, and the dictated text is hidden from clipboard managers.

A short sound plays as Quill starts and stops listening, and another if it cancels. Pick a different one (Classic, Soft, Bubble, Wood, Glass, Marimba, Chirp or Digital), or none, under **Settings → Dictation → Sound**; choosing one plays it.

Change the shortcut in **Settings → Dictation → Shortcut**.${isMac ? " You can also use the fn key on its own; see [Dictating with fn](#fn-key)." : " You can also use a single key; see [Dictating with one key](#one-key)."}`,
      },
      {
        id: "fn-key",
        title: "Dictating with fn",
        keywords: "fn globe key emoji input source",
        only: "mac",
        body: `Choose **fn** under **Settings → Dictation → Shortcut** to dictate by holding \`key:fn\` (🌐) on its own. fn with another key (fn+Delete, fn+arrows, fn+F-keys) keeps working as usual. It needs the same Accessibility permission as typing.

If pressing fn also opens the emoji picker or switches keyboard input source, set macOS's **Press 🌐 key to** to **Do Nothing**. The Dictations page links to that setting.`,
      },
      {
        id: "one-key",
        title: "Dictating with one key",
        keywords: "single key right ctrl control f key insert pause scroll lock hotkey",
        only: "windows",
        body: `Click the shortcut under **Settings → Dictation → Shortcut** and press a key that doesn't type text: \`key:Right Ctrl\`, an F key, \`key:Insert\`, \`key:Pause\` or \`key:Scroll Lock\`. Then dictate by holding just that key. Right Ctrl with another key (Ctrl+C and the like) keeps working as usual.

Any other key needs \`key:Ctrl\`, \`key:Alt\` or \`key:Shift\` with it, so it doesn't fire while you type. While you choose, the current shortcut is off, so you can press it again to keep it.`,
      },
      {
        id: "captions",
        title: "Live captions",
        keywords: "captions streaming preview words appear while speaking",
        body: `While you speak, what you've said so far appears above the dictation pill. The latest words are fainter until Quill is sure of them. Most of a long dictation is transcribed before you let go, so even a minute of speech is typed in about half a second.

The captions are on your screen, so they show in screen shares. Turn them off in **Settings → Dictation → Show what you're saying**.`,
      },
      {
        id: "clean-up",
        title: "Clean-up",
        keywords: "tidy polish filler um corrections lists formatting",
        body: `With **Tidy up** on (**Settings → Dictation → Clean-up**), a model:

- removes "um"s, "uh"s and repeated words;
- applies corrections you make as you speak: "at 3, no wait, 4" becomes "at 4";
- formats spoken lists as lists.

It runs only when there's something to fix, so plain sentences are typed straight away, and its result is checked before it's used, so it can't answer your question instead of typing it. Pick its model under **Model** in the same section, or turn it off to type exactly what Whisper heard.

**About you** tells the clean-up model your role, names you use and spelling preferences, so it gets them right. It's never added to your text.`,
      },
      {
        id: "styles",
        title: "Styles per app",
        keywords: "per-app style casual formal technical chat email code search period punctuation",
        body: `Quill adjusts to the app you're typing into:

| Style | Built in for | What it does |
|---|---|---|
| Casual | Slack, Messages, Discord, WhatsApp, Telegram, Teams, Signal | A one-sentence message loses its final period, as people type in chat. |
| Formal | Mail, Outlook, Spark, Superhuman, Word, Pages | Complete, properly punctuated sentences. |
| Technical | VS Code, Cursor, Zed, Xcode, JetBrains IDEs, Terminal, iTerm, Warp, Ghostty | Code names, file names, commands and flags kept exactly as spoken. |
| Default | Everything else | Your words, tidied up. |

Change any app's style in **Settings → Dictation → Style per app**; apps appear there once you've dictated into them. A style never adds delay.

In any app, a one-line text box (a search box, the address bar, a form field) gets no final period, so a search comes out as \`best pizza in Toronto\`, not a sentence. When the app doesn't say what kind of box it is, four words or fewer lose the period, except in Formal apps.

A sentence is typed with a space after it, so you can dictate a paragraph a sentence at a time and each one starts after the last. One-line boxes get no space.`,
      },
      {
        id: "words",
        title: "Vocabulary and replacements",
        keywords: "vocabulary names jargon spelling replacements custom words dictionary",
        body: `In **Settings → Dictation → Words**:

- **Vocabulary**: names and terms to spell exactly, one per line ("Priya", "Kubernetes"). These steer both speech recognition (with Parakeet, as words it listens out for) and clean-up.
- **Replacements**: what you say → what to type, one per line ("open router → OpenRouter").`,
      },
      {
        id: "teaching",
        title: "Teaching Quill your words",
        keywords: "correct correction fix mistake learn",
        body: `When Quill gets a name or term wrong:

1. Open that dictation from **Dictations** and click **Correct**.
2. Fix the text and save (\`key:${MOD}\` \`key:Enter\`).
3. Quill compares your fix with what it typed and suggests what to learn, under **Learn from your fixes** at the top of the Dictations page. Click **Add** to accept.

A misspelled name ("Quil" → "Quill") becomes a vocabulary word; words typed apart ("open router" → "OpenRouter") become a replacement. Ordinary word changes ("their" → "there") aren't learned, because as a rule for every dictation they'd be wrong as often as right. Nothing is learned until you click Add, and you can remove anything learned in **Settings → Dictation → Words**.`,
      },
      {
        id: "history",
        title: "Your dictation history",
        keywords: "history past dictations copy lost text clipboard time to insert wait delay latency speed",
        body: `**Dictations** lists what you've dictated, by day, newest first, with each day's totals and the app each went into, shown by its icon. Above the list are your all-time numbers, and a waveform shaped by your last 30 days that follows your voice while you dictate. **Copy** one with the button beside it (handy when text didn't land where you meant), or open it to **Correct** it or delete it.

**Time to insert** is how long you wait from when you stop speaking to the text appearing: the time you keep holding the shortcut after your last word, plus Quill transcribing and typing after you let go. Quill finds where your speech ended with the same silence detection it uses to skip quiet stretches, after the text is typed, so measuring it adds no delay. The panel shows the last week's median against the week before, how it splits between holding the key and Quill, and the last 30 days, each day's median with its slowest 1 in 10 marked above. Each dictation shows its own wait too. It's measured with **Skip silence** on (the default), from the dictations made since it was added.

Each dictation is colour-coded by what happened to it:

- **Tidied up** (blue): clean-up changed what Whisper heard.
- **As spoken** (no colour): typed exactly as heard.
- **Corrected** (purple): you fixed it afterwards.
- **Not typed** (pink): Quill couldn't paste it, so it was left on your clipboard.

The chips above the list show how many of each there are; click one to show only those. Tags also mark questions, lists, and dictations over 20 seconds, with their length.

History is kept only on your computer. Turn it off or delete it all in **Settings → Dictation → History**; words Quill has learned stay.`,
      },
      {
        id: "accuracy",
        title: "Measuring accuracy",
        keywords: "accuracy test samples review wer mistakes errors word error rate",
        body: `To see how often Quill gets your words right, turn on **Settings → Dictation → Save dictations for testing** and dictate as usual. Each dictation's audio and what was typed are kept, only on your computer (about 1 MB per minute of speech).

Then click **Review…** in the same section. Each sample plays as it appears, beside what was typed. If it's right, keep it (\`key:${MOD}\` \`key:Enter\`); if not, fix it into what you said and save. Quill shows the share of words typed right, the same from Whisper alone before clean-up, and your commonest mistakes, which are good candidates for [Vocabulary and replacements](#words).

Delete a sample that's muddled or recorded by accident. Delete them all in the same section when you're done.`,
      },
      {
        id: "language",
        title: "Languages",
        keywords: "language multilingual detect auto spanish french german",
        body: `Dictation uses your computer's language unless you choose another in **Settings → Dictation → Language**. **Detect each time** handles switching between languages, at about half a second extra per dictation. Whisper understands about 100 languages; clean-up and Quill's own tests are English-first. If dictation uses Parakeet (**Settings → Models**), this setting doesn't apply: Parakeet v2 understands only English, and v3 recognises which of its 25 European languages you speak.`,
      },
    ],
  },
  {
    id: "home",
    title: "Home and your usage",
    intro: "Your day at a glance, and what the numbers mean.",
    sections: [
      {
        id: "home-page",
        title: "Home",
        keywords: "dashboard today overview latest notes",
        body: `Home opens with a line about your day: how many meetings, how long on calls, and words dictated this week. Below it are the TL;DR and action items from your latest meeting, the meetings before it, and your usage. On a wide window, usage sits in a column beside your meetings.`,
      },
      {
        id: "usage",
        title: "Usage numbers",
        keywords: "insights statistics words saved streak activity chart top apps reset",
        body: `Your usage shows words dictated, time saved compared with typing, how fast you speak, your streak, the last 30 days of dictation or meeting time, meetings in detail (including how much of the talking you did), when in the day you dictate, and your top apps.

Time saved compares your speaking speed with typing at a typical rate. The numbers are counts only: Quill keeps numbers about your dictations, not what you said. **Reset** at the bottom clears the dictation numbers.`,
      },
    ],
  },
  {
    id: "models",
    title: "Models",
    intro: "The speech models that run on your computer, and the models that write.",
    sections: [
      {
        id: "speech-models",
        title: "Speech models",
        keywords: "whisper model size download delete re-download accuracy speed vad speaker gpu graphics card vulkan",
        body: `**Settings → Models** lists each job that needs a model (speech recognition, silence detection, telling speakers apart, meeting notes and dictation clean-up), which model does it, and whether it's downloaded.

- **Whisper** turns speech into text. Larger sizes are more accurate and slower; the default, \`turbo\`, is a good balance. Pick another and it downloads straight away.
- **NVIDIA Parakeet** is a second engine, offered under **Speech recognition** and, for dictation alone, under **Speech recognition for dictation**. It runs on the CPU: on a computer without a usable graphics card, dictation is typed in a fraction of a second where Whisper takes several; with one, Whisper is as quick. **v2** understands English only; **v3** knows 25 European languages and picks the one it hears, so the language setting doesn't apply to it. Your vocabulary still works: Parakeet listens out for those words as it transcribes.${
          isMac
            ? ""
            : `
- **Graphics card**: which one speech recognition runs on. **Automatic** prefers a dedicated card to integrated graphics and names the card it picked; choose another, or **CPU only** if a card misbehaves. The change applies the next time Quill transcribes.`
        }
- Any model can be **re-downloaded** (a fresh, checked copy) or **deleted**; a deleted model downloads again when next needed.
- Downloads no job uses are gathered at the end to delete in one go.

Models are kept outside the app, so updates don't download them again. Pressing the dictation shortcut before speech recognition is downloaded opens Dictations with the download's progress; Quill never starts a download silently.`,
      },
      {
        id: "llm-models",
        title: "Summary and clean-up models",
        keywords: "openrouter ollama llm language model claude gemini cost price local",
        body: `Two jobs use a language model: writing **meeting notes** (including live notes) and **dictation clean-up**. Each picker in **Settings → Models** offers:

- a few recommended models, with what each is good at and, for notes, the rough cost of an hour-long meeting;
- the models installed in Ollama, to stay fully local;
- **Other OpenRouter model…**, which checks the id you type and shows its prices.

The defaults are chosen for accuracy and cost for notes, and for speed for clean-up. The clean-up model can also be changed in **Settings → Dictation**.`,
      },
      {
        id: "fully-local",
        title: "Keeping everything on your computer",
        keywords: "offline local private ollama no cloud",
        body: `To keep all text on your computer too:

1. Install Ollama (from ollama.com) and pull a model (for example a recent Qwen or Llama).
2. In **Settings → Models**, choose that model for meeting notes and for clean-up.

Transcription and search already run locally. With both models in Ollama, Quill's only network use is downloading speech and search models and the daily update check, which you can turn off.`,
      },
    ],
  },
  {
    id: "privacy",
    title: "Privacy and your data",
    intro: "What stays on your computer, what doesn't, and where everything is kept.",
    sections: [
      {
        id: "what-leaves",
        title: "What leaves your computer",
        keywords: "privacy security data network cloud upload sent telemetry",
        body: `| Data | Where it goes |
|---|---|
| Audio, meetings and dictation | Never leaves your computer. |
| Meeting transcripts | To your notes model when notes are written, and the last 15 minutes every minute during a recording with live notes on. |
| Dictated text | To your clean-up model, only when clean-up runs. |
| API key | ${KEYS}. Never in files or logs. |
| Searches | Stay on your computer, including search by meaning. |
| Logs | On your computer: events, timings and errors, never what was said. |
| Telemetry | None. |
| Voice fingerprints | Never leave your computer. See [Known voices](#known-voices). |

Through OpenRouter, requests go only to providers that don't keep or train on your data. **Zero data retention only** (**Settings → General → OpenRouter**) is stricter and supported by fewer models. With Ollama, nothing leaves at all.

For dictation, Quill reads the *name* of the app you're typing into (for its style and your top apps) and whether the text box is one line or several (to leave periods off searches), and shows its icon, never its windows or contents.${isMac ? " To offer to record calls, it checks which apps are using a microphone, never their audio." : ""}`,
      },
      {
        id: "data-location",
        title: "Where your data lives",
        keywords: "folder files storage disk space location database backup",
        body: `Everything is in one folder:

\`${DATA}\`

It holds the library (meetings, transcripts, notes, dictation history), each meeting's audio, the downloaded models, your settings and the logs.

**Settings → Storage** shows how much space Quill takes and what it's used for. Recordings use about 70 MB per hour once transcribed. Use **Change…** there to move the recordings folder, for example to an external drive; existing recordings move with it.`,
      },
      {
        id: "logs",
        title: "Log files",
        keywords: "logs troubleshooting debug",
        body: `Quill keeps a log of what it did and any errors, one file per day, for two weeks. Logs never include transcripts, notes, dictated text or keys. Open them from **Settings → General → Troubleshooting → Log files → Show in ${FILES}**.`,
      },
    ],
  },
  {
    id: "updates",
    title: "Updates",
    intro: "How Quill keeps itself up to date.",
    sections: [
      {
        id: "updating",
        title: "Updating Quill",
        keywords: "update upgrade new version restart",
        body: `Quill checks for a new version once a day. When there is one, Home, **Settings → General → Updates** and the About window offer **Restart to update**. Nothing downloads or installs until you click it, and it waits while you're recording, dictating or a meeting is being written up.

Updates are checked against Quill's signing key before they install. Your recordings, settings, models and permissions stay as they are. Turn the daily check off with **Check for updates automatically**; **Check now** still works.`,
      },
    ],
  },
  {
    id: "troubleshooting",
    title: "Troubleshooting",
    intro: "Fixes for the problems people run into most.",
    sections: [
      {
        id: "dictation-not-typing",
        title: "Dictated text doesn't appear",
        keywords: "not typing paste nothing happens clipboard accessibility",
        body: `- ${isMac ? "**Check Accessibility.** The Dictations page says whether Quill may type for you, with **Allow…** if not. After allowing it in System Settings, quit Quill from the menu bar and open it again." : "**Check the field.** Some apps running as administrator don't accept typing from other apps. Try a plain text field."}
- **The text is on your clipboard** if Quill couldn't paste: paste it yourself.
- **Find it in Dictations.** Every dictation is listed there (unless history is off), with **Copy**.
- **Is dictation on?** The Dictations page and **Settings → Dictation** say whether it's ready.`,
      },
      {
        id: "shortcut-conflict",
        title: "The shortcut does something else",
        keywords: "shortcut conflict hotkey alt space launcher raycast alfred spotlight",
        body: `\`key:${ALT}\` \`key:Space\` is also the default for some launchers and assistants. If another app already has it, Quill says so; choose another in **Settings → Dictation → Shortcut**.${isMac ? " If fn opens the emoji picker, see [Dictating with fn](#fn-key)." : ""}`,
      },
      {
        id: "no-call-audio",
        title: "Only my side of the call was recorded",
        keywords: "missing audio other people silent system audio permission one track",
        body: `Quill records other people from your computer's sound. If the transcript has only **You**:

- ${isMac ? "**Check System Audio Recording** permission: run the access check in **Settings → General → Audio access**, which links to the right System Settings pane." : "**Check your output device**: Quill records the default playback device. Calls playing through a different device aren't heard."}
- **Was the call audible on this computer?** A call on your phone, or on another device, isn't recorded.

Your own voice missing? Check **Microphone** permission the same way, and that the right microphone is your computer's default input.`,
      },
      {
        id: "speakers-wrong",
        title: "Speakers are mixed up",
        keywords: "wrong speaker merged split same person two people diarization",
        body: `Open the meeting's **Transcript** tab. Set **People on the call** to the right number and Quill splits the voices again. Type an existing name on a speaker to merge them. Then **Re-summarize** so the notes use the corrected names. See [Naming speakers](#speakers).`,
      },
      {
        id: "no-notes",
        title: "Notes weren't written",
        keywords: "summary failed missing notes error key model",
        body: `- **Is a model set up?** OpenRouter needs a key (**Settings → General → OpenRouter**); Ollama needs to be running with the chosen model pulled.
- **Is Summarize automatically on?** (**Settings → Meetings**.) If not, write them from the meeting's Summary tab.
- **An error?** The meeting shows it. Fix the cause and click **Re-summarize** or **Retry**.`,
      },
      {
        id: "indicator-fullscreen",
        title: "The dictation pill doesn't appear",
        keywords: "indicator pill full screen overlay missing",
        body: `The pill may not show over full-screen apps on macOS. Dictation still works: listen for the start and stop sounds, or watch the ${TRAY}.`,
      },
      {
        id: "report-problem",
        title: "Reporting a problem",
        keywords: "bug report issue support diagnostic github",
        body: `Open **About Quill** (${isMac ? "in the Quill menu or " : ""}from the ${TRAY}) and click **Copy diagnostic info**: it copies your version, system and which models you use, with no keys or content. **Report a problem** in the same window${isMac ? " (or **Help → Report a Problem…** in the menu bar)" : ""} opens a new issue on GitHub to paste it into. Say what you did, what you expected, and what happened instead.`,
      },
    ],
  },
  {
    id: "shortcuts",
    title: "Keyboard shortcuts",
    intro: "Everything you can do from the keyboard.",
    sections: [
      {
        id: "shortcut-list",
        title: "All shortcuts",
        keywords: "keyboard hotkeys keys",
        body: `| Shortcut | What it does |
|---|---|
| \`key:${ALT}\` \`key:Space\` (hold or tap)${isMac ? ", or `key:fn`" : ""} | Dictate (change it in Settings → Dictation) |
| \`key:Esc\` | Cancel dictation |
| \`key:${MOD}\` \`key:K\` | Search everything (\`key:Esc\` closes it) |
| \`key:${MOD}\` \`key:\\\` | Collapse or expand the sidebar |
${isMac ? "| \`key:⌘\` \`key:,\` | Open Settings |\n| \`key:⌘\` \`key:0\` | Bring back Quill's window after closing it |\n" : ""}| \`key:${MOD}\` \`key:Enter\` | Save a corrected dictation |
| \`key:${MOD}\` \`key:F\` or \`key:/\` | Search Help (on this page) |`,
      },
    ],
  },
];

/** The topics, with sections for the other platform left out. */
export const TOPICS: HelpTopic[] = ALL.map((topic) => ({
  ...topic,
  sections: topic.sections.filter((s) => !s.only || s.only === (isMac ? "mac" : "windows")),
})).filter((topic) => topic.sections.length > 0);
