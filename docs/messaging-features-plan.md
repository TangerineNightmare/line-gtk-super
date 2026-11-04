# Kế hoạch tính năng nhắn tin

Trạng thái: đã triển khai và kiểm thử cục bộ nền tảng M0, 2026-11-04; M1–M5 chưa triển khai. Xem [hợp đồng operation, ma trận và bằng chứng M0](messaging-m0.md). Khảo sát media/announcement/notification đã ghi rõ phần được chứng minh bằng nguồn/fixture và phần còn cần tài khoản LINE hoặc desktop thật.

Mục tiêu: bổ sung chọn nhiều tin, chuyển tiếp/chia sẻ, ghim từng tin, tìm kiếm xuyên cuộc trò chuyện và trả lời từ thông báo. Chia thành các đợt có thể phát hành riêng. Các giới hạn đề xuất dưới đây là giới hạn của ứng dụng, cần điều chỉnh sau kiểm thử; không phải giới hạn được xác nhận của LINE.

## Hiện trạng và phần có thể dùng lại

| Phần | Hiện trạng trong repo | Hướng sử dụng |
| --- | --- | --- |
| Menu từng tin | `src/ui/replies.rs` gom trả lời, biểu cảm, thu hồi và sao chép | Thêm Chọn, Chuyển tiếp và Ghim theo khả năng của tin |
| Danh sách tin | `src/ui/virtual_list.rs` dùng `gtk::ListView` với `gtk::NoSelection`; lịch sử có cửa sổ hiển thị giới hạn | Quản lý lựa chọn bằng ID, độc lập với vị trí và vòng đời widget |
| Gửi tin | `protocol/src/commands.ts` xử lý văn bản/trích dẫn/mention; `protocol/src/outgoing_media.ts` xử lý media | Tách phần gửi dùng chung trước khi nối chuyển tiếp và trả lời thông báo |
| Tin chờ và bản nháp | `src/ui/cache_boot.rs` có outbox; composer có bản nháp theo chat | Dùng lại chính sách khôi phục, bổ sung nguồn gửi và đích cố định |
| Tìm kiếm | `protocol/src/message_search.ts` tìm chuỗi Unicode chuẩn hóa trong text/tên file, 50 kết quả/trang; RPC hiện bắt buộc một `chatMid` | Mở rộng phạm vi sang lịch sử đã lưu của tài khoản, giữ hành vi tìm trong chat |
| Lịch sử và mã hóa | `message_store.ts` ghi cache từng chat với `accountMid`; mặc định lưu 2.000 tin/chat, có cấu hình giới hạn; `cache_crypto.ts` mã hóa khi có khóa | Tìm kiếm tuân theo phạm vi lưu hiện có; không tạo chỉ mục văn bản rõ riêng |
| Nhảy đến tin | `src/ui/history.rs` có đường mở ngữ cảnh tin tìm kiếm/shared media, không chuyển về tin mới nhất | Dùng lại cho kết quả toàn cục và ghim; giữ chính sách đã đọc |
| Ghim trên LINE | Dependency có `getChatRoomAnnouncements`, `createChatRoomAnnouncement`, `removeChatRoomAnnouncement`; `TalkMessage.announce()` có ví dụ cho tin văn bản trong group/room | Khả thi để khảo sát, chưa chứng minh quyền, hạn mức hoặc hoạt động trên tài khoản thật |
| Thông báo | `src/desktop_notify.rs` dùng `notify-rust`, hiện có action mở chat; `src/ui/notifications.rs` có đường dự phòng GIO | Thêm action Trả lời và kiểm tra khả năng nhập trực tiếp của từng backend |

