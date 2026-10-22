---
name: site-video
description: Plans, scripts, renders and publishes the looping header video on Quill's website (site/media/hero-light.mp4 and hero-dark.mp4), recorded from the app's real UI in the browser preview. Use this whenever the website's hero or demo video needs making or updating, for example after features ship ("the site video is out of date", "regenerate the hero video", "add the new Stats tab to the website video", "make a demo video for the homepage"), or when someone wants to change a scene, caption, pacing or theme in it. Use it even if they only say "update the website" after a release that changed what the app looks like.
---

# The website's header video

Quill's homepage opens with a 20–30 second video that loops silently next to
the headline. It shows the real app, recorded from the browser preview with its
sample data, so it can be regenerated whenever the feature set grows. This skill
is the whole job: work out what Quill does now, decide what the video should
show, script it, render it, look at the result critically, and put it on the
site.

The pieces:

| Path | What |
|---|---|
| `site/video/script.json` | The script: scenes, captions, actions, camera. You write this. |
| `site/video/render.mjs` | Plays the script in headless Chrome and encodes the video. Run, don't edit (unless it's genuinely missing something). |
| `site/video/stages/*.js` | Backdrops for scenes whose subject is one of Quill's floating windows (live notes, the dictation pill): a stand-in call or app around it, and helpers the scene calls. |
| `site/media/hero-{light,dark}.{mp4,jpg}` | The output: video and poster per theme. |
| `site/media/card.jpg` | The link preview card, which shows the light poster (`site/card/render.mjs`). |
| `site/video/out/contact-*.jpg` | One frame per scene, for review. Not committed. |
| `app/src/dev/mock.ts` | The sample data and URL flags the scenes run on. |

## 1. Take stock of what Quill does now

The video is only as current as your understanding of the app, so build it
fresh each time rather than trusting the last script. Read:

- `README.md`: the Features and Using Quill sections say what users get.
- `git log --oneline <based_on>..HEAD` where `based_on` is the commit recorded
  in `script.json`: what changed since the video was last made. `feat:` commits
  are the candidates.
- `app/src/components/`: what each view actually looks like, and
  `app/src/dev/mock.ts`: which states the preview can show (its header lists
  the URL flags).
- `site/index.html`: the headline and copy the video sits next to. The video
  should prove the headline, not wander off from it.

Write a short inventory for yourself: each user-visible feature, one line on
why a first-time visitor would care, and whether the preview can show it. Rank
by that visitor's interest, not by how recently it was built: a new settings
toggle is rarely video material; a new view usually is.

## 2. Decide the story

Five or six scenes, 3.5 to 5 seconds each, 20 to 30 seconds in all. One idea per
scene. A good order:

1. The outcome: meeting notes appearing for a call (why you'd install it).
2. Proof it understood the call: the transcript with speakers told apart.
3. Something visually striking that's newer or unique (e.g. the Stats tab).
4. The second job: dictation (holding the key, text arriving).
5. A reason to come back: history, insights.

The last scene should end on a calm frame, because the video fades through the
page colour back to the first scene. If a new feature doesn't earn a scene, it
can still appear inside one (a tab you click past, a sidebar section in frame).
Dropping a scene to stay under 30 seconds is better than rushing them all.

**Captions** say what the visitor gets, in five words or fewer, sentence case,
no full stop: "Notes from every call", "Knows who said what". Not feature
names ("Stats tab"), not marketing superlatives. They're the only words a
muted, looping video has, so read them in sequence and check they tell the
story on their own.

## 3. Make sure the preview can show it

Every scene runs on the browser preview's sample data. If a feature you want
isn't reachable there (no mock handler, no sample data, no URL flag), extend
`app/src/dev/mock.ts`: that's also required by CLAUDE.md whenever a command is
added, so a gap is worth fixing anyway. Keep the sample data believable: real-
looking names, sensible numbers, consistent across scenes (the meeting in the
notes scene should be the one in the transcript scene).

## 4. Write the script

`site/video/script.json`:

```json
{
  "based_on": "<git rev-parse --short HEAD when you wrote this>",
  "label": "Quill in use: …, for screen readers",
  "viewport": { "width": 1120, "height": 720, "scale": 2 },
  "output": [1600, 1000],
  "crf": 24,
  "scenes": [
    {
      "id": "notes",
      "caption": "Notes from every call",
      "flags": "",
      "ready": ".home",
      "setup": [],
      "camera": ".main",
      "pad": 0,
      "cursorStart": "text:Record",
      "steps": [{ "click": "text:Product sync" }, { "wait": 600 }],
      "hold": 1800
    }
  ]
}
```

- `viewport` is the browser window in CSS pixels; `scale: 2` records at
  Retina resolution. `output` is the video size (16:10 fits the hero).
- `flags`: the preview's URL flags for this scene (`"backlog&others"`). The
  renderer adds `light` for the light theme.
- `ready`: a selector that exists once the scene's page has rendered.
- `setup`: steps run before recording starts (getting into position).
- `camera`: what to frame, as a selector or a box `[x, y, width]` in CSS
  pixels. The frame takes the element's width and the video's 16:10 shape,
  starting at the element's top (a short element is centred instead), and is
  measured after `setup`, so it stays put while the scene plays and content
  scrolls inside it. Omit it to show the whole window. On the site the video is about 560
  pixels wide, so a region much wider than 760 CSS pixels makes the app's text
  unreadably small: frame the part that matters (`.main` without the sidebar,
  a single card) rather than the whole window. `pad` adds room around it.
- `steps`: what happens on camera. Each is one of:
  - `{ "click": target }`: moves the cursor there and clicks.
  - `{ "hover": target }`: moves the cursor there (for tooltips).
  - `{ "scroll": target }`: scrolls it into the middle of view, or with
    `"to": 170` puts its top that many pixels from the top of the window
    (useful to keep a tooltip or result above the caption).
  - `{ "type": { "into": target, "text": "…", "cps": 16 } }`: types into a field.
  - `{ "emit": { "event": "dictation-state", "payload": {…} } }`: fires a
    backend event through the preview's `mockEmit` (how dictation states and
    new entries appear).
  - `{ "stage": "call" }`: runs `site/video/stages/call.js` in the page, to
    build a backdrop and define `window.__stage` helpers that later `eval`
    steps call (`__stage.note(…)`, `__stage.dictate("…")`).
  - `{ "wait": ms }`, `{ "cursor": "hide" | "show" | target }`, and
    `{ "eval": "js" }` as an escape hatch.
