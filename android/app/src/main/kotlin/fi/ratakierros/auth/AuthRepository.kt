package fi.ratakierros.auth

import android.content.Context
import androidx.security.crypto.EncryptedSharedPreferences
import androidx.security.crypto.MasterKey
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow

data class AuthState(val token: String? = null, val displayName: String? = null) {
    val isLoggedIn: Boolean get() = token != null
}

class AuthRepository(appContext: Context) {
    private val masterKey = MasterKey.Builder(appContext)
        .setKeyScheme(MasterKey.KeyScheme.AES256_GCM)
        .build()

    private val prefs = EncryptedSharedPreferences.create(
        appContext,
        "ratakierros_auth",
        masterKey,
        EncryptedSharedPreferences.PrefKeyEncryptionScheme.AES256_SIV,
        EncryptedSharedPreferences.PrefValueEncryptionScheme.AES256_GCM,
    )

    private val _state = MutableStateFlow(load())
    val state: StateFlow<AuthState> = _state.asStateFlow()

    private fun load(): AuthState {
        val token = prefs.getString(KEY_TOKEN, null)
        val name = prefs.getString(KEY_NAME, null)
        return AuthState(token, name)
    }

    fun setSession(token: String, displayName: String) {
        prefs.edit().putString(KEY_TOKEN, token).putString(KEY_NAME, displayName).apply()
        _state.value = AuthState(token, displayName)
    }

    fun signOut() {
        prefs.edit().remove(KEY_TOKEN).remove(KEY_NAME).apply()
        _state.value = AuthState()
    }

    private companion object {
        const val KEY_TOKEN = "jwt"
        const val KEY_NAME = "display_name"
    }
}