Nguồn phạm vi: [danh sách chưa hỗ trợ trong README](../README.vi.md#chưa-hỗ-trợ). Kế hoạch [shared media](shared-media-browser-plan.md) và [album](conversation-albums-plan.md) tiếp tục riêng.

## Phạm vi và thứ tự

| Đợt | Kết quả người dùng nhận được | Phụ thuộc | Mức rủi ro tương đối |
| --- | --- | --- | --- |
| M0 | Quy tắc gửi và hợp đồng dùng chung được xác định, kiểm tra được lỗi gửi không rõ kết quả | Không | Trung bình |
| M1 | Chọn nhiều tin, sao chép và lưu tệp đã chọn | Không; phát hành độc lập M0 | Thấp đến trung bình |
| M2 | Chuyển tiếp văn bản, sau đó ảnh/tệp, với tiến trình từng tin | M0 + M1; chuyển tiếp một tin không cần chế độ chọn | Trung bình đến cao |
| M3 | Danh sách tin ghim trên LINE, ghim/bỏ ghim và mở tin gốc | Khảo sát API announcement; dùng lại xử lý kết quả chưa rõ của M0 | Trung bình |
| M4 | Tìm văn bản/tên file trong mọi chat có lịch sử đã lưu của tài khoản | Không phụ thuộc gửi tin | Trung bình |
| M5 | Trả lời từ thông báo; nhập trực tiếp nếu backend hỗ trợ | M0 + khảo sát notification backend | Trung bình đến cao |

Thứ tự thực hiện đề xuất: **M0 → M1 → M2 → M3 → M4 → M5**. M1 có thể phát hành trước khi M0 hoàn thành. Nếu API ghim chưa đủ bằng chứng, tiếp tục M4 trong khi giữ M3 ở bước khảo sát. Chưa ước lượng ngày hoàn thành trước khi chốt khả năng gửi lại media, announcement và notification reply.

## Quy tắc chung

- Mọi thao tác giữ cố định tài khoản, phiên, chat nguồn và chat đích. Chuyển chat khi dialog đang mở không đổi đích gửi. Logout, đổi tài khoản hoặc thay phiên khi recovery vô hiệu hóa callback và công việc cũ.
- Khóa định danh là `(accountMid, chatMid, messageId)`; ID tin và announcement đi qua RPC dưới dạng chuỗi. Không chuyển ID lớn sang JavaScript `number`.
- Kiểm tra tin đã thu hồi, lỗi giải mã, quyền truy cập và trạng thái chặn tại sidecar trước mỗi bước có tác động từ xa. Giao diện vô hiệu hóa nút không thay cho kiểm tra dịch vụ.
- Phân biệt chưa gửi, đang gửi, đã xác nhận thành công, thất bại xác định, và chưa biết kết quả. Timeout sau khi có thể đã gửi không được tự động coi là thất bại để gửi lại.
- `operationId` cục bộ giúp gom yêu cầu trùng trong cùng phiên, không chứng minh server có idempotency. Chỉ tự thử lại khi chắc chắn chưa gửi hoặc khi có cơ chế đối soát đã xác minh.
- Khi chưa có đối soát đáng tin cậy, hiển thị Chưa xác định và yêu cầu kiểm tra đích trước khi chủ động gửi lại. Recovery không tự chạy lại các thao tác chưa rõ kết quả.
- Không tự đánh dấu đã đọc chỉ vì chọn tin, mở tìm kiếm, xem ghim, mở hộp chuyển tiếp hoặc mở hộp trả lời nhanh. Dùng lại read policy và kiểm thử tương tác khi thực sự mở chat.
- Giao diện theo GTK4/Libadwaita hiện có, thao tác được bằng bàn phím, focus rõ ràng, không phụ thuộc hover hoặc màu sắc. Dịch mọi nhãn mới trong bốn file ngôn ngữ Anh/Nhật/Việt/Thái.
- Không chỉnh các thay đổi chưa commit đang có khi triển khai từng đợt; xác định lại baseline và chia commit theo tính năng.

## M0 — Nền tảng gửi và khảo sát khả năng

- [x] Lập ma trận theo loại tin: TEXT, IMAGE, FILE, VIDEO, AUDIO, STICKER, Flex, poll, sự kiện, tin đã thu hồi và tin lỗi giải mã. Ghi loại nào được chọn, sao chép, lưu, chuyển tiếp và ghim.
- [x] Kiểm tra đường gửi hiện tại, đặc biệt chuỗi fallback trong `commands.ts::doSend`: lỗi không rõ kết quả ở lần gọi đầu có thể dẫn tới lần gọi tiếp theo. Chốt phân loại lỗi và điều kiện fallback trước khi tái sử dụng cho gửi hàng loạt.
- [x] Tách helper gửi có kết quả chuẩn hóa và kiểm tra phiên; giữ tương thích `send_message` và các đường gửi hiện có. Event và response dùng server message ID để tránh nhân đôi tin trên giao diện.
- [x] Định nghĩa vòng đời operation và chính sách recovery. Chỉ lưu nhật ký tối thiểu nếu cần đối soát qua restart, theo tài khoản và cơ chế mã hóa cache hiện có; không lưu thêm toàn bộ nội dung tin vào log.
- [ ] Xác minh khả năng lấy ảnh/tệp gốc từ tin cũ và gửi lại tới đích; không dùng thumbnail thay bản gốc. Chưa thấy wrapper chuyển tiếp chuyên dụng trong phần Talk đã kiểm tra, nên không giả định có server-side forward.
- [ ] Xác minh request/response, quyền xóa, kiểu chat, hạn mức, thông báo thay đổi và liên kết nguồn của announcement bằng dependency và fixture đã làm sạch.
- [x] Khảo sát khả năng reply của notification daemon, `notify-rust`, GIO và gói Flatpak; xác định có cần adapter/dependency mới. Không giả định desktop nào cũng có ô nhập trong thông báo.

Kết quả: helper văn bản, dedupe theo phiên, status/cancel RPC và bubble **Chưa xác định** đã có regression. Hai mục xác minh media/announcement ở trên mới có khảo sát nguồn và fixture tổng hợp; giữ chưa đánh dấu hoàn thành cho đến khi có bằng chứng liên thông/quyền/hạn mức thực tế. Native/Flatpak trên desktop thật và inline reply chưa được xác minh. Không mở các tính năng M2b/M3/M5b từ khảo sát này.

Nghiệm thu: có ma trận khả năng và hợp đồng operation, regression cho timeout/fallback/session change, xác định rõ phần nào chỉ được chứng minh bằng fixture và phần nào cần LINE/desktop thật.

## M1 — Chọn nhiều tin và thao tác cục bộ

- [ ] Thêm Chọn tin nhắn ở menu từng tin và lối vào từ menu cuộc trò chuyện. Tin khởi đầu được chọn sẵn; hiện checkbox, số tin đã chọn và nút Hủy.
- [ ] Lưu tập ID ở state theo tài khoản/chat, không lưu chỉ số dòng. Bỏ qua nhãn ngày, sentinel tải lịch sử và dòng sự kiện. Giữ tối đa 100 tin được chọn ở đợt đầu.
- [ ] Giữ lựa chọn khi widget được tái sử dụng hoặc tin rời cửa sổ render. Chỉ giữ metadata cần thiết, không giữ texture hoặc cả widget; tin thu hồi bị bỏ khỏi lựa chọn.
- [ ] Space bật/tắt tin đang focus trong chế độ chọn; Shift chọn khoảng trong phần lịch sử đã tải; Esc thoát và trả focus về tin khởi đầu. Không chiếm Ctrl+A khi người dùng đang chọn chữ trong composer hoặc hộp sao chép.
- [ ] Thanh thao tác có Sao chép, Lưu tệp, Chuyển tiếp. Chỉ hiện Chuyển tiếp khi M2 đã có. Hiển thị số tin áp dụng được cho từng thao tác, giải thích loại bị bỏ qua trước khi thực hiện.
- [ ] Sao chép theo thứ tự thời gian tăng dần, kèm tên người gửi và mốc thời gian; giữ nguyên nội dung và ngăn cách từng tin rõ ràng. Định dạng metadata theo locale, không đưa ID nội bộ vào clipboard.
- [ ] Lưu ảnh/tệp đã chọn dùng original, download preferences, xử lý trùng tên và hàng đợi tối đa ba lượt tải đồng thời. Có tiến trình, hủy phần chưa chạy và kết quả từng tệp. Dùng helper chung với shared-media batch save khi triển khai phần đó.
- [ ] Chuyển chat hoặc logout thoát chế độ chọn. Nếu đã mở dialog tác vụ, dialog giữ đích nguồn cố định; logout đóng và hủy tác vụ chưa chạy.

Chưa đưa thu hồi/xóa hàng loạt vào M1; đây là tác vụ có tác động khác với sao chép/lưu và cần thiết kế riêng.

Nghiệm thu: chọn, cuộn qua giới hạn cửa sổ, nhận thêm tin, thu hồi từ thiết bị khác và recovery không chọn nhầm dòng; có GTK fixture cho bàn phím, focus và vị trí cuộn.

## M2 — Chuyển tiếp và chia sẻ

### M2a: văn bản

- [ ] Thêm Chuyển tiếp ở menu một tin và thanh chọn nhiều tin. Dialog hiển thị preview nguồn, danh sách đích có tìm kiếm và số tin sẽ gửi.
- [ ] Đợt đầu chọn một chat đích cho mỗi tác vụ; hỗ trợ DM và nhóm đã tham gia, chỉ mở room nếu đã xác minh. Tài khoản chính thức/OpenChat không nằm trong đợt đầu.
- [ ] Hiện tên và ảnh đích cạnh nút Chuyển tiếp. Người dùng xem lại rồi bấm gửi; việc tìm/chọn đích không phát tin.
- [ ] Gửi từng tin theo thứ tự nguồn, tuần tự trong đích. Nội dung được gửi bằng tài khoản hiện tại; không giả danh tác giả gốc. Nếu giao thức không có nhãn forward, preview phải nói rõ nội dung sẽ được gửi lại thành tin mới.
- [ ] Không sao chép reply ID, read receipt, reaction, metadata poll/Flex hoặc mention gốc sang chat khác. Văn bản có mention được chuyển thành chữ thường; chỉ giữ LINE emoji khi transport được xác minh, nếu không phải hiển thị bản chuyển đổi trong preview.
- [ ] Khi có lỗi hoặc kết quả chưa rõ, dừng hàng đợi để giữ thứ tự. Cho tiếp tục/bỏ qua theo lựa chọn rõ ràng; không phát lại những tin đã thành công. Hủy chỉ dừng phần chưa chạy.

### M2b: ảnh và tệp

- [ ] Lấy original có kiểm tra account/chat/message, cache ownership và định dạng, sau đó upload bằng đường gửi phù hợp với đích và chính sách E2EE hiện có. Không tái sử dụng khóa hoặc URL xác thực của tin nguồn.
- [ ] Dùng bản sao tạm có giới hạn vòng đời để thao tác đang chạy không mất file khi cache bị dọn; giới hạn dung lượng, memory và lượt tải. Có cơ chế hủy và dọn file tạm.
- [ ] Tin cũ có original hết hạn hiển thị Không còn tệp gốc và không gửi thumbnail thay thế. Nếu nguồn bị thu hồi trước upload, dừng mục đó; phần đã gửi báo đúng trạng thái.
- [ ] Video, audio, sticker và các thẻ đặc biệt là bước sau M2b, mở từng loại khi đã xác minh transport, quyền sở hữu và nhận trên LINE chính thức. Không chuyển sticker thành ảnh mà không báo trước.
- [ ] Chia sẻ ra ngoài ứng dụng: sao chép văn bản hoặc lưu/mở tệp đã đủ dùng cho đợt đầu; share sheet tích hợp desktop là phần tiếp theo sau khảo sát portal.

Nghiệm thu: chuyển tiếp một/nhiều tin đúng đích và thứ tự, mixed types có preview chính xác, chặn đích không hợp lệ, retry không nhân đôi phần đã thành công, hủy/session change chặn phần chưa gửi. Kiểm tra cả tin từ chính mình và người khác.

## M3 — Ghim từng tin bằng LINE announcement

- [ ] Thêm adapter ứng dụng cho list/create/remove announcement, không sửa nguồn vendored trực tiếp. Bắt đầu bằng tin văn bản trong group; giới hạn kiểu chat và loại tin theo kết quả M0.
- [ ] Hiển thị thanh ghim gọn ở đầu cuộc trò chuyện; mở danh sách đầy đủ với nội dung, người ghim, thời gian và thao tác tới tin gốc.
- [ ] Chọn Ghim từ menu từng tin, xem lại nội dung và giải thích ghim được chia sẻ trong cuộc trò chuyện. Bỏ ghim theo `announcementSeq` và quyền server trả về.
- [ ] Nhấn tin ghim dùng ngữ cảnh lịch sử hiện có. Nếu nguồn không còn truy cập được, vẫn hiển thị nội dung announcement do server cung cấp và báo không thể mở nguồn.
- [ ] Không thực thi tùy ý URL từ announcement. Chỉ xử lý mẫu liên kết chat/message đã xác minh, kiểm tra chat nguồn và ID; liên kết khác đi qua quy tắc mở URL an toàn hiện có.
- [ ] Làm mới khi mở chat, cửa sổ lấy lại focus và sau thao tác ghi; coalesce request. Dùng event realtime nếu đã xác minh, không giả định một event riêng luôn được gửi.
- [ ] Đối soát lỗi create/remove chưa rõ bằng danh sách authoritative; nếu không xác định chắc chắn thì giữ trạng thái chưa rõ, không tự gửi lại. Không suy luận thành công chỉ từ text giống nhau.

Nghiệm thu: ghim/bỏ ghim xuất hiện đúng trên thiết bị khác khi đã kiểm tra live, quyền bị từ chối không sửa UI như đã thành công, dữ liệu cũ không tràn sang chat mới. Tính năng này riêng với ghim cuộc trò chuyện cục bộ đang có.

## M4 — Tìm kiếm xuyên mọi cuộc trò chuyện đã lưu

- [ ] Giữ Ctrl+F cho chat hiện tại; thêm Tìm trong mọi cuộc trò chuyện từ giao diện chính và Ctrl+Shift+F sau kiểm tra xung đột. Bộ chọn phạm vi luôn nhìn thấy được.
- [ ] Đợt đầu tìm text và tên file bằng semantics Unicode/literal hiện có. Không gọi đây là tìm toàn bộ lịch sử trên server: hiển thị rõ Chỉ trong lịch sử đã lưu trên máy.
- [ ] Thu thập hợp nhất chat trong memory và file cache hợp lệ, bao gồm chat không còn trong danh sách gần đây. Chỉ nhận dữ liệu có `accountMid` đúng; cache legacy không rõ chủ sở hữu không tự được đưa vào.
- [ ] Đọc/decrypt theo lô nhỏ, có hủy khi đổi query/tài khoản/clear cache. Không tải toàn bộ lịch sử mọi chat vào RAM và không hydrate media hàng loạt chỉ để tìm kiếm.
- [ ] Tái sử dụng tombstone thu hồi và lọc lỗi giải mã/sự kiện/tin pending. Hiển thị phạm vi còn thiếu khi file không đọc được hoặc khóa không có; không báo Không có kết quả như thể toàn bộ dữ liệu đã được tìm.
- [ ] Kết quả gồm tên chat, người gửi, đoạn trích, thời gian và loại tin; 50 mục/trang. Thứ tự ổn định theo thời gian, chat ID và message ID chính xác.
- [ ] Dùng phiên tìm kiếm có cursor opaque và revision. Khi có thay đổi làm kết quả hết hiệu lực, yêu cầu làm mới rõ ràng; không dùng offset trên tập kết quả đang thay đổi để bỏ sót/trùng tin.
- [ ] Không giữ toàn bộ kết quả trong memory: quét có giới hạn và giữ cửa sổ kết quả cần trả; đo thời gian/memory trước khi quyết định bổ sung chỉ mục. Nếu thêm chỉ mục bền vững sau này, phải tuân theo mã hóa và retention của nguồn.
- [ ] Mở kết quả chuyển đúng chat và nhảy tới ngữ cảnh nguồn, không tải latest trước hoặc tự gửi read receipt. Quay lại giữ query, phạm vi, trang và vị trí cuộn.
- [ ] Có thể dùng offline khi đã xác định được tài khoản và đọc được cache; tách điều kiện tìm local khỏi `session_ready`. Không suy ra danh tính tài khoản chỉ từ file cache. Điều chỉnh đường mở ngữ cảnh để không bắt buộc mạng cho tin đã lưu.
- [ ] Dữ liệu tìm kiếm tuân theo giới hạn lịch sử và clear-cache hiện có; trim/thu hồi/xóa cache không để lại nội dung trong phiên tìm kiếm hoặc snippet cũ.

Nghiệm thu: account isolation, Unicode Việt/Nhật/Thái, filename, ID lớn, chat trùng tên, phân trang ổn định, clear/trim/unsend trong lúc quét, offline và lỗi khóa đều có test. Đo với fixture đề xuất 100 chat × 2.000 tin và cache unlimited; ghi peak memory/thời gian, kiểm tra GTK vẫn tương tác được.

## M5 — Trả lời từ thông báo

### M5a: nút Trả lời và hộp soạn gọn

- [ ] Thêm action Trả lời mang token tra cứu context gồm tài khoản/phiên/chat/tin nguồn. Cập nhật notification hoặc fallback GIO không được vô tình đổi context của action cũ.
- [ ] Mở hộp soạn gọn có tên chat và preview tin, không đổi chat đang mở hoặc ghi đè bản nháp composer chính. Chỉ gửi sau thao tác rõ ràng của người dùng.
- [ ] Dùng gửi văn bản chung M0; có trích dẫn tin nguồn khi hợp lệ, xác minh nguồn vẫn thuộc chat. Nếu tin nguồn đã thu hồi, cho biết tình trạng và để người dùng chọn gửi tin thường, không âm thầm đổi ý nghĩa.
- [ ] Kiểm tra phiên, quyền gửi và chặn trước khi gửi. Thông báo cũ từ tài khoản khác/phiên cũ không thể phát tin; chuyển sang đăng nhập hoặc mở app với giải thích phù hợp.
- [ ] Mất mạng giữ nội dung trong hộp soạn; chỉ xếp hàng khi người dùng chọn và outbox giữ được chính xác account/chat/source. Không tự xếp hàng khi chưa chứng minh điều kiện đó.
- [ ] Kết quả thành công/lỗi/chưa rõ hiện ngay trong hộp soạn. Không coi bấm action hoặc nhập chữ là đã đọc; tôn trọng read pause và policy hiện có.

### M5b: nhập trực tiếp trong notification

- [ ] Chỉ bật với backend có khả năng và event nhận text đã xác minh. Thiếu hỗ trợ thì giữ M5a; không quảng cáo nhập trực tiếp trên mọi desktop.
- [ ] Adapter nhận text phải chuyển về GTK main context, tra token và kiểm tra phiên lại. Không đưa nội dung vào command line, shell hoặc log.
- [ ] Có trạng thái phản hồi khi gửi, chống double submit và thu hồi token khi notification đóng/hết hạn/logout. Nếu cần app activation sau restart, chỉ khôi phục context khi có bằng chứng tài khoản/phiên hợp lệ.
- [ ] Kiểm tra desktop thật GNOME/KDE theo ma trận hỗ trợ, notification daemon không có reply, notification bị thay thế và app chạy nền. Ghi riêng kết quả gói native/Flatpak; không coi fixture là bằng chứng tương thích desktop.

Nghiệm thu: đúng chat dù người dùng đang mở chat khác, không mất bản nháp chính, không gửi trùng khi action lặp, không đọc nhầm tin, có fallback hữu dụng và ghi rõ desktop đã xác minh.

## Hợp đồng RPC và vùng thay đổi dự kiến

Các tên bên dưới là RPC nội bộ được đề xuất, chưa phải endpoint LINE hoặc cam kết đã có:

| Nhu cầu | RPC dự kiến | Vùng triển khai |
| --- | --- | --- |
| Chọn tin | Không cần RPC riêng | `message_selection.rs` mới; tích hợp `replies.rs`, `messages.rs`, `virtual_list.rs`, `state.rs` |
| Lưu nhiều tệp | Tái sử dụng download có context; chỉ thêm batch orchestration nếu cần | `downloads.rs`, `shared_media.rs`, helper hàng đợi dùng chung |
| Chuyển tiếp | `preview_forward_messages`, `forward_messages`, `message_operation_status`, `cancel_message_operation` | `message_forward.ts` và `message_forward.rs` mới, helper gửi dùng chung |
| Ghim | `list_message_pins`, `pin_message`, `unpin_message` | `message_pins.ts` và `message_pins.rs` mới, adapter Talk |
| Tìm toàn cục | `search_all_messages`, `cancel_message_search`; giữ RPC tìm/context hiện có | `message_search.ts`, `message_store.ts`, `message_search.rs`, `history.rs` |
| Trả lời thông báo | Tái sử dụng gửi văn bản chung; không cần RPC publish riêng | `desktop_notify.rs`, `notifications.rs`, `tray.rs`, `notification_reply.rs` mới, GIO action routing |

`forward_messages` nhận `operationId`, `sourceChatMid`, danh sách message ID và `destinationChatMid`; payload chuẩn bị được giữ cố định cho operation. Sidecar tự resolve nguồn theo tài khoản, không nhận tùy ý text/path từ UI rồi tin đó là bản gốc. Kết quả/progress mang operation ID, account, các chat liên quan và trạng thái từng mục. Trạng thái chuẩn: `pending`, `sending`, `succeeded`, `failed`, `unknown`, `cancelled`; tin chưa gửi do tác vụ bị dừng vẫn phân biệt với tin đã thất bại.

Cập nhật đồng thời `protocol/src/main.ts`, `src/protocol.rs`, `src/sidecar.rs` và pending/event routing ở `src/ui/{state,events,mod}.rs`. Tránh tiếp tục dồn orchestration vào `main.ts` hoặc `mod.rs`. Những operation có thể đã chạy từ xa giữ semantics M0 kể cả khi RPC hết timeout.

## Kiểm thử và phát hành từng đợt

- [ ] Deno regression theo hành vi: session change trong lúc await, cùng ID khác chat/account, nguồn bị thu hồi, đích bị chặn/rời nhóm, file hết hạn, partial failure, timeout sau commit, hủy và restart. Không khẳng định exactly-once nếu server chưa chứng minh.
- [ ] Rust regression cho state lựa chọn, callback cũ, serializer ID lớn, operation outcome và routing notification. Thêm GTK fixture cô lập cho luồng thực tế; không tạo test chỉ lặp lại implementation.
- [ ] Kiểm tra thủ công bàn phím, focus sau dialog, cửa sổ hẹp, light/dark, bốn ngôn ngữ; chụp ảnh cho thay đổi giao diện. Xác nhận selection toolbar không che dòng đang focus.
- [ ] Với đợt có code: chạy `cargo fmt --all -- --check`, Clippy với `-D warnings`, Rust tests, Deno check task và build/smoke sidecar. Verify dependency khi có thay đổi phụ thuộc.
- [ ] Với dữ liệu/tài khoản thử được cho phép, kiểm tra liên thông LINE chính thức cho chuyển tiếp và ghim, bao gồm mất mạng sau submit. Tài liệu kế hoạch này không tự thực hiện gửi tin hoặc ghim trên dữ liệu thật.
- [ ] Sau mỗi đợt, cập nhật README ba ngôn ngữ, `docs/api-inventory.md`, phần trạng thái trong kế hoạch và giới hạn còn lại; chỉ đánh dấu hoàn thành khi đủ bằng chứng tương ứng.

## Phần nhắn tin mở rộng sau năm đợt

| Tính năng | Việc cần làm trước khi triển khai | Ưu tiên đề xuất |
| --- | --- | --- |
| Nền/theme riêng từng chat | Chốt lưu theo account/chat, độ tương phản, xử lý ảnh local và giữ tùy chọn theme chung | Sau M4, ít phụ thuộc protocol |
| Dịch tin nhắn | Chọn provider local/remote, ngôn ngữ, chi phí và sự đồng ý khi gửi nội dung ra dịch vụ; thiết kế cache và xóa dữ liệu | Sau khi có quyết định sản phẩm/provider |
| Hiệu ứng tin nhắn | Xác minh metadata LINE hoặc chọn rõ hiệu ứng local, giữ reduced motion và hiệu năng | Sau các thao tác nhắn tin chính |
| Keep/Keep Memo | Xác minh dịch vụ và khả năng còn truy cập được trước khi hứa hỗ trợ; bookmark local là phạm vi riêng | Chưa đưa vào lịch triển khai |
| Rich menu/nút/quick reply tài khoản chính thức | Kiểm kê message/action contract và xác minh postback, URL, ownership/context; không suy ra từ Flex rendering | Kế hoạch riêng sau M2 |

Gửi nhiều ảnh cùng lúc thuộc nhóm media; có thể tái sử dụng hàng đợi và tiến trình M2 nhưng tiếp tục thành đợt riêng. Sao lưu/khôi phục lịch sử, tìm sâu toàn bộ lịch sử server, sửa tin nhắn, xóa/thu hồi hàng loạt và đồng bộ bookmark cục bộ không thuộc năm đợt đầu.
