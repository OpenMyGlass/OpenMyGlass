# Everyday companion link over Bluetooth LE

Date: 2026-11-05. Status: design and feasibility verdict. Nothing here has run on a Glass.

Wi-Fi stays for big transfers. BLE carries the everyday traffic, because it also works on iPhone, where Bluetooth Classic RFCOMM needs MFi.

Each claim carries one label:

- **[Source]**: I read it in code or a file in this workspace. Kernel paths are in the 3.4.113 tree (`~/.cache/omg/ksrc`), glassd paths in glass-services at 42b3814, others in this repo at 523d7e5.
- **[Measured]**: an earlier run on this Glass, recorded in the project logs.
- **[Public]**: a document or code outside the project, listed at the end.
- **[Inferred]**: my reasoning, or what I remember, not checked further. It may be wrong.

## 1. Verdict

**Feasible, and worth starting, but not yet proven on this Glass.** Nothing in the source blocks it. Two facts can only be measured, and the first two experiments cost little:

| Question | State | Settled by |
|---|---|---|
| Does the chip's LE controller do advertising and the slave role? | Very likely | E1, E2 |
| Does CORE OFF still happen with the port open and the chip on? | **Open, the largest risk** | E5 |
| Does a BLE event wake the host from CORE OFF, without losing bytes? | Likely, the wiring exists | E6 |
| Is the throughput enough for everyday traffic? | Yes for text, no for live audio | E7 |

**Recommendation:** do not use BlueZ. Write a small Rust BLE peripheral that owns `/dev/ttyO1` directly in raw mode, loads the firmware patch itself, and bypasses the kernel's Bluetooth stack. No kernel change is needed for the first version.

**The session is authenticated, not encrypted.** This applies to Wi-Fi today as well. Before BLE goes beyond test benches, it needs an encrypted channel and an owner-approved first pairing (section 4). Today any device in radio range could pair with an unbonded Glass.

## 2. The chip and the host stack

### 2.1 Does BCM4330 do the BLE peripheral role?

Not proven directly, but several things point the same way:

- **[Source]** The patch file names itself `USI BCM4330B1 37.4 MHz Class 1.5 ExtLNA WLCSP`. It parses cleanly as 575 `0xFC4C` Write RAM records followed by one `0xFC4E` Launch RAM.
- **[Source]** The stock XRH35 `etc/permissions/hardware.xml` declares `android.hardware.bluetooth_le`. Stock `bluetooth.default.so` contains `BTA_GATTS_*`, though that is probably in every bluedroid build.
- **[Public]** The Broadcom data sheet says Bluetooth 4.0 + HS. Mainline Linux knows the part (`BCM4330B1` in `btbcm.c`, subversion 0x4103, and `brcm,bcm4330-bt`). The iPhone 4S used BCM4330.
- **[Inferred]** The iPhone 4S ran `CBPeripheralManager`, so the silicon can be a peripheral. Apple's firmware is not ours, so this shows the silicon only. The slave role is what a 4.0 LE dual-mode controller is expected to offer.

E1 settles it by reading the controller's own answers: LMP feature bit, `LE Read Local Supported Features`, `LE Read Supported States`, `LE Read Buffer Size`. Expect a 4.0 link layer: 27-byte PDUs, 1M PHY, no Data Length Extension, no 2M **[Inferred]**.

**[Public, Inferred]** Mainline `btbcm.c` treats `43:30:B1:00:00:00` as a known BCM4330B1 address, so this chip likely has no unique public address of its own. The design uses a static random address, generated once and stored on the Glass. That avoids inventing a public address and needs nothing from the factory data.

### 2.2 Kernel 3.4's stack

What is there, **[Source]**:

- `CONFIG_BT=y`, `CONFIG_BT_HCIUART=y`, `CONFIG_BT_HCIUART_H4=y`; no BCSP and no VHCI (`.config:963-983`).
- An incoming LE connection is accepted: `hci_le_conn_complete_evt` (`hci_event.c:3297`) adds the connection and `l2cap_le_conn_ready` (`l2cap_core.c:871`) hands it to a socket listening on CID 4. So the kernel could serve ATT through an `AF_BLUETOOTH` L2CAP socket.
- SMP exists (`smp.c`), legacy pairing only.
- LE support is off until `bluetooth.enable_le=1` is on the command line (`mgmt.c:38`, `:1182`). It is not set today.

What is missing, **[Source]**:

- **No advertising.** `hci.h` defines no LE Set Advertising command, and nothing in `net/bluetooth` starts advertising. It would have to be sent as raw HCI.
- **No way to ask for connection parameters.** `L2CAP_CONN_PARAM_UPDATE_REQ` appears only in the receive dispatch (`l2cap_core.c:3615`), never sent. On a 4.0 link this request is the only way a peripheral can ask for a longer interval, so an idle link would stay at whatever the phone chose. This matters more for power than anything else on this list.
- **mgmt is version 1.0** (`mgmt.c:40-41`), opcodes up to `0x0027`. Unknown opcodes are refused (`mgmt.c:2696`).
- `sockaddr_l2` has no `l2_bdaddr_type` (`l2cap.h:55`).
- `HCI_CHANNEL_USER` does not exist (`hci.h:1332-1334`). A raw socket shares the device with the kernel and cannot take it over.

