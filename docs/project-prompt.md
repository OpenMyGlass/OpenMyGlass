# Progetto: OpenMyGlass — reverse engineering di MyGlass e companion open source senza cloud Google

## Ruolo e obiettivo

Sei un ingegnere senior di reverse engineering e sviluppo mobile. Il tuo compito è:

1. Recuperare e analizzare l'app Android **MyGlass** (companion ufficiale di Google Glass) e la controparte lato occhiali nel firmware **XE24** della Glass Explorer Edition.
2. Mappare **tutte** le funzionalità native che MyGlass offriva e il modo in cui comunicava con gli occhiali.
3. Progettare e implementare in una nuova repo un companion open source, **prima per Android**, con un'architettura pronta per una **versione iOS successiva**.

Target funzionale:
- **Navigazione**: istruzioni turn-by-turn di **Google Maps** dal telefono visualizzate sugli occhiali.
- **Notifiche complete** del telefono sugli occhiali, con azioni e risposte dove possibile.
- **Gestione dell'occhiale** come faceva MyGlass: configurazione Wi-Fi, screencast, stato batteria, impostazioni, contatti, sincronizzazione foto/video, gestione delle app (Glassware), posizione GPS condivisa.
- **Zero dipendenze dal cloud Google per Glass**: niente Mirror API, niente account Google sugli occhiali, niente servizi server di MyGlass. Tutto locale tra telefono e occhiali. (Le app Google sul telefono, come Google Maps, possono essere usate come sorgente dati.)

Dispositivo target: **Glass Explorer Edition XE-C con firmware XE24** (Android 4.4.2 / API 19, GDK). Enterprise Edition 2 eventualmente in futuro, non ora.

## Passo 0 — Inventario degli strumenti

Prima di tutto elenca i plugin, gli MCP server, le skill e gli strumenti CLI disponibili in questa sessione (es. jadx, apktool, Ghidra o un MCP per Ghidra, adb, gradle, Android SDK, emulatori, strumenti web). Scegli quelli adatti per ogni fase e, se manca qualcosa di essenziale, installalo o dimmi esattamente cosa manca. Non proseguire assumendo strumenti che non hai verificato.

## Fase 1 — Recupero dei binari

- Recupera l'APK Android di MyGlass. Verifica il package name (dovrebbe essere `com.google.glass.companion`, ma controllalo) e scegli **l'ultima versione** disponibile (2017 o successiva); se utile, anche una versione del 2014 per confronto. Usa fonti affidabili, verifica firma e hash, e annota la fonte nella documentazione.
- Recupera l'immagine di sistema ufficiale **XE24** della Glass Explorer Edition dalla pagina "System and Kernel Downloads" di Google for Developers. Estrai le app e i servizi di sistema che implementano il lato occhiali del protocollo companion (servizi Bluetooth, setup, GlassHome/timeline, navigazione, notifiche, screencast, sincronizzazione media).
- **Non committare** APK, immagini di sistema né codice decompilato. Tienili in una cartella locale esclusa da `.gitignore` (es. `_re/`).

## Fase 2 — Reverse engineering e mappatura

Decompila (jadx/apktool) e analizza entrambi i lati. Produci documentazione, non copia di codice:

1. **Inventario funzionalità** (`docs/feature-inventory.md`): per ogni feature di MyGlass indica cosa faceva, quali classi la implementano su entrambi i lati, quali messaggi scambia, se dipendeva dal cloud Google, e come la ricostruiremo (o perché no).
2. **Specifica del protocollo** (`docs/protocol/`): trasporto (attesa: Bluetooth Classic RFCOMM, da verificare), UUID/SDP, pairing, handshake e autenticazione, framing, definizioni dei messaggi (probabilmente protobuf: ricostruisci i `.proto` a partire dalle classi generate), sequenze per ogni feature, gestione errori, keep-alive e riconnessioni. Indica in particolare se l'handshake richiede token o account Google e come si comporta il firmware XE24 dopo l'aggiornamento finale che ha rimosso l'obbligo dell'account.
3. **API interne lato Glass** (`docs/glass-internals.md`): intent, broadcast, content provider e servizi di sistema che il companion usava per pilotare le funzioni native (navigazione, timeline, notifiche, posizione, screencast). Indica cosa è raggiungibile da un'app di terze parti con API GDK/Android e cosa richiede permessi di sistema o root.
4. **Dipendenze cloud** (`docs/cloud-dependencies.md`): tutto ciò che passava da server Google (es. calcolo dei percorsi per la navigazione nativa, sincronizzazione timeline, installazione Glassware) e come sostituirlo in locale.

