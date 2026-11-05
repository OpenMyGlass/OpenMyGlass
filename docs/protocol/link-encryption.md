# Link encryption (OpenMyGlass companion link)

This page describes the OpenMyGlass link, the `omg-protocol` frames, not the MyGlass protocol the rest of this directory documents. The code is `rust/crates/omg-core/src/secure.rs`; `session.rs`, `companion_link.rs` and glassd's `companion/link.rs` drive it, so Android, iOS, the CLI and the Glass share one implementation.

Notes, location, settings and soon Wi-Fi passwords cross this link. Every frame after the handshake is encrypted and authenticated, whatever the transport. **Nothing relies on Bluetooth encryption**: bonding protects at most one path, and legacy pairing does not resist a passive sniffer.

## Sequence

Every step is one frame (`u32 big-endian length || postcard(Frame)`), in both directions at once.

```text
Hello                       capabilities must include "session-crypto-v1"
[PairRequest/Accept/Result] first pairing only
AuthChallenge               32-byte nonce
AuthResponse                ed25519("openmyglass-companion-auth-v1" || peer's nonce)
AuthResult
SessionKeyShare             X25519 ephemeral public key
SessionKeyProof             ed25519("omg-session-proof-v1" || role || T)
Sealed ...                  everything else, and nothing else
```

- **The transcript** `T` is SHA-256 over, each with a length prefix and the companion's first: the domain string `openmyglass-session-transcript-v1`; the fields of both `Hello`s (version, role, app version, device name, identity, capabilities); both challenge nonces; both ed25519 identities; both shares.
- **Keys:** `HKDF-SHA256(salt = T, ikm = X25519(e, e_peer))`, with info `openmyglass-session-key-v1 companion to glass` and `... glass to companion`: one 32-byte key per direction. A low-order peer share, or a copy of our own, is refused.
- **`Sealed`** is `ChaCha20-Poly1305(key, nonce = 0u32 || counter_be64, aad = empty, postcard(Frame))`, with the tag at the end. The counter starts at 0 per direction and is not sent.
- **Failure:** on the encrypted link, a frame that is not `Sealed`, a tag that fails, bytes after the ciphertext, or a sealed frame inside a sealed frame ends the link. A dropped, repeated or reordered frame fails its tag, because the counter no longer matches. The failure is final: later frames are refused too, and nothing is sent back.
- **Size:** `MAX_FRAME_LEN` stays 16 MiB; `MAX_SEALED_PLAINTEXT` is 32 bytes less, and a larger frame is refused before a counter value is used. Sealing adds at most 22 bytes. The largest note chunk (48 KiB) fits, and the worst-case note frames stay within the 64 KiB Bluetooth frame (test in `omg-protocol/src/notes.rs`).

## Changes from the first design, and why

1. **Two frames, not one.** A key "signed over both ephemeral keys" cannot be sent until the peer's key is known, and both sides send at once. So the shares go first and the signatures follow.
2. **The signature covers more.** Also signed: the role of the signer, which stops a proof being sent back to its author; both identities, which stops an unknown-key-share; and both `Hello`s, so a relay cannot change a device name or a capability list, on which behaviour depends.
3. **No challenge signature can pass as a proof.** The challenge exchange signs whatever the peer asks. The proof's context differs from `AUTH_CONTEXT` within its first bytes.
4. **Only one challenge per session.** A second `AuthChallenge` used to be answered, which made a signing oracle. Now it ends the handshake.
5. **`PairRequest` must name the `Hello`'s key.** The Glass did not check, and the companion already did the same check on `PairAccept`.
6. **An implicit counter**, which costs 8 bytes less per frame than a sent one, and makes "counter gap" and "tag failure" the same test. Each transport must therefore deliver bytes in order and reliably: TCP, RFCOMM, and a BLE byte stream with its own reliability layer.
7. **Hedged randomness**, see below.

## Old peers: refused

A peer without `session-crypto-v1` gets `Error{Unsupported}` right after its `Hello`, before any pairing frame, and the link closes. No bond is stored. There is no fallback, so there is nothing to downgrade to: a relay stripping the capability only causes the refusal, which it could cause anyway by cutting the link. Nothing but test images is deployed, so strictness costs nothing today. `PROTOCOL_VERSION` stays 1: the rules in `omg-protocol/src/lib.rs` gate new variants with a capability, and the old peer gets a reason it can show.

`omg-core` adds the capability to whatever list a frontend passes, so no configuration can turn encryption off. `encode` refuses application frames before the keys exist, so a mistake in a frontend can lose a frame but cannot send one in clear.

## What this protects against

| Attacker on the path | Result |
|---|---|
| Reads frames after the handshake | Sees ciphertext and lengths |
| Forges, replays, reorders, drops, or changes a bit | The link ends; the frame is not acted on |
| Replays frames from another session | Other keys: the link ends |
| Replaces the key shares, bonded peers | Both proofs fail (forward secrecy holds: keys are ephemeral) |
| Changes or strips anything in `Hello` | Proofs fail, or the capability refusal |
| Claims the bonded key without holding it | `AuthFailed` at the signature |

