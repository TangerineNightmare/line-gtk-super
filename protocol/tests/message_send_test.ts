import { assertEquals, assertRejects } from "@std/assert";
import { BaseClient, InternalError } from "@evex/linejs/base";
import type { Client } from "@evex/linejs";
import {
  guardTextSession,
  isDefiniteSendRejection,
  sendTextSafely,
} from "../src/message_send.ts";

const rejection = (code: string | number) =>
  new InternalError("RequestError", "fixture rejection", { code });

for (
  const error of [
    new DOMException("accepted, then timed out", "TimeoutError"),
    new Error("connection reset after commit"),
    new Error("LINE_HTTP_503"),
    new Error("E2EE_RETRY_ENCRYPT"),
    new InternalError("OtherError", "not a decoded rejection", { code: 82 }),
    rejection("SYSTEM_ERROR"),
    rejection("DUPLICATED"),
  ]
) {
  Deno.test(`ambiguous send ${error.name}:${error.message} never falls back`, async () => {
    let sends = 0;
    const result = await sendTextSafely({
      check: () => {},
      prepare: (encrypted) => Promise.resolve(encrypted),
      sending: () => {},
      submit: () => {
        sends++;
        return Promise.reject(error);
      },
    });
    assertEquals(result, { state: "unknown", error: "send_outcome_unknown" });
    assertEquals(sends, 1);
  });
}

for (const code of [82, "82", "E2EE_RETRY_ENCRYPT"]) {
  Deno.test(`decoded encryption rejection ${code} permits exactly one encrypted send`, async () => {
    const attempts: boolean[] = [];
    const result = await sendTextSafely({
      check: () => {},
      prepare: (encrypted) => Promise.resolve(encrypted),
      sending: () => {},
      submit: (encrypted) => {
        attempts.push(encrypted);
        return attempts.length === 1
          ? Promise.reject(rejection(code))
          : Promise.resolve({
            id: 9007199254740993123n,
            createdTime: 1700000000000n,
            contentType: "NONE",
          });
      },
    });
    assertEquals(attempts, [false, true]);
    assertEquals(result.state, "succeeded");
    assertEquals(result.sent?.id, "9007199254740993123");
    assertEquals(
      JSON.parse(JSON.stringify(result)).sent.createdTime,
      1700000000000,
    );
  });
}

Deno.test("encrypted timeout does not try the high-level chat route or resend plaintext", async () => {
  let sends = 0;
  const result = await sendTextSafely({
    check: () => {},
    prepare: (encrypted) => Promise.resolve(encrypted),
    sending: () => {},
    submit: () =>
      Promise.reject(++sends === 1 ? rejection(82) : new Error("timeout")),
  });
  assertEquals(result.state, "unknown");
  assertEquals(sends, 2);
});

Deno.test("session change while preparing or between rejected attempts prevents submission", async () => {
  for (const changeAt of ["prepare", "fallback"]) {
    let current = true;
    let sends = 0;
    const result = await sendTextSafely({
      check: () => {
        if (!current) throw new Error("session_changed");
      },
      prepare: () => {
        if (changeAt === "prepare") current = false;
        return Promise.resolve();
      },
      sending: () => {},
      submit: () => {
        sends++;
        current = false;
        return Promise.reject(rejection(82));
      },
    });
    assertEquals(result, { state: "failed", error: "session_changed" });
    assertEquals(sends, changeAt === "prepare" ? 0 : 1);
  }
});

Deno.test("preparation failure is definite; missing, unsafe or malformed server IDs are unknown", async () => {
  let sends = 0;
  const result = await sendTextSafely({
    check: () => {},
    prepare: () => Promise.reject(new Error("keys unavailable")),
    sending: () => {},
    submit: () => {
      sends++;
      return Promise.resolve();
    },
  });
  assertEquals(result.state, "failed");
  assertEquals(sends, 0);
  for (
    const id of [undefined, Number.MAX_SAFE_INTEGER + 1, "pending-1", {}, 0]
  ) {
    const result = await sendTextSafely({
      check: () => {},
      prepare: () => Promise.resolve(),
      sending: () => {},
      submit: () => Promise.resolve({ id }),
    });
    assertEquals(result.state, "unknown");
  }
});

Deno.test("only known decoded rejections are definite and group key repair is bounded", async () => {
  assertEquals(isDefiniteSendRejection(rejection("NOT_A_MEMBER")), true);
  assertEquals(isDefiniteSendRejection(rejection("SYSTEM_ERROR")), false);
  assertEquals(
    isDefiniteSendRejection({ data: { code: "NOT_A_MEMBER" } }),
    false,
  );
  let sends = 0;
  let repairs = 0;
  const result = await sendTextSafely({
    check: () => {},
    prepare: (encrypted) => Promise.resolve(encrypted),
    sending: () => {},
    submit: () => Promise.reject(rejection(++sends === 1 ? 82 : 99)),
    repairGroupKey: () => {
      repairs++;
      return Promise.resolve();
    },
  });
  assertEquals(result.state, "failed");
  assertEquals(sends, 3);
  assertEquals(repairs, 1);
});

Deno.test("session fetch gate survives a concurrent download wrapper and its restore", async () => {
  let current = true;
  let calls = 0;
  const base = new BaseClient({
    device: "DESKTOPWIN",
    fetch: () => {
      calls++;
      return Promise.resolve(new Response("fixture"));
    },
  });
  const client = { base } as Client;
  const beforeSend = base.fetch;
  // A download started before the first send and will later restore this fetch.
  const mutable = base as unknown as { fetch: typeof base.fetch };
  mutable.fetch = (...args) => beforeSend(...args);
  guardTextSession(client, () => current);
  await (await base.fetch("https://example.invalid")).body?.cancel();
  mutable.fetch = beforeSend;
  current = false;
  await assertRejects(
    () => base.fetch("https://example.invalid"),
    Error,
    "session_changed",
  );
  current = true;
  // A second download starts after the gate is already installed.
  const guarded = base.fetch;
  mutable.fetch = (...args) => guarded(...args);
  await (await base.fetch("https://example.invalid")).body?.cancel();
  mutable.fetch = guarded;
  current = false;
  await assertRejects(
    () => base.fetch("https://example.invalid"),
    Error,
    "session_changed",
  );
  assertEquals(calls, 2);
});
