import { assert, assertEquals, assertRejects } from "@std/assert";
import {
  createLineFetch,
  failureKind,
  legyEncrypted,
  restoreWithRetry,
} from "../src/connection.ts";

Deno.test("only explicit auth failures invalidate a session", () => {
  assertEquals(
    failureKind({ data: { code: "NOT_AUTHORIZED_DEVICE" } }),
    "auth",
  );
  assertEquals(failureKind(new Error("V3_TOKEN_CLIENT_LOGGED_OUT")), "auth");
  assertEquals(failureKind(new TypeError("fetch failed")), "network");
  assertEquals(
    failureKind(new DOMException("expired", "TimeoutError")),
    "network",
  );
  assertEquals(failureKind(new Error("LINE_HTTP_503")), "server");
  assertEquals(failureKind(new Error("LINE_HTTP_429")), "server");
  assertEquals(failureKind(new Error("LINE_HTTP_403")), "protocol");
  assertEquals(failureKind(new TypeError("cannot read property")), "protocol");
});

Deno.test("session restore retries transient failures with bounded backoff", async () => {
  let calls = 0;
  const waits: number[] = [];
  const attempts: number[] = [];
  const result = await restoreWithRetry(
    () => {
      if (++calls < 4) throw new Error("LINE_HTTP_503");
      return Promise.resolve("restored");
    },
    (n) => attempts.push(n),
    (ms) => {
      waits.push(ms);
      return Promise.resolve();
    },
  );
  assertEquals(result, "restored");
  assertEquals(waits, [1000, 2000, 4000]);
  assertEquals(attempts, [1, 2, 3]);
  calls = 0;
  await assertRejects(() =>
    restoreWithRetry(
      () => {
        calls++;
        throw new Error("fetch failed");
      },
      () => {},
      () => Promise.resolve(),
    )
  );
  assertEquals(calls, 4);
});

Deno.test("auth and protocol errors are never retried", async () => {
  for (const message of ["LOGGED_OUT", "API method not capable"]) {
    let calls = 0;
    await assertRejects(() =>
      restoreWithRetry(() => {
        calls++;
        throw new Error(message);
      }, () => {
        throw new Error("unexpected retry");
      })
    );
    assertEquals(calls, 1);
  }
});

Deno.test("HTTP failures are classified without replaying POST requests", async () => {
  let calls = 0;
  const fetcher = createLineFetch(1000, () => {
    calls++;
    return Promise.resolve(new Response("unavailable", { status: 503 }));
  });
  await assertRejects(
    () => fetcher(new Request("https://example.invalid", { method: "POST" })),
    Error,
    "LINE_HTTP_503",
  );
  assertEquals(calls, 1);
});

Deno.test("LINE requests have a deadline and retain caller cancellation", async () => {
  const fetcher = createLineFetch(
    5,
    (_input, init) =>
      new Promise((_resolve, reject) => {
        const signal = init!.signal!;
        if (signal.aborted) reject(signal.reason);
        else {signal.addEventListener("abort", () => reject(signal.reason), {
            once: true,
          });}
      }),
  );
  await assertRejects(
    () => fetcher(new Request("https://example.invalid")),
    DOMException,
  );
  const controller = new AbortController();
  controller.abort();
  await assertRejects(
    () =>
      fetcher(
        new Request("https://example.invalid", { signal: controller.signal }),
      ),
    DOMException,
  );
});

Deno.test("OBS uploads outlive the normal exchange deadline", async () => {
  const abortedAt: number[] = [];
  const fetcher = createLineFetch(
    50,
    (_input, init) =>
      new Promise((_resolve, reject) => {
        const signal = init!.signal!;
        const started = Date.now();
        signal.addEventListener(
          "abort",
          () => {
            abortedAt.push(Date.now() - started);
            reject(signal.reason);
          },
          { once: true },
        );
      }),
    undefined,
    400,
  );
  const upload = fetcher(
    new Request("https://obs.line-apps.com/r/talk/m/reqseq", {
      method: "POST",
      body: new Blob([new Uint8Array(64)]),
    }),
  );
  const plain = fetcher(new Request("https://legy.line-apps.com/api/v1/x"));
  await assertRejects(() => plain, DOMException);
  // The plain request gave up at ~50ms; the upload keeps going past that.
  assertEquals(abortedAt.length, 1);
  assert(abortedAt[0] < 200);
  await assertRejects(() => upload, DOMException);
  assertEquals(abortedAt.length, 2);
  assert(abortedAt[1] >= 350);
});

Deno.test("replacing a login cancels a pending session request", async () => {
  const controller = new AbortController();
  const fetcher = createLineFetch(1000, (_input, init) => {
    return new Promise((_resolve, reject) => {
      init!.signal!.addEventListener(
        "abort",
        () => reject(init!.signal!.reason),
        { once: true },
      );
    });
  }, controller.signal);
  const pending = fetcher(new Request("https://example.invalid"));
  controller.abort();
  await assertRejects(() => pending, DOMException);
});

Deno.test("LEGY transport defaults to plain and parses explicit modes", () => {
  assertEquals(legyEncrypted(undefined), false);
  assertEquals(legyEncrypted(""), false);
  assertEquals(legyEncrypted(" AUTO "), "auto");
  assertEquals(legyEncrypted("true"), true);
  assertEquals(legyEncrypted("false"), false);
  assertEquals(legyEncrypted("yes"), false);
});
