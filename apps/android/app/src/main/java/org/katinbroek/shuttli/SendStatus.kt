package org.katinbroek.shuttli

import uniffi.shuttli_mobile_ffi.MobileTransferState

/** Only real receipts settle a send. Missing results are never success. */
internal fun sendStatus(states: List<MobileTransferState>): String = when {
    states.isEmpty() -> "transfer_unknown"
    states.any { it == MobileTransferState.QUEUED || it == MobileTransferState.SENDING } -> "send_queued"
    states.all { it == MobileTransferState.APPLIED } -> "transfer_applied"
    states.any { it == MobileTransferState.APPLIED } -> "send_partial"
    states.any { it == MobileTransferState.UNKNOWN } -> "transfer_unknown"
    else -> "transfer_failed"
}