### 2.3 BlueZ against this kernel

Alpine 3.24 ships BlueZ 5.86-r2 **[Public]**. `hciattach`, `hciconfig` and `hcitool` are in `bluez-deprecated` **[Public, APKBUILD]**.

- **[Public]** `bluetoothd` 5.86 only requires mgmt 1.0 (`src/adapter.c`, "Version 1.0 or later"), so it would probably start.
- **[Public, Inferred]** `LEAdvertisingManager1` depends on `MGMT_OP_READ_ADV_FEATURES` and `MGMT_OP_ADD_ADVERTISING` (`src/advertising.c`), which this kernel does not have. D-Bus advertising would fail.
- **[Inferred]** A workaround exists for advertising: `hciconfig leadv` or raw commands. There is none for the parameter request above.
- Also missing or awkward: LE privacy, and any feature gated above 1.0, plus D-Bus and GLib running on the Glass. Mesh's kernel crypto options are unset (`CONFIG_CRYPTO_USER_API_*`), but only Mesh needs them according to the BlueZ README.

No Alpine BlueZ gives a working peripheral here without kernel backports. I did not run it: this conclusion comes from reading both sources.

### 2.4 Options

| Option | For | Against |
|---|---|---|
| **Own minimal GATT in Rust, raw tty** | Fits glassd's pure state-machine style; no runtime; the bring-up code is needed anyway; everything testable on the host | We own the spec corners and phone interop |
| TrouBLE (Rust) | Peripheral, GATT server, notifications, L2CAP CoC; `serial-hci` example on std; Apache-2.0 or MIT **[Public]** | Embedded and async; glassd and `omg-core` deliberately have no async runtime; vendor bring-up still ours |
| NimBLE host | Mature, Apache-2.0 **[Inferred]** | C in an all-Rust daemon, with a port layer to write |
| BTstack | Mature | **Rejected:** licence clause 4 forbids commercial use **[Public]**, which does not fit GPL-3.0 or Apache-2.0 |
| Bumble | Excellent test peer | Python, too heavy to run on the Glass, wakes cost more |
| BlueZ + kernel L2CAP | Standard | See 2.3 |
| Raw socket beside the kernel | Kernel keeps Classic (A2DP, RFCOMM) | See 2.6 |

**Recommended: own minimal stack.** A reasonable reviewer could choose TrouBLE, and it is the fallback if interop eats too much time. My estimate is about 1.5k lines plus tests **[Inferred]**: H4 framing, HCI commands and events, patch loader, ACL fragmentation and reassembly, L2CAP on CIDs 4, 5 and 6, and an ATT server.

ATT needs Exchange MTU, Read By Group Type, Find By Type Value, Read By Type, Find Information, Read, Write, Write Command and Handle Value Notification. CID 6 only answers SMP with Pairing Not Supported in the first version. Bumble should be the conformance peer on the PC, over its virtual controller, without hardware **[Inferred, check first]**.

### 2.5 Why a raw tty works here

**[Source]** The low-power wiring lives below the line discipline, not in `hci_uart`:

- Every UART start calls `wake_peer` (`serial_core.c:97`), which is `bcm_bt_lpm_exit_lpm_locked` for ttyO1 (`board-notle.c:1195`). It raises BT_WAKE and holds wakelock `bt_bcm_wake` until 1 s after the last transmission.
- The HOST_WAKE interrupt enables the port and holds `bt_host_wake` (`board-notle-bluetooth.c:185`, `:210`).
- rfkill handles REG_ON, reset and the 32 kHz clock (`board-notle-bluetooth.c:83`).

So a raw N_TTY reader gets the same power handling as `hci_uart` **[Inferred from that wiring; E4 checks it]**. The kernel never sees `hci0`, and its Bluetooth code stays idle.

### 2.6 What this gives up: Classic at the same time

If our engine owns the tty, **the kernel's RFCOMM and any A2DP or HFP stack cannot run at the same time.** `transport.rs` and `bt.rs` assume Bluetooth will be up for headphones anyway, so this is a real choice for the owner.

The way back is to keep the HCI link behind a small interface in the engine. Later it can attach to the kernel instead: set `N_HCI`, bring up `hci0`, bind `HCI_CHANNEL_RAW`, and send LE ACL through the raw queue (`hci_sock.c:873`). The kernel would see the LE link but do nothing with CID 4 when no socket listens (`l2cap_core.c:4457`). It would still claim SMP on CID 6 (`:4515`). Whether LE and Classic coexist well that way is **[Inferred]**, and it does not need solving now.

### 2.7 Loading the firmware patch: our own code

