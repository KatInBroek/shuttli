package org.katinbroek.shuttli

import android.os.Bundle
import android.content.Context
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge

class MainActivity : ComponentActivity() {
    private val state get() = (application as ShuttliApplication).mobileState
    override fun attachBaseContext(newBase: Context) { super.attachBaseContext(LanguageStore.localizedContext(newBase)) }
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()
        setContent { ShuttliScreen(state, this) }
    }
    override fun onStart() { super.onStart(); state.enterForeground() }
    override fun onStop() { state.enterBackground(); super.onStop() }
}
