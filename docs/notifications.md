# Notifications on Glass

Phone notifications appear on Glass. You can read them, dismiss them in sync both ways, and fire their actions from Glass. Dictated replies are plan 5b. Spec: [`architecture.md`](architecture.md) §5.3, §6.

## Path

1. The phone's `OpenMyGlassNotificationListener` reads each notification into a `NotificationItem` (`NotificationMapper`):
   - title: the title, or the conversation title;
   - text: big text, then text, then the newest MessagingStyle message;
   - the app's name and a 48 px icon;
   - the actions, each with a `reply` flag when it has a `RemoteInput`.
2. `NotificationFilter` (core) decides what reaches Glass:
   - **never:** ongoing notifications (navigation, music, downloads, foreground services), group summaries, muted apps, OpenMyGlass's own (except the debug test channel `glass-test`), and empty ones;
   - **low importance** is dropped unless the category is message or call.
3. `NotificationRelay` (core) keeps Glass equal to the filtered set:
   - a notification is sent only when it is new or its content changed; a removal only for one Glass has;
   - the app icon goes once per session;
   - on a new bridge session the whole set is resent, without waking.
4. On Glass, `NotificationState` keeps up to 20 notifications, newest first, and starts empty on each session (the phone resends).
   - `BridgeService` shows the newest one on a LiveCard; tapping opens `NotificationsActivity`: one card per notification, and tapping a card opens a menu with the notification's actions and Dismiss.
   - The `emulator` flavor shows the card with a button bar in place of the menu.
5. Actions chosen on Glass go back as `NotificationAction`:
   - **Dismiss** cancels the notification on the phone. Glass hides the card at once and removes it when the phone confirms; if no confirmation comes within 5 s (the app forbids clearing it), the card comes back;
   - **an action** fires its `PendingIntent`, with background activity start allowed on Android 14+;
   - an action for a notification that is already gone is ignored, and failures are logged.

## Limits sized for Bluetooth (architecture §10)

- Title 200 and text 1,000 code points (truncated with "…", never splitting surrogate pairs).
- App name 60, action titles 40, at most 3 titled actions.
- Icons 48 px, sent once per hash.
- The worst-case frame stays far below the 64 KiB cap (unit test).

## Battery and wake (§6)

- Glass wakes, by bringing the card forward with a 5 s screen wake lock, only for a **new** notification with `wake` set.
- `wake` is set for categories message, call, alarm and reminder, when the owner keeps "Wake Glass for new messages, calls, alarms and reminders" on.
- Updates, resends after a reconnect and other categories never wake.

## Settings (phone → Connection settings)

- Show phone notifications on Glass (turning it off removes them from Glass).
- Wake Glass for new messages, calls, alarms and reminders.
- Mute apps: the apps seen since OpenMyGlass started, one checkbox each.

## Privacy

Notification content leaves the phone only over the bridge to the owner's Glass. The relay neither logs nor stores it.

- **Private and secret notifications:** notifications an app marks private or secret on the lock screen (`VISIBILITY_PRIVATE`/`VISIBILITY_SECRET`) are shown in full on Glass. The display is seen only by the wearer, but anyone looking at the prism up close could read it. Mute those apps if that matters.
- **Debug capture:** the debug capture tool is opt-in and records only notifications from Google Maps and OsmAnd (to study navigation), all of them, not only the navigation ones. Its file contains your destinations and addresses: delete it after use (the debug screen shows its path).

## Bridge messages (`openmyglass.v1`, additive)

| Frame field | Direction | Message |
|---|---|---|
| 6 `notification` | phone → Glass | `NotificationPosted{Notification{key, app_name, title, text, posted_at_ms, icon_sha256?, actions[{index, title, reply}], wake}}` |
| 7 `notification_removed` | phone → Glass | `NotificationRemoved{key}` |
| 8 `notification_action` | Glass → phone | `NotificationAction{key, action_index \| dismiss}` |

## Verified on the emulator (2026-10-20)