Use our own HCI reset and patchram, not `hciattach`. We write HCI anyway, it avoids a package from `bluez-deprecated`, and it gives exact control of the LPM command.

Sequence, from libbt at the tag Glass used (`hardware.c`, android-4.4.4_r2.0.1) **[Public; the Glass manifest pins that tag: Source]**:

1. `rfkill unblock bluetooth`, then open `/dev/ttyO1` raw at 115200 with RTS/CTS.
2. `HCI_Reset` `0x0C03`.
3. `Update Baudrate` `0xFC18`, then change the host's baud. libbt uses 3 Mbaud. Optional: at 115200 the 56.7 KB patch takes about 5 s, and BLE itself would fit within 115200. Start without it and add it if boot time matters.
4. `Read Local Name` `0x0C14`, logged as a check.
5. `Download Minidriver` `0xFC2E`, then wait 50 ms.
6. Send the HCD records one at a time, waiting for each Command Complete.
7. After `Launch RAM` `0xFC4E`, the host goes back to 115200, because the patched firmware restarts at that speed. Then wait for the settling delay.
8. `HCI_Reset` again, baud again if used.
9. `Write Sleep Mode` `0xFC27`, 12 bytes (section 6.3).
10. LE setup: event mask, LE event mask, `LE Set Random Address`, advertising data and parameters, enable.

**[Source]** The patch is a Google/vendor blob at `model-a-kitkat-salvage/vendor-glass-1/proprietary/vendor/firmware/bcm4330.hcd`. It stays out of git and out of anything published; test images are local. E1 also checks whether the unpatched ROM does LE at all, but the patch name suggests it also carries this board's clock and RF setup, so do not expect to drop it **[Inferred]**.

## 3. The link over GATT

### 3.1 The service

Own UUIDs, in the same style as the RFCOMM one. The bytes spell `omg-ble-link-v1` plus an index:

| Item | UUID | Properties |
|---|---|---|
| Service | `6f6d672d-626c-652d-6c69-6e6b2d763100` | |
| `rx`, phone to Glass | `...763101` | Write Without Response |
| `tx`, Glass to phone | `...763102` | Notify, with CCCD |

Plus the usual GAP service. Advertising data has flags and the 128-bit service UUID; the name goes in the scan response. Keep the handle table fixed, because phones may cache it **[Inferred]**.

### 3.2 A byte stream, nothing more

**[Source]** The wire is `u32 BE length || postcard(Frame)`, parsed by `FrameDecoder`, which already handles splitting across reads (`wire.rs`, tested byte by byte). A decode error is final by design.

So GATT carries the raw byte stream with **no per-packet header**. Each write or notification is the next run of bytes. Everything above stays the same, and the protocol cannot tell which radio it is on.

- **Ordering and loss.** One ATT bearer on one LE link is in order, and the link layer retransmits. The ATT layer has no acknowledgement for these two PDU types, so loss can only come from a host dropping data **[Inferred]**: typically an iOS write without checking `canSendWriteWithoutResponse`. The encrypted channel in section 4 detects any such loss. The answer is to drop the link and reconnect. Notes sync and settings already resume from disk **[Source: `companion-notes-sync.md`]**.
- **MTU.** The Glass answers Exchange MTU with at most 247. Notification payload is the negotiated MTU minus 3. Over 27-byte PDUs this wastes little: about 24.4 useful bytes per PDU at MTU 247, against 20 at MTU 23. Before the exchange the phone must assume 23. Android has to call `requestMtu`; iOS negotiates by itself **[Inferred]**.
- **Flow control, Glass to phone.** Send only with HCI ACL credits (Number of Completed Packets).
- **Flow control, phone to Glass.** If the engine stops reading, the tty's RTS and the controller push back to the phone's link layer **[Inferred; E7 checks it]**. HCI events share the tty, so the engine must keep draining it into a bounded buffer, and buffer stall only applies to the session socket.
- **Lifetime.** The session starts when the phone subscribes to `tx`. Glass output waits until then. The session ends with the BLE connection, so link supervision replaces any heartbeat.

### 3.3 Fitting behind the transports

There is no single transport trait to implement. **[Source]** There are three seams:

1. **glassd**: `Stream` and `Transport` in `companion/transport.rs`. Add `Transport::Ble` and a Unix listener, and a `Ble` arm in `notes::profile_for`.
2. **Phone apps**: `ByteLink` in `LinkController.kt`; a Swift twin later. The core only sees `on_bytes` and `pump`.
3. **omg-transport**: `TransportKind::Ble` is already ranked in `manager.rs`, but only `omg-cli` uses that crate. A PC BLE client is optional and can wait.

**Engine placement: a separate small process, `omg-bled`.** It owns the tty. For each BLE session it connects to a Unix socket that glassd listens on, and glassd treats that like an accepted TCP connection. The patch load takes seconds and must not stall the UI loop, a crash must not take the UI down, and the same binary serves the experiments. `Radio::set_bluetooth` (`radio.rs`) starts and stops it, replacing `hciconfig` and `bluetoothctl`, which are not in the image. Stop the engine before `rfkill block`; the `kick_bt_uart` byte can stay.

