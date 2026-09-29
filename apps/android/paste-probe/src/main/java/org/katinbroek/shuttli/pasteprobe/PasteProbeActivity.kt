package org.katinbroek.shuttli.pasteprobe

import android.app.Activity
import android.content.ClipboardManager
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.widget.TextView
import java.security.MessageDigest

/** Separate test package: exercises Android's cross-app clipboard URI grant. */
class PasteProbeActivity : Activity() {
    private lateinit var result: TextView
    private var read = false

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        result = TextView(this).apply { text = "WAITING"; textSize = 22f; setPadding(36, 90, 36, 36) }
        setContentView(result)
    }

    override fun onWindowFocusChanged(hasFocus: Boolean) {
        super.onWindowFocusChanged(hasFocus)
        if (hasFocus && !read) {
            read = true
            Handler(Looper.getMainLooper()).postDelayed({ readClipboard() }, 300)
        }
    }

    private fun readClipboard() {
        result.text = runCatching {
            val clipboard = getSystemService(ClipboardManager::class.java)
            val uri = clipboard.primaryClip?.getItemAt(0)?.uri ?: return@runCatching "NO_IMAGE_URI"
            if (contentResolver.getType(uri) != "image/png") return@runCatching "WRONG_MIME"
            val bytes = contentResolver.openInputStream(uri)?.use { stream ->
                val output = java.io.ByteArrayOutputStream()
                val buffer = ByteArray(65536)
                while (true) {
                    val count = stream.read(buffer)
                    if (count < 0) break
                    if (output.size() + count > 8 * 1024 * 1024) return@runCatching "OVERSIZE"
                    output.write(buffer, 0, count)
                }
                output.toByteArray()
            } ?: return@runCatching "NO_READ"
            val digest = MessageDigest.getInstance("SHA-256").digest(bytes).joinToString("") { "%02x".format(it) }
            if (digest == intent.getStringExtra("expected")) "MATCH ${bytes.size}" else "MISMATCH $digest"
        }.getOrElse { "ERROR ${it.javaClass.simpleName}" }
    }
}
