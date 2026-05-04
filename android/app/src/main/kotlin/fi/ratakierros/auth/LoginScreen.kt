package fi.ratakierros.auth

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SegmentedButton
import androidx.compose.material3.SegmentedButtonDefaults
import androidx.compose.material3.SingleChoiceSegmentedButtonRow
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.unit.dp
import androidx.compose.foundation.text.KeyboardOptions
import fi.ratakierros.AppContainer
import kotlinx.coroutines.launch

private enum class Mode { Login, Register }

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun LoginScreen(container: AppContainer, onDone: () -> Unit) {
    var mode by remember { mutableStateOf(Mode.Login) }
    var email by remember { mutableStateOf("") }
    var password by remember { mutableStateOf("") }
    var displayName by remember { mutableStateOf("") }
    var error by remember { mutableStateOf<String?>(null) }
    var working by remember { mutableStateOf(false) }
    val scope = rememberCoroutineScope()

    Scaffold(topBar = { TopAppBar(title = { Text("Tili") }) }) { padding ->
        Column(
            modifier = Modifier
                .fillMaxSize()
                .padding(padding)
                .padding(16.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            SingleChoiceSegmentedButtonRow(modifier = Modifier.fillMaxWidth()) {
                SegmentedButton(
                    selected = mode == Mode.Login,
                    onClick = { mode = Mode.Login },
                    shape = SegmentedButtonDefaults.itemShape(0, 2),
                ) { Text("Kirjaudu") }
                SegmentedButton(
                    selected = mode == Mode.Register,
                    onClick = { mode = Mode.Register },
                    shape = SegmentedButtonDefaults.itemShape(1, 2),
                ) { Text("Rekisteröidy") }
            }

            OutlinedTextField(
                value = email,
                onValueChange = { email = it },
                label = { Text("Sähköposti") },
                singleLine = true,
                keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Email),
                modifier = Modifier.fillMaxWidth(),
            )
            if (mode == Mode.Register) {
                OutlinedTextField(
                    value = displayName,
                    onValueChange = { displayName = it },
                    label = { Text("Nimimerkki") },
                    singleLine = true,
                    modifier = Modifier.fillMaxWidth(),
                )
            }
            OutlinedTextField(
                value = password,
                onValueChange = { password = it },
                label = { Text("Salasana") },
                singleLine = true,
                visualTransformation = PasswordVisualTransformation(),
                keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Password),
                modifier = Modifier.fillMaxWidth(),
            )

            error?.let { Text(it, color = Color.Red) }

            Spacer(Modifier.height(8.dp))

            Button(
                enabled = !working && email.isNotBlank() && password.length >= 6 &&
                    (mode == Mode.Login || displayName.isNotBlank()),
                onClick = {
                    working = true
                    error = null
                    scope.launch {
                        try {
                            val resp = if (mode == Mode.Login) {
                                container.apiClient.login(email, password)
                            } else {
                                container.apiClient.register(email, displayName, password)
                            }
                            container.auth.setSession(resp.token, resp.displayName)
                            onDone()
                        } catch (t: Throwable) {
                            error = t.message ?: "Pyyntö epäonnistui."
                        } finally {
                            working = false
                        }
                    }
                },
                modifier = Modifier.fillMaxWidth(),
            ) {
                if (working) CircularProgressIndicator(modifier = Modifier.height(16.dp))
                else Text(if (mode == Mode.Login) "Kirjaudu" else "Rekisteröidy")
            }
        }
    }
}
