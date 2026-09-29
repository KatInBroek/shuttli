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
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import uniffi.shuttli_mobile_ffi.MobileHistoryMode
import java.io.ByteArrayOutputStream
import java.security.MessageDigest

@RunWith(AndroidJUnit4::class)
class MobileInstrumentedTest {
    @get:Rule val rule = createAndroidComposeRule<MainActivity>()

    @Test fun importIsExplicitAndDoesNotSendOrReplaceTheClipboard() {
        val activity = rule.activity
        val clipboard = activity.getSystemService(ClipboardManager::class.java)
        val original = "android explicit import fixture"
        rule.runOnUiThread { clipboard.setPrimaryClip(ClipData.newPlainText("fixture", original)) }
        rule.onNodeWithText(activity.getString(R.string.paste_clipboard)).performClick()
        rule.waitUntil(5000) { (activity.application as ShuttliApplication).mobileState.snapshot.draft != null }
        rule.onNodeWithText(original).assertExists()
        assertEquals(original, clipboard.primaryClip?.getItemAt(0)?.text?.toString())
        assertTrue((activity.application as ShuttliApplication).mobileState.snapshot.transfers.isEmpty())
        rule.onNodeWithText(activity.getString(R.string.history)).performClick()
        rule.onNodeWithText(activity.getString(R.string.history_empty)).assertExists()
    }

    @Test fun exportedImageStaysReadableAfterHistoryClear() {
        val activity = rule.activity
        val bitmap = Bitmap.createBitmap(2, 2, Bitmap.Config.ARGB_8888)
        val output = ByteArrayOutputStream()
        bitmap.compress(Bitmap.CompressFormat.PNG, 100, output)
        bitmap.recycle()
        val png = output.toByteArray()
        Log.i("ShuttliPasteTest", "PNG_SHA256=" + MessageDigest.getInstance("SHA-256").digest(png)
            .joinToString("") { "%02x".format(it) })
        assertTrue(ClipboardImageProvider.copyToClipboard(activity, png))
        val clipboard = activity.getSystemService(ClipboardManager::class.java)
        val uri = clipboard.primaryClip?.getItemAt(0)?.uri ?: error("missing image URI")
        (activity.application as ShuttliApplication).mobileState.clearHistory()
        rule.waitForIdle()
        val actual = activity.contentResolver.openInputStream(uri)?.use { it.readBytesBounded(8 * 1024 * 1024) }
        assertArrayEquals(png, actual)
        assertEquals("image/png", activity.contentResolver.getType(uri))
    }

    @Test fun imageImportRequiresTapAndBackgroundDiscardsTheDraft() {
        val activity = rule.activity
        val bitmap = Bitmap.createBitmap(2, 2, Bitmap.Config.ARGB_8888)
        val output = ByteArrayOutputStream()
        bitmap.compress(Bitmap.CompressFormat.PNG, 100, output)
        bitmap.recycle()
        assertTrue(ClipboardImageProvider.copyToClipboard(activity, output.toByteArray()))
        val state = (activity.application as ShuttliApplication).mobileState
        assertTrue(state.snapshot.draft == null)
        rule.onNodeWithText(activity.getString(R.string.paste_clipboard)).performClick()
        rule.waitUntil(5000) { state.snapshot.draft is Draft.Image }
        assertTrue(state.snapshot.transfers.isEmpty())
        rule.activityRule.scenario.moveToState(Lifecycle.State.CREATED)
        rule.waitUntil(5000) { state.snapshot.draft == null }
    }

    @Test fun historyModeAndLimitPersistWithoutChangingTheClipboard() {
        val activity = rule.activity
        val state = (activity.application as ShuttliApplication).mobileState
        try {
            state.setHistoryMode(MobileHistoryMode.OFF)
            rule.waitUntil(5000) { state.snapshot.historyMode == MobileHistoryMode.OFF }
            state.setHistoryLimit(0)
            rule.waitUntil(5000) { state.snapshot.historyLimit == 0 }
            assertEquals(MobileHistoryMode.OFF, HistorySettingsStore.load(activity).mode)
            assertEquals(0, HistorySettingsStore.load(activity).limit)
            rule.onNodeWithText(activity.getString(R.string.settings)).performClick()
            rule.onNodeWithText(activity.getString(R.string.history_mode)).assertExists()
        } finally {
            state.setHistoryMode(MobileHistoryMode.CONTENT)
            rule.waitUntil(5000) { state.snapshot.historyMode == MobileHistoryMode.CONTENT }
            state.setHistoryLimit(20)
            rule.waitUntil(5000) { state.snapshot.historyLimit == 20 }
        }
    }
}
