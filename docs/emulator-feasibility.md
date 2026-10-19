# Spike: emulare Glass XE24 senza hardware

**Domanda:** possiamo eseguire il lato occhiali di XE24 (sistema intero o le sue app companion) in un emulatore su questo Mac (Apple Silicon, arm64), per testare il companion senza Glass?

**Risposta breve:** **no**, né il sistema XE24 completo né le app Glass di sistema girano qui in modo utile. Per lo sviluppo useremo un **peer simulato** e, se l'architettura prevede un'app ponte, un **AVD arm64 con display 640×360 e stub GDK**. Il comportamento del firmware originale si verifica solo leggendo il codice decompilato finché non c'è hardware.

Data: 2026-10-19. Ambiente: macOS arm64, Android Emulator 36.5.11, cmdline-tools 9862592. Tutto il materiale di prova è in `_re/emulator/` ed è usa e getta.

## Cosa è stato provato

### A. Immagine XE24 o AVD API 19 ARM standard

- XE24 è **ARMv7 a 32 bit** (OMAP4, `board=glass_1`; le librerie in `system/lib` sono ELF 32-bit ARM EABI5) con HAL OMAP, non goldfish.
- Prima del trapianto ho verificato se gira l'immagine ufficiale `system-images;android-19;default;armeabi-v7a`. L'avvio fallisce subito:
  `FATAL | CPU Architecture 'arm' is not supported by the QEMU2 emulator, (the classic engine is deprecated!)`.
- Il binario `qemu-system-armel` esiste nell'SDK, ma l'emulatore rifiuta le immagini ARM32 con QEMU2. Il motore classic, l'unico che le eseguiva, è stato rimosso.
- **Esito:** bloccata alla radice. Se il sistema API 19 base non si avvia, non si avvia neanche XE24 trapiantato.

### B. App Glass di sistema su un AVD moderno (API 28 arm64-v8a)

- Gli APK in `system/priv-app` hanno META-INF che dichiara una firma v2 assente (`INSTALL_PARSE_FAILED_NO_CERTIFICATES`). Si installano solo dopo averli rifirmati con una chiave di debug.
- `GlassBluetooth` usa `sharedUserId=android.uid.system`, quindi senza la chiave di piattaforma non si installa (`INSTALL_FAILED_SHARED_USER_INCOMPATIBLE`).
- `MyGlassWear` e `GlassHome` includono librerie native solo `lib/armeabi`, mentre l'AVD espone solo `arm64-v8a`: `INSTALL_FAILED_NO_MATCHING_ABIS`. Apple Silicon non esegue codice ARM a 32 bit, quindi nessun AVD arm64 su questo Mac offre quell'ABI.
- `MyGlassWear` senza librerie native si installa, ma all'avvio di qualsiasi componente va in crash: `NoClassDefFoundError: com/google/android/glass/hidden/HiddenSystemProperties`, perché manca il framework Glass (`glass-core.jar`, `glass-services.jar`, classi `com.google.android.glass.*`).
- **Esito:** il framework Glass e il codice nativo ARM32 sono dipendenze dure, e ricostruirle in emulatore costerebbe più dell'hardware.

### C. GDK add-on

- Il pacchetto GDK (`addon-google_gdk-google-19`) **non è più nel repository ufficiale dell'SDK** (`addon2-1.xml`). Va recuperato altrove (archivi, mirror) per compilare l'eventuale app ponte; da affrontare in Fase 4.

## Scoperta laterale importante

I componenti registrati da `MyGlassWear.apk` (XE24, package `com.google.glass.companion`) appartengono al **framework Android Wear / clockwork**, per esempio `com.google.android.clockwork.stream.bridger.NotificationBridgerService`, `com.google.android.clockwork.host.DispatchingWearableListenerService` e `com.google.android.wearable.app.RemoteActionReceiver`. Le notifiche nelle ultime versioni potrebbero quindi passare dal **Wearable Data Layer di Google Play services** e non da un protocollo Glass dedicato. Da verificare nel Task 6: incide direttamente sull'opzione A dell'architettura.

## Raccomandazione per i test senza hardware

1. **Peer simulato lato occhiali** in `tools/` (Python o JVM) che parla il nostro modello dei messaggi su **TCP**. Il telefono emulato lo raggiunge con `adb reverse`/`adb forward`, e il trasporto TCP sostituisce RFCOMM, che l'emulatore non offre: nessun AVD ha Bluetooth Classic reale.
2. **App ponte su AVD arm64** con profilo display 640×360 e **stub delle classi GDK** (`CardBuilder`, `LiveCard`, …) per UI e logica, se la Fase 3 sceglie l'opzione B o C. Il codice dell'app ponte va scritto senza librerie native ARM32.
3. **Emulazione del protocollo originale (opzione A):** verificabile solo con hardware o, in parte, rieseguendo nel peer simulato le sequenze ricavate dal codice decompilato. Non è un test del firmware vero.
4. **Alternativa non provata:** un host Linux x86_64 con un vecchio emulatore che ha ancora il motore classic potrebbe avviare API 19 ARM. Il trapianto di XE24 avrebbe comunque i problemi di HAL OMAP. Sconsigliato salvo necessità forte.