- Targets are CSS selectors, or `"text:Label"` for the visible button, tab,
  link or list item whose text starts with Label.
- `hold` is how long the scene rests after its last step. Viewers need about
  1.5 s to take in a finished frame.
- `push` (optional): how far the camera pushes in over the scene, 0.035 by
  default; 0 holds it still.

**Floating windows.** The live notes panel (`flags: "live"`) and the
dictation pill (`flags: "indicator"`) each fill the preview's window, as
they're separate small windows in the app. Their scenes use a stage to put
them where they really appear: the panel at the edge of a call
(`stages/call.js`), the pill and its captions below a message being written
(`stages/compose.js`). Stages stand in for other apps, so keep them generic:
no real product's look, name or logo. Build them from the app's colour
tokens so both themes match.

**Viewport.** At 1000 px wide the main area is under 900 px, so Home stays in
one column and every page frames at about 780 px. Wider, Home splits into
two columns and a `.page` camera gets too wide to read.

Pacing: the cursor takes 0.3 to 0.95 s per move, depending on the distance.
Give each scene one or two actions, not five; let the result of an action
stay on screen.

**Keeping it lively.** The renderer does the handling: curved cursor paths
with ease-out, a slight aim offset, a drift at rest, a press and ripple on
click, typing with a rhythm, a slow push-in, and captions that rise as they
fade in. Randomness is seeded by scene id, so light and dark match. The
script's part is to give each scene something that *moves on its own*
(a note arriving, captions growing, numbers counting up, a waveform) rather
than a still page with a click, and to end each scene with the cursor
somewhere purposeful (where the next scene starts, or back on the subject).

## 5. Render

```sh
node site/video/render.mjs --scene stats --theme light   # iterate on one scene
node site/video/render.mjs                               # everything, both themes
```

It starts its own preview server (port 1431), so it doesn't disturb one
you're running. A full render takes a minute or two.

## 6. Review it like a picky designer

Open the contact sheets (`site/video/out/contact-light.jpg`,
`contact-dark.jpg`) with the Read tool. For each scene, check:

- The caption is legible and doesn't cover what the scene is about. The
  bottom 30% of every captioned frame fades into the page colour behind the
  caption, so the scene's subject belongs in the top two-thirds.
- The thing the scene is about is in frame, big enough to read at hero size,
  and not cut off at an edge.
- No error banners, empty states, "Not transcribed yet", loading spinners or
  dev-only artefacts.
- The cursor ended up where the action was, not stranded mid-screen.
- Both themes look right (sample colours, contrast).

Then pull a few frames from the middle of transitions and actions in the
finished video
(`ffmpeg -ss 7.5 -i site/media/hero-light.mp4 -frames:v 1 /tmp/f.jpg`) to check
motion isn't janky, and check the size: under about 4 MB per video keeps the
page light (raise `crf` to shrink it; 28 is the most before text smears). The
renderer prints page errors; treat any as a bug to fix.

Fix and re-render until you'd be happy to see it on someone else's product
page.

## 7. Put it on the site

The hero in `site/index.html` plays `media/hero-<theme>.mp4` and `main.js`
picks the theme and respects reduced motion; `build.mjs` copies `site/media/`.
That's normally already wired, so only touch them if the structure changed.
Preview the site at desktop and phone widths in both themes:

```sh
node site/build.mjs && python3 -m http.server 4173 -d site/_site
```

Browser screenshot tools often capture a blank page while a video is
playing; pause it first (`document.querySelector(".hero-video").pause()`).

Update `label` in `script.json` so it describes what the new video shows,
scene by scene, for people who can't see it; `build.mjs` puts it in the
video's `aria-label`.

The link preview card shows the light poster, so re-render it too and look
at it: `node site/card/render.mjs` writes `site/media/card.jpg`. If the
poster's moment changed, check that `CARD_ALT` in `build.mjs` still
describes it.

## 8. Commit

Commit `script.json`, the four hero files and `card.jpg` in `site/media/`,
and any mock or site changes together, with a body listing the scenes. The videos are binary and
every regeneration adds a few megabytes to the repository's history, so
regenerate when what the app does has changed, not to nudge a caption.
