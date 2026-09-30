package org.katinbroek.shuttli

import android.content.Context
import android.net.ConnectivityManager
import android.net.NetworkCapabilities
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import android.util.AtomicFile
import org.json.JSONObject
import java.net.Inet4Address
import java.security.KeyStore
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec
import uniffi.shuttli_mobile_ffi.generateIdentityBytes
import uniffi.shuttli_mobile_ffi.MobileHistoryMode

internal object LocalSecret {
    private const val KEY_NAME = "shuttli-local-v1"

    private fun key(): SecretKey {
        val store = KeyStore.getInstance("AndroidKeyStore").apply { load(null) }
        (store.getKey(KEY_NAME, null) as? SecretKey)?.let { return it }
        val generator = KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, "AndroidKeyStore")
        generator.init(KeyGenParameterSpec.Builder(KEY_NAME,
            KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT)
            .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
            .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
            .setKeySize(256).build())
        return generator.generateKey()
    }

    fun encrypt(bytes: ByteArray): ByteArray {
        val cipher = Cipher.getInstance("AES/GCM/NoPadding")
        cipher.init(Cipher.ENCRYPT_MODE, key())
        return cipher.iv + cipher.doFinal(bytes)
    }

    fun decrypt(bytes: ByteArray): ByteArray? = runCatching {
        if (bytes.size < 29) return null
        val cipher = Cipher.getInstance("AES/GCM/NoPadding")
        cipher.init(Cipher.DECRYPT_MODE, key(), GCMParameterSpec(128, bytes.copyOfRange(0, 12)))
        cipher.doFinal(bytes.copyOfRange(12, bytes.size))
    }.getOrNull()
}

internal object DeviceIdentityStore {
    fun loadOrCreate(context: Context): ByteArray? = runCatching {
        val file = AtomicFile(context.noBackupFilesDir.resolve("device-identity-v1"))
        if (file.baseFile.exists()) return LocalSecret.decrypt(file.readFully().takeIf { it.size < 8192 } ?: return null)
        val identity = generateIdentityBytes()
        if (identity.isEmpty()) return null
        val output = file.startWrite()
        try { output.write(LocalSecret.encrypt(identity)); file.finishWrite(output) }
        catch (error: Exception) { file.failWrite(output); throw error }
        identity
    }.getOrNull()
}

internal data class Directions(val send: Boolean, val receive: Boolean, val text: Boolean = true, val image: Boolean = true)

internal object DevicePolicyStore {
    fun load(context: Context): Map<String, Directions> = runCatching {
        val file = AtomicFile(context.noBackupFilesDir.resolve("device-directions-v1"))
        if (!file.baseFile.exists()) return emptyMap()
        val bytes = file.readFully()
        if (bytes.size > 16384) return emptyMap()
        val source = JSONObject(String(bytes, Charsets.UTF_8))
        val map = mutableMapOf<String, Directions>()
        val keys = source.keys()
        while (keys.hasNext()) {
            val id = keys.next()
            if (!id.matches(Regex("[0-9a-f]{64}"))) continue
            val item = source.getJSONObject(id)
            map[id] = Directions(item.getBoolean("send"), item.getBoolean("receive"), item.optBoolean("text", true), item.optBoolean("image", true))
        }
        map.takeIf { it.size <= 32 } ?: emptyMap()
    }.getOrDefault(emptyMap())

    fun save(context: Context, id: String, directions: Directions): Boolean = runCatching {
        if (!id.matches(Regex("[0-9a-f]{64}"))) return false
        val all = load(context).toMutableMap()
        all[id] = directions
        if (all.size > 32) return false
        val json = JSONObject()
        all.forEach { (key, value) -> json.put(key, JSONObject().put("send", value.send).put("receive", value.receive).put("text", value.text).put("image", value.image)) }
        val bytes = json.toString().toByteArray(Charsets.UTF_8)
        if (bytes.size > 16384) return false
        val file = AtomicFile(context.noBackupFilesDir.resolve("device-directions-v1"))
        val output = file.startWrite()
        try { output.write(bytes); file.finishWrite(output) }
        catch (error: Exception) { file.failWrite(output); throw error }
        true
    }.getOrDefault(false)
}

internal data class HistorySettings(val mode: MobileHistoryMode, val limit: Int)

internal object HistorySettingsStore {
    fun load(context: Context): HistorySettings = runCatching {
        val file = AtomicFile(context.noBackupFilesDir.resolve("history-settings-v1"))
        if (!file.baseFile.exists()) return HistorySettings(MobileHistoryMode.CONTENT, 20)
        val bytes = file.readFully()
        if (bytes.size > 1024) return HistorySettings(MobileHistoryMode.OFF, 0)
        val json = JSONObject(String(bytes, Charsets.UTF_8))
        val mode = MobileHistoryMode.valueOf(json.getString("mode"))
        val limit = json.getInt("limit")
        if (limit !in 0..10_000) return HistorySettings(MobileHistoryMode.OFF, 0)
        HistorySettings(mode, limit)
    }.getOrDefault(HistorySettings(MobileHistoryMode.OFF, 0))

    fun save(context: Context, settings: HistorySettings): Boolean = runCatching {
        if (settings.limit !in 0..10_000) return false
        val file = AtomicFile(context.noBackupFilesDir.resolve("history-settings-v1"))
        val bytes = JSONObject().put("mode", settings.mode.name).put("limit", settings.limit)
            .toString().toByteArray(Charsets.UTF_8)
        val output = file.startWrite()
        try { output.write(bytes); file.finishWrite(output) }
        catch (error: Exception) { file.failWrite(output); throw error }
        true
    }.getOrDefault(false)
}

internal object TailnetAddress {
    fun currentIPv4(context: Context): String? {
        val manager = context.getSystemService(ConnectivityManager::class.java) ?: return null
        return manager.allNetworks.asSequence().filter { network ->
            manager.getNetworkCapabilities(network)?.hasTransport(NetworkCapabilities.TRANSPORT_VPN) == true
        }.flatMap { network ->
            manager.getLinkProperties(network)?.linkAddresses?.asSequence() ?: emptySequence()
        }.mapNotNull { it.address as? Inet4Address }.firstOrNull { address ->
            val bytes = address.address
            (bytes[0].toInt() and 255) == 100 && (bytes[1].toInt() and 255) in 64..127
        }?.hostAddress
    }
}
