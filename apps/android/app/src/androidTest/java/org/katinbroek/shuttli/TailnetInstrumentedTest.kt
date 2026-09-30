package org.katinbroek.shuttli

import android.content.ClipData
import android.content.ClipboardManager
import android.graphics.Bitmap
import android.util.Log
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.lifecycle.Lifecycle
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import org.junit.Assert.*
import org.junit.Assume.assumeTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import uniffi.shuttli_mobile_ffi.MobileHistoryActivity
import uniffi.shuttli_mobile_ffi.MobileContentKind
import uniffi.shuttli_mobile_ffi.MobileTransferState
import java.io.ByteArrayOutputStream
import java.io.File
import java.security.MessageDigest

/** Opt-in real VPN test. The external coordinator controls an isolated desktop profile. */
@RunWith(AndroidJUnit4::class)
class TailnetInstrumentedTest {
    @get:Rule val rule = createAndroidComposeRule<MainActivity>()

    @Test fun clipboardHistoryAndConsentOverRealTailnet() {
        val args = InstrumentationRegistry.getArguments()
        val peerId = args.getString("tailnetPeer") ?: ""
        val token = args.getString("tailnetToken") ?: ""
        assumeTrue(peerId.matches(Regex("[0-9a-f]{64}")) && token.matches(Regex("[a-z0-9]{8,32}")))
        val activity = rule.activity
        val state = (activity.application as ShuttliApplication).mobileState
        val clipboard = activity.getSystemService(ClipboardManager::class.java)
        fun await(predicate: () -> Boolean) = rule.waitUntil(60_000, predicate)
        fun checkpoint(step: String) {
            val file = File(activity.filesDir, "tailnet-$token-$step")
            file.delete()
            Log.i("ShuttliTailnetTest", "CHECKPOINT=$step")
            await { file.exists() }
            assertEquals("ok", file.readText().trim())
            file.delete()
        }
        fun clipboardText(): String? {
            var value: String? = null
            rule.runOnUiThread { value = clipboard.primaryClip?.getItemAt(0)?.text?.toString() }
            return value
        }
        fun setText(text: String) = rule.runOnUiThread {
            clipboard.setPrimaryClip(ClipData.newPlainText("fixture", text))
        }
        fun remoteText(text: String, source: String = peerId) {
            await { state.snapshot.history.any { it.eventKey.startsWith(source) && it.kind == MobileContentKind.TEXT && it.available } }
            await {
                val row = state.snapshot.history.firstOrNull { it.eventKey.startsWith(source) && it.kind == MobileContentKind.TEXT && it.available }
                if (row != null && state.snapshot.selected?.eventKey != row.eventKey) state.selectHistory(row)
                state.snapshot.preview?.toString(Charsets.UTF_8) == text
            }
        }
        await { state.snapshot.devices.any { it.id == peerId && it.online } }
        state.setDirections(state.snapshot.devices.first { it.id == peerId }, send = false, receive = true)
        await { state.snapshot.devices.any { it.id == peerId && !it.send && it.receive } }
        setText("phone clipboard sentinel")
        checkpoint("desktop-text")
        remoteText("desktop tailnet text $token")
        assertEquals("phone clipboard sentinel", clipboardText())
        await { state.snapshot.devices.any { it.id == peerId && it.historyActivity == MobileHistoryActivity.UPDATED } }
        checkpoint("desktop-incremental")
        remoteText("desktop incremental text $token")
        assertTrue(state.snapshot.history.count { it.eventKey.startsWith(peerId) && it.available } >= 2)
        assertEquals("phone clipboard sentinel", clipboardText())
        state.copyToPhone(state.snapshot.selected!!)
        await { state.snapshot.actionStatus == "copied_to_phone" }
        assertEquals("desktop incremental text $token", clipboardText())
        state.selectHistory(null)
        await { state.snapshot.selected == null }

        val device = state.snapshot.devices.first { it.id == peerId }
        assertFalse("sending must require permission", device.send)
        state.setDirections(device, send = true)
        await { state.snapshot.devices.any { it.id == peerId && it.send } }
        setText("phone tailnet text $token")
        rule.onNodeWithText(activity.getString(R.string.paste_clipboard)).performClick()
        await { state.snapshot.draft is Draft.Text }
        assertTrue(state.snapshot.transfers.isEmpty())
        rule.onNodeWithText(activity.getString(R.string.send_to_devices)).performClick()
        await { state.snapshot.transfers.any { it.state == MobileTransferState.APPLIED } }
        checkpoint("phone-text-applied")

        val bitmap = Bitmap.createBitmap(3, 2, Bitmap.Config.ARGB_8888)
        bitmap.eraseColor(0xff38a169.toInt())
        val output = ByteArrayOutputStream()
        bitmap.compress(Bitmap.CompressFormat.PNG, 100, output)
        bitmap.recycle()
        rule.runOnUiThread { assertTrue(ClipboardImageProvider.copyToClipboard(activity, output.toByteArray())) }
        rule.onNodeWithText(activity.getString(R.string.paste_clipboard)).performClick()
        await { state.snapshot.draft is Draft.Image }
        val image = (state.snapshot.draft as Draft.Image).png
        Log.i("ShuttliTailnetTest", "PHONE_PNG_SHA256=" + MessageDigest.getInstance("SHA-256").digest(image)
            .joinToString("") { "%02x".format(it) })
        val previousTransfers = state.snapshot.transfers.map { it.eventKey }.toSet()
        rule.onNodeWithText(activity.getString(R.string.send_to_devices)).performClick()
        await { state.snapshot.transfers.any { it.eventKey !in previousTransfers && it.state == MobileTransferState.APPLIED } }
        checkpoint("phone-image-applied")

        rule.activityRule.scenario.moveToState(Lifecycle.State.CREATED)
        await { state.snapshot.connected == 0u && state.snapshot.draft == null }
        checkpoint("background")
        rule.activityRule.scenario.moveToState(Lifecycle.State.RESUMED)
        remoteText("desktop offline text $token")
        assertTrue("history catch-up must not replace the phone image", clipboardText() == null)
        state.selectHistory(null)
        await { state.snapshot.selected == null }
        checkpoint("desktop-image")
        await { state.snapshot.history.any { !it.isLocal && it.kind == MobileContentKind.IMAGE && it.available } }
        val row = state.snapshot.history.first { !it.isLocal && it.kind == MobileContentKind.IMAGE && it.available }
        state.selectHistory(row)
        await { state.snapshot.preview != null && state.snapshot.selected?.eventKey == row.eventKey }
        val receivedImage = state.snapshot.preview!!
        Log.i("ShuttliTailnetTest", "DESKTOP_PNG_SHA256=" + MessageDigest.getInstance("SHA-256").digest(receivedImage)
            .joinToString("") { "%02x".format(it) })
        state.copyToPhone(row)
        await { state.snapshot.actionStatus == "copied_to_phone" }
        rule.runOnUiThread {
            val uri = clipboard.primaryClip?.getItemAt(0)?.uri ?: error("missing received image URI")
            assertArrayEquals(receivedImage, activity.contentResolver.openInputStream(uri)!!.use { it.readBytes() })
        }
        state.selectHistory(null)
        await { state.snapshot.selected == null }

        val current = state.snapshot.devices.first { it.id == peerId }
        state.setDirections(current, receive = false)
        await { state.snapshot.devices.any { it.id == peerId && !it.receive } }
        state.clearHistory()
        await { state.snapshot.history.none { it.eventKey.startsWith(peerId) } }
        checkpoint("receive-disabled")
        // Let several two-second refresh cycles pass while the coordinator confirms no new rows.
        Thread.sleep(20_000)
        assertTrue(state.snapshot.history.none { it.eventKey.startsWith(peerId) })
        state.setDirections(state.snapshot.devices.first { it.id == peerId }, receive = true)
        await { state.snapshot.devices.any { it.id == peerId && it.receive } }
        remoteText("desktop denied text $token")
        checkpoint("receive-reenabled")
        state.selectHistory(null)
        await { state.snapshot.selected == null }
        checkpoint("desktop-send-disabled")
        state.clearHistory()
        await { state.snapshot.history.none { it.eventKey.startsWith(peerId) } }
        await { state.snapshot.devices.any { it.id == peerId && it.historyActivity == MobileHistoryActivity.DENIED } }
        Thread.sleep(20_000)
        assertTrue("desktop consent must gate history", state.snapshot.history.none { it.eventKey.startsWith(peerId) })
        checkpoint("desktop-send-reenabled")
        remoteText("desktop consent text $token")
        state.setDirections(state.snapshot.devices.first { it.id == peerId }, send = false, receive = true)
        args.getString("tailnetSecondPeer")?.let { secondId ->
            assertTrue(secondId.matches(Regex("[0-9a-f]{64}")))
            checkpoint("multiple-sources")
            await { state.snapshot.devices.any { it.id == secondId && it.online } }
            remoteText("second desktop text $token", secondId)
            val secondRow = state.snapshot.selected!!
            val firstRow = state.snapshot.history.first { it.eventKey.startsWith(peerId) && it.available }
            assertNotEquals(firstRow.sourceName, secondRow.sourceName)
            assertEquals(state.snapshot.history.size, state.snapshot.history.map { it.eventKey }.distinct().size)
            assertFalse(state.snapshot.devices.first { it.id == secondId }.send)
            checkpoint("multiple-sources-merged")
        }
        Log.i("ShuttliTailnetTest", "TAILNET_ACCEPTANCE=PASS")
    }
}
