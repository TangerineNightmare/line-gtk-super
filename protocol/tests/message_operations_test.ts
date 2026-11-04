import { assertEquals, assertThrows } from "@std/assert";
import { createMessageOperations } from "../src/message_operations.ts";
import type { SendOutcome } from "../src/message_send.ts";

function fixture() {
  let session: { owner: object; accountMid: string } | null = {
    owner: {},
    accountMid: "u-self",
  };
  const service = createMessageOperations(() => session);
  return {
    service,
    change(accountMid: string) {
      session = { owner: {}, accountMid };
    },
    logout() {
      session = null;
    },
  };
}

Deno.test("same operation coalesces while sending and reuses only its confirmed result", async () => {
  const { service } = fixture();
  const completion = Promise.withResolvers<SendOutcome>();
  let sends = 0;
  const action = (_check: () => void, sending: () => void) => {
    sends++;
    sending();
    return completion.promise;
  };
  const first = service.run("forward-1", "u-dest", { text: "private" }, action);
  const second = service.run(
    "forward-1",
    "u-dest",
    { text: "private" },
    action,
  );
  await Promise.resolve();
  assertEquals(service.status("forward-1").state, "sending");
  assertEquals(service.cancel("forward-1").state, "sending");
  completion.resolve({ state: "succeeded", sent: { id: "9007199254740993" } });
  assertEquals(await first, await second);
  assertEquals(
    await service.run("forward-1", "u-dest", { text: "private" }, action),
    await first,
  );
  assertEquals(sends, 1);
  assertEquals(
    JSON.stringify(service.status("forward-1")).includes("private"),
    false,
  );
});

Deno.test("unknown results are never automatically replayed or reclassified by cancellation", async () => {
  const { service } = fixture();
  let sends = 0;
  const action = () => {
    sends++;
    return Promise.resolve({ state: "unknown" } as const);
  };
  await service.run("text-1", "c-group", "payload", action);
  await service.run("text-1", "c-group", "payload", action);
  assertEquals(service.cancel("text-1").state, "unknown");
  assertEquals(sends, 1);
  assertThrows(
    () => service.run("text-1", "c-other", "payload", action),
    Error,
    "operation_payload_changed",
  );
  assertThrows(
    () => service.run("text-1", "c-group", "changed", action),
    Error,
    "operation_payload_changed",
  );
});

Deno.test("cancelling before dispatch, logout and replacement invalidate pending work", async () => {
  for (const change of ["cancel", "logout", "account", "session"]) {
    const f = fixture();
    let sends = 0;
    const request = f.service.run("text-1", "c-group", "payload", () => {
      sends++;
      return Promise.resolve({ state: "succeeded" });
    });
    if (change === "cancel") f.service.cancel("text-1");
    else if (change === "logout") f.logout();
    else f.change(change === "account" ? "u-other" : "u-self");
    const result = await request;
    assertEquals(result.state, change === "cancel" ? "cancelled" : "failed");
    assertEquals(sends, 0);
    if (change !== "cancel") assertThrows(() => f.service.status("text-1"));
  }
});

Deno.test("restart has no recovery queue and unknown IDs cannot initiate a send", async () => {
  const f = fixture();
  await f.service.run(
    "text-1",
    "u-dest",
    "payload",
    () => Promise.resolve({ state: "unknown" }),
  );
  const restarted = fixture();
  assertThrows(
    () => restarted.service.status("text-1"),
    Error,
    "message_operation_not_found",
  );
  assertThrows(
    () => restarted.service.cancel("text-1"),
    Error,
    "message_operation_not_found",
  );
});
