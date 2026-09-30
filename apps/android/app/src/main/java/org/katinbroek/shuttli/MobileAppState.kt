package org.katinbroek.shuttli

import android.app.Activity
import android.app.Application
import android.content.ClipboardManager
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.net.Uri
import android.os.Build
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withContext
import uniffi.shuttli_mobile_ffi.MobileDeviceRow
import uniffi.shuttli_mobile_ffi.MobileHistoryRow
import uniffi.shuttli_mobile_ffi.MobileSession
import uniffi.shuttli_mobile_ffi.MobileTransferRow
import uniffi.shuttli_mobile_ffi.MobileContentKind
import uniffi.shuttli_mobile_ffi.MobileHistoryMode
import java.io.ByteArrayOutputStream

class ShuttliApplication : Application() {
    val mobileState: MobileAppState by lazy { MobileAppState(this) }
}

internal sealed interface Draft {
    data class Text(val value: String) : Draft
    data class Image(val png: ByteArray) : Draft
}

internal data class UiSnapshot(
    val history: List<MobileHistoryRow> = emptyList(),
    val devices: List<MobileDeviceRow> = emptyList(),
    val transfers: List<MobileTransferRow> = emptyList(),
    val connected: UInt = 0u,
    val status: String = "waiting",
    val draft: Draft? = null,
    val actionStatus: String? = null,
    val selected: MobileHistoryRow? = null,
    val preview: ByteArray? = null,
    val historyMode: MobileHistoryMode = MobileHistoryMode.CONTENT,
    val historyLimit: Int = 20,
    val fingerprint: String = "",
    val actionEvent: String? = null
)

class MobileAppState(private val app: Application) {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
    private val lifecycle = Mutex()
    private val session by lazy { MobileSession() }
    private var refreshJob: Job? = null
    @Volatile private var active = false
    private var policyRestored = false
    private var identity: ByteArray? = null
    internal var snapshot by mutableStateOf(UiSnapshot())
        private set

    fun enterForeground() {
        scope.launch {
            lifecycle.withLock {
                if (active) return@withLock
                active = true
                session.configureImageCache(app.cacheDir.absolutePath)
                val settings = HistorySettingsStore.load(app)
                session.setHistoryMode(settings.mode)
                session.setHistoryLimit(settings.limit.toUInt())
                post { it.copy(historyMode = settings.mode, historyLimit = settings.limit) }
                session.enterForeground()
                identity = DeviceIdentityStore.loadOrCreate(app)
                val fingerprint = identity?.let(session::identityFingerprint).orEmpty()
                post { it.copy(fingerprint = fingerprint) }
                connect()
                refreshJob = scope.launch {
                    while (active) {
                        connect()
                        refresh()
                        delay(2_000)
                    }
                }
            }
        }
    }

    fun enterBackground() {
        scope.launch {
            lifecycle.withLock {
                active = false
                refreshJob?.cancel()
                refreshJob = null
                session.enterBackground()
                withContext(Dispatchers.Main) { snapshot = snapshot.copy(draft = null, selected = null, preview = null, connected = 0u) }
            }
        }
    }

    private suspend fun connect() {
        val bytes = identity ?: run { post { it.copy(status = "identity_unavailable") }; return }
        val ip = TailnetAddress.currentIPv4(app) ?: run { post { it.copy(status = "tailscale_unavailable") }; return }
        if (!policyRestored) {
            val restored = DevicePolicyStore.load(app).all { (id, value) ->
                session.restoreDevicePolicy(id, value.send, value.receive, value.text, value.image)
            }
            if (!restored) { post { it.copy(status = "listener_unavailable") }; return }
            policyRestored = true
        }
        val error = session.startListener(bytes, ip, Build.MODEL.take(256))
        post { it.copy(status = if (error.isEmpty()) "waiting" else "listener_unavailable") }
    }

