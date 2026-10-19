# Fasi 0–2: strumenti, recupero binari, reverse engineering — Piano

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Arrivare all'inventario completo delle funzionalità di MyGlass (`docs/feature-inventory.md`) con specifica del protocollo, API interne di Glass, dipendenze cloud e prior art documentati, senza hardware Glass.

**Architecture:** Lavoro di analisi, non di prodotto. I binari e il codice decompilato vivono solo in `_re/` (gitignored); nella repo entrano documentazione, `.proto` ricostruiti e piccoli script di supporto in `tools/`. Senza occhiali, la verifica avviene sul codice decompilato e, se fattibile, su un emulatore che esegue l'immagine XE24.

**Tech Stack:** REA MCP (`rea-agents`, registrato in locale; usare i suoi strumenti per l'analisi assistita sopra jadx), jadx, apktool, apksigner (Android build-tools), protobuf (`protoc`), e2fsprogs/simg2img per l'immagine di sistema, Android Emulator, bash, Python 3.

**Spec:** [`docs/project-prompt.md`](../../project-prompt.md) (Passo 0, Fasi 1–2) + vincolo "nessun hardware" in [`docs/progress.md`](../../progress.md).

## Global Constraints

- Target: Glass Explorer Edition XE-C, firmware **XE24** (Android 4.4.2 / API 19, GDK).
- Package atteso di MyGlass: `com.google.glass.companion` — **da verificare**, non assumere.
- Versione MyGlass: **l'ultima disponibile (2017 o successiva)**; opzionale una del 2014 per confronto.
- Immagine XE24: solo dalla pagina ufficiale Google for Developers "System and Kernel Downloads".
- **Mai committare** APK, immagini di sistema, `.dex`, `.jar`, codice decompilato. Tutto in `_re/`.
- Documentazione, non copia di codice: descrivere classi/messaggi, non incollarne il corpo.
- Non inventare UUID, campi protobuf, intent o formati: ogni affermazione cita classe/file in `_re/` da cui deriva, oppure è marcata **[non verificato]**.
- Nessun hardware Glass: niente test su dispositivo in queste fasi.
- Aggiornare `docs/progress.md` alla fine di ogni task; commit frequenti.
- **Stop obbligatorio** dopo il Task 10: mostrare l'inventario all'utente, non iniziare la Fase 3.

## Review Focus

1. **Commit accidentale di materiale Google** (APK, dex, sorgenti decompilati incollati in `docs/`): ci si aspetta che il controllo clean-room lo blocchi prima del push → test nel Task 1.
2. **APK da mirror con firma non Google o repackaged**: ci si aspetta che venga scartato, non analizzato → controllo del certificato nel Task 2.
3. **APK distribuito come split/bundle (`.apks`/`.xapk`)**: ci si aspetta che venga riconosciuto e ricomposto o rifiutato con un messaggio chiaro → caso gestito nello script del Task 2.
4. **Immagine di sistema in formato sparse o con partizioni annidate**: ci si aspetta una conversione documentata, non un'estrazione silenziosamente vuota → verifica del numero di APK estratti nel Task 3.
5. **Nomi offuscati (ProGuard) diversi tra versioni**: ogni riferimento a una classe nei documenti deve indicare la versione dell'APK da cui proviene → regola di citazione nei Task 6–10.

---

## File Structure

| File | Responsabilità |
|---|---|
| `tools/check-tools.sh` | Verifica la presenza e la versione degli strumenti richiesti |
| `tools/check-clean-room.sh` | Fallisce se file tracciati da git contengono binari o codice Google |
| `tools/re/apk-info.sh` | Stampa package, versione, SHA-256 del file e del certificato di un APK |
| `docs/sources.md` | Provenienza, versioni, hash e firme di tutti i binari analizzati |
| `docs/emulator-feasibility.md` | Esito dello spike sull'emulazione XE24 e strategia di test senza hardware |
| `docs/protocol/README.md` | Indice della specifica del protocollo |
| `docs/protocol/transport.md` | Trasporto, UUID/SDP, pairing, framing, keep-alive, riconnessione |
| `docs/protocol/handshake.md` | Handshake, autenticazione, requisiti account/token, comportamento XE24 |
| `docs/protocol/messages.md` | Catalogo messaggi e sequenze per feature |
| `docs/protocol/proto/*.proto` | Schemi ricostruiti dalle classi generate |
| `docs/glass-internals.md` | Intent, broadcast, provider, servizi lato Glass e loro raggiungibilità |
| `docs/cloud-dependencies.md` | Cosa passava dai server Google e come sostituirlo |
| `docs/prior-art.md` | Analisi dei progetti esistenti e delle loro licenze |
| `docs/feature-inventory.md` | Inventario funzionalità (deliverable dello stop) |

---

### Task 1: Strumenti e guardia clean-room

**Files:**
- Create: `tools/check-tools.sh`, `tools/check-clean-room.sh`
- Modify: `docs/progress.md` (tabella Fase 0)

**Interfaces:**
- Produces: `tools/check-tools.sh` (exit 0 se tutto presente), `tools/check-clean-room.sh` (exit 1 se trova violazioni; usato dai task successivi prima di ogni commit).

- [ ] **Step 1: Installare gli strumenti**

```bash
brew install jadx apktool protobuf e2fsprogs simg2img p7zip
export ANDROID_HOME="$HOME/Library/Android/sdk"
ls "$ANDROID_HOME/build-tools"   # annotare la versione più recente
```

Se `simg2img` non è disponibile in brew, annotarlo in `docs/progress.md` e usare `brew install android-platform-tools` + alternativa documentata nel Task 3.

- [ ] **Step 2: Scrivere `tools/check-tools.sh`**

```bash
#!/usr/bin/env bash
# Verifica gli strumenti necessari alle Fasi 0–2.
set -u
SDK="${ANDROID_HOME:-$HOME/Library/Android/sdk}"
BT="$(ls -d "$SDK"/build-tools/* 2>/dev/null | sort -V | tail -1)"
missing=0
check() { if command -v "$1" >/dev/null 2>&1 || [ -x "$2" ]; then printf 'ok    %s\n' "$1"; else printf 'MISS  %s\n' "$1"; missing=1; fi; }
check jadx ""
check apktool ""
check protoc ""
check debugfs "$(brew --prefix e2fsprogs 2>/dev/null)/sbin/debugfs"
check simg2img ""
check 7z ""
check adb "$SDK/platform-tools/adb"
check apksigner "$BT/apksigner"
check emulator "$SDK/emulator/emulator"
exit $missing
```

- [ ] **Step 3: Eseguirlo**

Run: `bash tools/check-tools.sh`
Expected: tutte le righe `ok`, exit 0. Ogni `MISS` va risolto o documentato prima di proseguire.

- [ ] **Step 4: Scrivere il test della guardia clean-room (deve fallire)**

```bash
mkdir -p /tmp/cr-test && cd /tmp/cr-test && git init -q
printf 'PK' > fake.apk && printf 'package com.google.glass.companion;\n' > Leak.java
git add -f fake.apk Leak.java
bash /Users/giorgio/Documents/google-glass-project/tools/check-clean-room.sh; echo "exit=$?"
```

Expected (prima dell'implementazione): `No such file` → il test fallisce.

- [ ] **Step 5: Scrivere `tools/check-clean-room.sh`**

```bash
#!/usr/bin/env bash
# Fallisce se i file tracciati (o in stage) contengono binari o codice Google.
set -u
fail=0
files="$(git ls-files --cached)"
bad_ext="$(printf '%s\n' "$files" | grep -Ei '\.(apk|apks|xapk|dex|odex|img|jar)$' | grep -v 'gradle-wrapper.jar' || true)"
if [ -n "$bad_ext" ]; then echo "Binari tracciati:"; echo "$bad_ext"; fail=1; fi
leaks="$(printf '%s\n' "$files" | grep -E '\.(java|smali|kt)$' | xargs -I{} grep -lE '^(package|\.class .*L)com/?\.?google' {} 2>/dev/null || true)"
if [ -n "$leaks" ]; then echo "Sorgenti con package Google:"; echo "$leaks"; fail=1; fi
if printf '%s\n' "$files" | grep -q '^_re/'; then echo "_re/ è tracciata"; fail=1; fi
[ $fail -eq 0 ] && echo "clean-room ok"
exit $fail
```

- [ ] **Step 6: Rieseguire il test**

Run: lo stesso comando dello Step 4.
Expected: elenca `fake.apk` e `Leak.java`, `exit=1`. Poi, nella repo: `bash tools/check-clean-room.sh` → `clean-room ok`, exit 0.

- [ ] **Step 7: Aggiornare `docs/progress.md` e committare**

Aggiornare la tabella della Fase 0 con le versioni reali (`jadx --version`, `apktool --version`, `protoc --version`).

```bash
chmod +x tools/*.sh
bash tools/check-clean-room.sh && git add tools docs/progress.md && git commit -m "tools: tool check and clean-room guard"
```

---

### Task 2: Recupero e verifica dell'APK MyGlass

**Files:**
- Create: `tools/re/apk-info.sh`, `docs/sources.md`
- Local only: `_re/apk/`

**Interfaces:**
- Consumes: `apksigner` dal Task 1.
- Produces: `_re/apk/myglass-<versionName>.apk` (ultima), opzionale `_re/apk/myglass-2014-<versionName>.apk`; `tools/re/apk-info.sh <apk>`.

- [ ] **Step 1: Scrivere `tools/re/apk-info.sh`**

```bash
#!/usr/bin/env bash
# Uso: apk-info.sh <file.apk> — stampa package, versione, hash del file e del certificato.
set -eu
apk="$1"
case "$apk" in *.apks|*.xapk) echo "ERRORE: bundle split ($apk). Estrarre base.apk con 7z e rieseguire." >&2; exit 2;; esac
SDK="${ANDROID_HOME:-$HOME/Library/Android/sdk}"
BT="$(ls -d "$SDK"/build-tools/* | sort -V | tail -1)"
"$BT/aapt" dump badging "$apk" | grep -E "^package:" 
echo "file_sha256: $(shasum -a 256 "$apk" | cut -d' ' -f1)"
"$BT/apksigner" verify --print-certs "$apk" | grep -E 'Signer #1 certificate (DN|SHA-256)'
```

- [ ] **Step 2: Provarlo su un APK noto**

Run: `touch /tmp/x.xapk && bash tools/re/apk-info.sh /tmp/x.xapk; echo "exit=$?"`
Expected: messaggio "bundle split", `exit=2`.
Run: `bash tools/re/apk-info.sh "$(ls ~/Library/Android/sdk/system-images/*/*/*/system.img 2>/dev/null | head -0; ls ~/.android/avd 2>/dev/null >/dev/null; echo /nonexistent.apk)"; echo "exit=$?"`
Expected: errore di `aapt` su file inesistente, exit ≠ 0.

- [ ] **Step 3: Scaricare l'APK**

Cercare MyGlass su APKMirror (fonte preferita: verifica la firma originale) e, in alternativa, archive.org. Salvare in `_re/apk/`. Scegliere l'ultima versione (≥2017); se disponibile, una del 2014.

- [ ] **Step 4: Verificare**

Run: `bash tools/re/apk-info.sh _re/apk/<file>.apk`
Expected: `package: name='...'` (annotare il valore reale), certificato con DN Google (`O=Google Inc.` o simile). **Se il DN non è Google, scartare l'APK** e annotarlo.

- [ ] **Step 5: Scrivere `docs/sources.md`**

Per ogni binario: nome, URL esatto della fonte, data di download, package, versionName/versionCode, SHA-256 file, DN e SHA-256 del certificato, eventuali note.

- [ ] **Step 6: Commit**

```bash
bash tools/check-clean-room.sh && git add tools/re docs/sources.md && git commit -m "docs: MyGlass APK provenance and verification"
```

---

### Task 3: Recupero ed estrazione dell'immagine XE24

**Files:**
- Modify: `docs/sources.md`
- Local only: `_re/xe24/`

**Interfaces:**
- Produces: `_re/xe24/system/` (filesystem estratto), `_re/xe24/apk-list.txt` (elenco APK di sistema con path).

- [ ] **Step 1: Scaricare l'immagine**

Dalla pagina Google for Developers "System and Kernel Downloads" (Glass Explorer Edition) scaricare l'immagine di sistema **XE24** in `_re/xe24/`. Annotare URL, nome file, SHA-256 (`shasum -a 256`) e, se la pagina li pubblica, confrontare con gli hash ufficiali.

- [ ] **Step 2: Estrarre l'archivio e identificare `system.img`**

```bash
cd _re/xe24 && 7z x <archivio> && file *.img
```

Expected: `system.img` descritto come "Android sparse image" o "Linux ext4 filesystem".

- [ ] **Step 3: Convertire ed estrarre**

```bash
simg2img system.img system.raw.img   # solo se sparse
mkdir -p system && "$(brew --prefix e2fsprogs)/sbin/debugfs" -R "rdump / system" system.raw.img
find system -name '*.apk' > apk-list.txt && wc -l apk-list.txt
```

Expected: decine di APK (un risultato di 0 indica estrazione fallita: fermarsi e diagnosticare).

- [ ] **Step 4: Individuare il lato companion**

```bash
grep -iE 'companion|bluetooth|setup|home|nav|notif|screencast|sync|glass' apk-list.txt
```

Annotare in `docs/sources.md` gli APK candidati con package (via `apk-info.sh`) e motivo della scelta.

- [ ] **Step 5: Commit**

```bash
bash tools/check-clean-room.sh && git add docs/sources.md && git commit -m "docs: XE24 system image provenance and companion-side APKs"
```

---

### Task 4: Spike — emulatore XE24 e strategia di test senza hardware

**Files:**
- Create: `docs/emulator-feasibility.md`
- Modify: `docs/progress.md`

**Interfaces:**
- Consumes: `_re/xe24/` dal Task 3.
- Produces: raccomandazione su come testare il lato occhiali in Fase 4 (letta dal Task 10 e dalla Fase 3).

Timebox: **2 ore**. Lo spike è un'indagine: il materiale prodotto è usa e getta e resta in `_re/emulator/`.

- [ ] **Step 1: Cercare lavori esistenti**

Cercare progetti che abbiano avviato immagini Glass XE in emulatore (es. "Glass emulator XE system image", "GDK emulator"). Annotare link e stato, senza assumere che funzionino.

- [ ] **Step 2: Prova A — immagine XE24 su AVD API 19**

Valutare se l'emulatore su Apple Silicon può eseguire un'immagine ARMv7 API 19 (`sdkmanager --list | grep 'android-19'`). Se sì, provare ad avviare un AVD API 19 sostituendo `system.img` con quello XE24. Annotare i passi e il risultato (boot sì/no, log di `emulator -verbose -show-kernel`).

- [ ] **Step 3: Prova B — APK di sistema Glass su AVD API 19 stock**

Se la Prova A fallisce: installare gli APK companion lato Glass (Task 3) su un AVD API 19 stock con il GDK add-on, se recuperabile. Annotare quali partono e quali falliscono per dipendenze di sistema.

- [ ] **Step 4: Bluetooth in emulatore**

Verificare e documentare che l'emulatore non espone Bluetooth Classic reale; annotare l'alternativa per i test (trasporto TCP via `adb forward`/`adb reverse` verso un peer simulato in `tools/`).

- [ ] **Step 5: Scrivere `docs/emulator-feasibility.md`**

Sezioni: domanda, cosa è stato provato, risultato di ciascuna prova, raccomandazione (es. "peer simulato in Python + AVD API 19 per l'app ponte" oppure "XE24 in emulatore funziona per X ma non per Y").

- [ ] **Step 6: Commit**

```bash
bash tools/check-clean-room.sh && git add docs/emulator-feasibility.md docs/progress.md && git commit -m "docs: emulator feasibility spike for XE24"
```

---

### Task 5: Decompilazione e indice delle classi

**Files:**
- Local only: `_re/jadx/<nome>/`, `_re/apktool/<nome>/`, `_re/index/<nome>.txt`

**Interfaces:**
- Produces: sorgenti decompilati per l'APK MyGlass e per ogni APK companion lato Glass; indici di stringhe chiave usati nei Task 6–10.

- [ ] **Step 1: Decompilare**

```bash
for apk in _re/apk/*.apk <apk lato Glass dal Task 3>; do n=$(basename "$apk" .apk)
  jadx -d "_re/jadx/$n" --show-bad-code "$apk"
  apktool d -f -o "_re/apktool/$n" "$apk"
done
```

Expected: ogni cartella `_re/jadx/<n>/sources` non vuota.

- [ ] **Step 1b: Analisi assistita con REA** — puntare gli strumenti dell'MCP `rea` agli APK in `_re/apk/` e lato Glass per una prima mappa di componenti, punti di ingresso Bluetooth e classi protobuf; usarla per orientare gli indici sotto, verificando ogni risultato sul codice jadx. Se REA non supporta un APK, annotarlo e proseguire con jadx.

- [ ] **Step 2: Costruire gli indici**

```bash
mkdir -p _re/index
for d in _re/jadx/*; do n=$(basename "$d")
  grep -rhoE '[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}' "$d" | sort -u > "_re/index/$n.uuids.txt"
  grep -rlE 'extends (GeneratedMessage|GeneratedMessageLite|MessageNano)|MessageNano' "$d" > "_re/index/$n.protos.txt"
  grep -rlE 'listenUsingRfcomm|createRfcommSocket|BluetoothServerSocket' "$d" > "_re/index/$n.rfcomm.txt"
  grep -rhoE '"(com\.google\.glass|com\.google\.android\.glass)[A-Za-z0-9_.]*"' "$d" | sort -u > "_re/index/$n.intents.txt"
  grep -rhoE 'https?://[A-Za-z0-9./_-]+' "$d" | sort -u > "_re/index/$n.urls.txt"
done
wc -l _re/index/*
```

- [ ] **Step 3: Annotare in `docs/progress.md`** il riepilogo dei conteggi (non i contenuti) e committare solo `docs/progress.md`.

---

### Task 6: Specifica del protocollo

**Files:**
- Create: `docs/protocol/transport.md`, `docs/protocol/handshake.md`, `docs/protocol/messages.md`, `docs/protocol/proto/*.proto`
- Modify: `docs/protocol/README.md`

**Interfaces:**
- Consumes: indici `_re/index/*` dal Task 5.
- Produces: `.proto` compilabili, usati dalla Fase 3 per decidere tra le opzioni A/B/C.

Regola di citazione: ogni affermazione indica `APK@versione → classe (nome offuscato e, se ricostruibile, ruolo)`.

- [ ] **Step 1: Trasporto** — partendo da `*.rfcomm.txt` e `*.uuids.txt`, documentare in `transport.md`: tipo di socket, UUID e nome del servizio SDP (lato Glass server o client?), pairing, framing (lunghezza/delimitatori, endianness), keep-alive, timeout, riconnessione.
- [ ] **Step 2: Handshake** — in `handshake.md`: sequenza iniziale, campi scambiati, presenza di token/account Google, controlli su firma o identità del companion, comportamento di XE24 dopo l'aggiornamento che ha rimosso l'obbligo dell'account (citare il codice che lo prova).
- [ ] **Step 3: Ricostruire i `.proto`** — dalle classi in `*.protos.txt` ricavare messaggi, numeri di campo, tipi ed enum in `docs/protocol/proto/`. Campi dubbi marcati con commento `// [non verificato]`.
- [ ] **Step 4: Verificare che compilino**

Run: `protoc -I docs/protocol/proto --descriptor_set_out=/dev/null docs/protocol/proto/*.proto`
Expected: nessun output, exit 0.

- [ ] **Step 5: Messaggi e sequenze** — in `messages.md`, per ogni feature la sequenza di messaggi (diagramma testuale) e la gestione degli errori.
- [ ] **Step 6: Commit**

```bash
bash tools/check-clean-room.sh && git add docs/protocol && git commit -m "docs: companion protocol specification and reconstructed protos"
```

---

### Task 7: API interne lato Glass

**Files:**
- Create: `docs/glass-internals.md`

- [ ] **Step 1:** Dai `AndroidManifest.xml` in `_re/apktool/<apk Glass>/` elencare activity, service, receiver e provider esportati, con permessi richiesti (`android:permission`, `protectionLevel`).
- [ ] **Step 2:** Incrociare con `*.intents.txt` per individuare intent/broadcast usati dal companion per navigazione, timeline, notifiche, posizione, screencast.
- [ ] **Step 3:** Per ciascuno, una tabella: nome, scopo, chi lo invia, permesso, **raggiungibile da app di terze parti? (sì / signature|system / root)**, prova (file + riga del manifest).
- [ ] **Step 4:** Commit: `bash tools/check-clean-room.sh && git add docs/glass-internals.md && git commit -m "docs: Glass-side internal APIs"`.

---

### Task 8: Dipendenze cloud

**Files:**
- Create: `docs/cloud-dependencies.md`

- [ ] **Step 1:** Da `*.urls.txt` e dalle classi che li usano, classificare ogni endpoint: funzione (auth, timeline/Mirror, Glassware install, routing nav, contatti, sync media, telemetria), lato (telefono/occhiali).
- [ ] **Step 2:** Per ciascuno: cosa succede oggi se il server non risponde (dal codice), e sostituzione locale proposta o "non sostituibile" con motivo.
- [ ] **Step 3:** Commit: `bash tools/check-clean-room.sh && git add docs/cloud-dependencies.md && git commit -m "docs: Google cloud dependencies and local replacements"`.

---

### Task 9: Prior art

**Files:**
- Create: `docs/prior-art.md`
- Local only: `_re/prior-art/`

- [ ] **Step 1: Clonare**

```bash
mkdir -p _re/prior-art && cd _re/prior-art
for r in thorikawa/GlassBluetoothProtocol thorikawa/GlassRemote thalmiclabs/myo-glass-companion inckie/AnotherGlass; do git clone --depth 1 "https://github.com/$r.git"; done
git clone --depth 1 https://code.nym.sh/kennethnym/aris-old.git || echo "aris-old non clonabile: annotare"
for d in */; do echo "$d: $(ls "$d" | grep -iE '^(license|copying)' || echo 'NESSUNA LICENZA')"; done
```

- [ ] **Step 2:** Per ogni progetto: licenza (e compatibilità con GPL-3.0), cosa copre, ultimo commit, se è coerente con la nostra specifica del Task 6 (confermare/contraddire i nostri UUID e messaggi), cosa manca, cosa è riusabile.
- [ ] **Step 3:** Commit: `bash tools/check-clean-room.sh && git add docs/prior-art.md && git commit -m "docs: prior art analysis"`.

---

### Task 10: Inventario funzionalità e STOP

**Files:**
- Create: `docs/feature-inventory.md`
- Modify: `docs/progress.md`

- [ ] **Step 1:** Elencare **tutte** le feature di MyGlass trovate (almeno: setup/pairing, Wi-Fi, screencast, batteria/info, impostazioni, contatti, foto/video sync, Glassware, GPS, navigazione, notifiche/SMS/chiamate, timeline). Per ciascuna: cosa faceva; classi lato telefono e lato Glass (`APK@versione → classe`); messaggi (link a `docs/protocol/messages.md`); dipendenza cloud (sì/no, link a `cloud-dependencies.md`); come la ricostruiamo (protocollo originale / app ponte / non ricostruibile + alternativa).
- [ ] **Step 2:** Aggiungere una sezione "Verificabilità senza hardware": per ogni feature, se è verificabile con l'approccio del Task 4.
- [ ] **Step 3:** Aggiornare `docs/progress.md`: Fasi 1–2 completate, problemi aperti.
- [ ] **Step 4:** Commit e push

```bash
bash tools/check-clean-room.sh && git add docs && git commit -m "docs: MyGlass feature inventory (phase 2 complete)" && git push
```

- [ ] **Step 5: STOP.** Presentare all'utente il riepilogo di `docs/feature-inventory.md` e attendere conferma prima della Fase 3.
