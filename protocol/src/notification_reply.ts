import { conversationMid } from "./messages.ts";
import { replyMessageId } from "./replies.ts";

type Json = Record<string, unknown>;
export type NotificationReply = {
  accountMid: string;
  sourceMessageId: string;
  quote: boolean;
};

/** An explicit opt-out is required before an unavailable source becomes plain text. */
export function notificationReply(value: unknown): NotificationReply {
  const raw = value as Partial<NotificationReply> | null;
  if (
    !raw || !/^u[A-Za-z0-9_-]{1,128}$/.test(String(raw.accountMid ?? "")) ||
    !replyMessageId(raw.sourceMessageId) || typeof raw.quote !== "boolean"
  ) throw new Error("invalid_notification_reply");
  return {
    accountMid: String(raw.accountMid),
    sourceMessageId: String(raw.sourceMessageId),
    quote: raw.quote,
  };
}

export function checkNotificationSource(
  source: Json | undefined,
  chatMid: string,
  messageId: string,
) {
  if (
    !source || String(source.id) !== messageId ||
    conversationMid(source) !== chatMid || source.decryptionFailed ||
    source.chatEvent ||
    ["UNSENT", "18", "CALL", "CHAT_EVENT", "CHATEVENT"].includes(
      String(source.contentType).toUpperCase(),
    )
  ) throw new Error("notification_reply_source_unavailable");
}
