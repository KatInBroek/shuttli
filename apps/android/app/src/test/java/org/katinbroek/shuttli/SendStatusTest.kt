package org.katinbroek.shuttli

import org.junit.Assert.assertEquals
import org.junit.Test
import uniffi.shuttli_mobile_ffi.MobileTransferState.*

class SendStatusTest {
    @Test fun completedReceiptsReplaceSending() {
        assertEquals("send_queued", sendStatus(listOf(QUEUED)))
        assertEquals("send_queued", sendStatus(listOf(APPLIED, SENDING)))
        assertEquals("transfer_applied", sendStatus(listOf(APPLIED, APPLIED)))
    }
    @Test fun uncertaintyAndPartialDeliveryCannotBecomeSuccess() {
        assertEquals("transfer_unknown", sendStatus(emptyList()))
        assertEquals("transfer_unknown", sendStatus(listOf(FAILED, UNKNOWN)))
        assertEquals("transfer_failed", sendStatus(listOf(FAILED)))
        assertEquals("send_partial", sendStatus(listOf(APPLIED, FAILED)))
        assertEquals("send_partial", sendStatus(listOf(APPLIED, UNKNOWN)))
    }
}
