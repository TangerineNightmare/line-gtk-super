import type { Client } from "@evex/linejs";
import { InternalError } from "@evex/linejs/base";
import { LINEStruct } from "@evex/linejs/thrift";
import {
  needsGroupKeyRecreation,
  recreateGroupKey,
} from "./group_key_recovery.ts";

export type SendState =
  | "pending"
  | "sending"
  | "succeeded"
  | "failed"
  | "unknown"
  | "cancelled";

export type SentEnvelope = {
  id: string;
  createdTime?: number;
  contentType?: string | number;
};
export type SendOutcome = {
  state: SendState;
  sent?: SentEnvelope;
  error?: string;
};
export type TextPayload = {
  chatMid: string;
  text: string;
  contentMetadata: Record<string, string>;
  relatedMessageId?: string;
};

// Only a decoded, explicit rejection permits fallback. Do not infer one from
// HTTP status, exception text, a malformed response, or a transport timeout.
export function rejectionCode(error: unknown): string | null {
  if (!(error instanceof InternalError) || error.type !== "RequestError") {
    return null;
  }
  const code = error.data?.code;
  return typeof code === "string" || typeof code === "number"
    ? String(code)
    : null;
}

const rejected = new Set([
  "0",
  "ILLEGAL_ARGUMENT",
  "1",
  "AUTHENTICATION_FAILED",
  "5",
  "NOT_FOUND",
  "6",
  "INVALID_LENGTH",
  "7",
  "NOT_AVAILABLE_USER",
  "8",
  "NOT_AUTHORIZED_DEVICE",
  "9",
  "INVALID_MID",
  "10",
  "NOT_A_MEMBER",
  "13",
  "NOT_AVAILABLE_SESSION",
  "14",
  "NOT_AUTHORIZED_SESSION",
  "17",
  "NOT_AUTHENTICATED",
  "25",
  "NO_SUCH_MESSAGE_BOX",
  "26",
  "NOT_AVAILABLE_MESSAGE_BOX",
  "35",
  "ABUSE_BLOCK",
  "36",
  "NOT_FRIEND",
  "38",
  "BLOCK_FRIEND",
  "48",
  "EXCEED_DAILY_QUOTA",
  "64",
  "MESSAGE_NOT_FOUND",
  "70",
  "FEATURE_RESTRICTED",
  "75",
  "SEND_MESSAGE_NOT_PERMITTED_FROM_LINE_AT",
  "76",
  "SEND_MESSAGE_NOT_PERMITTED_WHILE_AUTO_REPLY",
  "81",
  "E2EE_INVALID_PROTOCOL",
  "82",
  "E2EE_RETRY_ENCRYPT",
  "83",
  "E2EE_UPDATE_SENDER_KEY",
  "84",
  "E2EE_UPDATE_RECEIVER_KEY",
  "85",
  "E2EE_INVALID_ARGUMENT",
  "86",
  "E2EE_INVALID_VERSION",
  "87",
  "E2EE_SENDER_DISABLED",
  "88",
  "E2EE_RECEIVER_DISABLED",
  "89",
  "E2EE_SENDER_NOT_ALLOWED",
  "90",
  "E2EE_RECEIVER_NOT_ALLOWED",
  "97",
  "E2EE_PRIMARY_NOT_SUPPORT",
  "98",
  "E2EE_RETRY_PLAIN",
  "99",
  "E2EE_RECREATE_GROUP_KEY",
  "100",
  "E2EE_GROUP_TOO_MANY_MEMBERS",
]);

export function isDefiniteSendRejection(error: unknown): boolean {
  const code = rejectionCode(error);
  return code !== null && rejected.has(code);
}

