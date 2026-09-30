package org.katinbroek.shuttli

import android.app.Activity
import android.graphics.BitmapFactory
import androidx.activity.compose.BackHandler
import androidx.compose.foundation.Image
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material3.Button
import androidx.compose.material3.FilterChip
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.NavigationBar
import androidx.compose.material3.NavigationBarItem
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.ui.text.input.KeyboardType
import uniffi.shuttli_mobile_ffi.MobileContentKind
import uniffi.shuttli_mobile_ffi.MobileHistoryRow
import uniffi.shuttli_mobile_ffi.MobileHistoryMode
import uniffi.shuttli_mobile_ffi.MobileHistoryActivity
import uniffi.shuttli_mobile_ffi.MobileTransferState
import java.text.DateFormat
import java.util.Date

@Composable
internal fun ShuttliScreen(state: MobileAppState, activity: Activity) {
    val snapshot = state.snapshot
    var tab by remember { mutableIntStateOf(0) }
    MaterialTheme {
        BackHandler(snapshot.selected != null) { state.selectHistory(null) }
        Scaffold(bottomBar = {
            NavigationBar {
                listOf(R.string.home, R.string.history, R.string.devices, R.string.settings).forEachIndexed { index, label ->
                    NavigationBarItem(selected = tab == index, onClick = { state.selectHistory(null); tab = index },
                        icon = { Text(listOf("⌂", "◷", "▣", "⚙")[index]) }, label = { Text(stringResource(label)) })
                }
            }
        }) { padding ->
            Column(Modifier.fillMaxSize().padding(padding).padding(horizontal = 18.dp)) {
                Text(stringResource(R.string.app_name), style = MaterialTheme.typography.headlineMedium,
                    modifier = Modifier.padding(vertical = 18.dp))
                if (snapshot.selected != null) {
                    HistoryDetail(snapshot, state)
                } else when (tab) {
                    0 -> HomePage(snapshot, state, activity)
                    1 -> HistoryPage(snapshot, state)
                    2 -> DevicesPage(snapshot, state)
                    else -> SettingsPage(snapshot, state)
                }
            }
        }
    }
}

@Composable
private fun HomePage(data: UiSnapshot, state: MobileAppState, activity: Activity) {
    LazyColumn(verticalArrangement = Arrangement.spacedBy(12.dp)) {
        item {
            Text(statusText(data), style = MaterialTheme.typography.titleMedium)
            Text(stringResource(R.string.connect_help), style = MaterialTheme.typography.bodySmall)
            data.devices.forEach { device ->
                Text(device.name + ": " + historyActivityText(device.historyActivity), style = MaterialTheme.typography.bodySmall)
            }
        }
        item {
            HorizontalDivider()
            Text(stringResource(R.string.send_from_phone), style = MaterialTheme.typography.titleMedium,
                modifier = Modifier.padding(top = 12.dp))
            Text(stringResource(R.string.manual_clipboard_help), style = MaterialTheme.typography.bodySmall)
            Button(onClick = { state.importClipboard(activity) }) { Text(stringResource(R.string.paste_clipboard)) }
            when (val draft = data.draft) {
                is Draft.Text -> Text(draft.value.take(300), maxLines = 5, overflow = TextOverflow.Ellipsis)
                is Draft.Image -> Text(stringResource(R.string.image_draft, draft.png.size))
                null -> Unit
            }
            if (data.draft != null) {
                Button(onClick = state::sendDraft, enabled = data.devices.any { it.online && it.send }) {
                    Text(stringResource(R.string.send_to_devices))
                }
            }
            data.actionStatus?.let { Text(actionText(it), color = MaterialTheme.colorScheme.secondary) }
        }
        item { HorizontalDivider(); Text(stringResource(R.string.recent_history), style = MaterialTheme.typography.titleMedium) }
        if (data.history.isEmpty()) item { Text(stringResource(R.string.history_empty)) }
        items(data.history.take(3), key = { it.eventKey }) { HistoryRow(it) { state.selectHistory(it) } }
        if (data.transfers.isNotEmpty()) {
            item { Text(stringResource(R.string.sent_from_phone), style = MaterialTheme.typography.titleMedium) }
            items(data.transfers.take(3), key = { it.eventKey + it.targetId }) { transfer ->
                Text("${transfer.targetName}: ${transferState(transfer.state)}")
            }
        }
    }
}

