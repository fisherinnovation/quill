// OpenRouter for the overview video: speech (ElevenLabs), video clips (Veo
// and others) and music (Lyria). Uses the key the app keeps in the OS
// keychain, read into memory here and never printed or written anywhere.

import { spawnSync } from "node:child_process";

const API = "https://openrouter.ai/api/v1";
const SERVICE = "com.fisherinnovation.quill";
const ACCOUNT = "openrouter";

let cached;

/** The OpenRouter key: QUILL_OPENROUTER_API_KEY, else the app's keychain entry. */
export function key() {
  if (cached) return cached;
  let k = process.env.QUILL_OPENROUTER_API_KEY;
  if (!k && process.platform === "darwin") {
    const r = spawnSync("security", ["find-generic-password", "-s", SERVICE, "-a", ACCOUNT, "-w"], {
      encoding: "utf8",
    });
    k = r.status === 0 ? r.stdout : "";
  } else if (!k && process.platform === "win32") {
    // The keyring crate's Windows entry: a generic credential named
    // "<account>.<service>", its password stored as UTF-16.
    const ps = `
      $sig = '[DllImport("advapi32.dll", CharSet=CharSet.Unicode, SetLastError=true)] public static extern bool CredReadW(string t, int type, int flags, out IntPtr c);
              [DllImport("advapi32.dll")] public static extern void CredFree(IntPtr c);
              [StructLayout(LayoutKind.Sequential, CharSet=CharSet.Unicode)] public struct CRED { public int Flags; public int Type; public string TargetName; public string Comment;
                public System.Runtime.InteropServices.ComTypes.FILETIME LastWritten; public int BlobSize; public IntPtr Blob; public int Persist;
                public int AttributeCount; public IntPtr Attributes; public string TargetAlias; public string UserName; }'
      $t = Add-Type -MemberDefinition $sig -Name Cred -Namespace QuillOverview -PassThru
      $p = [IntPtr]::Zero
      if (-not $t[0]::CredReadW('${ACCOUNT}.${SERVICE}', 1, 0, [ref]$p)) { exit 1 }
      $c = [Runtime.InteropServices.Marshal]::PtrToStructure($p, [type]$t[1])
      $b = New-Object byte[] $c.BlobSize
      [Runtime.InteropServices.Marshal]::Copy($c.Blob, $b, 0, $c.BlobSize)
      $t[0]::CredFree($p)
      [Console]::Out.Write([Text.Encoding]::Unicode.GetString($b))`;
    const r = spawnSync("powershell.exe", ["-NoProfile", "-NonInteractive", "-Command", ps], { encoding: "utf8" });
    k = r.status === 0 ? r.stdout : "";
  }
  k = (k ?? "").trim();
  if (!k) throw new Error("no OpenRouter key: add one in Quill's settings, or set QUILL_OPENROUTER_API_KEY");
  return (cached = k);
}

const headers = () => ({ Authorization: `Bearer ${key()}`, "Content-Type": "application/json" });

async function fail(res, what) {
  const body = await res.text();
  let message = body.slice(0, 400);
  try {
    message = JSON.parse(body).error?.message ?? message;
  } catch {}
  throw new Error(`${what}: HTTP ${res.status}: ${message}`);
}

/** Speech for `input`: MP3 bytes. */
export async function speech({ model, voice, input }) {
  const res = await fetch(`${API}/audio/speech`, {
    method: "POST",
    headers: headers(),
    body: JSON.stringify({ model, voice, input, response_format: "mp3" }),
  });
  if (!res.ok) await fail(res, "speech");
  return Buffer.from(await res.arrayBuffer());
}

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

/** A generated video clip: MP4 bytes. Polls until it's done (a few minutes). */
export async function video({ model, prompt, seconds, resolution, aspect, audio = false, extra = {}, log = () => {} }) {
  const res = await fetch(`${API}/videos`, {
    method: "POST",
    headers: headers(),
    body: JSON.stringify({
      model,
      prompt,
      duration: seconds,
      resolution,
      aspect_ratio: aspect,
      generate_audio: audio,
      ...extra,
    }),
  });
  if (!res.ok) await fail(res, "video");
  const job = await res.json();
  const id = job.id;
  // So a clip that's paid for can still be fetched if this run dies.
  log(` job ${id}`);
  for (let i = 0; ; i++) {
    await sleep(i === 0 ? 15000 : 10000);
    const poll = await fetch(`${API}/videos/${id}`, { headers: headers() });
    if (!poll.ok) await fail(poll, "video status");
    const s = await poll.json();
    if (s.status === "completed") {
      const url = s.unsigned_urls?.[0] ?? s.output?.[0]?.url ?? s.url ?? s.video_url ?? s.data?.[0]?.url;
      const content = url ?? `${API}/videos/${id}/content`;
      const dl = await fetch(content, url?.startsWith(API) || !url ? { headers: headers() } : {});
      if (!dl.ok) await fail(dl, "video download");
      return { bytes: Buffer.from(await dl.arrayBuffer()), cost: s.usage?.cost };
    }
    if (s.status === "failed" || s.status === "cancelled" || s.error) {
      throw new Error(`video ${id} ${s.status}: ${JSON.stringify(s.error ?? s).slice(0, 400)}`);
    }
    if (i % 3 === 0) log(`  …${s.status ?? "waiting"}`);
  }
}

/** A piece of music from a prompt: audio bytes and their format. */
export async function music({ model, prompt }) {
  const res = await fetch(`${API}/chat/completions`, {
    method: "POST",
    headers: headers(),
    body: JSON.stringify({
      model,
      messages: [{ role: "user", content: prompt }],
      modalities: ["text", "audio"],
      stream: true,
      usage: { include: true },
    }),
  });
  if (!res.ok) await fail(res, "music");
  // Server-sent events: audio arrives as base64 chunks in delta.audio.data.
  const chunks = [];
  let format;
  let cost;
  let rest = "";
  for await (const part of res.body) {
    rest += Buffer.from(part).toString("utf8");
    const lines = rest.split("\n");
    rest = lines.pop();
    for (const line of lines) {
      if (!line.startsWith("data: ") || line === "data: [DONE]") continue;
      const msg = JSON.parse(line.slice(6));
      if (msg.error) throw new Error(`music: ${msg.error.message ?? JSON.stringify(msg.error)}`);
      const delta = msg.choices?.[0]?.delta ?? {};
      if (delta.audio?.data) chunks.push(delta.audio.data);
      format ??= delta.audio?.format;
      if (msg.usage?.cost !== undefined) cost = msg.usage.cost;
    }
  }
  if (chunks.length === 0) throw new Error("music: no audio in the response");
  return { bytes: Buffer.from(chunks.join(""), "base64"), format, cost };
}
