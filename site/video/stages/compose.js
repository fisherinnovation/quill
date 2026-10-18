// Stage for the dictation scene: the dictation pill (the preview's
// ?indicator window) over a message being written in some other app. The
// app is a generic stand-in, not any real one. __stage.dictate() plays a
// dictation through the real pill: the keys held, live captions word by
// word, the voice level, then "Typing…", and the text landing.
(() => {
  const style = document.createElement("style");
  style.textContent = `
    body { background: var(--bg); }
    /* The pill (and its captions) where they'd sit on the screen, below
       the message. Hidden until the shortcut is held, as in the app. */
    #root { position: fixed; z-index: 2; left: 0; right: 0; top: 0; height: 362px;
      opacity: 0; transform: translateY(6px); transition: opacity 0.25s ease, transform 0.25s ease; }
    #root.on { opacity: 1; transform: none; }
    #root .indicator-stage { height: 100%; }
    .stage-desk { position: fixed; inset: 0; z-index: 0; background: var(--bg-wash); }
    .stage-app {
      position: fixed; z-index: 1; left: 170px; width: 660px; top: 64px;
      border-radius: 14px; overflow: hidden; background: var(--bg-elevated);
      border: 1px solid var(--border); box-shadow: var(--shadow-lg);
      font: 14px/1.45 -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif; color: var(--text);
    }
    .stage-head { display: flex; align-items: center; gap: 8px; padding: 12px 16px; border-bottom: 1px solid var(--border); font-weight: 650; }
    .stage-head small { font-weight: 400; color: var(--text-3); }
    .stage-msg { display: flex; gap: 10px; padding: 14px 16px 6px; }
    .stage-avatar { flex: none; width: 30px; height: 30px; border-radius: 8px; display: grid; place-items: center;
      font-weight: 650; font-size: 13px; color: #fff; background: #6c6ea0; }
    .stage-avatar.me { background: #ff1053; }
    .stage-msg b { font-weight: 650; }
    .stage-msg p { margin: 2px 0 0; }
    .stage-msg.mine { animation: stage-in 0.35s cubic-bezier(0.2, 0.8, 0.2, 1) both; }
    @keyframes stage-in { from { opacity: 0; transform: translateY(6px); } }
    .stage-compose { margin: 10px 16px 16px; padding: 10px 12px; min-height: 22px; border-radius: 10px;
      border: 1.5px solid var(--border-strong); color: var(--text-3); }
    .stage-compose.focus { border-color: var(--accent); }
    .stage-compose.focus::after { content: ""; display: inline-block; width: 1.5px; height: 17px; margin-left: 1px;
      vertical-align: -3px; background: var(--text); animation: stage-caret 1s steps(1) infinite; }
    .stage-compose.filled { color: var(--text); }
    .stage-compose.filled::after { display: none; }
    @keyframes stage-caret { 50% { opacity: 0; } }
    /* The shortcut, held. */
    .stage-keys { position: fixed; z-index: 3; top: 322px; right: calc(50% + 66px); display: flex; gap: 5px;
      opacity: 0; transform: translateY(4px); transition: opacity 0.2s ease, transform 0.2s ease; }
    .stage-keys.on { opacity: 1; transform: none; }
    .stage-keys kbd { height: 34px; min-width: 34px; padding: 0 10px; font-size: 15px; border-radius: 7px;
      border-bottom-width: 1px; box-shadow: 0 1px 0 var(--border-strong); transform: translateY(1px); }
  `;
  document.head.append(style);
  const desk = document.createElement("div");
  desk.className = "stage-desk";
  const app = document.createElement("div");
  app.className = "stage-app";
  app.innerHTML = `
    <div class="stage-head"># launch <small>· 4 people</small></div>
    <div class="stage-msg"><span class="stage-avatar">P</span><div><b>Priya</b><p>Who's taking the rollout plan to support?</p></div></div>
    <div class="stage-thread"></div>
    <div class="stage-compose">Message #launch</div>`;
  const keys = document.createElement("div");
  keys.className = "stage-keys";
  const mac = navigator.userAgent.includes("Mac");
  keys.innerHTML = `<kbd>${mac ? "⌥" : "Alt"}</kbd><kbd>Space</kbd>`;
  document.body.prepend(desk, app);
  document.body.append(keys);

  const root = document.getElementById("root");
  const compose = app.querySelector(".stage-compose");
  const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
  // Seeded, so both themes' videos get the same rhythm.
  let seed = 11;
  const random = () => (seed = (seed * 16807) % 2147483647) / 2147483647;
  const emit = (event, payload) => window.mockEmit(event, payload);

  window.__stage = {
    focus() {
      compose.classList.add("focus");
      compose.textContent = "";
    },
    async dictate(text, wordsPerSecond = 3.2) {
      const words = text.split(" ");
      keys.classList.add("on");
      root.classList.add("on");
      await emit("dictation-state", { phase: "listening", error: null });
      // A talking-like level for the pill's meter, through to the release.
      let talking = true;
      const level = (async () => {
        while (talking) {
          const t = performance.now() / 1000;
          await emit("dictation-level", 0.06 + Math.abs(Math.sin(t * 7.3) * Math.sin(t * 2.1)) * 0.35);
          await sleep(50);
        }
        await emit("dictation-level", 0.003);
      })();
      // Words arrive as they're said; the last few stay tentative until a
      // pause, as the app's captions do.
      for (let n = 1; n <= words.length; n++) {
        const locked = Math.max(0, n - 3);
        await emit("dictation-caption", {
          committed: words.slice(0, locked).join(" "),
          tentative: words.slice(locked, n).join(" "),
        });
        const word = words[n - 1];
        await sleep((1000 / wordsPerSecond) * (0.75 + random() * 0.5) + (/[,.]$/.test(word) ? 160 : 0));
      }
      await emit("dictation-caption", { committed: text, tentative: "" });
      await sleep(350);
      talking = false;
      await level;
      keys.classList.remove("on");
      await emit("dictation-state", { phase: "processing", error: null });
      await sleep(420);
      await emit("dictation-state", { phase: "idle", error: null });
      root.classList.remove("on");
      compose.classList.add("filled");
      compose.textContent = text;
      // A beat to see it typed, then sent: the reply joins the thread.
      await sleep(900);
      const reply = document.createElement("div");
      reply.className = "stage-msg mine";
      reply.innerHTML = '<span class="stage-avatar me">Y</span><div><b>You</b><p></p></div>';
      reply.querySelector("p").textContent = text;
      app.querySelector(".stage-thread").append(reply);
      compose.className = "stage-compose";
      compose.textContent = "Message #launch";
    },
  };
})();
