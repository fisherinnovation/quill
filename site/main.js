// Quill's website: a little motion and interaction. Everything works
// without it; nothing here talks to the network.
(() => {
  document.documentElement.classList.remove("no-js");

  // Waveforms: thin bars rising towards one accent bar, like a level meter.
  // Heights are fixed (seeded), so the layout never jumps.
  let seed = 7;
  const random = () => ((seed = (seed * 16807) % 2147483647) - 1) / 2147483646;
  for (const el of document.querySelectorAll("[data-bars]")) {
    const n = Number(el.dataset.bars);
    const level = el.classList.contains("levels");
    const hot = Math.round(n * 0.78);
    for (let i = 0; i < n; i++) {
      const bar = document.createElement("i");
      const rise = level ? 0.35 + random() * 0.65 : 0.12 + (Math.min(i, hot) / hot) * 0.75 * (0.55 + random() * 0.45);
      bar.style.setProperty("--h", `${Math.round((i > hot ? 0.1 + random() * 0.12 : rise) * 100)}%`);
      bar.style.setProperty("--d", `${(-random() * 2.4).toFixed(2)}s`);
      if (!level && i === hot) bar.className = "hot";
      el.append(bar);
    }
  }

  // Reveal on scroll, staggered within each row.
  const reveals = [...document.querySelectorAll(".reveal")];
  if ("IntersectionObserver" in window) {
    const io = new IntersectionObserver(
      (entries) => {
        for (const e of entries) {
          if (!e.isIntersecting) continue;
          const siblings = [...e.target.parentElement.children].filter((c) => c.classList.contains("reveal"));
          e.target.style.setProperty("--delay", `${Math.min(siblings.indexOf(e.target), 4) * 70}ms`);
          e.target.classList.add("shown");
          io.unobserve(e.target);
        }
      },
      { rootMargin: "0px 0px -8% 0px" },
    );
    for (const el of reveals) io.observe(el);
  } else {
    for (const el of reveals) el.classList.add("shown");
  }

  // Hero video: the one for the visitor's theme. With reduced motion it
  // stays on its poster, with controls to play it on purpose.
  const video = document.querySelector(".hero-video");
  if (video) {
    const dark = matchMedia("(prefers-color-scheme: dark)");
    const calm = matchMedia("(prefers-reduced-motion: reduce)");
    const pick = () => {
      const theme = dark.matches ? "dark" : "light";
      if (video.getAttribute("src") !== video.dataset[theme]) {
        video.poster = `media/hero-${theme}.jpg`;
        video.src = video.dataset[theme];
      }
      video.controls = calm.matches;
      if (calm.matches) video.pause();
      else video.play().catch(() => {});
    };
    dark.addEventListener("change", pick);
    calm.addEventListener("change", pick);
    pick();
  }

  // Privacy: the model choice is a toggle you can play with.
  const choices = [...document.querySelectorAll(".choice-opt")];
  for (const c of choices) {
    c.addEventListener("click", () => {
      for (const o of choices) o.setAttribute("aria-pressed", String(o === c));
    });
  }

  // Navigation gets a hairline once the page scrolls.
  const nav = document.getElementById("nav");
  const onScroll = () => nav.classList.toggle("scrolled", window.scrollY > 8);
  window.addEventListener("scroll", onScroll, { passive: true });
  onScroll();

  // Downloads: put this computer's installer first.
  const os = /Windows/i.test(navigator.userAgent) ? "windows" : /Mac/i.test(navigator.userAgent) ? "mac" : null;
  const mine = os && document.querySelector(`.dl[data-os="${os}"]`);
  if (mine) {
    mine.parentElement.prepend(mine);
    for (const b of document.querySelectorAll(".dl")) if (b !== mine) b.classList.add("other");
  }
})();
