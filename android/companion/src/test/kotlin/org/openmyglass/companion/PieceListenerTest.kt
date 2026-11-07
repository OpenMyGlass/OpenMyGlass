package org.openmyglass.companion

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class PieceListenerTest {
    @Test
    fun aLateStopOfAnEarlierPieceDoesNotEndThisOne() {
        val piece = TtsPiece()
        val listener = PieceListener("piece-2", piece)
        listener.onStop("piece-1", true)
        listener.onError("piece-1", -1)
        listener.onDone("piece-1")
        assertEquals(1L, piece.done.count)
        assertNull(piece.error.get())

        listener.onDone("piece-2")
        assertEquals(0L, piece.done.count)
        assertNull(piece.error.get())
    }

    @Test
    fun aStopOfThisPieceFailsIt() {
        val piece = TtsPiece()
        PieceListener("piece-7", piece).onStop("piece-7", true)
        assertEquals(0L, piece.done.count)
        assertEquals("stopped", piece.error.get())
    }
}
