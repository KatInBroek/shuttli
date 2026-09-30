package org.katinbroek.shuttli

import android.app.Activity
import android.graphics.BitmapFactory
import android.os.Build
import androidx.activity.compose.BackHandler
import androidx.compose.foundation.Image
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import uniffi.shuttli_mobile_ffi.*
import java.text.DateFormat
import java.util.Date

@Composable
internal fun ShuttliScreen(state: MobileAppState, activity: Activity) {
    val data = state.snapshot
    var tab by rememberSaveable { mutableIntStateOf(0) }
    var selectedDevice by rememberSaveable { mutableStateOf<String?>(null) }
    val device = data.devices.find { it.id == selectedDevice }
    ShuttliTheme {
        BackHandler(selectedDevice != null || data.selected != null) {
            selectedDevice = null; state.selectHistory(null)
        }
        Scaffold(containerColor = MaterialTheme.colorScheme.background, bottomBar = {
            Column {
                if (tab == 0 && data.selected == null) SendPanel(data, state, activity)
                NavigationBar(containerColor = MaterialTheme.colorScheme.surfaceVariant, tonalElevation = 0.dp) {
                    listOf(R.string.home to AppIcon.Home, R.string.devices to AppIcon.Devices,
                        R.string.settings to AppIcon.Settings).forEachIndexed { index, (label, icon) ->
                        NavigationBarItem(selected = tab == index, onClick = {
                            tab = index; selectedDevice = null; state.selectHistory(null)
                        }, icon = { Icon(icon.vector, null) }, label = { Text(stringResource(label)) })
                    }
                }
            }
        }) { padding ->
            Column(Modifier.fillMaxSize().padding(padding).padding(horizontal = 16.dp)) {
                when {
                    data.selected != null -> HistoryDetail(data, state)
                    device != null && tab == 1 -> DeviceDetail(device, state) { selectedDevice = null }
                    tab == 0 -> HistoryPage(data, state)
                    tab == 1 -> DevicesPage(data) { selectedDevice = it }
                    else -> SettingsPage(data, state, activity)
                }
            }
        }
    }
}

@Composable
private fun PageHeader(title: String, back: (() -> Unit)? = null, trailing: @Composable RowScope.() -> Unit = {}) {
    Row(Modifier.fillMaxWidth().padding(top = 12.dp, bottom = 6.dp).heightIn(min = 48.dp), verticalAlignment = Alignment.CenterVertically) {
        if (back != null) IconButton(onClick = back) { Icon(AppIcon.Back.vector, stringResource(R.string.back)) }
        Text(title, style = MaterialTheme.typography.headlineSmall, modifier = Modifier.weight(1f))
        trailing()
    }
}

@Composable
private fun HistoryPage(data: UiSnapshot, state: MobileAppState) {
    var filter by rememberSaveable { mutableIntStateOf(0) }
    PageHeader(stringResource(R.string.history)) {
        val checked = data.devices.filter { it.receive }.map { it.checkedAtMs }.filter { it > 0uL }.minOrNull()
        if (checked != null) Text(stringResource(R.string.checked_time, time(checked)), style = MaterialTheme.typography.labelSmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant)
        IconButton(onClick = state::refreshHistory) { Icon(AppIcon.Refresh.vector, stringResource(R.string.refresh)) }
    }
    Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
        listOf(R.string.all, R.string.sent_from_phone, R.string.images).forEachIndexed { index, label ->
            FilterChip(selected = filter == index, onClick = { filter = index }, label = { Text(stringResource(label)) })
        }
    }
    val rows = data.history.filter { filter == 0 || filter == 1 && it.isLocal || filter == 2 && it.kind == MobileContentKind.IMAGE }
    LazyColumn(contentPadding = PaddingValues(top = 6.dp, bottom = 16.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
        if (data.status != "connected") item {
            Surface(color = MaterialTheme.colorScheme.surfaceVariant, shape = RoundedCornerShape(14.dp)) {
                Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
                    Text(statusText(data), style = MaterialTheme.typography.titleSmall)
                    Text(stringResource(R.string.connect_help), style = MaterialTheme.typography.bodySmall)
                }
            }
        }
        if (rows.isEmpty()) item {
            Column(Modifier.fillMaxWidth().padding(vertical = 48.dp), horizontalAlignment = Alignment.CenterHorizontally) {
                Icon(AppIcon.Copy.vector, null, Modifier.size(36.dp), tint = MaterialTheme.colorScheme.onSurfaceVariant)
                Spacer(Modifier.height(12.dp))
                Text(stringResource(R.string.history_empty), style = MaterialTheme.typography.titleMedium)
                Text(stringResource(R.string.history_empty_help), style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.padding(top = 6.dp))
            }
        }
        items(rows, key = { it.eventKey }) { row -> HistoryCard(row, data, state) }
    }
}

