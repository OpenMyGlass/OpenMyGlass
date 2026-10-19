# Phase 5a: Notifications on Glass (read, dismiss, actions) — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans. Steps use checkbox (`- [ ]`) syntax.

**Goal:** Phone notifications appear on Glass. Each one can be read, dismissed in sync both ways, and its own actions fired from Glass. Dictated replies come in plan 5b; actions with a `RemoteInput` are marked `reply` so 5b can use them.

**Architecture:**
- **Phone:**
  - `NotificationListenerService` maps each `StatusBarNotification` to a core `NotificationItem`.
  - A pure `NotificationFilter` decides what reaches Glass.
  - `NotificationRelay` (core, like `NavigationRelay`) keeps the active set and sends `NotificationPosted`/`NotificationRemoved` only on change, plus the full set on attach, with app icons once per hash.
  - It receives `NotificationAction` from Glass and calls back the phone (dismiss → `cancelNotification`, action → fire its `PendingIntent`).
  - `BridgeManager` feeds both relays on each session.
- **Glass:**
  - `NotificationState` keeps the active notifications (newest first, capped).
  - A summary LiveCard shows the newest notification and the count.
  - Tapping opens an immersion with one card per notification; tapping a card opens its menu: the notification's actions and Dismiss.
  - The `emulator` flavor shows the same list in an activity, with buttons for the actions.

**Tech Stack:** as phases 4c–4e (Kotlin core/phone, Java API 19 Glass, protobuf-javalite, JUnit).

**Spec:** [`docs/architecture.md`](../../architecture.md) §5.3 (phone listener, cards, actions, Dismiss; A fallback not in this plan), §6 (wake only for notifications the owner allowed), §10 (messages sized for BLE).

## Global Constraints
- Bridge protocol stays `openmyglass.v1`; new fields are additive (Frame oneof 6–8). Frames ≤ 64 KiB: title ≤ 200 chars, text ≤ 1,000 chars, ≤ 3 actions, app icon ≤ 48 px PNG sent once per hash.
- **Privacy:** notification content leaves the phone only over the bridge to the owner's Glass; nothing is logged or written to disk by the relay (the debug capture tool stays opt-in).
- Battery (§6): Glass wakes for a new notification only if the item's `wake` flag is set (owner setting, default **on** for messaging/call categories, off otherwise); updates to an existing notification never wake.
- English only; no Claude attribution; `bash tools/check-clean-room.sh` after `git add`.

## Review Focus
1. **Notification storms and updates** (a chat app updating one notification many times, progress bars): only changed content is sent; ongoing/progress/foreground-service notifications are filtered → Task 1/2 tests.
2. **Removal races:** the user dismisses on the phone while Glass shows the menu, or dismisses on Glass while the phone already removed it: no crash, the action for a missing key is ignored → Task 2 and Task 4 tests.
3. **Reconnect:** after a new bridge session Glass holds exactly the phone's current set (stale ones removed) → Task 2/4 tests.
4. **Huge or odd content:** very long text, emoji, RTL, null title, no actions → truncation and defaults, frame stays under the cap → Task 1 tests.
5. **Action PendingIntent cancelled** or the app blocks background starts: no crash, Glass gets the item removed or unchanged → Task 3.

---

