# Fonti dei binari analizzati

I binari restano solo in `_re/` (escluso da git). Questo file registra da dove vengono e come sono stati verificati.

## MyGlass (lato telefono)

| Campo | Valore |
|---|---|
| File locale | `_re/apk/myglass-3.5.8.apk` |
| Fonte | https://archive.org/download/myglass-3.5.8-android/myglass-3.5.8.apk (item `myglass-3.5.8-android`, caricato da terzi il 2026-03-03) |
| Scaricato | 2026-10-19 |
| Package | `com.google.glass.companion` (verificato con `aapt`) |
| Versione | versionName `3.5.8`, versionCode `5331` |
| SDK | minSdk 15, targetSdk 22, compilato con platform 26 (O) |
| SHA-1 file | `95ec430bc6ed4bc96fa90484fd3704381b8f90bc` (coincide con lo SHA-1 pubblicato da archive.org) |
| SHA-256 file | `4f8ae50dbec42b79c67c95a155df9744a3bccc67c46d19d8a5ff2eb60ba2dcd2` |
| Firma | v1 + v2 valide (`apksigner verify`) |
| Certificato DN | `CN=Unknown, OU="Google, Inc", O="Google, Inc", L=Mountain View, ST=CA, C=US` |
| Certificato SHA-1 | `24bb24c05e47e0aefa68a58a766179d9b613a600` |
| Certificato SHA-256 | `3d7a1223019aa39d9ea0e3436ab7c0896bfb4fb679f4de5fe7c23f326c8f994a` |

**Perché ci fidiamo:** l'APK proviene da un caricamento di terzi, ma la firma è valida e il certificato
SHA-1 `24bb24c0…a600` è la chiave di firma delle app Google ufficiali (la stessa di YouTube, Google Foto,
Google Translate, come risulta dalle pagine di download di APKPure). La dimensione (12.674.492 byte) coincide
con quella indicata da APKMirror per la 3.5.8 (caricata lì il 2017-06-19). APKMirror stesso blocca i download
automatici (HTTP 403).

**Versione 2014 per confronto:** non recuperata. APKMirror (3.4.0 del 2014-11-05, 3.3.0, 2.2.0) richiede un
browser. Opzionale secondo il piano; da recuperare manualmente se l'analisi della 3.5.8 lascia dubbi.

## Immagine di sistema Glass Explorer Edition XE24 (lato occhiali)

| Campo | Valore |
|---|---|
| Pagina ufficiale | https://developers.google.com/glass/tools-downloads/system |
| Factory image | https://storage.googleapis.com/support-kms-prod/bTh25b2gcZx5f7apQdJU3lULYTTBoZDHqdsr → `_re/xe24/signed-glass_1-img-5585826.zip` |
| Scaricato | 2026-10-19 |
| Dimensione | 442.720.482 byte |
| SHA-1 | `46430bc827267796566f0a73d9b503059759561c` — **coincide** con il checksum ufficiale |
| SHA-256 | `76a9d3822077e993bba62c586e5e7cbb1217a81171d78033ff7c06ba285f2363` |
| Boot image | https://storage.googleapis.com/glass_gfw/glass_1-img-5585826/boot.img — il download restituisce 391 byte (non è un'immagine; SHA-1 non coincide con l'ufficiale `4e582d8d…`). Si usa il `boot.img` contenuto nella factory image. |
| Sorgenti kernel | https://android.googlesource.com/kernel/omap/+/glass-omap-xrr02 |