- A shell notification and the debug test notification appeared on the Glass emulator card ("1 of 2", app icon, time).
- The reply action was hidden, as planned for 5a.
- "Ping" from Glass fired the action on the phone.
- Dismiss from Glass removed the notification on the phone.
- "Clear all" on the phone removed both from Glass.

## Not verified yet (hardware checklist in `progress.md`)

On XE24, the touchpad's swipes reaching `NotificationsActivity` as D-pad/Tab keys, the options menu rendering, and the wake behavior.

## Dictated replies (plan 5b)

A notification action with a `RemoteInput` shows as **Reply** on Glass.
1. Glass records with the microphone (16 kHz mono PCM16; 44.1 kHz microphones are downsampled in Java) and streams ~100 ms chunks over the bridge. The microphone is on only while recording, which stops:
   - when you tap;
   - after 2 s of silence following speech;
   - after 8 s without speech;
   - at 30 s.
2. The phone transcribes **offline** with [Vosk](https://alphacephei.com/vosk/) (Apache 2.0).
   - The Italian (`vosk-model-small-it-0.22`, 48 MB) or English (`vosk-model-small-en-us-0.15`, 40 MB) model is downloaded once, only when you press "Download speech model" in settings.
   - Its SHA-256 is checked before unpacking.
   - No cloud speech service is used; audio and text are never stored or logged.
3. Glass shows the transcript; tap opens Send / Retry / Cancel (the emulator shows buttons).
4. On Send the phone fills the notification's `RemoteInput` and fires its action, then Glass shows "Sent" or why it failed. Possible failures:
   - the notification is gone;
   - the app refused the reply;
   - the request expired: 60 s after the transcript reached Glass.

Errors reach Glass at once: no model downloaded, nothing heard, phone not reachable. One dictation at a time; a new one cancels the older one.

Bridge messages (Frame fields 9–14):

| Message | Direction |
|---|---|
| `DictationStart{request_id, key, action_index, sample_rate, free}` | Glass → phone |
| `AudioChunk{request_id, seq, pcm}` | Glass → phone |
| `DictationEnd{request_id, cancelled}` | Glass → phone |
| `Transcript{request_id, text, final, error}` | phone → Glass |
| `ReplyConfirm{request_id, send}` | Glass → phone |
| `ReplyResult{request_id, ok, message}` | phone → Glass |

**Free dictation** (`DictationStart.free = true`): speech that is not a notification reply, such as a note or a voice command argument for the Glass apps (glass-services `docs/api.md`, purpose `"free"`).
- `key` and `action_index` are ignored.
- The flow ends with the final `Transcript`: the phone forgets the request at once, Glass sends no `ReplyConfirm`, and no reply is sent. A stray `ReplyConfirm` with `send` gets `ReplyResult{ok: false}`; one without `send` gets no answer.
- Limit: the phone forgets any request 60 s after `DictationStart`, and the expiry is not extended while audio arrives. A free dictation must therefore end (`DictationEnd`) within 60 s; after that the phone sends nothing, so Glass needs its own timeout. Today's Glass recorder stops at 30 s.
- The phone lists the capability `dictation-free` in its bridge Hello. Glass should check it before offering free dictation. An older phone ignores `free`: it still transcribes, but keeps the request waiting for a `ReplyConfirm` until it expires (60 s), and a confirm with `send` would try to reply to an empty notification key and fail.
- The Glass side (setting `free`, skipping the confirm step) is not implemented yet.

**Verified on the emulator (2026-10-20):**
- Model download: Italian model from settings in 12 s, checksum ok.
- Transcription: an Italian sentence (macOS voice "Alice", 3.2 s) was transcribed offline as "arrivo tra dieci minuti ci vediamo davanti al bar", with the transcript on Glass about 5 s after Reply.
- Send: the test notification's `RemoteInput` received the text ("Replied: …" on the phone and on the Glass card).
- With the English model missing, Glass showed "Download the English speech model…" at once.

**Not verified yet:** Glass XE24 microphone quality and 16 kHz support, real latency over RFCOMM, the touchpad flow (hardware checklist).