**[Inferred]** The session socket needs a small `SO_SNDBUF`. A default Unix buffer of around 160 KB would be close to a minute of queue at BLE speed, in front of every notification.

### 3.4 Throughput

Arithmetic, not measurement **[Inferred]**: rate is PDUs per connection event × 24.4 bytes ÷ interval.

| Interval | 2 PDUs | 4 PDUs | 6 PDUs |
|---|---|---|---|
| 15 ms | 3.3 kB/s | 6.5 kB/s | 9.8 kB/s |
| 30 ms | 1.6 kB/s | 3.3 kB/s | 4.9 kB/s |
| 300 ms | 0.16 kB/s | 0.33 kB/s | 0.49 kB/s |

PDUs per event is the unknown, and E7 measures it. **Plan on 3 kB/s while busy.** What follows from that:

| Traffic | Fit |
|---|---|
| Handshake, settings, notifications, nav updates, battery | Yes |
| Note text | Yes |
| Short voice notes | Slowly: 256 KiB takes about 90 s |
| Photos, video, OTA | No, use Wi-Fi |
| **Live dictation audio** | **No.** `AudioChunk` is raw 16-bit PCM (`messages.rs:811`), which is 32 kB/s at 16 kHz, ten times the budget. Those frames are also not handled in glassd yet (`link.rs:553`) |

Changes needed in code **[Source for the current values]**:

- `LinkProfile::constrained()` is 4 KiB × 2 chunks, with a 16 MiB auto-sync limit. At 3 kB/s, 16 MiB is about 90 minutes of radio time. Use 1 KiB × 2 and **256 KiB**, so a waiting notification sees under a second of delay and anything larger waits for Wi-Fi.
- glassd's `Conn.out` is one FIFO. With the smaller chunks that is good enough; do not add priorities yet.

### 3.5 Moving to Wi-Fi

**[Source]** A gap: `WifiBringUpResult` carries the SSID and IP, and glassd never sends it. `E::WifiBringUp` only sets the Wi-Fi setting and reports radio state (`daemon.rs:1842`).

**[Source]** A second connected session closes the first (`companion/mod.rs:758`). That is acceptable: the phone asks over BLE, gets the IP, dials TCP, BLE closes, the transfer runs with all traffic on Wi-Fi, the phone finishes, Wi-Fi tears down, BLE reconnects. No concurrent links, no new state. Both devices must share a network, which is the phone's hotspot today, 2.4 GHz only **[Measured: wifi29, wifi30]**. On iOS, hotspot control from an app is not available **[Inferred]**, which makes Wi-Fi transfers from an iPhone a real limit to test.

## 4. Security: read this one

### 4.1 What exists today

**[Source: `omg-core/src/identity.rs`, `session.rs`; glassd `companion/link.rs`, `identity.rs`]**

- Each side has an ed25519 key. After a first pairing each stores the other's public key. Each session each side signs `"openmyglass-companion-auth-v1" || nonce`.
- **There is no encryption.** A search of the Rust crates finds no cipher, key agreement or TLS. Every frame after authentication travels in clear: notifications, notes, transcripts, settings. This is true on Wi-Fi TCP today.
- **Authentication is not bound to the channel.** The signature covers only a nonce, not the Hellos, and no key comes out of the exchange. After the handshake, an attacker on the path can inject or alter frames, including `Unpair`, `SetRadio` and `SettingsSet`. Nothing after the handshake is authenticated.
- **First pairing has no check.** `pairing_enabled` defaults to true and the daemon never calls `Companion::set_pairing_enabled`. `bootstrap_token` is not checked on the Glass. An unbonded Glass bonds to the first peer that asks.

On a private hotspot that was tolerable for tests. On BLE the Glass announces itself to the room. A stranger can bond first and lock the owner out ("already bonded to another companion").

### 4.2 Why not rely on BLE bonding

- **[Source]** The kernel's SMP is legacy pairing, and a 4.0 controller suggests legacy on our side too **[Inferred]**. Legacy Just Works gives no protection against a passive sniffer who sees pairing, and passkey entry is brute-forceable from a capture **[Public knowledge; Inferred]**.
- Bonding protects only the BLE path. Wi-Fi would stay in clear.
- It adds key storage and a large interop surface, and it cannot show the owner which device is on the other end.

### 4.3 Recommendation: an encrypted channel under the frames, on every transport

