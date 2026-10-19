# OpenMyGlass

Companion open source per **Google Glass Explorer Edition (XE24)**, senza cloud Google.
Ricostruisce in chiave clean-room le funzioni dell'app MyGlass: navigazione Google Maps sugli occhiali,
notifiche complete del telefono, configurazione Wi-Fi, screencast, batteria, sincronizzazione foto/video,
gestione Glassware e GPS condiviso. Prima Android, con architettura pronta per iOS.

> Stato: **Fase 0 – impostazione**. Vedi [`docs/progress.md`](docs/progress.md).

## Struttura

| Cartella | Contenuto |
|---|---|
| `docs/` | Documentazione di reverse engineering, protocollo e architettura |
| `protocol/` | Definizioni dei messaggi (`.proto` o schema), versionate |
| `core/` | Logica delle feature indipendente dal trasporto |
| `android/` | App companion Android |
| `glass-xe/` | App ponte GDK per Explorer XE24 (se prevista dall'architettura) |
| `tools/` | Peer simulati, harness di test, script adb |

## Approccio clean-room

Nessun binario, immagine di sistema o codice decompilato di Google viene committato.
L'analisi avviene in una cartella locale `_re/` (esclusa da git); la specifica in `docs/`
fa da ponte tra analisi e implementazione, che è codice originale.

## Licenza

[GPL-3.0](LICENSE)