@Composable
private fun HistoryCard(row: MobileHistoryRow, data: UiSnapshot, state: MobileAppState) {
    val body by produceState<ByteArray?>(null, row.eventKey, row.available) { value = state.historyPreview(row) }
    var expanded by rememberSaveable(row.eventKey) { mutableStateOf(false) }
    Surface(shape = RoundedCornerShape(18.dp), color = MaterialTheme.colorScheme.surface) {
        Column(Modifier.fillMaxWidth().padding(16.dp), verticalArrangement = Arrangement.spacedBy(10.dp)) {
            Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                Text(if (row.isLocal) stringResource(R.string.sent_from_phone) else row.sourceName,
                    style = MaterialTheme.typography.labelLarge, fontWeight = FontWeight.SemiBold,
                    maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.weight(1f, fill = false))
                Text(time(row.copiedAtMs) + " · " + stringResource(if (row.kind == MobileContentKind.TEXT) R.string.text_item else R.string.image_item),
                    style = MaterialTheme.typography.labelSmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
            }
            when {
                body != null && row.kind == MobileContentKind.TEXT -> {
                    val text = remember(body) { body!!.toString(Charsets.UTF_8) }
                    Text(text, style = MaterialTheme.typography.bodyLarge, maxLines = if (expanded) Int.MAX_VALUE else 8, overflow = TextOverflow.Ellipsis)
                    if (text.length > 350 || text.count { it == '\n' } >= 8) TextButton(onClick = { expanded = !expanded }) {
                        Text(stringResource(if (expanded) R.string.show_less else R.string.show_more))
                    }
                }
                body != null -> ImagePreview(body!!, Modifier.fillMaxWidth().heightIn(max = 260.dp)
                    .clip(RoundedCornerShape(8.dp)).clickable { state.selectHistory(row) })
                else -> Text(stringResource(if (row.receiving) R.string.history_receiving else R.string.item_listed),
                    style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
            }
            if (row.isLocal) {
                val transfers = data.transfers.filter { it.eventKey == row.eventKey }
                if (transfers.isNotEmpty()) HorizontalDivider(color = MaterialTheme.colorScheme.outlineVariant)
                transfers.forEach { transfer ->
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        Text(transfer.targetName, style = MaterialTheme.typography.bodySmall, modifier = Modifier.weight(1f))
                        Text(transferSymbol(transfer.state) + " " + transferState(transfer.state),
                            style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                    }
                }
            }
            Row(verticalAlignment = Alignment.CenterVertically) {
                FilledTonalButton(onClick = { state.copyToPhone(row) }, enabled = body != null, contentPadding = PaddingValues(horizontal = 16.dp, vertical = 8.dp)) {
                    Icon(AppIcon.Copy.vector, null, Modifier.size(16.dp)); Spacer(Modifier.width(6.dp)); Text(stringResource(R.string.copy_to_phone))
                }
                if (data.actionEvent == row.eventKey && data.actionStatus != null) {
                    Text(actionText(data.actionStatus), style = MaterialTheme.typography.bodySmall, modifier = Modifier.padding(start = 8.dp))
                }
                if (row.isLocal) TextButton(onClick = { state.resend(row) }, enabled = body != null && data.devices.any { it.online && it.send }) {
                    Text(stringResource(R.string.send_again))
                }
            }
        }
    }
}

@Composable
private fun ImagePreview(body: ByteArray, modifier: Modifier = Modifier) {
    val bitmap by produceState<android.graphics.Bitmap?>(null, body) {
        value = withContext(Dispatchers.Default) {
            val options = BitmapFactory.Options().apply { inJustDecodeBounds = true }
            BitmapFactory.decodeByteArray(body, 0, body.size, options)
            options.inSampleSize = generateSequence(1) { it * 2 }.first { maxOf(options.outWidth, options.outHeight) / it <= 1024 }
            options.inJustDecodeBounds = false
            BitmapFactory.decodeByteArray(body, 0, body.size, options)
        }
    }
    if (bitmap != null) Image(bitmap!!.asImageBitmap(), stringResource(R.string.image_item), modifier)
    else Text(stringResource(R.string.preview_unavailable), style = MaterialTheme.typography.bodySmall)
}

@Composable
private fun SendPanel(data: UiSnapshot, state: MobileAppState, activity: Activity) {
    var showTargets by remember { mutableStateOf(false) }
    val targets = data.devices.filter { it.online && it.send && when (data.draft) {
        is Draft.Image -> it.image
        is Draft.Text -> it.text
        null -> it.text || it.image
    } }
    Surface(color = MaterialTheme.colorScheme.primaryContainer, shape = RoundedCornerShape(topStart = 20.dp, topEnd = 20.dp)) {
        Column(Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 12.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            if (data.draft != null) Row(verticalAlignment = Alignment.Top) {
                Column(Modifier.weight(1f)) {
                    Text(stringResource(R.string.ready_to_send), style = MaterialTheme.typography.labelLarge)
                    when (val draft = data.draft) {
                        is Draft.Text -> Text(draft.value, maxLines = 2, overflow = TextOverflow.Ellipsis, style = MaterialTheme.typography.bodyMedium)
                        is Draft.Image -> ImagePreview(draft.png, Modifier.height(60.dp))
                        null -> Unit
                    }
                }
                IconButton(onClick = state::dismissDraft) { Icon(AppIcon.Close.vector, stringResource(R.string.cancel)) }
            }
            Row(verticalAlignment = Alignment.CenterVertically) {
                Column(Modifier.weight(1f)) {
                    Text(stringResource(R.string.send_clipboard), style = MaterialTheme.typography.titleSmall)
                    Row(Modifier.clickable { showTargets = true }.padding(vertical = 4.dp), verticalAlignment = Alignment.CenterVertically) {
                        Text(stringResource(if (data.draft == null) R.string.devices_can_receive else R.string.to_devices, targets.size), style = MaterialTheme.typography.bodySmall)
                        if (data.draft != null) Icon(AppIcon.Down.vector, null, Modifier.size(16.dp))
                    }
                }
                Button(onClick = { if (data.draft == null) state.importClipboard(activity) else state.sendDraft() },
                    enabled = data.draft == null || targets.isNotEmpty(), modifier = Modifier.widthIn(min = 112.dp).heightIn(min = 48.dp)) {
                    Icon(AppIcon.Paste.vector, null, Modifier.size(18.dp)); Spacer(Modifier.width(8.dp))
                    Text(stringResource(if (data.draft == null) R.string.paste_clipboard else R.string.send_to_devices))
                }
            }
            data.actionStatus?.let { Text(actionText(it), style = MaterialTheme.typography.bodySmall) }
        }
    }
    if (showTargets) AlertDialog(onDismissRequest = { showTargets = false }, title = { Text(stringResource(R.string.send_targets)) },
        text = { Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
            if (targets.isEmpty()) Text(stringResource(R.string.no_send_targets))
            targets.forEach { Text(it.name) }
            Text(stringResource(R.string.targets_help), style = MaterialTheme.typography.bodySmall)
        } }, confirmButton = { TextButton(onClick = { showTargets = false }) { Text(stringResource(R.string.done)) } })
}