- **Noise XX** (`snow` crate: X25519, ChaCha20-Poly1305, Apache-2.0 or MIT **[Inferred, check licence and version]**), run before any `Frame`.
- Static Noise keys, derived from the same stored seed, replace the ed25519 challenge frames. That removes a custom protocol rather than adding one, and the handshake gets shorter: three messages, around 200 bytes **[Inferred]**, instead of the current auth exchange.
- Records under `FrameDecoder`: `u16 length || ciphertext`, 16-byte tag, implicit nonce, at most 1 KiB of plaintext on BLE. A lost or reordered byte fails the tag and closes the link.
- **First pairing:** only in a pairing mode the owner enters on the Glass. Both screens show a 6-digit code derived from the handshake hash, and the owner confirms. This is numeric comparison at the application level, with a real display on both ends.
- `PROTOCOL_VERSION` to 2. The code rejects a mismatch (`session.rs`), so both ends move together. Nothing is deployed but test units, and old test bonds get dropped.
- Handshake deadline: drop a BLE peer that has not finished within 10 s, and back off after repeated failures. One stranger connected means no advertising, so without this a stranger can block the owner and burn battery through repeated wakes.
- Parsers fuzzed, because they now face anyone in radio range.

This is a separate PR, done first or alongside, and experiments do not depend on it.

### 4.4 What stays exposed

**[Inferred]** Sniffers can still see timing and sizes. A fixed static random address plus our UUID makes the Glass trackable. A rotating advertisement is possible later and is not in the first version.

### 4.5 ANCS: the one thing that may need SMP

iOS gives apps no access to other apps' notifications. **The only route for iPhone notifications on the Glass is Apple's ANCS**, where the Glass is a GATT *client* of the phone, and which as I remember it needs an encrypted, bonded link **[Inferred; I could not confirm this from a current Apple source today]**. `docs/notifications.md` does not mention iOS.

So iPhone notifications would need a GATT client and SMP in our engine, on top of this design. The design does not block that, but it is more work, and the owner should know that "iPhone support" does not yet include this. Host-side SMP is added in Rust as a later layer.

## 5. Phones

Both apps keep the Rust core plus a thin native layer. The core needs no radio-specific code, only the profile changes in 3.4. **[Source]** `omg-ffi` already says the app owns the socket, and framing lives in Rust.

### 5.1 Android (test phone Redmi Note 8T, `minSdk 29`, `targetSdk 36`)

Details below are **[Inferred]**, from general platform knowledge, unless noted. E3 and E9 test them.

- `BleLink : ByteLink` with a blocking read queue fed by `onCharacteristicChanged`, and one GATT operation at a time on write.
- `connectGatt(TRANSPORT_LE, autoConnect = true)` for the reconnect. Android may lose the address type of a random address, and `getRemoteLeDevice` needs API 33. So after a Bluetooth toggle or reboot, do a filtered scan first.
- `requestMtu(247)`. `requestConnectionPriority(HIGH)` during a transfer, then back to `BALANCED`.
- Foreground service of type `connectedDevice`, needed for anything in the background; `companion-mobile.md` lists this as missing **[Source]**. `CompanionDeviceManager` for presence and exemptions. `BLUETOOTH_SCAN` and `BLUETOOTH_CONNECT` from API 31.
- OEM battery managers can still kill it. The owner's phone runs crDroid, which is a friendlier case than most.

### 5.2 iOS

- `CBCentralManager` with a restore identifier and the `bluetooth-central` background mode. A pending `connect()` has no timeout and completes when the Glass comes back, so that is the reconnect strategy **[Inferred]**.
- Background scanning requires naming the service UUID **[Inferred]**.
- Write with `.withoutResponse`, checking `canSendWriteWithoutResponse` and waiting for `peripheralIsReady`. Size with `maximumWriteValueLength`.
- **Force quit is a hard limit.** State restoration does not relaunch a force-quit app, except through AccessorySetupKit on iOS 26 and later **[Public: Apple DTS, forum]**. Nor after a Bluetooth off and on **[Public, unofficial]**.
- The app gets only seconds of background time per event **[Inferred]**. The 250 ms pump timer in `companion-mobile.md` will not run while suspended, so the pump must be driven from the delegate callbacks. Long transfers stop and resume, and the durable sync already allows that.
- The peripheral can only ask for parameters. Apple's rules **[Public, QA1931]**: interval min at least 15 ms in multiples of 15; min + 15 ≤ max; max × (latency + 1) ≤ 2 s; max × (latency + 1) × 3 < supervision timeout; latency ≤ 30; timeout 2 to 6 s. Section 6.2 complies.
- Apple's advertising advice **[Public, QA1931]**: 20 ms for 30 s, then exactly one of 152.5, 211.25, 318.75, 417.5, 546.25, 760, 852.5, 1022.5 or 1285 ms.

### 5.3 What needs the owner's Mac or Apple account

| Task | Mac | Apple account |
|---|---|---|
| Rust core: already compile-checks for `aarch64-apple-ios` **[Source]**, can run on Linux | No | No |
| Generate Swift bindings **[Source]** | No | No |
| Write `BleLink.swift` and a first screen | Xcode to build and test | No |
| Run on an iPhone | Yes. BLE does not work in the Simulator **[Inferred]** | Free Apple ID, with 7-day signing **[Inferred]** |
| Background mode on a free account | Yes | **Check it, I could not verify it** |
| TestFlight, or for others to install | Yes | Paid programme |
| App Store | Yes | Paid; also check GPL-3.0 and App Store terms, a known conflict **[Inferred]**, not legal advice |