## What it does not protect against

- **First pairing is trust on first use.** A MITM present at the first pairing can bond each side to its own key. After that, its key is the pinned one. `pair_token` from the QR is never checked. The bonded case is what the signatures protect.
- **Pairing is open.** `pairing_enabled` defaults to true, so a nearby stranger can bond first to an unbonded Glass. A pairing token or code through the QR is planned. Note that `T` is what a code shown on both screens would be derived from.
- **Metadata:** `Hello` is in clear, so a passive listener learns the device names, capabilities, versions and public identities. Frame lengths and timing are visible; frames are not padded.
- **An unauthenticated close.** A TCP reset or EOF looks like any lost link. Everything that matters is acknowledged at a higher level (note sync resumes), and a `Bye` is sealed.
- **Memory before authentication.** A relay can announce a 16 MiB frame and make the receiver buffer it, as before this change.

## Randomness on the Glass

Kernel 3.4 has no `getrandom(2)`. glassd binds no listener and dials nothing before `/dev/random` is readable (`entropy.rs`, `Rng::Ready`), and a first identity needs more: a seed file or 256 bits. So no handshake runs before the pool is gated. But that gate alone is thin for a later boot that has no seed file, for example after power was lost between using one seed and saving the next: `/dev/random` readable does not mean `/dev/urandom` was seeded.

So ephemeral keys and challenge nonces are hedged, as in RFC 8937: `HKDF(ikm = os_random || identity_seed, info = label, context, clock, counter)`. The identity was made behind the strict gate, so a thin pool alone cannot make these predictable; with a healthy pool they stay independent of the identity, so a stolen identity later does not expose past sessions. If the pool is thin and the identity is stolen, those sessions lose forward secrecy. The key exchange runs only after the peer has passed the signature check, so a stranger cannot make the Glass spend on it.

No timer, thread or wakeup is added. The handshake gains one round trip at connect, and suspend is not involved.

## Cost

Host measure, on an i7-3770 at 3.4 GHz, one thread, release build. **Not measured on the Glass** (OMAP4, about 1 GHz, 32-bit, no NEON path in `chacha20`), where expect several times slower, more for X25519 and ed25519:

- Key exchange: 0.2-0.25 ms per side, on top of the auth signatures.
- Handshake on the wire, bonded: 12 frames, about 820 bytes in total (412 per direction).
- 48 KiB note chunk: 387 µs to seal and open with postcard, 144 µs of which is the postcard work a plain frame also does, so about 120 µs per crypto operation (about 400 MB/s).
- `Ping`: about 1 µs to seal and open.

## Why not Noise XX

A separate BLE design (`docs/plans/2026-11-05-ble-companion-link.md`, branch `ble-link-design`) recommends Noise XX under the frames, with `snow`. It was weighed, and this design was kept for now.

What Noise XX would be better at:

- It is a pattern with published analysis, and `snow` is a reviewed implementation. This handshake is a signed Diffie-Hellman over a whole transcript, the same family as ISO 9798-3, SIGMA and TLS 1.3's `CertificateVerify`, but its composition here has had no external review.
- It hides identities and `Hello` from a passive listener.
- It is three messages and about 200 bytes (the BLE design's estimate), against 12 frames and about 820 bytes. On BLE that is more connection intervals at connect only, not per frame.

What it would cost:

- **Randomness.** With only the needed features, `snow` 0.10 pulls `getrandom` 0.3 and `ring` (checked with `cargo tree` for armv7 musl). That is a second random path past the one the pool gate reasons about, and the hedge would need a custom `CryptoResolver`. `ring` also adds C and assembly to the armv7 musl and Android builds.
- **Frame size.** A Noise message is at most 65535 bytes, and the 16 MiB frame limit was to stay. Note frames fit, so this is a real but moderate cost: split records, or take the keys out of `snow` and keep `Channel`.
- **Shape.** "Before any `Frame`" adds a raw record phase to all three frontends, and an old peer sees garbage rather than a reason. Keeping it after `AuthResult` loses the identity hiding and the short handshake, which are the gains.
- **Keys.** Noise authenticates with X25519 statics; bonds pin ed25519. Either derive statics from the seed and store a new pin (a re-pair, free today), or map ed25519 keys to X25519 and reuse one key across schemes.
- **Toolchain.** `snow` needs Rust 1.85; this workspace declares 1.82.

None of these rule it out, and with nothing deployed a switch stays cheap. The way to switch is contained: `KeyExchange` is the only part replaced, `Channel` and `encode` stay, and a new capability name marks the change. **If BLE ships with a passive-listener threat in mind, identity hiding is the reason to do it**, and that decision is the owner's.