    private suspend fun refresh() {
        val history = session.historyRows()
        val devices = session.deviceRows()
        val transfers = session.transferRows()
        val connected = session.connectedPeersCount()
        post { it.copy(history = history, devices = devices, transfers = transfers,
            connected = connected, status = if (connected > 0u) "connected" else it.status) }
    }

    private suspend fun post(change: (UiSnapshot) -> UiSnapshot) = withContext(Dispatchers.Main) {
        snapshot = change(snapshot)
    }

    fun setDirections(row: MobileDeviceRow, send: Boolean? = null, receive: Boolean? = null,
                      text: Boolean? = null, image: Boolean? = null) {
        scope.launch {
            lifecycle.withLock {
                val previous = DevicePolicyStore.load(app)[row.id] ?: Directions(row.send, row.receive, row.text, row.image)
                val value = Directions(send ?: previous.send, receive ?: previous.receive, text ?: previous.text, image ?: previous.image)
                if (!DevicePolicyStore.save(app, row.id, value)) {
                    post { it.copy(actionStatus = "policy_failed") }; return@withLock
                }
                if (!session.restoreDevicePolicy(row.id, value.send, value.receive, value.text, value.image)) {
                    DevicePolicyStore.save(app, row.id, previous)
                    post { it.copy(actionStatus = "policy_failed") }; return@withLock
                }
                post { it.copy(actionStatus = null) }
                session.requestHistoryRefresh()
                refresh()
            }
        }
    }

    fun refreshHistory() {
        scope.launch {
            connect()
            if (!session.requestHistoryRefresh()) post { it.copy(actionStatus = "refresh_unavailable") }
            else post { it.copy(actionStatus = null) }
            refresh()
        }
    }

    fun setHistoryMode(mode: MobileHistoryMode) {
        scope.launch {
            val value = HistorySettings(mode, snapshot.historyLimit)
            if (!HistorySettingsStore.save(app, value)) { post { it.copy(actionStatus = "settings_failed") }; return@launch }
            session.setHistoryMode(mode)
            post { it.copy(historyMode = mode, actionStatus = null) }
            refresh()
        }
    }

    fun setHistoryLimit(limit: Int) {
        scope.launch {
            val value = HistorySettings(snapshot.historyMode, limit)
            if (!HistorySettingsStore.save(app, value) || !session.setHistoryLimit(limit.toUInt())) {
                post { it.copy(actionStatus = "settings_failed") }
                return@launch
            }
            post { it.copy(historyLimit = limit, actionStatus = null) }
            refresh()
        }
    }

    fun importClipboard(activity: Activity) {
        if (!activity.hasWindowFocus()) return
        val manager = activity.getSystemService(ClipboardManager::class.java) ?: return
        val clip = manager.primaryClip ?: return
        val item = clip.getItemAt(0)
        val uri: Uri? = item.uri
        if (uri != null && clip.description.hasMimeType("image/*")) {
            scope.launch {
                val png = readImage(uri)
                post { it.copy(draft = png?.let(Draft::Image), actionStatus = if (png == null) "unsupported_image" else null) }
            }
            return
        }
        if (clip.description.hasMimeType("text/plain")) {
            val text = item.text?.toString()
            val accepted = text?.takeIf { it.isNotEmpty() && it.toByteArray().size <= 1024 * 1024 && '\u0000' !in it }
            scope.launch { post { it.copy(draft = accepted?.let(Draft::Text), actionStatus = if (accepted == null) "unsupported_text" else null) } }
        } else {
            scope.launch { post { it.copy(actionStatus = "unsupported_clipboard") } }
        }
    }