No Swift app exists yet **[Source]**. CI on a hosted macOS runner can compile it but cannot test BLE.

## 6. Power

### 6.1 Does a BT event wake the host from CORE OFF?

**The wiring is there. [Source, `arch/arm/mach-omap2/`]**

- This Glass is `V1.5 PROTO(5)` **[Measured: kmsg]**, so it uses the `evt2` tables (`board-notle.c:404-407`). BT_HOST_WAKE is GPIO 2, pad `SIM_RESET`, in the wakeup domain, muxed as `OMAP_PIN_INPUT | OMAP_WAKEUP_EN` (`board-notle.c:2015`).
- The interrupt is armed as wake: `irq_set_irq_wake(irq, 1)` (`board-notle-bluetooth.c:267`).
- Debug names exist: `gpio_wk2` / `BT_HOST_WAKE` (`board-notle-io.c:215`).
- **[Measured]** The sibling pad in the same domain, `gpio_wk0` / `WLAN_HOST_WAKE`, did wake this Glass from suspend (`out/kmsg_slink34.txt`, "At Resume pending wakeup I/O pad"). This is the closest evidence, not proof.
- **The UART RX pad is not a wake source**: it has no `OMAP_DEVICE_PAD_WAKEUP` (`board-notle.c:1114`), unlike the console's. Only HOST_WAKE can wake us, so the chip's sleep mode must be on, or data will wait forever.

**No bytes should be lost. [Source]** ttyO1 has `rts_mux_driver_control = 1`. On suspend the RTS pad gets a pull-up (`omap-serial.c:1275`, flow off), and it is released in `set_termios` on resume (`:946`). This needs the port to be open across suspend, so `uart_resume_port` runs `set_termios`. **[Inferred]** The chip holds data until RTS drops; E6 checks it.

**Wake timing. [Source]** The ISR takes `bt_host_wake`, released 500 ms after HOST_WAKE falls. Any transmission adds 1 s of `bt_bcm_wake`. In that window the engine writes to its socket, glassd's `poll` returns, and it sees BOOTTIME pulled ahead of MONOTONIC (`daemon.rs:1043`). Then `resume_from_suspend` re-takes the lock, as on a Wi-Fi wake. It also reads the on-head sensor, a cost this path may not want.

**CORE OFF with the port open and the chip on is the open question.**

- **[Measured]** CORE OFF happens with Wi-Fi associated (slink33, 50 of 54). It also happens with the port never opened (sleepfix17, 6 of 6).
- **[Measured, older, before the UART fix]** Test 11 was blocked with ttyO1 held open. Other early runs blamed on UART2 IDLEST were later attributed to the port never having transmitted.
- **Never run on Model B:** the chip awake with firmware and sleep mode, port open across suspend. Stock reaches about 5 mA, but what state its BT was in at that moment is not recorded. Patch 0010 was tested on Android with BT on, and "no pairing or long session tried".

### 6.2 Parameters to start from

| State | Advertising or connection | Why |
|---|---|---|
| Just disconnected | Advertise at 20 ms for 30 s | Quick reconnect, Apple's advice |
| Waiting | Advertise at 1285 ms; 546.25 ms on head | Longest Apple-listed value |
| Connected idle | Interval 285 to 300 ms, latency 5, timeout 6 s | 300 × 6 = 1.8 s ≤ 2 s; × 3 = 5.4 s < 6 s |
| Busy | Interval 15 to 30 ms, latency 0, timeout 4 s | For a queue over about 200 bytes; back to idle after 5 s quiet |

Latency delays only phone-to-Glass traffic: at most 1.8 s plus the wake. Glass-to-phone goes at the next event, up to 300 ms. Busy mode costs a host wake and is only a request, which a phone may ignore. Android in the foreground can also request it itself.

All of this runs inside the controller. Empty connection events and advertising need no host, so **idle BLE should not wake the host at all [Inferred from the HCI model; E5 checks it]**.

Keep no periodic traffic on the link. **[Source]** glassd sends no Ping of its own, and answers only. The FFI has no heartbeat timer. Battery pushes happen on change, and the 30 s poll runs only while awake (`daemon.rs:250`). Android's 250 ms pump must only write when the core has something.

### 6.3 Sleep mode: mandatory

Without `Write Sleep Mode`, the chip likely stays awake and drains milliamps **[Inferred]**. libbt's defaults **[Public]** would be `01 01 01 01 01 01 01 00 00 00 00 00`: mode 1 (UART), idle thresholds 300 ms, BT_WAKE and HOST_WAKE active high, then host sleep during SCO allowed, combined sleep modes, no TXD tri-state. The polarities match the kernel's use of both lines **[Source: `board-notle-bluetooth.c`]**. The Glass build may override these defaults, so read `lpm_param` from the stock `libbt-vendor.so` before E4 (offline), or capture it (E1b).

