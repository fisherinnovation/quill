---
name: site-overview
description: Writes, generates, reviews and publishes Quill's narrated overview video (site/media/overview.mp4), the 30–60 second rundown behind the website's "Watch the overview" button. It has an ElevenLabs voiceover, generated footage of people, a music bed and the real app, cut together in Remotion. Use this whenever the overview needs making or changing: after features ship ("the overview video is out of date"), to change the script, voice, music or a shot ("use a different voice", "the last shot should be the same woman", "make a 30-second cut"), or for a narrated product, explainer or marketing video of Quill. For the silent looping header video, use site-video instead.
---

# The narrated overview video

A 30–60 second video that tells someone what Quill does, why it's good and
how to use it, with a human feel: a warm voice, people on calls, music, and the
real app between them. It plays in a dialog from the website's hero.

`node site/overview/make.mjs` makes the whole thing from
`site/overview/script.json`. This skill is the job around that: write the
story, generate it, look at the result critically, and publish it.

| Path | What |
|---|---|
| `site/overview/script.json` | The script: voice, music, footage settings, and per scene the line (`say`) and the shot. You write this. |
| `site/overview/make.mjs` | Generates what's missing, records the app shots, renders, levels the sound. `--plan` shows what it would make and spend. |
| `site/overview/openrouter.mjs` | Speech, video and music through OpenRouter, with the key the app keeps in the OS keychain. |
| `site/overview/remotion/src/Overview.jsx` | The edit: layouts, motion, captions, the music dipping under the voice. |
| `site/overview/cache/` | Everything generated, named by what made it. Not committed. |
| `site/media/overview.{mp4,jpg}` | The output and its poster. |
| `site/overview/out/contact.jpg` | A frame from each scene, for review. |

## 1. Write the story

Start from what Quill does now (README, `app/src/help/content.ts`, the site's
copy) and from what the current script says. Then tell it as a story, not a
feature list: a moment people know (typing notes on a call), Quill helping
(live notes, the write-up, the transcript), how you use it (record, hold a
key), why to trust it (it all runs on your computer), and the payoff and name
at the end.

- About 2.5 words a second: 60 s is about 140 words, 30 s about 70. Each
  scene's length follows its line, so the script's word count *is* the
  video's length.
- Write it to be heard: short sentences, contractions, a question or two, an
  ellipsis for a pause ("nodding along... and"). Read it aloud.
- Claims must be true on both macOS and Windows. Check numbers against
  `docs/DESIGN_NOTES.md` before using them ("0.3 s" was measured on an M3 Pro,
  so the script says "right where you are" instead).

## 2. Shots

Each scene's `shot` is one of:

- `{"footage": "<prompt>", "seconds": 8, "cuts": [[0, 1.7], [3.9, 8]]}`:
  people, generated with `footage.model` (Veo 3.1). Describe the person, place,
  light, lens and action in plain sentences, and keep the laptop screen turned
  away or out of focus: generated screens show nonsense text. `cuts` (optional)
  jump-cut parts of the clip together when the best moment comes late. Change
  the prompt or `seconds` to regenerate the clip; changing `cuts` costs nothing.
- `{"app": "<id>"}`: a scene from `site/video/script.json`, re-recorded
  without its caption and held as long as the line. To show something new,
  add the scene there first (see the `site-video` skill).
- `{"card": {"title": …, "points": […]}}`: a title card on the site's
  background.
- `{"end": {"url": …}}`: the icon, the tagline, the platforms and the address.

Footage clips are 4–8 s (Veo), so keep footage lines to about 6 s or cut
them.

## 3. Generate

```sh
node site/overview/make.mjs --plan    # what's missing, and what it costs
node site/overview/make.mjs           # refuses to spend over $10 without --budget
```

Costs (October 2026): footage about $0.20 a second at 1080p, so $1.60 a
clip; the voice about $0.01 a line; music about $0.08. Everything generated is
cached by what made it, so changing one line pays for one line, and edits to
the layout or cuts cost nothing. `--fresh voice` (or `music`, `footage`, `app`)
regenerates a stage regardless; use it for a new take you weren't happy with.
Generated clips take a few minutes; their job ids are logged so a paid clip can
be fetched again if the run dies.

The key: the app's keychain entry (macOS Keychain, Windows Credential
Manager), read into memory by `openrouter.mjs`, or `QUILL_OPENROUTER_API_KEY`.
Never ask for the key in chat or put it on a command line.

## 4. Review it like a picky editor

You can't hear it, so look and measure, and leave the ears to the user.

- Open `site/overview/out/contact.jpg`. For each footage clip, make a strip
  of one frame a second
  (`ffmpeg -i <clip> -vf "fps=1,scale=480:-2,tile=4x2" -frames:v 1 strip.jpg`)
  and look for warped hands, faces, extra fingers, readable screens, and the
  moment the line is about (add `cuts` when it comes late).
- Captions: one or two balanced lines, nothing alone on a line.
- People across shots: two prompts for "the same woman" give two women. Say
  so, or anchor the second on a frame of the first (image-to-video, which
  OpenRouter's video API supports with `frame_images`).
- Sound: the render is levelled to -16 LUFS with peaks under -1.5 dB. Check
  with `ffmpeg -i site/media/overview.mp4 -af ebur128=peak=true -f null -`.
  The music is about 10 dB under the voice; change `music.volume`/`ducked` if
  the user says it's too loud or quiet.
- Then ask the user to watch it with sound, and say what you'd change.

## 5. Publish

`make.mjs` writes `site/media/overview.mp4` and its poster; the hero's
"Watch the overview" button appears whenever the file exists
(`build.mjs`). Update `label` in `script.json` so it describes the video for
people who can't see it. Commit `script.json`, any code changes and the two
media files together. The video is about 35 MB and every version stays in the
repository's history, so publish when the video has really changed.
