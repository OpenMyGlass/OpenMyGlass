# OsmAnd AIDL API (vendored)

- **Upstream:** https://github.com/osmandapp/OsmAnd/tree/master/OsmAnd-api
- **Commit:** `cfe331c0432312488cdde35556716cd12f690f41` (2026-10-19)
- **What is copied:** `OsmAnd-api/src/**` (AIDL interfaces and parcelable Java classes), unchanged. The upstream `res/` folder is **not** copied: OsmAnd's UI/UX artwork is CC-BY-NC-ND 4.0.
- **License:** OsmAnd source code is covered by the **GNU GPL v3** (upstream `LICENSE`: "The code in this repository is covered by GPLv3 (for code)"; the GPL text it quotes says "either version 3 of the License, or (at your option) any later version"). That is compatible with OpenMyGlass (GPL-3.0). The individual files carry no headers; the repository license applies.
- **Copyright:** © 2010–2026 OsmAnd BV and contributors.
- **Why:** OpenMyGlass uses OsmAnd's public AIDL service (`net.osmand.aidl.OsmandAidlServiceV2`) to receive navigation updates (`registerForNavigationUpdates`, `registerForVoiceRouterMessages`). See `docs/navigation-sources.md`.

To update: copy `OsmAnd-api/src` from a newer upstream commit and update the commit hash above.