The stock library also contains `/dev/ttyO1`, `/vendor/firmware/` and `userial_ioctl_init_bt_wake` **[Source]**, so the stock path resembles this one, though the wake line may be driven differently.

### 6.4 What it may cost

No current numbers exist for this chip with BLE; these are planning figures **[Inferred]**.

- Radio on air: about 3 ms at about 30 mA per advertising event at 1285 ms is 0.07 mA. Connected idle is less. The chip's sleep floor is unknown.
- Wake cost, from **[Measured]** 34 mA awake against 6.7 mA asleep: 1.5 s × 27 mA is about 0.011 mAh per event. A hundred notifications a day cost about 1 mAh, against about 160 mAh for a day asleep at 6.7 mA.
- So the design succeeds if the chip sleeps and the host stays asleep between events: expect well under 1 mA added. It fails if the chip does not sleep, or CORE OFF is blocked: tens of mA.

Qpassed readings over 30 s cycles ran 5.5 to 9.3 mA **[Measured]**, so a difference under 1 mA needs long cycles, at least 30 minutes per setting. Stop all logging first: slink33 and slink34 were spoiled by their probes **[Measured]**. Off head, the 10 s RTC watch for the first 10 minutes dominates anyway, and it stays. After `off_head_power_off_min` the Glass powers off and the link ends.

## 7. Experiments, smallest first

All run from a RAM boot (`fastboot-guard.sh boot`) with a test image built locally. **Model B has no adb [Measured]**, so each test image runs its script, logs to kmsg, and reboots. Then read `/proc/last_kmsg` from stock Android over adb. USB RNDIS is available for more interactive steps **[Measured: link28]**. The phone is driven over adb. **The dev PC shows no Bluetooth adapter today (`/sys/class/bluetooth` lists nothing) [Source]**; a USB dongle would give a headless central and sniffer, but is optional.

| # | What | Proves | Needs | Pass |
|---|---|---|---|---|
| E0 | Host only: HCD parser, HCI, ATT, tests against Bumble | Logic, before touching hardware | PC | Tests green |
| **E1** | `omg-bled probe`: unblock, reset, version, features, LE features, states, buffer size, before and after patch | **Go/no-go on the controller.** Also: raw tty works, HOST_WAKE path, patch load | RAM boot, cable | LE bit set, slave states present, `Launch RAM` ok, no UART errors |
| E1b | Optional: stock Android, btsnoop on through a bind-mounted `bt_stack.conf` | Gold capture of the stock bring-up, LPM values | Stock boot, cable; nothing written to flash | Capture has `0xFC27` and LE features |
| E2 | Advertise for 5 min | Radio on air, right clock | E1, phone scan through a small app | Phone sees the UUID, RSSI sane |
| E3 | Connect, MTU, echo service, parameter request | Peripheral role end to end; interop | E2, Android test screen or app | Echo correct at MTU 247; params accepted in `LE Connection Update Complete` |
| E4 | `Write Sleep Mode`, cable on: log BT_WAKE, HOST_WAKE, wakelocks, UART RPM status while idle and connected | LPM works on a raw tty | E3 | Lines toggle, UART goes `suspended`, `CM_L4PER_UART2_CLKCTRL` reads 3 |
| **E5** | Off head, cable out, BLE advertising, then connected idle; no probes | **Go/no-go on power:** CORE OFF, added current | E4, **owner unplugs** (with a cable the kernel never suspends **[Source]**) | CORE OFF on most cycles, under 1 mA added over ≥30 min each |
| **E6** | Asleep and connected: phone writes a counter once a minute | **BT wake:** `gpio_wk2` fires, no byte lost | E5, owner unplugs | `BT_HOST_WAKE` in kmsg, every counter arrives, wake time logged |
| E7 | Transfer 1 MiB both ways with SHA-256; print PDUs per event | Throughput, ordering, flow control | E3, cable | Hashes match, rate recorded, profile chosen |
| E8 | Real session: glassd + `omg-bled` + Android `BleLink`, pair, settings, note sync | The whole stack behind `ByteLink` | E3, E7 | The same flows as wifi30 pass over BLE |
| E9 | E8 with Wi-Fi on; handover; cable-out day-long soak; phone reboot and BT toggle | Coexistence on the shared chip; reconnects | E8, owner for the soak | No lost session state; reconnects with no user action |
| E10 | iOS: scan, connect, echo, background reconnect | The reason for choosing BLE | E3, **owner's Mac, iPhone** | Foreground first, then locked screen |

E1 and E5 can each stop or reshape the project, and they come early on purpose. If E5 fails, the fallback order is: compare with stock's UART state, fix the idle path in the kernel, then turn BLE off while off head with an RTC-timed window to reconnect.

Do E10 in step with E3 if the Mac is available, because a design that works only on Android has missed the point.

## 8. What could go wrong

