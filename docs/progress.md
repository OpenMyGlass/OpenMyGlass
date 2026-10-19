# Progress

Piano completo del progetto: [`project-prompt.md`](project-prompt.md).

## Stato delle fasi

| Fase | Stato |
|---|---|
| 0 — Inventario strumenti e impostazione repo | ✅ completata (2026-10-19); `tools/check-tools.sh` verifica, `tools/check-clean-room.sh` blocca materiale Google |
| 1 — Recupero binari (MyGlass APK, immagine XE24) | ⏳ da pianificare |
| 2 — Reverse engineering e mappatura | ⏳ — **stop: revisione inventario funzionalità** |
| 3 — Architettura | ⏳ — **stop: approvazione architettura** |
| 4 — Implementazione Android | ⏳ |
| 5 — Piano iOS | ⏳ (solo su richiesta) |

## Fase 0 — Inventario strumenti (macOS, 2026-10-19)

| Strumento | Stato | Note |
|---|---|---|
| git, gh | ✅ | gh autenticato |
| Java (OpenJDK 21) | ✅ | |
| Python 3, Homebrew | ✅ | |
| Android SDK | ✅ | `~/Library/Android/sdk` (platform-tools, build-tools, emulator, system-images); `adb` non nel PATH (gli script usano il path completo) |
| jadx | ✅ 1.5.6 | brew |
| apktool | ✅ 3.0.3 | brew |
| Ghidra | ❌ | opzionale, solo per codice nativo |
| gradle, kotlin | ❌ | arriveranno via Gradle wrapper in Fase 4 |
| protoc | ✅ libprotoc 36.2 | brew `protobuf` |
| e2fsprogs (debugfs), simg2img, 7z | ✅ | brew; per estrarre l'immagine XE24 |
| apksigner, aapt | ✅ | build-tools 37.0.0 |
| REA MCP (`rea-agents` 6.7.0) | ✅ | registrato a livello di progetto (`claude mcp add --scope local`); usa JADX/Ghidra come provider |
| GDK add-on (API 19) | ❌ | da recuperare/verificare in Fase 4 per `glass-xe/` |

## Fase 2 — Decompilazione (2026-10-19)

Decompilati con jadx 1.5.6 e apktool 3.0.3 in `_re/` (non versionato). Gli indici in `_re/index/` contano file o stringhe, non i contenuti:

| Target | .java | UUID | classi proto | RFCOMM | intent Glass | URL | Wearable API |
|---|---|---|---|---|---|---|---|
| MyGlass 3.5.8 (telefono) | 3780 | 9 | 134 | 7 | 155 | 39 | 81 |
| MyGlassWear (occhiali) | 2806 | 7 | 62 | 4 | 153 | 63 | 299 |
| GlassBluetooth | 943 | 7 | 29 | 9 | 114 | 15 | 0 |
| GlassHome | 2385 | 7 | 64 | 4 | 197 | 51 | 243 |
| GlassLiteSetup | 577 | 6 | 13 | 4 | 18 | 5 | 0 |
| GlassSync | 2293 | 7 | 62 | 4 | 150 | 51 | 239 |
| GlassPhone | 1976 | 7 | 27 | 4 | 99 | 50 | 176 |
| GlassRemoteIme | 1285 | 7 | 105 | 4 | 70 | 5 | 0 |
| GlassSettings | 2723 | 7 | 65 | 4 | 227 | 57 | 261 |
| GlassVoice | 2737 | 9 | 99 | 4 | 145 | 65 | 285 |
| GlassCamera | 2321 | 7 | 61 | 4 | 169 | 51 | 240 |
| glass-core.jar | 142 | 0 | 0 | 1 | 43 | 0 | 0 |
| glass-services.jar | 83 | 0 | 0 | 0 | 18 | 0 | 0 |

I conteggi simili tra le app Glass (UUID 7, RFCOMM 4) suggeriscono una libreria comune inclusa in ciascuna.

## Decisioni

- 2026-10-19 — Licenza **GPL-3.0**.
- 2026-10-19 — Repo pubblica; workspace di reverse in `_re/` escluso da git.
- 2026-10-19 — **Nessun hardware Glass disponibile per ora.** Si parte con emulatore/simulatore: valutare se l'immagine XE24 è avviabile in un emulatore Android (API 19) e, in ogni caso, peer simulati lato occhiali in `tools/`. I test con hardware (adb, HCI snoop) restano pianificati ma rinviati.

## Problemi aperti

- Emulatore Glass XE24 **non fattibile** su questo Mac (vedi [`emulator-feasibility.md`](emulator-feasibility.md)). Test senza hardware tramite peer simulato su TCP e AVD arm64 con stub GDK.
- GDK add-on assente dal repository SDK ufficiale: da recuperare per la Fase 4.
- `MyGlassWear.apk` (lato occhiali) è costruito sul framework Android Wear/clockwork: verificare nel Task 6 quanto il protocollo dipenda da Google Play services Wearable.
