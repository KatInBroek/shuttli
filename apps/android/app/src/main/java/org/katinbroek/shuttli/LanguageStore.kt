package org.katinbroek.shuttli

import android.app.Activity
import android.content.Context
import android.content.res.Configuration
import java.util.Locale

internal object LanguageStore {
    val choices = listOf("", "en", "nl", "de", "fr")
    fun current(context: Context): String = context.getSharedPreferences("appearance", Context.MODE_PRIVATE)
        .getString("language", "").orEmpty().takeIf { it in choices }.orEmpty()

    fun localizedContext(context: Context): Context {
        val code = current(context)
        if (code.isEmpty()) return context
        val configuration = Configuration(context.resources.configuration).apply { setLocale(Locale.forLanguageTag(code)) }
        return context.createConfigurationContext(configuration)
    }

    fun change(activity: Activity, code: String): Boolean {
        if (code !in choices || !activity.getSharedPreferences("appearance", Context.MODE_PRIVATE)
                .edit().putString("language", code).commit()) return false
        activity.recreate()
        return true
    }
}