@Composable
private fun HistoryPage(data: UiSnapshot, state: MobileAppState) {
    var filter by remember { mutableIntStateOf(0) }
    Column {
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            listOf(R.string.all, R.string.from_devices, R.string.sent_from_phone).forEachIndexed { index, label ->
                FilterChip(selected = filter == index, onClick = { filter = index }, label = { Text(stringResource(label)) })
            }
        }
        val rows = data.history.filter { filter == 0 || (filter == 2) == it.isLocal }
        if (rows.isEmpty()) Text(stringResource(R.string.history_empty))
        else LazyColumn { items(rows, key = { it.eventKey }) { HistoryRow(it) { state.selectHistory(it) } } }
    }
}

@Composable
private fun HistoryRow(row: MobileHistoryRow, onClick: () -> Unit) {
    Column(Modifier.fillMaxWidth().clickable(onClick = onClick).padding(vertical = 12.dp)) {
        Text(if (row.kind == MobileContentKind.TEXT) stringResource(R.string.text_item) else stringResource(R.string.image_item),
            style = MaterialTheme.typography.titleMedium)
        Text(if (row.isLocal) stringResource(R.string.this_phone) else row.sourceName)
        Text(DateFormat.getDateTimeInstance(DateFormat.SHORT, DateFormat.SHORT).format(Date(row.copiedAtMs.toLong())),
            style = MaterialTheme.typography.bodySmall)
        Text(stringResource(if (row.available) R.string.item_available else if (row.receiving) R.string.history_receiving else R.string.item_listed),
            style = MaterialTheme.typography.bodySmall)
        HorizontalDivider()
    }
}

@Composable
private fun HistoryDetail(data: UiSnapshot, state: MobileAppState) {
    val row = data.selected ?: return
    Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
        TextButton(onClick = { state.selectHistory(null) }) { Text(stringResource(R.string.back)) }
        Text(if (row.isLocal) stringResource(R.string.this_phone) else row.sourceName,
            style = MaterialTheme.typography.titleLarge)
        Text(stringResource(R.string.bytes, row.bytes.toLong()))
        val body = data.preview
        if (body == null) Text(stringResource(R.string.content_unavailable))
        else if (row.kind == MobileContentKind.TEXT) {
            Text(body.toString(Charsets.UTF_8).take(10000), maxLines = 15)
        } else {
            val bitmap = remember(row.eventKey, body) { BitmapFactory.decodeByteArray(body, 0, body.size) }
            if (bitmap != null) Image(bitmap.asImageBitmap(), contentDescription = stringResource(R.string.image_item),
                modifier = Modifier.fillMaxWidth().height(240.dp))
            else Text(stringResource(R.string.preview_unavailable))
        }
        Button(onClick = { state.copyToPhone(row) }, enabled = body != null) { Text(stringResource(R.string.copy_to_phone)) }
        if (row.isLocal) {
            Text(stringResource(R.string.sent_from_phone), style = MaterialTheme.typography.titleMedium)
            data.transfers.filter { it.eventKey == row.eventKey }.forEach { transfer ->
                Text("${transfer.targetName}: ${transferState(transfer.state)}")
            }
            Button(onClick = { state.resend(row) }, enabled = body != null && data.devices.any { it.online && it.send }) {
                Text(stringResource(R.string.send_again))
            }
        }
        data.actionStatus?.let { Text(actionText(it)) }
    }
}

