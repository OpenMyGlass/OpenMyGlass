# Progress

Piano completo del progetto: [`project-prompt.md`](project-prompt.md).

## Stato delle fasi

| Fase | Stato |
|---|---|
| 0 — Inventario strumenti e impostazione repo | ✅ completata (2026-10-19) |
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
| Android SDK | ✅ | `~/Library/Android/sdk` (platform-tools, build-tools, emulator, system-images); `adb` non nel PATH |
| jadx | ❌ | `brew install jadx` — necessario in Fase 2 |
| apktool | ❌ | `brew install apktool` — necessario in Fase 2 |
| Ghidra | ❌ | opzionale, solo per codice nativo |
| gradle, kotlin | ❌ | arriveranno via Gradle wrapper in Fase 4 |
| protoc | ❌ | `brew install protobuf` — se lo schema sarà protobuf |
| REA MCP (`rea-agents` 6.7.0) | ✅ | registrato a livello di progetto (`claude mcp add --scope local`); usa JADX/Ghidra come provider |
| GDK add-on (API 19) | ❌ | da recuperare/verificare in Fase 4 per `glass-xe/` |

## Decisioni

- 2026-10-19 — Licenza **GPL-3.0**.
- 2026-10-19 — Repo pubblica; workspace di reverse in `_re/` escluso da git.
- 2026-10-19 — **Nessun hardware Glass disponibile per ora.** Si parte con emulatore/simulatore: valutare se l'immagine XE24 è avviabile in un emulatore Android (API 19) e, in ogni caso, peer simulati lato occhiali in `tools/`. I test con hardware (adb, HCI snoop) restano pianificati ma rinviati.

## Problemi aperti

- Fattibilità di un emulatore Glass XE24 (avvio dell'immagine di sistema in AVD/QEMU, Bluetooth emulato assente → serve un trasporto alternativo per i test, es. TCP).
