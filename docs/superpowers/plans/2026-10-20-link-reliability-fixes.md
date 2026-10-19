# Link reliability fixes — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans. Each issue: failing test first, then fix.

**Goal:** Fix the reconnection and session-handling issues found in the phase 4d/4e reviews, so links stay correct and cheap on battery before new features build on them.

**Spec:** the GitHub issues listed per task (each states where, the failure scenario and the fix); [`docs/architecture.md`](../../architecture.md) §4, §6.

## Global Constraints
- Battery (§6): no polling, bounded backoff; English only; no Claude attribution; `bash tools/check-clean-room.sh` after `git add`.
- Each commit message references the issues it fixes (`Fixes #N`).

### Task 1: LinkManager (core) — #13, #14, #15, #16
- #15: reset backoff only after a session stayed up for `stableAfter` (default 30 s). Test: `a session failing right after the handshake keeps backing off`, `a stable session resets the backoff`.
- #14: drain a pending wake signal before each wait. Test: `a reconnectNow during one wait does not shorten the next`.
- #13: state writes and `stop()` under one lock; nothing is published after `Stopped`. Test: `no state after stopped` (repeated start/stop).
- #16: tests await on the `onStateChange` history, not on the conflated StateFlow.

### Task 2: phone service — #17, #18
- #17: ACL_CONNECTED reconnects only for the configured Glass. Pure `BluetoothEvents.shouldReconnect(...)`; tests.
- #18: notification says why the bridge is unavailable (no Glass chosen, Bluetooth off, permission, bridge off); bridge mode OFF does not run the bridge loop. `TransportTargets.bridgeUnavailableReason`; tests.

### Task 3: Glass BridgeServer and NavCardState — #20, #21, #22, #24, #25, #26
- #20: `onSessionEnded(Sender)`; a stale end is ignored. Test in NavCardStateTest.
- #21: a Hello completing after `close()` does not start a session. Test.
- #22: a channel reopened while closing is closed again. Test with a fake channel.
- #24: a TCP port that cannot be bound is retried in the background instead of throwing. Test.
- #25: requested-icon marks are cleared on a new session. Test.
- #26: a Hello with an unknown protocol version is rejected. Test.
