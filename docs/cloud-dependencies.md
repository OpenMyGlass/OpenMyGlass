# Google cloud dependencies and local replacements

Endpoints were collected from the URL indexes (`_re/index/*.urls.txt`) and traced to the classes that use them.
Status checked on 2026-10-19: `https://glass.google.com/` and `https://glass.google.com/myglass` answer **HTTP 404** — the MyGlass backend is gone.

## Phone side (MyGlass 3.5.8)

| Function | Endpoint / mechanism | Source | Works today? | Local replacement |
|---|---|---|---|---|
| **MyGlass "frontend API"** — Glassware list/search/enable/disable, contacts (add/remove/get, calling/messaging persona), Glass settings get/set, devices list, setup config, user/common config, experiments, factory reset, acknowledgements | `https://glass.google.com/` (prod), `glass.sandbox.google.com/{qa,autopush}/fe/` (test); protos in `com.google.googlex.glass.frontend.api.proto.nano.*Request` | `com/google/glass/net/ServerConstants`, `defpackage/mu` (`myglass_web_frontend_prefix`), `defpackage/mq`, `nb`, `oq`, `ln` | ❌ backend returns 404 | Settings and contacts: send directly over the companion channel or the bridge app. Glassware: local APK sideload through adb or a bridge-app installer. Factory reset: Glass-side settings. |
| OAuth tokens for the above | Android `AccountManager`, scope `https://www.googleapis.com/auth/glass.myglass` | `companion/auth/CompanionAuthUtils` | ⚠️ token obtainable, but the API behind it is gone | Not needed once the frontend API is dropped. |
| Time zone lookup | `https://maps.googleapis.com/maps/api/timezone/json` | phone URL index | ⚠️ needs an API key | Send the phone's own time zone (`Envelope.timezoneC2G`, already supported natively). |
| Analytics | `www.google-analytics.com/collect`, `ssl.google-analytics.com/collect` | phone URL index | irrelevant | Drop. |
| Help, forum, terms | `www.google.com/glass/help`, `glass.google.com/myglass/community_forum`, `…/terms` | strings | ❌ | Drop or link to project docs. |

## Glass side (XE24)

| Function | Endpoint / mechanism | Source | Works today? | Local replacement |
|---|---|---|---|---|
| **Timeline sync** with the Glass cloud (Mirror API cards, sharing, Glassware cards) | Glass frontend (same `ServerConstants`, prefix `https://glass.google.com/`), protos `TimelineSyncNano`, `BatchNano`, `AttachmentSyncNano` | GlassSync, GlassHome | ❌ backend gone | **Companion channel** (Envelope field 5) inserts timeline items locally, no cloud involved. |
| **Push** (cloud → Glass) | GCM: `com.google.android.gcm.GCMBroadcastReceiver`, permission `com.google.glass.home.permission.C2D_MESSAGE` | GlassHome manifest | ❌ nothing to push | Not needed. |
| **Glassware install/updates** | server-side enablement (`GlasswareNano`, `BoutiqueNano`) followed by download through sync | GlassSync, phone frontend API | ❌ | `adb install` (developer mode) or an installer in the bridge app. Without adb and without a bridge app, nothing can be installed. |
| **Native navigation** (turn-by-turn) | separate Glassware `com.google.glass.maps` (not in the XE24 image), started by `google.navigation:` URIs; routing done by Google servers **[unverified: the app is not in the image, so its endpoints were not analyzed]** | GlassHome references, phone `LaunchNavigationActivity` | ❌ on a stock image | Our own navigation card fed from the phone (Google Maps notification parsing, brief's option 1). |
| **Voice recognition** beyond hotwords | `https://www.google.com/m/voice-search/fallback`, scope `…/auth/speech/personalization` | GlassVoice URL index | ⚠️ unknown; likely broken without an account | Dictation on the **phone** (`SpeechRecognizer`), text sent back to Glass. Hotwords ("ok glass") work offline (`GlassVoice` hotword models, `greco_config.binary_proto`). |
| **Connectivity check** | `https://www.google.com/generate_204` | GlassHome/GlassSync | ✅ (works with any Internet link) | None needed. |
| **Google Now / web history / Hangouts** | scopes `…/auth/googlenow`, `…/auth/webhistory`, `…/auth/glass.hangouts`; `history.google.com/history/api/delete` | GlassHome/GlassVoice | ❌ | Out of scope. |
| **Calendar** | `www.google.com/calendar/feeds/…` (GData) | GlassHome/GlassSync | ❌ (GData Calendar shut down) | Optional later: send calendar cards from the phone as timeline items. |
| **Play services / Android Wear** | GmsCore 6.1.11 on Glass, Wear node in MyGlassWear | system image | ⚠️ needs Google Play services and a Wear peer on the phone **[unverified]** | Not relied upon; notifications go through the companion channel or the bridge app. |
| Google account on Glass | — | `GlassLiteSetup` provisions without account (see `protocol/handshake.md`) | ✅ not required | — |

## What is cloud-free already

From the protocol analysis ([`protocol/messages.md`](protocol/messages.md)) these features run entirely between phone and Glass, with no server: handshake, device info and battery, time zone, Wi-Fi setup, location forwarding, screencast and touch injection, timeline items (including SMS), photo-sync notifications (thumbnails), remote keyboard input.

## Not rebuildable as it was

- **Native turn-by-turn navigation** (`com.google.glass.maps`): the app is not in the image and depended on Google routing. Alternative: our navigation card driven by the phone's Google Maps notification.
- **Glassware store**: the server-side catalog is gone. Alternative: sideloading.
- **Server-side voice search** and Google Now cards: no replacement planned; dictation moves to the phone.