@Composable
private fun HistoryDetail(data: UiSnapshot, state: MobileAppState) {
    val row = data.selected ?: return
    PageHeader(if (row.isLocal) stringResource(R.string.this_phone) else row.sourceName, { state.selectHistory(null) })
    LazyColumn(verticalArrangement = Arrangement.spacedBy(12.dp)) {
        item { Text(stringResource(R.string.bytes, row.bytes.toLong()), style = MaterialTheme.typography.bodySmall) }
        item {
            val body = data.preview
            if (body == null) Text(stringResource(R.string.content_unavailable))
            else if (row.kind == MobileContentKind.TEXT) Text(body.toString(Charsets.UTF_8))
            else ImagePreview(body, Modifier.fillMaxWidth())
        }
        item { Button(onClick = { state.copyToPhone(row) }, enabled = data.preview != null) { Text(stringResource(R.string.copy_to_phone)) } }
        data.actionStatus?.let { item { Text(actionText(it)) } }
    }
}

@Composable
private fun DevicesPage(data: UiSnapshot, onSelect: (String) -> Unit) {
    PageHeader(stringResource(R.string.devices))
    Text(stringResource(R.string.device_permissions_help), style = MaterialTheme.typography.bodySmall,
        color = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.padding(bottom = 16.dp))
    LazyColumn(verticalArrangement = Arrangement.spacedBy(12.dp), contentPadding = PaddingValues(bottom = 16.dp)) {
        if (data.devices.isEmpty()) item { Text(stringResource(R.string.no_connected_devices)) }
        items(data.devices, key = { it.id }) { device ->
            Surface(shape = RoundedCornerShape(18.dp), onClick = { onSelect(device.id) }) {
                Row(Modifier.fillMaxWidth().padding(16.dp), verticalAlignment = Alignment.CenterVertically) {
                    Icon(AppIcon.Devices.vector, null, Modifier.size(28.dp), tint = MaterialTheme.colorScheme.onSurfaceVariant)
                    Spacer(Modifier.width(12.dp))
                    Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(4.dp)) {
                        Text(device.name, style = MaterialTheme.typography.titleMedium)
                        Text(stringResource(if (device.online) R.string.device_online else R.string.device_offline), style = MaterialTheme.typography.bodySmall)
                        Text(historyActivityText(device.historyActivity), style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                    }
                    Icon(AppIcon.Chevron.vector, stringResource(R.string.device_details))
                }
            }
        }
    }
}

