package fi.ratakierros.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.unit.dp
import androidx.compose.foundation.text.KeyboardOptions
import fi.ratakierros.AppContainer
import kotlinx.coroutines.launch

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun LogRunScreen(
    container: AppContainer,
    trackId: Long,
    onDone: () -> Unit,
    onLoginNeeded: () -> Unit,
) {
    val authState by container.auth.state.collectAsState()
    var secondsText by remember { mutableStateOf("") }
    var error by remember { mutableStateOf<String?>(null) }
    var success by remember { mutableStateOf(false) }
    var working by remember { mutableStateOf(false) }
    val scope = rememberCoroutineScope()

    LaunchedEffect(authState.isLoggedIn) {
        if (!authState.isLoggedIn) onLoginNeeded()
    }

    val seconds = secondsText.replace(',', '.').toDoubleOrNull()
    val canSubmit = seconds != null && seconds in 30.0..600.0 && authState.isLoggedIn && !working

    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text("Kirjaa aika") },
                actions = { TextButton(onClick = onDone) { Text("Sulje") } },
            )
        }
    ) { padding ->
        Column(
            modifier = Modifier
                .fillMaxSize()
                .padding(padding)
                .padding(16.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            Text("Aika sekunteina (30–600)")
            OutlinedTextField(
                value = secondsText,
                onValueChange = { secondsText = it },
                singleLine = true,
                keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Decimal),
                modifier = Modifier.fillMaxWidth(),
                placeholder = { Text("esim. 65.4") },
            )
            error?.let { Text(it, color = Color.Red) }
            if (success) Text("✓ Aika kirjattu!", color = Color(0xFF2E7D32))

            Button(
                enabled = canSubmit,
                onClick = {
                    val token = authState.token ?: run { onLoginNeeded(); return@Button }
                    val s = seconds ?: return@Button
                    working = true
                    error = null
                    success = false
                    scope.launch {
                        try {
                            container.apiClient.logRun(trackId, s, token)
                            success = true
                        } catch (t: Throwable) {
                            error = t.message ?: "Pyyntö epäonnistui."
                        } finally {
                            working = false
                        }
                    }
                },
                modifier = Modifier.fillMaxWidth(),
            ) {
                if (working) CircularProgressIndicator(modifier = Modifier.padding(end = 8.dp))
                Text("Tallenna")
            }
        }
    }
}
