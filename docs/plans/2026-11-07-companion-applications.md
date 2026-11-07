# Companion: the Applications section

Status: design, plus a host-tested core in `rust/crates/omg-core/src/apps/` and `rust/crates/omg-core/tests/apps.rs`. Nothing here has run on a phone or a Glass. There is no Android UI, no iOS code, no SDK and no installer on the Glass.

Tags used below:

- **[built]**: implemented in the core and tested on the host, with no real network. Where a part of a tagged section is not built, it says so.
- **[design]**: decided here, not built.
- **[SDK]**: depends on an SDK that does not exist yet. These parts are proposals and will change with it.

Owner requirement (2026-11-04): an Applications section like HACS. Add a repository by URL, read a manifest and the author's releases, install, flag updates. An optional community index lists known repositories, and custom ones are always allowed. The iOS route is the same flow: the phone only transfers packages that run on the Glass.

Sections 3 to 9 hold the design. Section 12 lists what the owner has to decide.

## 1. What exists today

**There is no SDK and no app model.** Checked in `glass-apps` and `glass-services` at their `origin/main` on 2026-11-07:

- An "app" on the Glass today is a screen compiled into the `glass-launcher` binary: home, settings, setup, Take a note. There is no package format, no installer, no app directory.
- `Platform::open_app(package)` exists in the launcher. On Android it starts an Android package. It is not a third-party app mechanism and has no permission model.
- The launcher owns the display. There is no API for another process to draw a card or a window.
- The nearest thing to an SDK is the `glass-api` crate and `docs/api.md` in glass-services (Apache-2.0): a client for the `glassd` socket. `Hello`, `Subscribe`, input focus, settings, dictation, location.
- That socket has **no per-client permissions**. On Linux, membership of the group `glass` gives access to everything, including `SetSettings`, the broadcast topics and input focus: "Any client may take the focus in version 1", "Peers are not authenticated beyond the socket permissions". This is fine for our own launcher and unacceptable for code from the internet.
- `omg-protocol` has scaffold frames `AddonInstall { id, source_url }` and friends. No handler uses them. They are replaced in section 10, because the owner's model is that the phone transfers packages, so the Glass must never be told to fetch a URL.

**What the kernel offers for isolation**, read from `notle_defconfig`, which `configs/tuned.config` does not change for these options, and not from a running Glass: `CONFIG_NAMESPACES` off, `CONFIG_SECCOMP` off, `CONFIG_NETFILTER_XT_MATCH_OWNER` off, `CONFIG_ANDROID_PARANOID_NETWORK` on, SELinux compiled in. So on kernel 3.4 a native process can be confined by Unix user, by file permissions, and by leaving out group 3003 (`AID_INET`) so it cannot create an internet socket. No syscall filter is available. I did not check whether the Model B image loads an SELinux policy.

## 2. What HACS does, and what we change