| Risk | Effect | Handling |
|---|---|---|
| Port open plus chip on blocks CORE OFF | Tens of mA idle, plan at risk | E5 early; kernel idle work; fallback above |
| Chip does not sleep or HOST_WAKE misbehaves | Same | E4, E1b capture |
| Firmware's LE too old or buggy; few PDUs per event | Slower link, interop bugs | E1, E3, E7; only text is promised |
| Parameter request refused or ignored | Idle at the phone's short interval, more power | Measure per phone; Android can request too |
| Own stack hits phone interop bugs | Time | Bumble tests; TrouBLE fallback |
| Fixed handle table and phone caching | Stale GATT after an update | Keep it stable; rotate address if forced |
| iOS force quit, Bluetooth toggle | Link silently gone until the app opens | Tell the owner; AccessorySetupKit later |
| Android address type lost | No reconnect | Scan first |
| Stranger connects first, or holds the connection | Lockout, battery drain, DoS | Section 4 |
| Classic at the same time | No A2DP while engine owns the tty | Owner decision, 2.6 |
| Model A track | KitKat has no public advertising API **[Inferred]** | Owner's porting rule applies; bluedroid is already forked **[Source: manifest]**; plan separately |

## 9. Order of work

1. E0 and E1. Stop if the controller says no.
2. E2, E3, E4, with `omg-bled`, in glass-services.
3. E5 and E6, with the owner.
4. Noise channel and pairing mode, in `omg-protocol`, glassd, `omg-ffi`. This also fixes Wi-Fi.
5. `Transport::Ble`, `Radio` integration, `BleLink`, smaller profile, `WifiBringUpResult`. E7, E8, E9.
6. iOS app. E10.

Docs to correct when this lands: `companion-mobile.md` ("BLE peripheral does not exist yet"), `companion-rust-core.md` (transport order), and `bt.rs` and `transport.rs`, which still call RFCOMM the normal link.

## 10. Decisions for the owner

1. **Approve the direction:** own Rust stack on a raw tty, no BlueZ.
2. **Classic:** are A2DP or HFP headphones wanted at the same time as this link? If yes, plan 2.6.
3. **Security:** agree to Noise plus on-Glass code confirmation, and that this comes before everyday use.
4. **iPhone notifications:** does "works on iPhone" include them? That adds ANCS (4.5).
5. **Dictation:** accept no live audio over BLE? The options are Wi-Fi, recording first then sending as a note, or a codec later.
6. **Hands-on:** unplug the cable for E5, E6 and the soak.
7. **Apple:** a Mac, an iPhone, and free or paid account. Say when these are available.
8. **Optional:** a USB Bluetooth dongle on the dev PC.

## Sources

- [Apple QA1931, advertising and connection parameters](https://developer.apple.com/library/content/qa/qa1931/_index.html)
- [Apple forum: state restoration and force quit](https://developer.apple.com/forums/thread/844477)
- [Background BLE summary (unofficial)](https://uynguyen.github.io/2018/07/23/Best-practice-How-to-deal-with-Bluetooth-Low-Energy-in-background/)
- [BlueZ 5.86 `src/adapter.c`](https://raw.githubusercontent.com/bluez/bluez/5.86/src/adapter.c), [`src/advertising.c`](https://raw.githubusercontent.com/bluez/bluez/5.86/src/advertising.c), [README](https://raw.githubusercontent.com/bluez/bluez/5.86/README)
- [Alpine bluez package, v3.24](https://pkgs.alpinelinux.org/packages?name=bluez&branch=v3.24&repo=&arch=armhf), [APKBUILD](https://git.alpinelinux.org/aports/plain/main/bluez/APKBUILD?h=3.24-stable)
- [libbt `hardware.c`, android-4.4.4_r2.0.1](https://android.googlesource.com/platform/hardware/broadcom/libbt/+/refs/tags/android-4.4.4_r2.0.1/src/hardware.c), [`bt_vendor_brcm.h`](https://android.googlesource.com/platform/hardware/broadcom/libbt/+/refs/tags/android-4.4.4_r2.0.1/include/bt_vendor_brcm.h)
- [Linux `btbcm.c`](https://raw.githubusercontent.com/torvalds/linux/master/drivers/bluetooth/btbcm.c), [`hci_bcm.c`](https://raw.githubusercontent.com/torvalds/linux/master/drivers/bluetooth/hci_bcm.c)
- [Broadcom BCM4330 data sheet (mirror)](https://en.eeworld.com.cn/datasheet/view/31056514.html), [iPhone 4S teardown, TechInsights](https://www.techinsights.com/ja/node/16630), [AnandTech iPhone 4S review](https://at-web1.www.anandtech.com/show/4971/apple-iphone-4s-review-att-verizon/9)
- [BTstack licence](https://raw.githubusercontent.com/bluekitchen/btstack/master/LICENSE), [TrouBLE README](https://raw.githubusercontent.com/embassy-rs/trouble/main/README.md)
