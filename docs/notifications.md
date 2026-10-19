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
   - **Dismiss** cancels the notification on the phone;
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

Notification content leaves the phone only over the bridge to the owner's Glass. The relay neither logs nor stores it. The debug capture tool is opt-in and records only notifications from Google Maps and OsmAnd (to study navigation), all of them, not only the navigation ones; delete its file after use.

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