@Composable
private fun DevicesPage(data: UiSnapshot, state: MobileAppState) {
    if (data.devices.isEmpty()) { Text(stringResource(R.string.no_connected_devices)); return }
    LazyColumn {
        items(data.devices, key = { it.id }) { device ->
            Column(Modifier.fillMaxWidth().padding(vertical = 10.dp)) {
                Text(device.name, style = MaterialTheme.typography.titleMedium)
                Text(stringResource(if (device.online) R.string.device_online else R.string.device_offline))
                Text(historyActivityText(device.historyActivity), style = MaterialTheme.typography.bodySmall)
                if (device.checkedAtMs > 0uL) Text(stringResource(R.string.history_checked,
                    DateFormat.getDateTimeInstance(DateFormat.SHORT, DateFormat.SHORT).format(Date(device.checkedAtMs.toLong()))),
                    style = MaterialTheme.typography.bodySmall)
                if (device.historyPartial) Text(stringResource(R.string.history_partial), style = MaterialTheme.typography.bodySmall)
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Text(stringResource(R.string.receive_from_device), modifier = Modifier.weight(1f))
                    Switch(checked = device.receive, onCheckedChange = { state.setDirections(device, receive = it) })
                }
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Text(stringResource(R.string.send_to_device), modifier = Modifier.weight(1f))
                    Switch(checked = device.send, onCheckedChange = { state.setDirections(device, send = it) })
                }
                Text(stringResource(R.string.device_consent_help), style = MaterialTheme.typography.bodySmall)
                HorizontalDivider()
            }
        }
    }
}

@Composable
private fun SettingsPage(data: UiSnapshot, state: MobileAppState) {
    var confirm by remember { mutableStateOf(false) }
    var limitText by remember(data.historyLimit) { mutableStateOf(data.historyLimit.toString()) }
    Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
        Text(stringResource(R.string.app_version, BuildConfig.VERSION_NAME), style = MaterialTheme.typography.bodySmall)
        Text(stringResource(R.string.privacy), style = MaterialTheme.typography.titleLarge)
        Text(stringResource(R.string.manual_clipboard_help))
        Text(stringResource(R.string.history_mode), style = MaterialTheme.typography.titleMedium)
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            listOf(MobileHistoryMode.OFF to R.string.history_off,
                MobileHistoryMode.STATUS to R.string.history_status,
                MobileHistoryMode.CONTENT to R.string.history_content).forEach { (mode, label) ->
                FilterChip(selected = data.historyMode == mode, onClick = { state.setHistoryMode(mode) },
                    label = { Text(stringResource(label)) })
            }
        }
        OutlinedTextField(value = limitText, onValueChange = { if (it.length <= 5 && it.all(Char::isDigit)) limitText = it },
            label = { Text(stringResource(R.string.history_limit)) },
            keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Number))
        Button(onClick = { limitText.toIntOrNull()?.let(state::setHistoryLimit) },
            enabled = limitText.toIntOrNull()?.let { it in 0..10_000 } == true) { Text(stringResource(R.string.apply_history_limit)) }
        Text(stringResource(R.string.history_limit_help), style = MaterialTheme.typography.bodySmall)
        Button(onClick = { confirm = true }) { Text(stringResource(R.string.clear_history)) }
        if (confirm) {
            androidx.compose.material3.AlertDialog(onDismissRequest = { confirm = false },
                title = { Text(stringResource(R.string.clear_history)) },
                text = { Text(stringResource(R.string.clear_history_help)) },
                confirmButton = { TextButton(onClick = { state.clearHistory(); confirm = false }) { Text(stringResource(R.string.clear)) } },
                dismissButton = { TextButton(onClick = { confirm = false }) { Text(stringResource(R.string.cancel)) } })
        }
        Text(stringResource(R.string.history_count, data.history.size))
        data.actionStatus?.let { Text(actionText(it)) }
    }
}

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
private fun statusText(data: UiSnapshot): String = when (data.status) {
    "connected" -> stringResource(R.string.devices_connected, data.connected.toInt())
    "tailscale_unavailable" -> stringResource(R.string.tailscale_unavailable)
    "identity_unavailable" -> stringResource(R.string.identity_unavailable)
    "listener_unavailable" -> stringResource(R.string.listener_unavailable)
    else -> stringResource(R.string.waiting_for_devices)
}

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
    else -> R.string.policy_failed
})

@Composable
private fun transferState(state: MobileTransferState): String = stringResource(when (state) {
    MobileTransferState.QUEUED -> R.string.transfer_queued
    MobileTransferState.SENDING -> R.string.transfer_sending
    MobileTransferState.APPLIED -> R.string.transfer_applied
    MobileTransferState.FAILED -> R.string.transfer_failed
    MobileTransferState.UNKNOWN -> R.string.transfer_unknown
})