    private fun readImage(uri: Uri): ByteArray? = runCatching {
        val bytes = app.contentResolver.openInputStream(uri)?.use { it.readBytesBounded(8 * 1024 * 1024) } ?: return null
        val bounds = BitmapFactory.Options().apply { inJustDecodeBounds = true }
        BitmapFactory.decodeByteArray(bytes, 0, bytes.size, bounds)
        if (bounds.outWidth !in 1..16384 || bounds.outHeight !in 1..16384 ||
            bounds.outWidth.toLong() * bounds.outHeight > 4_194_304) return null
        val bitmap = BitmapFactory.decodeByteArray(bytes, 0, bytes.size) ?: return null
        val output = ByteArrayOutputStream()
        try {
            if (!bitmap.compress(Bitmap.CompressFormat.PNG, 100, output) || output.size() > 8 * 1024 * 1024) return null
            output.toByteArray()
        } finally { bitmap.recycle() }
    }.getOrNull()

    fun sendDraft() {
        scope.launch {
            if (!active) return@launch
            val draft = snapshot.draft ?: return@launch
            val count = when (draft) {
                is Draft.Text -> session.sendText(draft.value)
                is Draft.Image -> session.sendImage(draft.png)
            }
            post { it.copy(draft = if (count > 0u) null else it.draft,
                actionStatus = if (count > 0u) "send_queued" else "no_send_targets") }
            refresh()
        }
    }

    fun resend(row: MobileHistoryRow) {
        if (!row.isLocal) return
        scope.launch {
            if (!active) return@launch
            val count = session.resendLocal(row.eventKey)
            post { it.copy(actionStatus = if (count > 0u) "send_queued" else "no_send_targets") }
            refresh()
        }
    }

    fun copyToPhone(row: MobileHistoryRow) {
        scope.launch {
            if (!active) return@launch
            val bytes = session.historyBody(row.eventKey)
            if (bytes.isEmpty()) { post { it.copy(actionStatus = "content_unavailable") }; return@launch }
            val copied = if (row.kind == MobileContentKind.TEXT) {
                withContext(Dispatchers.Main) {
                    if (!active) return@withContext false
                    val text = bytes.toString(Charsets.UTF_8)
                    val manager = app.getSystemService(ClipboardManager::class.java)
                    manager?.setPrimaryClip(android.content.ClipData.newPlainText(app.getString(R.string.app_name), text))
                    manager?.primaryClip?.getItemAt(0)?.text?.toString() == text
                }
            } else {
                val uri = ClipboardImageProvider.export(app, bytes)
                val written = withContext(Dispatchers.Main) {
                    if (!active || uri == null) false else {
                        val manager = app.getSystemService(ClipboardManager::class.java)
                        manager?.setPrimaryClip(android.content.ClipData.newUri(app.contentResolver, app.getString(R.string.app_name), uri))
                        manager != null
                    }
                }
                written && uri != null && app.contentResolver.openInputStream(uri)?.use {
                    it.readBytesBounded(8 * 1024 * 1024)?.contentEquals(bytes)
                } == true
            }
            post { it.copy(actionStatus = if (copied) "copied_to_phone" else "copy_uncertain", actionEvent = row.eventKey) }
        }
    }

    internal suspend fun historyPreview(row: MobileHistoryRow): ByteArray? = withContext(Dispatchers.IO) {
        if (!row.available) null else session.historyBody(row.eventKey).takeIf(ByteArray::isNotEmpty)
    }

    fun dismissDraft() {
        scope.launch { post { it.copy(draft = null, actionStatus = null) } }
    }

    fun selectHistory(row: MobileHistoryRow?) {
        scope.launch {
            if (row == null) {
                post { it.copy(selected = null, preview = null) }
            } else {
                val bytes = session.historyBody(row.eventKey)
                post { it.copy(selected = row, preview = bytes.takeIf(ByteArray::isNotEmpty)) }
            }
        }
    }

    fun clearHistory() {
        scope.launch {
            lifecycle.withLock {
                if (!active) return@withLock
                refreshJob?.cancel()
                session.clearHistory()
                session.enterBackground()
                session.enterForeground()
                policyRestored = false
                connect()
                post { it.copy(history = emptyList(), transfers = emptyList(), draft = null, actionStatus = null) }
                refreshJob = scope.launch { while (active) { connect(); refresh(); delay(2_000) } }
            }
        }
    }
}