/** Separate preparation from submission, so session/cancel checks precede POST. */
export async function sendTextSafely<T>(runtime: {
  check: () => void;
  prepare: (encrypted: boolean) => Promise<T>;
  submit: (prepared: T) => Promise<unknown>;
  sending: () => void;
  repairGroupKey?: () => Promise<void>;
}): Promise<SendOutcome> {
  let encrypted = false;
  let repaired = false;
  for (;;) {
    let prepared: T;
    try {
      runtime.check();
      prepared = await runtime.prepare(encrypted);
      runtime.check();
    } catch (error) {
      const reason = error instanceof Error ? error.message : "";
      return {
        state: reason === "operation_cancelled" ? "cancelled" : "failed",
        error: [
            "operation_cancelled",
            "session_changed",
            "contact_blocked_send",
            "invalid_reply_target",
            "notification_reply_source_unavailable",
          ].includes(reason)
          ? reason
          : "send_prepare_failed",
      };
    }
    try {
      runtime.sending();
      const raw = await runtime.submit(prepared) as {
        id?: unknown;
        createdTime?: unknown;
        contentType?: unknown;
      } | null;
      // Missing or rounded IDs cannot reconcile an event with the RPC response.
      const id = raw?.id;
      if (
        !["string", "bigint", "number"].includes(typeof id) ||
        (typeof id === "number" && !Number.isSafeInteger(id)) ||
        !/^[1-9]\d*$/.test(String(id))
      ) return { state: "unknown", error: "send_missing_server_id" };
      const createdTime = Number(raw?.createdTime);
      return {
        state: "succeeded",
        sent: {
          id: String(id),
          createdTime: Number.isSafeInteger(createdTime) && createdTime > 0
            ? createdTime
            : Date.now(),
          contentType: typeof raw?.contentType === "number" ||
              typeof raw?.contentType === "string"
            ? raw.contentType
            : "NONE",
        },
      };
    } catch (error) {
      const code = rejectionCode(error);
      if (!encrypted && ["82", "E2EE_RETRY_ENCRYPT"].includes(code ?? "")) {
        encrypted = true;
        continue;
      }
      if (
        encrypted && !repaired && runtime.repairGroupKey &&
        needsGroupKeyRecreation(error)
      ) {
        repaired = true;
        try {
          runtime.check();
          await runtime.repairGroupKey();
          runtime.check();
        } catch {
          return { state: "failed", error: "send_group_key_unavailable" };
        }
        continue;
      }
      return {
        state: isDefiniteSendRejection(error) ? "failed" : "unknown",
        // Do not expose transport errors containing response bodies or headers.
        error: isDefiniteSendRejection(error)
          ? `send_rejected:${code}`
          : "send_outcome_unknown",
      };
    }
  }
}

const guarded = new WeakSet<Client>();

/** Gate even the dependency's post-token-refresh request on the pinned session. */
export function guardTextSession(client: Client, isCurrent: () => boolean) {
  if (guarded.has(client)) return;
  guarded.add(client);
  type Fetcher = typeof client.base.fetch;
  let fetcher = client.base.fetch;
  const originals = new WeakMap<Fetcher, Fetcher>();
  const wrappers = new WeakMap<Fetcher, Fetcher>();
  // Downloads temporarily replace fetch to count bytes. An accessor keeps the
  // session gate around those wrappers and also around their eventual restore.
  Object.defineProperty(client.base, "fetch", {
    configurable: true,
    get() {
      const original = fetcher;
      let wrapper = wrappers.get(original);
      if (!wrapper) {
        wrapper = (...args) => {
          if (!isCurrent()) return Promise.reject(new Error("session_changed"));
          return original.apply(client.base, args);
        };
        originals.set(wrapper, original);
        wrappers.set(original, wrapper);
      }
      return wrapper;
    },
    set(next: Fetcher) {
      fetcher = originals.get(next) ?? next;
    },
  });
}

/** Application adapter; keeps vendored helpers' implicit retries out of sends. */
export function sendText(
  client: Client,
  payload: TextPayload,
  check: () => void,
  sending: () => void,
): Promise<SendOutcome> {
  const base = client.base;
  return sendTextSafely({
    check,
    sending,
    prepare: async (encrypted) => {
      const chunks = encrypted
        ? await base.e2ee.encryptE2EEMessage(
          payload.chatMid,
          payload.text,
          "NONE",
        )
        : undefined;
      check();
      const seq = await base.getReqseq();
      check();
      return LINEStruct.sendMessage_args({
        seq,
        message: {
          to: payload.chatMid,
          text: encrypted ? undefined : payload.text,
          contentType: "NONE",
          contentMetadata: encrypted
            ? {
              ...payload.contentMetadata,
              e2eeVersion: "2",
              contentType: "0",
              e2eeMark: "2",
            }
            : payload.contentMetadata,
          chunks,
          createdTime: 0,
          deliveredTime: 0,
          hasContent: false,
          sessionId: 0,
          relatedMessageId: payload.relatedMessageId,
          ...(payload.relatedMessageId
            ? {
              messageRelationType: "REPLY",
              relatedMessageServiceCode: "TALK",
            }
            : {}),
        },
      });
    },
    submit: (prepared) => {
      check();
      return base.request.request(
        prepared,
        "sendMessage",
        base.talk.protocolType,
        true,
        base.talk.requestPath,
      );
    },
    repairGroupKey: payload.chatMid.startsWith("c")
      ? async () => {
        await recreateGroupKey(base, payload.chatMid);
      }
      : undefined,
  });
}
