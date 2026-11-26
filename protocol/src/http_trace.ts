/**
 * Opt-in HTTP trace for the line-gtk protocol sidecar (local patch, not upstream).
 *
 * Enable with LINE_GTK_TRACE_HTTP=1. Under Deno, linejs sends every Thrift/OBS
 * request through globalThis.fetch, so wrapping it here sees all of them.
 * Lines go to stderr, which line-gtk copies into ~/.local/share/line-gtk/line-gtk.log
 * (target line_gtk::protocol).
 *
 * Never logged: query strings, request/response bodies of successful calls,
 * and any credential header (x-line-access, x-line-next-access, authorization,
 * cookies, x-line-channeltoken, x-ls). Error bodies (status >= 400) are logged
 * truncated to 300 chars and only when they are text/JSON.
 */

// x-lcs (the RSA-wrapped per-session LEGY key) is deliberately not logged.
const SAFE_REQ_HEADERS = ["x-line-application", "user-agent", "x-lal", "x-lpv", "x-le", "content-type"];
const SECRET = /^(x-line-access|x-line-next-access|authorization|cookie|set-cookie|x-line-channeltoken|x-ls|x-lst|x-line-session-id)$/i;

function safeUrl(u: string): string {
  try {
    const url = new URL(u);
    return `${url.origin}${url.pathname}${url.search ? "?<redacted>" : ""}`;
  } catch {
    return "<unparseable url>";
  }
}

function pickReq(h: Headers): Record<string, string> {
  const out: Record<string, string> = {};
  for (const k of SAFE_REQ_HEADERS) {
    const v = h.get(k);
    if (v != null) out[k] = k === "x-line-application" ? v.replaceAll("\t", " | ") : v;
  }
  return out;
}

function pickRes(h: Headers): Record<string, string> {
  const out: Record<string, string> = {};
  h.forEach((v, k) => {
    if (!SECRET.test(k)) out[k] = v.length > 120 ? v.slice(0, 120) + "…" : v;
  });
  return out;
}

let seq = 0;

export function installHttpTrace(): void {
  if (Deno.env.get("LINE_GTK_TRACE_HTTP") !== "1") return;
  const orig = globalThis.fetch.bind(globalThis);
  globalThis.fetch = async (input: RequestInfo | URL, init?: RequestInit): Promise<Response> => {
    const id = ++seq;
    // Read method/url/headers without constructing a Request: new Request(input)
    // takes ownership of input's body stream and the real fetch then fails with
    // "Input request's body is unusable".
    const isReq = input instanceof Request;
    const method = init?.method ?? (isReq ? input.method : "GET");
    const url = isReq ? input.url : String(input);
    const headers = new Headers(init?.headers ?? (isReq ? input.headers : undefined));
    const t0 = performance.now();
    const tag = `[http #${id}] ${method} ${safeUrl(url)}`;
    console.error(`${tag} -> ${JSON.stringify(pickReq(headers))}`);
    try {
      const res = await orig(input, init);
      const ms = Math.round(performance.now() - t0);
      let errBody = "";
      if (res.status >= 400) {
        const ct = res.headers.get("content-type") ?? "";
        if (/json|text/i.test(ct)) {
          try {
            errBody = " body=" + JSON.stringify((await res.clone().text()).slice(0, 300));
          } catch { /* ignore */ }
        }
      }
      console.error(`${tag} <- ${res.status} ${ms}ms ${JSON.stringify(pickRes(res.headers))}${errBody}`);
      return res;
    } catch (e) {
      const ms = Math.round(performance.now() - t0);
      console.error(`${tag} !! ${ms}ms ${e instanceof Error ? `${e.name}: ${e.message}` : String(e)}`);
      throw e;
    }
  };
  console.error("[http] trace enabled (LINE_GTK_TRACE_HTTP=1); credentials and query strings redacted");
}
