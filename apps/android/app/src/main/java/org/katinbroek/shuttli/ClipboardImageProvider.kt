package org.katinbroek.shuttli

import android.content.ClipData
import android.content.ContentProvider
import android.content.ContentValues
import android.content.Context
import android.database.Cursor
import android.database.MatrixCursor
import android.net.Uri
import android.os.ParcelFileDescriptor
import android.provider.OpenableColumns
import android.util.AtomicFile
import org.json.JSONObject
import java.security.SecureRandom
import java.util.concurrent.Executors
import java.util.concurrent.Semaphore

class ClipboardImageProvider : ContentProvider() {
    companion object {
        private const val AUTHORITY = "org.katinbroek.shuttli.clipboard"
        private const val LIFETIME_MS = 30L * 60L * 1000L
        private val workers = Executors.newFixedThreadPool(2)
        private val slots = Semaphore(2)

        private fun imageFile(context: Context) = AtomicFile(context.noBackupFilesDir.resolve("clipboard-image-v1"))
        private fun metaFile(context: Context) = AtomicFile(context.noBackupFilesDir.resolve("clipboard-image-meta-v1"))

        private fun readMeta(context: Context): JSONObject? = runCatching {
            val bytes = metaFile(context).readFully()
            if (bytes.size > 512) return null
            JSONObject(String(bytes, Charsets.UTF_8))
        }.getOrNull()

        private fun write(file: AtomicFile, bytes: ByteArray) {
            val stream = file.startWrite()
            try { stream.write(bytes); file.finishWrite(stream) }
            catch (error: Exception) { file.failWrite(stream); throw error }
        }

        fun export(context: Context, png: ByteArray): Uri? = runCatching {
            if (png.isEmpty() || png.size > 8 * 1024 * 1024) return null
            val token = ByteArray(16).also { SecureRandom().nextBytes(it) }
                .joinToString("") { "%02x".format(it) }
            val expires = System.currentTimeMillis() + LIFETIME_MS
            write(imageFile(context), LocalSecret.encrypt(png))
            write(metaFile(context), JSONObject().put("id", token).put("expires", expires)
                .put("size", png.size).toString().toByteArray(Charsets.UTF_8))
            Uri.Builder().scheme("content").authority(AUTHORITY).appendPath("image").appendPath(token).build()
        }.getOrNull()

        fun copyToClipboard(context: Context, png: ByteArray): Boolean {
            val uri = export(context, png) ?: return false
            val manager = context.getSystemService(android.content.ClipboardManager::class.java) ?: return false
            return runCatching {
                manager.setPrimaryClip(ClipData.newUri(context.contentResolver, context.getString(R.string.app_name), uri))
                context.contentResolver.openInputStream(uri)?.use { stream ->
                    val observed = stream.readBytesBounded(8 * 1024 * 1024)
                    observed?.contentEquals(png) == true
                } == true
            }.getOrDefault(false)
        }
    }

    override fun onCreate(): Boolean = true

    private fun valid(uri: Uri): JSONObject? {
        val ctx = context ?: return null
        if (uri.scheme != "content" || uri.authority != AUTHORITY || uri.pathSegments.size != 2 || uri.pathSegments[0] != "image") return null
        val meta = readMeta(ctx) ?: return null
        if (uri.pathSegments[1] != meta.optString("id") || System.currentTimeMillis() >= meta.optLong("expires")) return null
        if (meta.optInt("size") !in 1..(8 * 1024 * 1024)) return null
        return meta
    }

    override fun getType(uri: Uri): String? = if (valid(uri) != null) "image/png" else null

    override fun query(uri: Uri, projection: Array<out String>?, selection: String?, selectionArgs: Array<out String>?, sortOrder: String?): Cursor? {
        val meta = valid(uri) ?: return null
        val columns = projection ?: arrayOf(OpenableColumns.DISPLAY_NAME, OpenableColumns.SIZE)
        val cursor = MatrixCursor(columns)
        cursor.addRow(columns.map { name -> when (name) {
            OpenableColumns.DISPLAY_NAME -> "clipboard.png"
            OpenableColumns.SIZE -> meta.getInt("size")
            else -> null
        } })
        return cursor
    }

    override fun openFile(uri: Uri, mode: String): ParcelFileDescriptor? {
        if (mode != "r" || valid(uri) == null || !slots.tryAcquire()) return null
        val ctx = context ?: run { slots.release(); return null }
        val plaintext = runCatching {
            val ciphertext = imageFile(ctx).readFully()
            if (ciphertext.size > 8 * 1024 * 1024 + 128) return null
            LocalSecret.decrypt(ciphertext)
        }.getOrNull()?.takeIf { it.size == valid(uri)?.optInt("size") }
        if (plaintext == null) { slots.release(); return null }
        val pipe = ParcelFileDescriptor.createPipe()
        workers.execute {
            try { ParcelFileDescriptor.AutoCloseOutputStream(pipe[1]).use { it.write(plaintext) } }
            finally { slots.release() }
        }
        return pipe[0]
    }

    override fun insert(uri: Uri, values: ContentValues?): Uri? = null
    override fun delete(uri: Uri, selection: String?, selectionArgs: Array<out String>?): Int = 0
    override fun update(uri: Uri, values: ContentValues?, selection: String?, selectionArgs: Array<out String>?): Int = 0
}

internal fun java.io.InputStream.readBytesBounded(max: Int): ByteArray? {
    val output = java.io.ByteArrayOutputStream()
    val buffer = ByteArray(65536)
    while (true) {
        val count = read(buffer)
        if (count < 0) break
        if (output.size() + count > max) return null
        output.write(buffer, 0, count)
    }
    return output.toByteArray()
}
