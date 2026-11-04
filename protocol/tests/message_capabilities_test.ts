import { assertEquals } from "@std/assert";
import { BaseClient } from "@evex/linejs/base";
import { enums } from "@evex/linejs-types";
import type { NestedArray } from "@evex/linejs/thrift";

// Synthetic contract fixture: proves local wrapper shapes, not live permission,
// group/room availability, limits, cross-device sync, or server idempotency.
Deno.test("announcement wrappers preserve permissions, source links and i64 sequence", async () => {
  const base = new BaseClient({ device: "DESKTOPWIN" });
  const seq = 9007199254740993123n;
  const messageId = "9007199254740993124";
  const contents = {
    text: "Fixture announcement",
    link: `line://nv/chatMsg?chatId=c-fixture&messageId=${messageId}`,
  };
  const announcement = {
    announcementSeq: seq,
    type: "MESSAGE",
    contents,
    creatorMid: "u-fixture",
    createdTime: 1700000000000n,
    deletePermission: "CREATOR_ONLY",
  };
  const calls: { method: string; args: NestedArray }[] = [];
  base.request.request = ((args: NestedArray, method: string) => {
    calls.push({ method, args });
    return Promise.resolve(
      method === "getChatRoomAnnouncements"
        ? [announcement]
        : method === "createChatRoomAnnouncement"
        ? announcement
        : undefined,
    );
  }) as typeof base.request.request;
  const listed = await base.talk.getChatRoomAnnouncements({
    chatRoomMid: "c-fixture",
  });
  assertEquals(listed[0].deletePermission, "CREATOR_ONLY");
  assertEquals(String(listed[0].announcementSeq), String(seq));
  const created = await base.talk.createChatRoomAnnouncement({
    reqSeq: 1,
    chatRoomMid: "c-fixture",
    type: "MESSAGE",
    contents,
  });
  assertEquals(created.contents.link, contents.link);
  await base.talk.removeChatRoomAnnouncement({
    reqSeq: 2,
    chatRoomMid: "c-fixture",
    announcementSeq: seq,
  });
  assertEquals(calls.map((call) => call.method), [
    "getChatRoomAnnouncements",
    "createChatRoomAnnouncement",
    "removeChatRoomAnnouncement",
  ]);
  assertEquals(calls[0].args, [[11, 2, "c-fixture"]]);
  assertEquals(calls[2].args, [[8, 1, 2], [11, 2, "c-fixture"], [10, 3, seq]]);
  assertEquals(enums.Pb1_W2, { ANYONE_IN_CHAT: 0, CREATOR_ONLY: 1, NO_ONE: 2 });
});
