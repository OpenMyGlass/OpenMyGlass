package org.openmyglass.core.companion

import org.junit.jupiter.api.Assertions.assertEquals
import org.junit.jupiter.api.Test

class CompanionConstantsTest {

    @Test
    fun `version matches XE24`() {
        assertEquals(655366, CompanionConstants.calculateVersion(6, 10))
        assertEquals(720902, CompanionConstants.calculateVersion(6, 11))
        assertEquals(6, CompanionConstants.majorOf(655366))
        assertEquals(10, CompanionConstants.minorOf(655366))
        assertEquals(655366, CompanionConstants.VERSION)
    }

    @Test
    fun `uuids match the spec`() {
        assertEquals("F15CC914-E4BC-45CE-9930-CB7695385850", CompanionConstants.SECURE_UUID.toString().uppercase())
        assertEquals("DC935393-831A-4736-89E3-C8CE5BB20EEA", CompanionConstants.PING_SOCKET_UUID.toString().uppercase())
        assertEquals("Companion", CompanionConstants.SOCKET_NAME)
        assertEquals("PingCompanion", CompanionConstants.PING_SOCKET_NAME)
    }
}
