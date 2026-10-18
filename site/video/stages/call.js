// Stage for the live notes scene: the panel (the preview's ?live window)
// at the right edge of a screen, beside a call in progress. The call is a
// generic stand-in, not any real app: two people's tiles, one of them
// talking. Colours come from the app's own tokens, so both themes match.
(() => {
  const style = document.createElement("style");
  style.textContent = `
    body { background: var(--bg); }
    /* The panel at the size and place the app gives it. */
    #root { position: fixed; z-index: 2; top: 14px; right: 14px; width: 352px; height: 650px; }
    #root .live-stage { height: 100%; }
    .stage-desk { position: fixed; inset: 0; z-index: 0; background: var(--bg-wash); }
    .stage-call {
      position: fixed; z-index: 1; top: 24px; left: 300px; width: 316px; bottom: 24px;
      display: grid; grid-template-rows: 1fr 1fr auto; gap: 8px; padding: 8px;
      border-radius: 16px; background: #15151c; box-shadow: var(--shadow-lg);
    }
    .stage-tile {
      position: relative; border-radius: 11px; overflow: hidden;
      display: grid; place-items: center;
    }
    .stage-tile.a { background: radial-gradient(120% 90% at 30% 20%, #3a3c66, #22233a); }
    .stage-tile.b { background: radial-gradient(120% 90% at 70% 30%, #2f4d5e, #1c2a33); }
    .stage-face {
      width: 84px; height: 84px; border-radius: 50%; display: grid; place-items: center;
      font: 600 32px/1 -apple-system, BlinkMacSystemFont, sans-serif; color: #fff;
    }
    .a .stage-face { background: #6c6ea0; }
    .b .stage-face { background: #3f86a8; }
    /* Whoever's talking gets a ring that moves with their voice. */
    .stage-tile.talking::after {
      content: ""; position: absolute; inset: 0; border-radius: 11px;
      box-shadow: inset 0 0 0 3px #66c7f4; animation: stage-talk 0.9s ease-in-out infinite alternate;
    }
    .stage-tile.talking .stage-face { animation: stage-voice 0.45s ease-in-out infinite alternate; }
    @keyframes stage-talk { from { opacity: 0.55; } to { opacity: 1; } }
    @keyframes stage-voice { from { box-shadow: 0 0 0 0 rgb(102 199 244 / 0.5); } to { box-shadow: 0 0 0 14px rgb(102 199 244 / 0); } }
    .stage-name {
      position: absolute; left: 10px; bottom: 9px; padding: 3px 9px; border-radius: 7px;
      font: 500 12px/1.3 -apple-system, BlinkMacSystemFont, sans-serif; color: #fff; background: rgb(0 0 0 / 0.45);
    }
    .stage-bar { display: flex; justify-content: center; gap: 10px; padding: 6px 0 4px; }
    .stage-bar span { width: 34px; height: 34px; border-radius: 50%; background: #2a2a35; }
    .stage-bar span:last-child { width: 52px; border-radius: 17px; background: #d23a4f; }
  `;
  document.head.append(style);
  const desk = document.createElement("div");
  desk.className = "stage-desk";
  const call = document.createElement("div");
  call.className = "stage-call";
  call.innerHTML = `
    <div class="stage-tile a talking"><div class="stage-face">P</div><span class="stage-name">Priya</span></div>
    <div class="stage-tile b"><div class="stage-face">D</div><span class="stage-name">Dan</span></div>
    <div class="stage-bar"><span></span><span></span><span></span></div>`;
  document.body.prepend(desk, call);

  const invoke = (cmd, args) => window.__TAURI_INTERNALS__.invoke(cmd, args ?? {});
  window.__stage = {
    /** The other person starts talking. */
    async talk(who) {
      for (const tile of call.querySelectorAll(".stage-tile")) {
        tile.classList.toggle("talking", tile.querySelector(".stage-name").textContent === who);
      }
    },
    /** A note arrives, as from the next update: it animates in. */
    async note(item, gist) {
      const state = await invoke("live_notes_state");
      state.notes = { gist: gist ?? state.notes.gist, items: [item, ...state.notes.items] };
      state.updated_ms = Date.now();
      window.mockEmit("live-notes", state);
    },
  };
})();