From the HACS publishing documentation ([general requirements](https://www.hacs.xyz/docs/publish/start/), [integrations](https://www.hacs.xyz/docs/publish/integration/), [default inclusion](https://hacs.xyz/docs/publish/include)), read 2026-11-07. No HACS code was read or copied.

| | HACS | Here | Why |
|---|---|---|---|
| Where | Public GitHub repositories only | GitHub or any HTTPS site | Owner requirement |
| Manifest | `hacs.json` at the repository root: display name, file names, minimum versions | `omg-apps.json`: the same role, and it also lists releases | A personal site has no releases API |
| Versions | Tag of the latest GitHub release; last 5 releases plus the default branch are offered; else the commit | Only listed, hashed, signed releases | A branch has no hash and no signature |
| Integrity | The pages I read mention no hash or signature | SHA-256, an ed25519 signature per release, and a signed state per app | The result runs on a device on the owner's face, with a camera and a microphone |
| Index | `hacs/default`, automated checks on a PR, then HACS review | Own index repository, CI-merged with no owner approval (owner, 2026-11-04) | Section 9 |
| Kill switch | A `critical` list in `hacs/default` for dangerous repositories | Proposed, with a human step | Section 9, decision D5 |
| Custom repositories | Allowed | Allowed | Same |

## 3. The repository manifest **[built]**

One JSON file. On GitHub it sits at the root of the default branch. On a site it is at any HTTPS address.

```json
{
  "schema": 1,
  "name": "Example apps",
  "apps": [
    {
      "id": "com.example.tally",
      "name": "Tally",
      "summary": "Counts things on a card.",
      "license": "Apache-2.0",
      "homepage": "https://example.org/tally",
      "publisher_key": "see below: 64 hex digits",
      "sequence": 3,
      "withdrawn": ["1.0.0"],
      "state_signature": "see below: 128 hex digits",
      "releases": [
        {
          "version": "1.1.0",
          "target": "armv7-linux-musleabihf",
          "min_api": 1,
          "url": "https://github.com/example/tally/releases/download/v1.1.0/tally.omgapp",
          "size": 48211,
          "sha256": "6a09e667bb67ae853c6ef372a54ff53a510e527f9b05688c1f83d9ab5be0cd19",
          "permissions": ["input.head"],
          "signature": "see below: 128 hex digits"
        },
        {
          "version": "1.0.0",
          "target": "armv7-linux-musleabihf",
          "min_api": 1,
          "url": "https://github.com/example/tally/releases/download/v1.0.0/tally.omgapp",
          "size": 47102,
          "sha256": "bb67ae856a09e667a54ff53a3c6ef3721f83d9ab510e527f5be0cd199b05688c",
          "permissions": [],
          "signature": "see below: 128 hex digits"
        }
      ]
    }
  ]
}
```

A test reads this block from this file, signs it and parses it, so it cannot drift from the code.

`target`, `min_api` and the meaning of `permissions` are **[SDK]**: the fields are parsed and signed now, and their values are provisional.

`withdrawn` may name versions no longer listed, and `sequence` goes up with every state the author publishes. Only two things in this file are unsigned: a release's `url` and the repository's `name`. A wrong URL can make a download fail, because the hash and size still have to match, and the UI shows the repository's name as unverified.

### Strict parsing

The whole file is accepted or refused. A half-understood manifest is never used.

| Rule | Limit or behaviour |
|---|---|
| File size | 512 KiB, checked before parsing, and the download stops at the limit |
| Apps per repository | 1 to 32 |
| Releases per app | 1 to 16. With one state signature per app, at most 544 signature checks per file |
| Withdrawn versions per app | 64 |
| Permissions per release | 16 |
| Package size | 1 byte to 64 MiB |
| Unknown field | Refused at every level. A field we do not understand may be a restriction we would ignore |
| Duplicate JSON key | Refused, so two parsers cannot read one file differently |
| `schema` | Read first. A higher number gives "update the companion", not a parse error |
| App id | Reverse DNS, lowercase letters, digits and `_`, at least 2 labels, 64 bytes. Usable as a path and a user name |
| `org.openmyglass` and `org.openmyglass.*` | Reserved, refused |
| Displayed text | One line, no outer space, and only an allowlist: printable ASCII, Latin-1 and Latin Extended-A letters, typographic quotes and dashes. A name needs at least one letter or digit |
| Version | Strict semver, no build metadata, one spelling per version, no duplicates in an app |
| Hashes, keys, signatures | Lowercase hex of exact length |
| Errors | Name a position and a field, never text from the file |

The text rule is an allowlist because a denylist of invisible characters cannot be complete: a review found four more that passed one (U+2800, U+1D173, U+1BCA0, U+13430). It matches the project's four languages and the Glass's font. It refuses Cyrillic, Greek, Hebrew, Arabic and CJK names, which is decision D11. Lookalikes inside Latin remain possible (`rn` and `m`), so text is never the proof of who something is from: the UI shows the repository's label and the key fingerprint.

There are deliberately no icons, screenshots, HTML or Markdown in the format. Each would add an image or rich-text parser fed with untrusted bytes. If screenshots are wanted later they should be files with a hash and limits, like packages.

### URLs

All URLs from a repository, and all redirects, pass one narrow parser (`apps::url`): HTTPS only, ASCII, a real DNS name, no credentials, no IP addresses, no fragments. The limit is 1024 bytes, and 8192 for a redirect, since a GitHub release asset redirect was measured at 924 bytes. Internationalised names must be in `xn--` form.

Path rules apply to the path: no dot segments, including `..;` and `..%20`, and no escapes that decode to a separator, a control character or overlong UTF-8. A query only needs well-formed escapes, because signed storage URLs carry encoded slashes and colons there. These rules narrow what a server can normalize into another path. They are not a proof, and they do not need to be, since a path decides nothing: the size, hash and signature do.

Internal names are refused by a denylist: `.local`, `.lan`, `.test`, `.home.arpa`, `localhost`, `fritz.box`, `speedport.ip`, `nip.io` and similar. **A denylist cannot be complete**, and any public name can resolve to a private address (T7).

## 4. Adding a repository **[built: address parsing and label. Not built: UI and repository list]**

The owner pastes an address, and `RepositorySource::parse` reads it:

| Pasted | Manifest read from |
|---|---|
| `https://github.com/OWNER/REPO` (optional `/` or `.git`, any case) | `https://raw.githubusercontent.com/owner/repo/HEAD/omg-apps.json` |
| That raw URL | The same, so it is the same repository |
| Any other `github.com` or raw path, or another port on them | Refused with a clear message, not guessed |
| `https://site/dir` or `https://site/dir/` | `https://site/dir/omg-apps.json` |
| `https://site/x/name.json` | That file |

The canonical manifest URL is the repository's identity, with one spelling per repository: GitHub names lowercased, `:443` dropped, repeated slashes collapsed. On 2026-11-07 I checked one repository by hand: `raw.githubusercontent.com` answered 200 for `HEAD` in both the original and lowercase spelling. No test uses the network.

The UI shows `RepositorySource::label()`: `github.com/owner/repo`, or the origin and path for a site. A host alone would not do, since every GitHub repository is on one host.

GitHub is read as a plain file, not through its API. The API is rate limited per address without a token, and this way GitHub and a personal site share one code path.

A manifest may redirect only within its origin. Otherwise an open redirect on a trusted host would show someone else's manifest under that host's name, and an attacker's own key can sign for any repository URL. `apex` to `www` counts as another origin, so the owner adds the final address. On a site shared between users, a same-origin redirect can still reach another user's manifest. The label would then be wrong, and signatures do not help when the other user signs for that URL. So a shared host is one trust domain.

Flow, once the UI exists **[design]**:

1. Fetch and parse the manifest. On failure show the error and store nothing.
2. Show the label, the repository's name marked as unverified, the apps, and for each the publisher key fingerprint. State plainly that this is unreviewed code from this source.
3. The owner confirms and the repository is remembered. Nothing is installed by adding one.

## 5. Versions and updates **[built, except scheduling and saving what is installed]**

`select_release` ranks: the highest version not withdrawn, not below the installed floor, that runs on this Glass (`target`, `min_api`) and asks only for known permissions. Pre-releases only if the owner opted in. File order means nothing, and it reports what it skipped and why.

**`plan_install` is the only way to start an install.** `fetch_package` and `DownloadVerifier` need the `InstallTicket` it returns, so the key pin, the same-id rule, the floor and rollback checks cannot be skipped on the install path. The compiler enforces that for code using this API. It does not stop a frontend from writing its own download, and the Glass's own checks are for that (section 10). The ticket also says whether this is a first use and which permissions need the owner's yes, but showing those is the UI's job and nothing in the core can force it.

`check_update` compares an installed app with the manifest:

| Result | When | What the UI does |
|---|---|---|
| `UpdateAvailable` | A strictly higher version can be offered | Offers it. `added_permissions` not empty means a fresh question |
| `UpToDate` | Nothing higher | Can say "newer version needs a newer Glass" from `newer_skipped`, and warn if the installed one was withdrawn |
| `Blocked(OtherSource)` | The manifest is not the one the app came from | Offers nothing. To switch, forget the app first |
| `Blocked(PublisherChanged)` | The key differs from the pinned one | Offers nothing, shows a loud warning |
| `Blocked(StateRolledBack)` | A lower `sequence` than already seen | Same: the host is hiding something |
| `Blocked(StateForked)` | Two different states with one `sequence` | Same: the key holder did this |
| `Blocked(ReleaseChanged)` | An installed version is now listed with a different hash | Same: a published version never changes |

Only a strictly higher version is an update, and nothing goes below the highest version installed, even after an uninstall.

**[design]** The companion checks each repository at most once a day and when the owner opens the section, and never installs an update without the owner's action (decision D4). Which versions are on the Glass will come from the Glass (`AppList`, section 10). `InstalledApp` is only an input here.

## 6. Hashes and signatures **[built on the companion; the Glass side is design]**

A hash in the manifest only proves the download matches the manifest. Anyone who controls the manifest's host, or the author's GitHub account, controls the hash too. So there are signatures.

- Each app has an ed25519 `publisher_key`. The author keeps the secret key off the hosting account.
- **A release signature** covers what one package is: domain string, app id, version, target, `min_api`, size, SHA-256, sorted permissions. It is binary and length-prefixed, not JSON, so there is no canonicalisation problem (`release_statement`). `Release` and `VerifiedPackage` keep it, so the phone can pass it to the Glass, which can check it alone.
- **A state signature** covers everything else the author says about the app at one moment: this repository's URL, `sequence`, name, summary, licence, homepage, withdrawn versions, and the hash of each release statement (`app_state_statement`). So without the key a host cannot edit the text, clear a withdrawal, drop a fix, or copy the manifest to another site.
- `Manifest::parse` verifies every signature with `verify_strict` and refuses weak keys. One bad signature refuses the file, so an attacker cannot add a bogus "newest" release and have clients quietly fall back.

A signature proves "same author as last time", **not** "good author". A hostile author with a valid key is handled only by section 7.

### What is remembered

`TrustStore` keeps one record per app id in a file, written atomically: the pinned key, the repository, the highest `sequence` seen and its digest, every version seen withdrawn, and the highest version installed with its hash. **Uninstalling does not touch it.** So a reinstall meets the same key and the same floor. A missing file is an empty memory, and a damaged one is an error, never treated as empty, because that would drop every pin. The file is on the phone and only as safe as the app's private storage.

`forget` is the only way out: the next install of that id is a first use, under any key, from any repository. It is how an author's lost key or moved repository is handled, and it is also what an attacker would ask a victim to do, so the UI must say so.

### What a static manifest can and cannot prevent

A signed state can only be edited by the key holder, but a host can still **serve an older genuine state**. Memory catches that only for what this companion has already seen.

| Attack, by someone with the hosting but not the key | Result |
|---|---|
| Edit the name or summary, clear a withdrawal, drop a release, add a release | Refused: bad signature |
| Serve an older state, to a companion that has seen a newer one | Refused: `StateRolledBack` |
| Serve a state without a withdrawal already seen | Refused as above, and the version stays withdrawn in memory even under a later genuine state |
| Serve an older state, to a companion that **never saw the newer one** | **Not detected.** A release withdrawn after that state is offered, and a fix published after it is hidden. A test states this |
| Keep serving one old state forever (freezing) | **Not detected.** Only looks like an author who went quiet |
| Serve an old state to a **first install** | **Not detected.** It gets a genuine but possibly withdrawn release |
| Change a `url`, or answer with an error | The download fails. Nothing else |

Closing the undetected rows needs freshness: a state that expires, so the author has to re-sign regularly, or a second party that vouches for the latest `sequence`. I left expiry out because hobby authors would not re-sign on schedule and their apps would stop updating. The community index could later publish each listed app's latest `sequence`, which would make these attacks need both the host and the index.

Other limits, stated plainly:

- **First use is trust.** Whoever controls the repository on the day of the first install chooses the key. The fingerprint is shown, and the index can carry a key (section 9).
- **A lost key means a new identity:** forget and reinstall, with the warning. A stolen key has no recovery short of that, and a thief with the key can publish anything, though `StateForked`, the floor and the remembered withdrawals limit what they can restore. Signed key rotation is a later schema.
- **Moving a repository** needs the author to re-sign for the new URL, and each owner to forget and re-add it. That is a real cost of binding a state to its repository, and it is what stops an old signed manifest being replayed from any other site as a valid first install. Replay **from the same repository** is the "first install" row above.
- **Authors need a tool**: `omg app keygen` and `omg app sign` do not exist yet. Signing must bump `sequence` every time.

### Downloads

The core has no network. `Fetcher` is injected, and `HttpsUrl` is the only type it accepts, so it can only be handed a checked URL.

- Redirects come back to the core, and each hop passes the URL rules before any request. Limit: 5.
- `DownloadVerifier` counts and hashes each chunk. The transfer stops at the first byte over the declared size, and that chunk is not stored.
- `PackageStore` gives each download its own `<sha256>.<random>.part`, opened with `O_EXCL`, mode 0600, so concurrent downloads of one hash cannot touch each other's bytes. On success it hashes **the file itself**, fsyncs and renames to `<sha256>.omgapp`. Any failure removes the part. `remove_stale_parts` clears what a crash left, at startup. **File names come only from the hash**, so nothing from a manifest reaches a path.
- `FetchControl` carries a total deadline and a cancel switch, checked before each request and each read: 60 seconds for a manifest, and for a package 60 seconds plus the size at 16 KiB/s. That also acts as a minimum rate. The core cannot interrupt a `read` blocked inside the fetcher.

`Fetcher` contract, **enforced only by review**: platform certificate validation with no way off, no automatic redirects, no cookies, credentials or `Referer`, connect and read timeouts within `FetchControl::remaining` and at most 30 seconds per read, stop on `is_cancelled`, and refuse a name resolving to a private address where the platform allows it.

On phones **[design]**: `docs/companion-mobile.md` says the app owns its sockets and the only foreign callbacks are the blocking speech engines. `Fetcher` would be one more blocking foreign trait of the same kind, on a thread the core starts. The alternative is `DownloadVerifier` alone: the app runs the HTTP, for instance a background `URLSession` on iOS, which survives suspension better, and feeds chunks in. `omg-ffi` needs to export whichever we pick, along with `HttpsUrl::redirect`. Not done in this pass. Neither `apps` nor `TrustStore` is exported to Kotlin or Swift yet.

## 7. What an app may access **[SDK]**

Everything in this section depends on the SDK and runner. `KNOWN_PERMISSIONS` in the code is a placeholder list.

**The manifest's permission list is a request, not a limit.** Only the Glass can enforce it: the runner and `glassd` have to give each app only what was granted. A permission nobody enforces is worse than none, because it reassures for nothing.

Proposed baseline, with no permission and no question: show cards while in front, get touchpad gestures while in front, read screen and wear state, own private storage, API version.

| Permission | What it gives | Source today |
|---|---|---|
| `input.head` | Head gestures | `TOPIC_HEAD` |
| `input.button` | Camera button | `TOPIC_BUTTON` |
| `location` | Position from the phone | `TOPIC_LOCATION` |
| `dictation` | Speech to text through the phone | `TOPIC_DICTATION` |
| `notifications.read` | Phone notifications shown on the Glass | `TOPIC_COMPANION` |
| `notes.write` | Add a note | Note API |
| `microphone` | Raw audio | No API yet |
| `camera` | Pictures | No API yet |
| `network` | Internet sockets | Group 3003 on Model B |
| `companion.fetch` | HTTPS through the phone | No API yet |
| `background` | Run while not in front | Runner, battery cost |

Never grantable: `SetSettings`, radios, power, pairing, the companion identity, Wi-Fi passwords, another app's data, installing apps, screen capture, input while another app is in front.

Rules: an update asking for more needs a new yes; the owner can see and revoke grants; camera and microphone use by third-party apps needs a separate owner decision on what the person in front of the Glass can see, since camera decision 2026-11-06 was "no LED" for our own capture.

### How it could be enforced

`glassd`'s socket needs a real permission check before any third-party code runs, whichever runtime is chosen. Decision D2 chooses the runtime:

| | A. Wasm in an interpreter on the Glass | B. Native ARM process per app |
|---|---|---|
| Isolation | The app can do only what host functions allow. It has no syscalls. Kernel exposure is the runner's | Per-app Unix user, file modes, no `AID_INET`. No seccomp, no namespaces, on a 3.4 kernel: every syscall is open, so a kernel bug gives root |
| Portability | One package for Model A and Model B | musl static may run on both, untested for this use. Android has no user for it in `packages.list`, so the peer check needs new work |
| Speed | Slower on a dual-core A9. **Not measured**; an interpreter's cost needs a spike | Full speed |
| Languages | Rust at once, others via their Wasm targets | Rust at once, others as cross-compilers allow |
| Owner decision 2026-11-04 ("Rust plus an open socket protocol") | Keeps Rust and the message set; the socket becomes host calls | Exactly as decided |
| GPU, camera buffers | Through host APIs only | Possible directly, with more exposure |

With B the honest statement to the owner of a Glass is: permissions stop a polite app, and a hostile one may get root. With A that statement is much weaker, though the runner itself becomes the attack surface.

### Drawing **[SDK]**

The first SDK should not hand out a surface: apps send card descriptions (text, image, layout from `glass-ui`) and the launcher draws them. It needs no GPU access, avoids a compositor between processes on an SGX540 route that only just works, matches the Glass's card design, and is enough for most Glass apps. Live-drawing apps can come later.

## 8. Threat model: a repository is untrusted input

| # | Threat | Answer | Left over |
|---|---|---|---|
| T1 | Malformed manifest: huge, deep, wrong types, parser differentials | Size check before parsing, typed parser with depth limit, no unknown or duplicate keys, 20,000-mutation test | **[built]** |
| T2 | **Hostile app with a valid key** | Only the sandbox and the owner's consent. Signing does nothing here | **The main risk.** Sections 7 and D2 |
| T3 | Stolen repository, account or site, without the key | Pinned key survives an uninstall; nothing edited verifies; the install path enforces it | First install is still trust, and `forget` clears the pin. **[built]**, warnings in the UI not built |
| T4 | Tampered package at a CDN or mirror | Size, hash, signature. Bytes unusable until verified | **[built]** |
| T5 | Rollback, un-withdrawing, freezing | Signed state; `StateRolledBack`; remembered withdrawals; floor | Only for states already seen: section 6 has the table of what is not detected. **[built]** |
| T6 | Spoofing: name, id, odd characters, reserved prefix | Allowlisted text, signed; the UI shows the label; `org.openmyglass` refused; same id elsewhere blocked | Lookalike Latin names and a similar GitHub owner name are still possible. **[built]**, UI not built |
| T7 | Making the phone call into the LAN | No IPs, a denylist of internal names, every redirect checked | **A denylist cannot be complete.** A public name can resolve to a private address, which the core cannot see. On Android the fetcher can filter in OkHttp's `Dns`, and that is in its contract but not built; I know of no equally simple hook on iOS. What is left is a credential-less GET whose answer the attacker does not get back, though timing or error differences may reveal something. **[built in part]** |
| T8 | Resource exhaustion: endless bodies, big packages, slow servers | Byte limits; total deadline and cancel between reads | A `read` blocked in the fetcher needs its own timeout, by review only. Free space on the Glass is unchecked until the installer exists. Scheduling of checks not built. **[built in part]** |
| T9 | Hostile package: path traversal, symlinks, archive bombs, permissions disagreeing with the manifest | Section 10 | **[SDK]** |
| T10 | Compromised phone or companion | The Glass verifies the signature and pin itself, refuses downgrades, confirms by tap (D3) | A paired phone can already change settings. **[design]** |
| T11 | Installed app attacks kernel or `glassd` | Section 7, permission check on the socket | Depends on D2. **[SDK]** |
| T12 | Installed app leaks data | No network or phone fetch unless granted | A granted app can leak what it was granted. **[SDK]** |
| T13 | Index abuse: flooding, lookalikes, wrong keys | Index only names repositories, adds no trust, and the UI says so; user can replace it | Section 9 |
| T14 | Tracking by repository hosts: IP and time | Fewer checks, and no identifying headers or per-user URLs in the fetcher contract | **Cannot be prevented or enforced.** Every repository the owner adds sees the phone's address and when it checks, and a repository can make its URLs unique per release or per reader. An author can log who downloads what. Only a proxy would change that. **[design]** |
| T15 | A failed or killed install leaves a broken app | Atomic staging and swap, previous version kept | Section 10. On the phone, a killed download leaves only a `.part`. **[built on the phone; Glass side design]** |

## 9. Community index **[design]**

Owner decisions 2026-11-04: a repository under OpenMyGlass, entries added by PR, validated and merged automatically by GitHub CI with no owner approval, and the user may set a different index.

Proposed: one file with the same strict rules, `{ "schema": 1, "repositories": [ { "url": "...", "publisher_keys": { "com.example.tally": "..." } } ] }`. A first install from a listed repository could then compare the key before pinning it. It could also publish each app's latest `sequence`, as section 6 notes.

CI checks: the PR only adds or changes entries for its author's repository; the manifest parses with this same core code; signatures verify; the id is not already claimed by another repository; the repository has a licence.

Because merging is automatic, **being listed means "well-formed", not "reviewed"**, and the UI has to say that in those words. Key changes for an existing entry must not auto-merge, or the index becomes a way to move someone's app.

A revocation list needs a human step (D5): if it were automatic, anyone could get a rival's app marked dangerous. The companion would warn and offer removal, and never remove on its own.

## 10. What the Glass needs **[SDK, not built]**

No install on the Glass in this pass. This is the interface it would need.

### Package **[SDK]**

`.omgapp`, proposed: a flat, uncompressed format with a table of regular files. No directories beyond one level, no symlinks, no device files, no absolute paths, no `..`, with a total size and file count limit checked against the header before extraction. A JSON `omg-app.json` inside carries the id, version, target, API and permissions, and must equal the signed statement. If they disagree the package is refused. A custom layout can be simpler to parse safely than a general archive; compression is not worth a decompressor fed with untrusted data, since the link already limits size.

### Frames

Appended at the end of `Frame` (postcard encodes by position), inside the encrypted session, replacing the unused `Addon*` scaffold. No frame carries a URL: the phone downloads and the Glass never fetches.

```
Companion -> Glass  AppOffer { statement, signature, publisher_key }   (all from VerifiedPackage)
Glass -> Companion  AppOfferReply { accepted, reason, resume_at, free_bytes }
Companion -> Glass  AppChunk { sha256, offset, bytes }
Glass -> Companion  AppChunkAck { next_offset }
Companion -> Glass  AppCommit { sha256 }
Glass -> Companion  AppInstallResult { id, version, code }       (a closed enum, no free text)
Companion -> Glass  AppListRequest                                Glass -> Companion  AppList { apps }
Companion -> Glass  AppRemove { id, keep_data }
Companion -> Glass  AppSetGrants { id, permissions }
```

### What the Glass must do itself

The phone is not the last check (T10).

1. Verify the signature on `AppOffer` with its own ed25519 code. For a known id the key must equal the Glass's pin, which also survives an uninstall.
2. Refuse a version not higher than the installed one, and one below the highest it ever installed, unless the wearer clears the record on the Glass.
3. Check free space first. Stage under a private directory. Writes resume from an offset; a link drop loses nothing.
4. Hash on commit, then validate the package fully and compare the inner `omg-app.json` with the statement.
5. Ask the wearer to confirm by tap (D3), showing name and permissions in the launcher's own UI.
6. Swap atomically and keep the previous version until the new one has started once.
7. Keep the pin and the grants in `glassd`, not in the app's reach. An app cannot read or change them.

Also needed: a store on persistent storage (Model B layout to be decided), a launcher app list, a runner, per-client permissions in `glassd`, and holding off deep sleep during a transfer.

### The transfer link

Measured earlier (2026-11-05, one setup, `out/kmsg_ble*`, `out/ap*`): BLE phone to Glass 5.4 to 6.1 kB/s; Glass access point Wi-Fi phone to Glass 3.2 to 4.2 MB/s. By arithmetic a 1 MiB package takes about 3 minutes over BLE, and the 64 MiB limit about 3 hours, but about 20 seconds over Wi-Fi. So the BLE path should carry small packages, with a limit set by a `LinkProfile`, and big ones need the Wi-Fi path. The Glass access point flow on iPhone may need a paid Apple account, which is unverified (owner note, 2026-11-05), so a size warning belongs in the UI.

### Model A

Under the port-fixes rule this interface must also work on the Android track. With Wasm (D2-A) the runner is one process there too. With native apps (D2-B) the per-app user and `packages.list` peer check is unsolved.

## 11. iOS and licensing

**iOS.** All logic is in the Rust core, and the flow has no Android-only step. Risks:

- App Store Review Guideline 2.5.2 (read 2026-11-07) forbids downloading code "which introduces or changes features or functionality of the app, including other apps". Packages run on the Glass and not on the phone, so my reading is that the rule does not bite. Guideline 4.7 covers software offered inside an app and may be argued either way. I found nothing specific to external devices in the part of the page I read, and the end of the page was cut off. This is my reading, not advice, and it only matters when the app goes to the App Store. The owner's free account does not reach that point.
- A free account's 7-day expiry and capability limits are noted as unverified in the owner's notes.
- Background checks are unreliable on iOS: check when the owner opens the section.

**Licensing.** This repository, and so the Rust core, is GPL-3.0. Apps do not link it: a package is only moved through it. The SDK must not be GPL if apps of any licence are wanted, and `glass-api` and `glass-ui` are already Apache-2.0 (D7). `license` in a manifest is the author's statement and nobody verifies it. The new dependency, `semver`, is MIT OR Apache-2.0, and is in `THIRD_PARTY_NOTICES.md`. Only HACS's public documentation was read.

## 12. Owner decisions

| # | Question | Recommendation |
|---|---|---|
| D1 | Signatures mandatory for every release? | **Yes.** Built that way. Unsigned means a stolen account updates every Glass. Cost: authors manage a key, and the tool has to exist |
| D2 | App runtime: Wasm interpreter or native processes? | **Wasm for third-party apps, after a spike** measuring it on the Glass, because there is no seccomp and a 3.4 kernel. It changes the "open socket protocol" part of your 2026-11-04 decision, so it is your call. Native stays possible as an explicit developer mode |
| D3 | Tap on the Glass to confirm an install or a new permission? | **Yes.** A cheap defence against a compromised phone, once the launcher has an installer screen |
| D4 | Automatic updates? | **No, notify only**, as in HACS. Consider later for apps with no new permissions |
| D5 | Revocation list in the index, given auto-merge? | **Yes, as a separate file that requires a maintainer's review.** Warn and offer removal, never remove on its own. This is the one human step in an otherwise automatic index |
| D6 | First SDK draws by card descriptions, with no surface for apps? | **Yes.** Smallest, safest, and enough for most Glass apps |
| D7 | SDK licence? | **Apache-2.0**, like `glass-api` and `glass-ui` |
| D8 | Permissions in the first SDK? | **Only baseline plus `input.head` and `input.button`.** No camera, microphone, network or notifications until the enforcement is proven |
| D9 | Static manifest instead of reading GitHub releases through the API, and no install from a branch? | **Yes.** It differs from HACS and your "reads the author's releases" wording: assets stay on GitHub releases, but the list is in the manifest, written by the author or by a release script |
| D10 | Refuse LAN names, including a personal NAS at `.local`? | **Yes for now.** Allowing them later should be an explicit per-repository choice |
| D11 | Display text only in Latin scripts, so a Japanese or Russian app name is refused? | **Yes for now**, since the Glass's font and the project's four languages are Latin, and it removes whole classes of spoofing. It does cost an open ecosystem something, so it is your call. Ids and code are unaffected |
| D12 | Accept that an old signed state can fool a companion that never saw the newer one? | **Yes for the first version**, with no expiry. Section 6 has the table. Revisit with the index publishing latest `sequence` numbers |
| D13 | A repository move needs the author to re-sign and each owner to forget and re-add? | **Yes.** This is the cost of refusing replayed manifests |

## 13. Build order

1. **[built]** Parsing, signatures, selection, trust memory, verification, with no real network.
2. Decide D1, D2, D6. Everything below depends on them. Spike Wasm on the Glass if D2 leans that way.
3. SDK and package format, with permission checks in `glassd`.
4. Installer on the Glass, frames, launcher app list.
5. `omg app keygen|sign|verify`, `omg app install` in the CLI against the simulated Glass.
6. `omg-ffi` export, real fetcher, Android UI, then iOS.
7. Community index and its CI.