Studia anche i progetti esistenti e riusa ciò che è riusabile rispettando le licenze:
- `thorikawa/GlassBluetoothProtocol` e `thorikawa/GlassRemote`: reverse del protocollo companion Bluetooth.
- `thalmiclabs/myo-glass-companion`: uso del protocollo companion da un telefono Android per controllare l'interfaccia nativa di Glass.
- `inckie/AnotherGlass`: companion Android open source senza servizi Google (GPS, notifiche, Wi-Fi), con protocollo proprio e app lato occhiali.
- IrisGlass (`code.nym.sh/kennethnym/aris-old`, cartella `IrisGlass`): app XE24 che fa da BLE central verso un iPhone, con LiveCard GDK. Utile per la futura fase iOS.

Riassumi in `docs/prior-art.md` cosa coprono, cosa funziona ancora e cosa manca.

**Fermati qui e mostrami l'inventario delle funzionalità prima di procedere.**

## Fase 3 — Architettura (fermati e chiedimi conferma)

Scrivi `docs/architecture.md` con le opzioni e una raccomandazione motivata. Punti da valutare esplicitamente:

### Strategia di comunicazione su Android
- **A. Emulazione del protocollo originale:** il companion Android impersona MyGlass via Bluetooth Classic RFCOMM e parla con il servizio companion già presente nel firmware. Vantaggio: funzioni native di Glass (navigazione, timeline, setup Wi-Fi, screencast) potenzialmente **senza installare app sugli occhiali**. Da verificare: handshake, eventuali controlli di identità/firma, feature che dipendevano da risposte del cloud.
- **B. Protocollo proprio + app ponte sugli occhiali** (stile AnotherGlass): massima libertà, UI ricostruita con GDK (LiveCard, CardBuilder, gesti, voice trigger).
- **C. Ibrido:** protocollo originale per ciò che funziona nativamente, app ponte per ciò che manca o dipendeva dal cloud.

### Astrazione per il futuro iOS
Separa nettamente il **livello trasporto** dalla **logica applicativa** e dal **modello dei messaggi**:
- interfaccia `Transport` con implementazioni intercambiabili (RFCOMM, BLE, Wi-Fi/TCP);
- modello dei messaggi versionato (`.proto` o schema equivalente) usato da tutti i client;
- logica delle feature (notifiche, navigazione, gestione) indipendente dal trasporto.

Motivo: un'app iOS **non può** aprire connessioni Bluetooth Classic RFCOMM verso dispositivi non certificati MFi, quindi la versione iOS userà BLE e/o Wi-Fi e un'app ponte sugli occhiali, riusando lo stesso modello dei messaggi.

### Notifiche su Android
- `NotificationListenerService` per leggere le notifiche di tutte le app: titolo, testo, app, icona, categoria, priorità, azioni.
- Esecuzione delle azioni (`PendingIntent`) e **risposte** tramite `RemoteInput`, con dettatura vocale dagli occhiali.
- Filtri per app, notifiche silenziose, raggruppamento, aggiornamento e rimozione sincronizzati con gli occhiali.

### Navigazione Google Maps su Android
Valuta e documenta queste opzioni, in ordine di preferenza:
1. **Lettura della notifica di navigazione di Google Maps** tramite `NotificationListenerService`: durante la navigazione Maps pubblica una notifica persistente con prossima manovra, distanza, via ed eventualmente un'icona della manovra. Estrai i dati (testo e bitmap dell'icona) in modo robusto alle varianti di lingua e versione, e inviali agli occhiali in una card di navigazione ottimizzata per 640×360. Documenta i campi effettivamente disponibili.
2. **Alimentare la navigazione nativa di Glass** tramite il protocollo originale, se il reverse dimostra che è possibile senza il calcolo del percorso lato cloud.
3. **Mirror dello schermo con MediaProjection**, ritagliando l'area della mappa e scalandola per gli occhiali. Modalità sperimentale: valuta latenza, banda Wi-Fi e consumo batteria.

