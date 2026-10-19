package org.openmyglass.core.nav

import org.junit.jupiter.api.Assertions.assertEquals
import org.junit.jupiter.api.Test

class OsmAndNotificationParserTest {

    private fun n(bigText: String?) = NotificationText(title = null, text = null, subText = null, bigText = bigText, iconSha256 = null)

    @Test
    fun `instruction and eta from big text`() {
        assertEquals(
            OsmAndNotificationParser.Extra("Turn right and go Obergass", "8:17 PM"),
            OsmAndNotificationParser.parse(n("Turn right and go Obergass 60 m\n2.9 km • 5 min • 8:17 PM • 0 km/h")),
        )
        assertEquals(
            OsmAndNotificationParser.Extra("Take 2 exit and go Herrengasse toward Feldkirch, Schaan", "8:18 PM"),
            OsmAndNotificationParser.parse(n("Take 2 exit and go Herrengasse toward Feldkirch, Schaan 1.8 mi\n2.0 mi • 7 min • 8:18 PM • 0 mph")),
        )
    }

    @Test
    fun `missing big text gives nothing`() {
        assertEquals(OsmAndNotificationParser.Extra(null, null), OsmAndNotificationParser.parse(n(null)))
        assertEquals(OsmAndNotificationParser.Extra("Arrived", null), OsmAndNotificationParser.parse(n("Arrived")))
    }

    @Test
    fun `stats-only text near the destination gives only the eta`() {
        // Captured at the end of a route: OsmAnd drops the instruction line
        assertEquals(OsmAndNotificationParser.Extra(null, "22:23"), OsmAndNotificationParser.parse(n("100 m • <1 min • 22:23 • 0 km/h")))
    }
}