@Composable
private fun DeviceDetail(device: MobileDeviceRow, state: MobileAppState, onBack: () -> Unit) {
    var confirm by remember(device.id) { mutableStateOf(false) }
    PageHeader(device.name, onBack)
    LazyColumn(verticalArrangement = Arrangement.spacedBy(16.dp), contentPadding = PaddingValues(bottom = 24.dp)) {
        item { SettingsSection(stringResource(R.string.connection)) {
            Text(stringResource(if (device.online) R.string.device_online else R.string.device_offline), style = MaterialTheme.typography.titleSmall)
            Text(historyActivityText(device.historyActivity), style = MaterialTheme.typography.bodySmall)
            Text(device.endpoint, style = MaterialTheme.typography.bodySmall, fontFamily = FontFamily.Monospace)
            if (device.checkedAtMs > 0uL) Text(stringResource(R.string.history_checked, time(device.checkedAtMs)), style = MaterialTheme.typography.bodySmall)
            if (device.historyPartial) Text(stringResource(R.string.history_partial), style = MaterialTheme.typography.bodySmall)
        } }
        item { SettingsSection(stringResource(R.string.receiving)) {
            SettingToggle(stringResource(R.string.receive_from_device), device.receive) { state.setDirections(device, receive = it) }
            Text(stringResource(R.string.receive_help), style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
        } }
        item { SettingsSection(stringResource(R.string.sending)) {
            SettingToggle(stringResource(R.string.send_to_device), device.send) { if (it) confirm = true else state.setDirections(device, send = false) }
            Text(stringResource(R.string.send_help), style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
        } }
        item { SettingsSection(stringResource(R.string.content_types)) {
            SettingToggle(stringResource(R.string.text_item), device.text) { state.setDirections(device, text = it) }
            SettingToggle(stringResource(R.string.images), device.image) { state.setDirections(device, image = it) }
            Text(stringResource(R.string.content_types_help), style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
        } }
        item { SettingsSection(stringResource(R.string.device_fingerprint)) {
            Text(device.id.chunked(4).joinToString(" "), fontFamily = FontFamily.Monospace, style = MaterialTheme.typography.bodySmall)
        } }
        state.snapshot.actionStatus?.let { item { Text(actionText(it)) } }
    }
    if (confirm) AlertDialog(onDismissRequest = { confirm = false }, title = { Text(stringResource(R.string.confirm_device)) },
        text = { Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
            Text(stringResource(R.string.fingerprint_help, device.name))
            Text(device.id.chunked(4).joinToString(" "), fontFamily = FontFamily.Monospace, style = MaterialTheme.typography.bodySmall)
        } }, confirmButton = { TextButton(onClick = { state.setDirections(device, send = true); confirm = false }) { Text(stringResource(R.string.allow_sending)) } },
        dismissButton = { TextButton(onClick = { confirm = false }) { Text(stringResource(R.string.cancel)) } })
}

@Composable
private fun SettingsSection(title: String, content: @Composable ColumnScope.() -> Unit) {
    Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
        Text(title, style = MaterialTheme.typography.labelLarge, color = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.padding(start = 4.dp))
        Surface(color = MaterialTheme.colorScheme.surface, shape = RoundedCornerShape(18.dp)) {
            Column(Modifier.fillMaxWidth().padding(16.dp), verticalArrangement = Arrangement.spacedBy(10.dp), content = content)
        }
    }
}

@Composable
private fun SettingToggle(title: String, value: Boolean, change: (Boolean) -> Unit) {
    Row(verticalAlignment = Alignment.CenterVertically) {
        Text(title, Modifier.weight(1f), style = MaterialTheme.typography.bodyLarge)
        Switch(checked = value, onCheckedChange = change)
    }
}

@Composable
private fun SettingsPage(data: UiSnapshot, state: MobileAppState, activity: Activity) {
    var confirm by remember { mutableStateOf(false) }
    var limitText by remember(data.historyLimit) { mutableStateOf(data.historyLimit.toString()) }
    PageHeader(stringResource(R.string.settings))
    LazyColumn(verticalArrangement = Arrangement.spacedBy(18.dp), contentPadding = PaddingValues(bottom = 24.dp)) {
        item { SettingsSection(stringResource(R.string.this_phone)) {
            Text(Build.MODEL, style = MaterialTheme.typography.titleMedium)
            Text(stringResource(R.string.device_fingerprint), style = MaterialTheme.typography.labelMedium)
            Text(data.fingerprint.chunked(4).joinToString(" "), style = MaterialTheme.typography.bodySmall, fontFamily = FontFamily.Monospace)
            Text(stringResource(R.string.manual_clipboard_help), style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
        } }
        item { SettingsSection(stringResource(R.string.history_mode)) {
            Row(horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                listOf(MobileHistoryMode.CONTENT to R.string.history_content, MobileHistoryMode.STATUS to R.string.history_status,
                    MobileHistoryMode.OFF to R.string.history_off).forEach { (mode, label) ->
                    FilterChip(selected = data.historyMode == mode, onClick = { state.setHistoryMode(mode) }, label = { Text(stringResource(label)) })
                }
            }
            Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                OutlinedTextField(value = limitText, onValueChange = { if (it.length <= 5 && it.all(Char::isDigit)) limitText = it },
                    label = { Text(stringResource(R.string.history_limit)) }, singleLine = true, modifier = Modifier.weight(1f),
                    keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Number))
                TextButton(onClick = { limitText.toIntOrNull()?.let(state::setHistoryLimit) },
                    enabled = limitText.toIntOrNull()?.let { it in 0..10_000 } == true) { Text(stringResource(R.string.apply_history_limit)) }
            }
            Text(stringResource(R.string.history_limit_help), style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
            HorizontalDivider(color = MaterialTheme.colorScheme.outlineVariant)
            Text(stringResource(R.string.history_count, data.history.size), style = MaterialTheme.typography.bodySmall)
            TextButton(onClick = { confirm = true }) { Text(stringResource(R.string.clear_history), color = MaterialTheme.colorScheme.error) }
        } }
        item { SettingsSection(stringResource(R.string.language)) {
            var showLanguages by remember { mutableStateOf(false) }
            var failed by remember { mutableStateOf(false) }
            val names = listOf(stringResource(R.string.system_language), "English", "Nederlands", "Deutsch", "Français")
            Box {
                TextButton(onClick = { showLanguages = true }) {
                    Text(names[LanguageStore.choices.indexOf(LanguageStore.current(activity))])
                    Spacer(Modifier.weight(1f)); Icon(AppIcon.Down.vector, null)
                }
                DropdownMenu(expanded = showLanguages, onDismissRequest = { showLanguages = false }) {
                    LanguageStore.choices.forEachIndexed { index, code ->
                        DropdownMenuItem(text = { Text(names[index]) }, onClick = {
                            showLanguages = false
                            if (LanguageStore.current(activity) != code) failed = !LanguageStore.change(activity, code)
                        })
                    }
                }
            }
            if (failed) Text(stringResource(R.string.settings_failed), color = MaterialTheme.colorScheme.error)
        } }
        item { SettingsSection(stringResource(R.string.about)) {
            Text(stringResource(R.string.app_name), style = MaterialTheme.typography.titleMedium)
            Text(stringResource(R.string.app_version, BuildConfig.VERSION_NAME), style = MaterialTheme.typography.bodySmall)
        } }
        data.actionStatus?.let { item { Text(actionText(it)) } }
    }
    if (confirm) AlertDialog(onDismissRequest = { confirm = false }, title = { Text(stringResource(R.string.clear_history)) },
        text = { Text(stringResource(R.string.clear_history_help)) },
        confirmButton = { TextButton(onClick = { state.clearHistory(); confirm = false }) { Text(stringResource(R.string.clear)) } },
        dismissButton = { TextButton(onClick = { confirm = false }) { Text(stringResource(R.string.cancel)) } })
}

private fun time(value: ULong): String = DateFormat.getTimeInstance(DateFormat.SHORT).format(Date(value.toLong()))

@Composable
private fun historyActivityText(state: MobileHistoryActivity): String = stringResource(when (state) {
    MobileHistoryActivity.WAITING -> R.string.history_waiting
    MobileHistoryActivity.UPDATING -> R.string.history_updating
    MobileHistoryActivity.RECEIVING -> R.string.history_receiving
    MobileHistoryActivity.UPDATED -> R.string.history_updated
    MobileHistoryActivity.DENIED -> R.string.history_denied
    MobileHistoryActivity.FAILED -> R.string.history_failed
    MobileHistoryActivity.PAUSED -> R.string.history_paused
    MobileHistoryActivity.UNAVAILABLE -> R.string.history_unavailable
})

@Composable
private fun statusText(data: UiSnapshot): String = stringResource(when (data.status) {
    "connected" -> R.string.history_updated
    "tailscale_unavailable" -> R.string.tailscale_unavailable
    "identity_unavailable" -> R.string.identity_unavailable
    "listener_unavailable" -> R.string.listener_unavailable
    else -> R.string.waiting_for_devices
})

@Composable
private fun actionText(key: String): String = stringResource(when (key) {
    "send_queued" -> R.string.send_queued
    "no_send_targets" -> R.string.no_send_targets
    "unsupported_image" -> R.string.unsupported_image
    "unsupported_text" -> R.string.unsupported_text
    "unsupported_clipboard" -> R.string.unsupported_clipboard
    "content_unavailable" -> R.string.content_unavailable
    "copied_to_phone" -> R.string.copied_to_phone
    "copy_uncertain" -> R.string.copy_uncertain
    "settings_failed" -> R.string.settings_failed
    "refresh_unavailable" -> R.string.refresh_unavailable
    else -> R.string.policy_failed
})

private fun transferSymbol(state: MobileTransferState): String = when (state) {
    MobileTransferState.APPLIED -> "✓"
    MobileTransferState.UNKNOWN -> "?"
    MobileTransferState.FAILED -> "!"
    else -> "↑"
}

@Composable
private fun transferState(state: MobileTransferState): String = stringResource(when (state) {
    MobileTransferState.QUEUED -> R.string.transfer_queued
    MobileTransferState.SENDING -> R.string.transfer_sending
    MobileTransferState.APPLIED -> R.string.transfer_applied
    MobileTransferState.FAILED -> R.string.transfer_failed
    MobileTransferState.UNKNOWN -> R.string.transfer_unknown
})