In ogni caso inoltra la **posizione GPS** del telefono agli occhiali (che non hanno GPS) come mock location, in modo che le app sugli occhiali la usino.

**Fermati e chiedimi conferma dell'architettura prima di implementare.**

## Fase 4 — Implementazione Android

Struttura proposta della repo (adattala all'architettura approvata):

```
/docs                 documentazione di reverse e architettura
/protocol             definizioni dei messaggi (.proto o schema), versionate
/core                 logica delle feature indipendente dal trasporto
/android              app companion Android
/glass-xe             app ponte GDK per Explorer XE24 (se prevista)
/tools                peer simulati, harness di test, script adb
/ios                  (fase successiva, non ora)
```

Requisiti:
- **App Android**: Kotlin, target SDK attuale, minSdk ragionevole (indica la scelta), foreground service di tipo `connectedDevice`/`location`, gestione corretta dei permessi runtime (Bluetooth Android 12+, notifiche, posizione, accesso alle notifiche), ottimizzazione batteria esclusa su richiesta dell'utente, riconnessione automatica.
- **UI Android**: pairing e stato connessione, batteria e info occhiali, configurazione Wi-Fi, filtri notifiche, navigazione, screencast, galleria foto/video sincronizzati, impostazioni, gestione app sugli occhiali.
- **App sugli occhiali** (se prevista): compatibile con API 19 e GDK; scegli una toolchain che compili davvero per il GDK add-on e documentala. Servizio persistente, consumo minimo, UI coerente con lo stile Glass.
- **Test senza hardware**: peer simulato per entrambi i lati, test unitari su serializzazione, state machine della connessione e parsing delle notifiche di Google Maps (con notifiche di esempio), test di integrazione end-to-end con emulatore.
- **Test con hardware**: script adb per installare, leggere i log (`logcat`) e catturare il traffico Bluetooth (HCI snoop log del telefono) per confrontarlo con la specifica.

Ordine di sviluppo:
1. Pairing, connessione, handshake, keep-alive.
2. Stato batteria e info dispositivo.
3. Notifiche (lettura, visualizzazione, rimozione), poi azioni e risposte.
4. Posizione GPS condivisa.
5. Navigazione Google Maps.
6. Configurazione Wi-Fi.
7. Sincronizzazione foto/video.
8. Screencast.
9. Gestione app e impostazioni.
10. Mirror MediaProjection sperimentale.

## Fase 5 — iOS (solo dopo che Android è stabile, e solo se te lo chiedo)

Prepara solo `docs/ios-plan.md` con il piano, senza implementare finché non lo chiedo:
- Trasporto BLE (CoreBluetooth) e/o Wi-Fi locale verso l'app ponte sugli occhiali, riusando `/protocol` e la logica di `/core` dove possibile.
- Notifiche tramite **ANCS** (Apple Notification Center Service) lato occhiali: l'app iOS non può leggere le notifiche delle altre app; ANCS permette azioni positive/negative ma non risposte testuali libere.
- Navigazione: iOS non espone le istruzioni di Google Maps o Apple Maps ad altre app. Opzioni: routing proprio con MapKit (`MKDirections`, `MKMapSnapshotter`), mirror sperimentale con ReplayKit Broadcast Upload Extension, notifiche di navigazione via ANCS se presenti.
- Limiti del background iOS e cosa funziona con app chiusa.

## Regole

- Lavora per fasi e aggiorna `docs/progress.md` con stato, decisioni e problemi aperti.
- Committa spesso con messaggi chiari. Nessun binario o codice di Google nella repo: solo documentazione e codice originale (approccio clean-room, la specifica in `docs/` fa da ponte tra analisi e implementazione).
- Quando una feature non è ricostruibile (permessi di sistema, firma, dipendenza cloud non sostituibile), dillo esplicitamente e proponi l'alternativa migliore invece di simularla.
- Se un'informazione è incerta, verificala nel codice decompilato o sul dispositivo; non inventare UUID, campi protobuf, intent o formati di notifica.
- Fermati e chiedimi conferma: dopo la Fase 2 (inventario) e dopo la Fase 3 (architettura).
