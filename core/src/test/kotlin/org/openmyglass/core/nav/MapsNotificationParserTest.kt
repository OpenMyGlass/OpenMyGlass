package org.openmyglass.core.nav

import org.junit.jupiter.api.Assertions.assertEquals
import org.junit.jupiter.api.Assertions.assertNull
import org.junit.jupiter.api.Test

class MapsNotificationParserTest {

    private fun n(title: String?, text: String?, subText: String?, icon: String? = null) =
        NotificationText(title, text, subText, bigText = null, iconSha256 = icon)

    @Test
    fun `english car notification`() {
        val u = MapsNotificationParser.parse(n("200 m", "28 / Landstrasse", "2 min · 1.3 km · 8:33 PM ETA"))!!
        assertEquals(200, u.distanceMeters)
        assertEquals("Landstrasse", u.street)
        assertEquals("8:33 PM", u.eta)
        assertEquals("gmaps", u.source)
        assertNull(u.instruction)
    }

    @Test
    fun `italian toward street without distance`() {
        val u = MapsNotificationParser.parse(n("", "verso Äulestrasse", "24 min · 1,6 km · Arrivo 20:56"))!!
        assertNull(u.distanceMeters)
        assertEquals("Äulestrasse", u.street)
        assertEquals("20:56", u.eta)
    }

    @Test
    fun `english toward street`() {
        assertEquals("Im Mühleholz", MapsNotificationParser.parse(n("0 m", "toward Im Mühleholz", "3 min · 1.4 km · 8:33 PM ETA"))!!.street)
    }

    @Test
    fun `road reference only is the street`() {
        assertEquals("28", MapsNotificationParser.parse(n("180 m", "28", "5 min · 3,2 km · Arrivo 21:16"))!!.street)
    }

    @Test
    fun `sentences become the instruction`() {
        val reroute = MapsNotificationParser.parse(n("", "Rielaborazione percorso...", ""))!!
        assertEquals("Rielaborazione percorso...", reroute.instruction)
        assertNull(reroute.street)
        val uturn = MapsNotificationParser.parse(n("10 m", "Fai una inversione a U", "5 min · 3,2 km · Arrivo 21:16"))!!
        assertEquals("Fai una inversione a U", uturn.instruction)
        val starting = MapsNotificationParser.parse(n("Avvio della navigazione in corso…", null, null))!!
        assertEquals("Avvio della navigazione in corso…", starting.instruction)
        assertNull(starting.distanceMeters)
    }

    @Test
    fun `known icon hash gives the maneuver and the hash is kept`() {
        val hash = "13b4feacacd89ee060f297a714a5ed9a09a929d50e7bb10f71e7689f195e96c1"
        val u = MapsNotificationParser.parse(n("10 m", "Fai una inversione a U", "x", hash))!!
        assertEquals(Maneuver.U_TURN_LEFT, u.maneuver)
        assertEquals(hash, u.iconSha256)
        assertEquals(Maneuver.UNKNOWN, MapsNotificationParser.parse(n("10 m", "28", "x", "ff".repeat(32)))!!.maneuver)
    }

    @Test
    fun `all blank means no update`() {
        assertNull(MapsNotificationParser.parse(n("", "", "")))
        assertNull(MapsNotificationParser.parse(n(null, null, null)))
    }

    @Test
    fun `every captured fixture parses`() {
        for (file in listOf("gmaps-car-en.jsonl", "gmaps-walk-it.jsonl", "gmaps-car-it-icons.jsonl")) {
            val lines = javaClass.getResource("/nav-fixtures/$file")!!.readText().lines().filter { it.isNotBlank() }
            lines.forEach { line ->
                fun field(key: String) = Regex("\"" + Regex.escape(key) + "\":\\s*\"((?:\\\\.|[^\"\\\\])*)\"")
                    .find(line)?.groupValues?.get(1)?.replace("\\u00a0", " ")
                MapsNotificationParser.parse(n(field("android.title"), field("android.text"), field("android.subText"), field("sha256")))
            }
        }
    }
}