### Task 1: Protocol and core model + filter
**Files:** `protocol/openmyglass/v1/bridge.proto` (add `NotificationPosted notification = 6`, `NotificationRemoved notification_removed = 7`, `NotificationAction notification_action = 8`; messages `Notification{key, app_name, title, text, posted_at_ms, optional icon_sha256, repeated Action actions, bool wake}`, `Action{index, title, bool reply}`, `NotificationRemoved{key}`, `NotificationAction{key, oneof {uint32 action_index; bool dismiss}}`); `core/.../notif/NotificationItem.kt`, `NotificationFilter.kt`; tests.
- `NotificationItem(key, packageName, appName, title, text, postedAt, category, ongoing, isGroupSummary, priority, actions: List<ItemAction(title, reply)>, iconSha256)`; `fun toProto(wake)` with truncation (title 200, text 1,000 chars on code-point boundaries, ≤ 3 actions).
- `NotificationFilter(ownPackage, mutedPackages, wakeCategories = {msg, call, email? no: msg, call, alarm, reminder})`: `fun accept(item): Boolean` (rejects ongoing, group summaries, own package, muted apps, empty title+text, navigation apps' ongoing notification, `priority < DEFAULT` unless category msg/call); `fun wake(item)`.
- [ ] Tests: `ongoing and group summaries are filtered`, `muted apps are filtered`, `own app filtered`, `low priority filtered unless messaging`, `wake only for configured categories`, `long text truncated on code points`, `at most three actions`, `frame for worst-case item stays under 64 KiB`. RED → GREEN → commit.

### Task 2: NotificationRelay and BridgeManager dispatch
**Files:** `core/.../notif/NotificationRelay.kt`, modify `BridgeManager.kt` (attach both relays; route `NotificationAction` frames and icon requests); tests `NotificationRelayTest.kt`, extend `BridgeManagerTest`.
- `NotificationRelay(filter, icons, onAction: suspend (key, ActionRequest) -> Unit, scope)`: `fun posted(item)`, `fun removed(key)`, `suspend fun attach(onFailure, send): Session` (sends the whole current set), `detach`, `suspend fun onAction(NotificationAction)` (unknown key → ignored).
- [ ] Tests: `posted sends once and only on change`, `removed sends removal for shown items only`, `filtered items never sent and their removal is silent`, `attach sends the full current set`, `action for an unknown key is ignored`, `action and dismiss reach the callback`, `app icon sent once per session before the first notification using it`. BridgeManager: `notification actions from glass reach the relay`. RED → GREEN → commit.

### Task 3: Phone listener, actions and settings
**Files:** `android/.../notifications/NotificationMapper.kt` (pure-ish: extras → `NotificationItem`; title from `EXTRA_TITLE`/`EXTRA_CONVERSATION_TITLE`, text from `EXTRA_BIG_TEXT` ?: `EXTRA_TEXT` ?: last `EXTRA_MESSAGES`), `AppIcons.kt` (48 px PNG + SHA-256), `NotificationActions.kt` (dismiss → `cancelNotification(key)`, action → `PendingIntent.send` with `ActivityOptions` background-start allowed where needed; failures caught), modify `OpenMyGlassNotificationListener.kt` (feed relay; keep nav + capture), `NavigationHub` → `PhoneHub` holds both relays, `Settings` (notifications on/off, muted apps, wake setting), `SettingsActivity` (toggle + muted-app list of apps seen this session), `OpenMyGlassService` (pass the notification relay to `BridgeManager`); tests `NotificationMapperTest` (fake extras via a small adapter interface, no Robolectric).
- [ ] Tests: mapping of messaging-style, big-text, missing title, actions with RemoteInput → `reply = true`. Build, lint, commit.

### Task 4: Glass notification state and renderers
**Files:** `glass-xe/.../notif/NotificationState.java` (implements the frame handling for notifications; newest first, cap 20; listener `onChanged(List<Item>)`, `onNew(Item, wake)`; `requestAction(key, index)`, `dismiss(key)` send `NotificationAction` via the session's Sender; unknown key ignored); `BridgeServer` handler composition (`FrameHandler` fan-out to nav and notification state); glass flavor: `NotificationsLiveCardService` part of `NavLiveCardService` → rename to `BridgeService` owning both (summary LiveCard `notif_card.xml`, wake 5 s for `wake` items), `NotificationsActivity` (CardScrollView of cards; tap → menu with actions + Dismiss); emulator flavor: notifications list under the nav card with action buttons; tests `NotificationStateTest`.
- [ ] Tests: `posted adds newest first`, `update replaces in place without onNew`, `removed removes`, `new session replaces the whole set` (Review Focus 3), `cap 20 drops the oldest`, `action on a removed key is not sent` (Review Focus 2), `dismiss sends and removes locally`. RED → GREEN; both GDK builds → commit.

### Task 5: End to end and docs
- [ ] Emulator: debug button "Post test notification" (own channel `glass-test`, allowed by the filter for testing; one action + one RemoteInput action); a message from `adb shell cmd notification post`; check card, Dismiss from Glass removes it on the phone, phone dismissal removes it on Glass, action fires (debug toast/log).
- [ ] Docs: `docs/notifications.md` (filter rules, privacy, settings, protocol messages), `docs/progress.md` (status; hardware checklist: GDK immersion + menu on XE24, wake per notification, battery with a chatty group), `docs/protocol` note for the new bridge messages.
- [ ] Commit `docs: notifications on Glass`.
