import { createHash } from "node:crypto";
import type { SendOutcome, SendState } from "./message_send.ts";

export type MessageOperation = SendOutcome & {
  operationId: string;
  accountMid: string;
  destinationChatMid: string;
};
type Entry = {
  fingerprint: string;
  result: MessageOperation;
  cancelled: boolean;
  promise: Promise<MessageOperation>;
};

/** In-memory dedupe only. Never an assertion of server-side idempotency. */
export function createMessageOperations(
  session: () => { owner: object; accountMid: string } | null,
) {
  let owner: object | null = null;
  let accountMid = "";
  const entries = new Map<string, Entry>();
  function current() {
    const active = session();
    if (active?.owner !== owner || active?.accountMid !== accountMid) {
      entries.clear();
      owner = active?.owner ?? null;
      accountMid = active?.accountMid ?? "";
    }
    if (!active || !active.accountMid) throw new Error("not_logged_in");
    return active;
  }
  const fingerprint = (payload: unknown) =>
    createHash("sha256").update(JSON.stringify(payload)).digest("hex");
  return {
    run(
      operationId: string,
      destinationChatMid: string,
      payload: unknown,
      action: (
        check: () => void,
        sending: () => void,
      ) => Promise<SendOutcome>,
    ): Promise<MessageOperation> {
      const active = current();
      if (!/^[a-zA-Z0-9_-]{1,128}$/.test(operationId)) {
        throw new Error("invalid_operation_id");
      }
      const hash = fingerprint([destinationChatMid, payload]);
      const previous = entries.get(operationId);
      if (previous) {
        if (previous.fingerprint !== hash) {
          throw new Error("operation_payload_changed");
        }
        return previous.promise;
      }
      // Do not evict an ID and accidentally make its next use submit again.
      if (entries.size >= 1024) throw new Error("message_operation_limit");
      const entry: Entry = {
        fingerprint: hash,
        cancelled: false,
        result: {
          operationId,
          accountMid: active.accountMid,
          destinationChatMid,
          state: "pending",
        },
        promise: Promise.resolve(null as never),
      };
      entries.set(operationId, entry);
      const check = () => {
        const now = session();
        if (
          now?.owner !== active.owner || now.accountMid !== active.accountMid
        ) {
          throw new Error("session_changed");
        }
        if (entry.cancelled) throw new Error("operation_cancelled");
      };
      entry.promise = Promise.resolve().then(async () => {
        let result: SendOutcome;
        try {
          check();
          result = await action(check, () => {
            check();
            entry.result.state = "sending";
          });
        } catch {
          const state: SendState = entry.result.state === "sending"
            ? "unknown"
            : entry.cancelled
            ? "cancelled"
            : "failed";
          result = { state, error: "message_operation_interrupted" };
        }
        entry.result = { ...entry.result, ...result };
        return { ...entry.result };
      });
      return entry.promise;
    },
    status(operationId: string): MessageOperation {
      current();
      const entry = entries.get(operationId);
      if (!entry) throw new Error("message_operation_not_found");
      return { ...entry.result };
    },
    cancel(operationId: string): MessageOperation {
      current();
      const entry = entries.get(operationId);
      if (!entry) throw new Error("message_operation_not_found");
      // Once a request may be in flight, cancellation cannot undo its outcome.
      if (entry.result.state === "pending") {
        entry.cancelled = true;
        entry.result.state = "cancelled";
      }
      return { ...entry.result };
    },
  };
}
