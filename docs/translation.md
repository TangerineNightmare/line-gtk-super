# Message translation

Line-GTK Super can translate chat messages through any server that implements the
[LibreTranslate](https://github.com/LibreTranslate/LibreTranslate) HTTP API. It was written
for mixed Thai/English group chats, but the languages are configurable.

## Using it

- **One message:** right-click a text message and choose **Translate**. The translation
  appears under the bubble with a `th → en` style label. Choose **Translate** again to hide it.
- **A whole chat:** open **Conversation settings** (⋮ menu, or right-click the chat) and
  switch on **Auto-translate this chat**. Incoming messages from other people are translated
  as they appear, unless they are already in your language. The setting is stored per
  account and per chat in `settings.json` (`auto_translate_chats`).

Translations are cached in memory per message ID, so scrolling away and back doesn't
send them again. Changing translation settings clears the cache.

## Settings

**Settings → Features → Translation**

| Key in `settings.json` | UI label | Default |
| --- | --- | --- |
| `translate_url` | Server URL | `http://127.0.0.1:5000` |
| `translate_api_key` | API key | empty (not sent) |
| `translate_language` | Translate into | `en` |
| `translate_partner_language` | Other language | `th` |
| `auto_translate_chats` | (per chat switch) | none |

Language codes are the ones the server uses (ISO 639-1 such as `en`, `th`, `ja`, `zh`, `vi`).
**Test connection** translates a short sample and reports the result or the server's error.

## How the direction is chosen

1. If the text contains Thai script (U+0E00–U+0E7F), it is sent as `source: "th"` to
   your language. If your language is Thai, it goes to the other language instead.
2. Otherwise it is sent with `source: "auto"` to your language.
3. If the server reports that the text was already in your language:
   - a manual **Translate** makes a second request into the *other language*, so you can
     check how your English reads in Thai;
   - auto-translate leaves the message alone.

## Wire format

```http
POST {translate_url}/translate
Content-Type: application/json

{"q": "...", "source": "th" | "auto", "target": "en", "format": "text", "api_key": "..."}
```

Expected reply: `{"translatedText": "...", "detectedLanguage": {"language": "th", "confidence": 90}}`.
`detectedLanguage` may be missing, or an array as some servers return it. `{"error": "..."}`
is shown to the user.

Requests run on two background threads with a 30-second timeout and a queue of 512, so
auto-translating a busy chat won't freeze the UI or flood a small server. Only the first
auto-translate error in a session is shown as a toast; later ones go to the log under the
`line_gtk::translate` target (`RUST_LOG=line_gtk::translate=debug`).

## Servers

| Server | Notes |
| --- | --- |
| [LibreTranslate](https://github.com/LibreTranslate/LibreTranslate) | Argos models. Light and fast, fine for plain sentences, weak on casual Thai. |
| [LTEngine](https://github.com/LibreTranslate/LTEngine) | Same API, backed by a local LLM (Gemma 3 via llama.cpp). Much better with slang and mixed-language messages. `gemma3-4b` needs ~4 GB RAM and takes about 3–4 s per message on a recent laptop CPU; `gemma3-12b` needs ~8 GB and is slower. A GPU helps a lot. |
| Hosted LibreTranslate (e.g. libretranslate.com) | Needs an API key. Your messages leave your machine. |

LibreTranslate, English and Thai only:

```bash
podman run -d --name libretranslate --restart=unless-stopped \
  -p 127.0.0.1:5000:5000 -v libretranslate-models:/home/libretranslate/.local \
  docker.io/libretranslate/libretranslate:v1.9.6 --load-only en,th
```

LTEngine: see [packaging/ltengine](../packaging/ltengine/README.md).

## Privacy

The configured server receives the full text of every message you translate, including
the decrypted text of Letter Sealing (end-to-end-encrypted) chats. The default URL is
localhost. If you point it at another machine, use one you control, preferably over a
private network, and set an API key.

## Testing

Unit tests cover direction selection and reply parsing. A live test runs against a real
server:

```bash
LINE_GTK_TRANSLATE_TEST_URL=http://127.0.0.1:5000 cargo test -- --ignored translate
```
