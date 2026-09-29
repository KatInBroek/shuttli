package org.katinbroek.shuttli

import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent

class MainActivity : ComponentActivity() {
    private val state get() = (application as ShuttliApplication).mobileState
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContent { ShuttliScreen(state, this) }
    }
    override fun onStart() { super.onStart(); state.enterForeground() }
    override fun onStop() { state.enterBackground(); super.onStop() }
}
