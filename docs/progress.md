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
| GDK add-on (API 19) | ❌ | da recuperare/verificare in Fase 4 per `glass-xe/` |

## Decisioni

- 2026-10-19 — Licenza **GPL-3.0**.
- 2026-10-19 — Repo pubblica; workspace di reverse in `_re/` escluso da git.

## Problemi aperti

- Nessuno.
