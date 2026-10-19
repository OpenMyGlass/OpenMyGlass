# Phase 5b: Dictated replies with offline transcription — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans. Steps use checkbox (`- [ ]`) syntax.

**Goal:** From a notification on Glass, the owner chooses Reply and speaks. Glass streams the audio to the phone, and the phone transcribes it **offline** (Vosk, Italian and English). Glass shows the transcript for confirmation, and on Send the phone fires the notification's `RemoteInput` with the text.

**Architecture:**
- **Glass (API 19):** `DictationRecorder` records 16 kHz mono PCM16 with `AudioRecord`. When 16 kHz is unavailable it records at 44.1 kHz and downsamples in Java. Audio goes in 100 ms chunks (3.2 KB) over the bridge, off the main thread.
  - It stops on a tap, after 2 s of silence (energy threshold) or at 30 s.
  - `DictationActivity` shows the status ("Speak now", level, then the transcript) and a menu with Send / Retry / Cancel.
- **Phone:**
  - **core:** `DictationSession` is the per-request state machine (start → chunks → end → transcript → confirm/cancel, plus timeouts). `SpeechToText` is the interface: `start(sampleRate)`, `feed(pcm)`, `finish(): String`, with partial results.
  - **app:** `VoskSpeechToText` (`com.alphacephei:vosk-android`, Apache 2.0); a `ModelStore` that downloads `vosk-model-small-it-0.22` / `vosk-model-small-en-us-0.15` once, checks SHA-256 and unpacks into app storage; `ReplySender`, which fills the `RemoteInput` via `RemoteInput.addResultsToIntent` and sends the action's `PendingIntent`.
- **Protocol `openmyglass.v1` (additive):**
  - Glass → phone: `DictationStart{request_id, key, action_index, sample_rate}`, `AudioChunk{request_id, seq, pcm}`, `DictationEnd{request_id, cancelled}`, `ReplyConfirm{request_id, send}`.
  - Phone → Glass: `Transcript{request_id, text, final}`, `ReplyResult{request_id, ok, message}`.

**Spec:** [`docs/architecture.md`](../../architecture.md) §5.3 (dictated reply: Glass records, phone transcribes offline, confirm on Glass, phone fires `RemoteInput`), §6 (audio only while dictating), §10 (BLE-sized chunks); research in local `_re/research/offline-stt.md`.

## Global Constraints
- **Offline transcription only:** no cloud speech service. The one-time model download is the only network use, and it is explicit: the setting shows the size and asks before downloading.
- **Audio** is never written to disk or logged. Transcripts are not logged. Each request is discarded after confirm, cancel or a 60 s timeout.
- **Battery:** the microphone is on only between Reply and stop. No codec on Glass.
- English only; no Claude attribution; clean-room guard after `git add`.

## Review Focus
1. **Link drops mid-dictation:** both sides time out and clean up; nothing is sent; Glass says "Phone not reachable".
2. **Notification removed** (or its reply action gone) between Reply and Send: the phone answers `ReplyResult{ok=false}` and Glass shows it.
3. **No model downloaded yet / language mismatch:** the phone answers at once with a clear message; Glass does not record for nothing.
4. **Silence or noise only:** an empty transcript is not sent; Glass offers Retry.
5. **Concurrent requests** (a second Reply while one is pending): the newest wins and the older one is cancelled.

### Task 1: Protocol + core `DictationSession` + `SpeechToText` interface (fake engine in tests)
### Task 2: Phone: Vosk engine, model store and settings (language, download), `ReplySender`, wiring into `BridgeManager`/`PhoneHub`
### Task 3: Glass: `DictationRecorder` (format negotiation, downsampling, silence stop: unit-tested pure parts), `DictationActivity`, Reply in the menus (Glass and emulator)
### Task 4: End to end on the emulator
- Italian test clip made with macOS `say -v Alice` → 16 kHz WAV.
- The emulator flavor has a debug "Dictate from test clip" input.
- Check: transcript shown, Send fires the test notification's `RemoteInput`, and the phone receives the text.

Then docs (`docs/notifications.md` replies section, progress, hardware checklist: Glass mic quality, 16 kHz support on XE24, latency).
