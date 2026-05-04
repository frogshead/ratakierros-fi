package fi.ratakierros.ui

import android.Manifest
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.wrapContentSize
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.LocationOn
import androidx.compose.material.icons.filled.Person
import androidx.compose.material.icons.filled.PersonOutline
import androidx.compose.material3.Button
import androidx.compose.material3.Card
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import fi.ratakierros.AppContainer
import fi.ratakierros.model.Track
import kotlinx.coroutines.launch

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun ClosestTrackScreen(
    container: AppContainer,
    onLogRun: (Long) -> Unit,
    onLeaderboard: (Long) -> Unit,
    onLogin: () -> Unit,
) {
    val authState by container.auth.state.collectAsState()
    var loading by remember { mutableStateOf(false) }
    var track by remember { mutableStateOf<Track?>(null) }
    var error by remember { mutableStateOf<String?>(null) }
    var permissionDenied by remember { mutableStateOf(false) }
    val scope = rememberCoroutineScope()

    val locationLauncher = rememberLauncherForActivityResult(
        ActivityResultContracts.RequestMultiplePermissions()
    ) { result ->
        val granted = result[Manifest.permission.ACCESS_FINE_LOCATION] == true ||
            result[Manifest.permission.ACCESS_COARSE_LOCATION] == true
        if (granted) {
            permissionDenied = false
            scope.launch { fetch(container, onLoading = { loading = it }, onResult = { track = it; error = null }, onError = { error = it }) }
        } else {
            permissionDenied = true
        }
    }

    LaunchedEffect(Unit) {
        if (container.location.hasPermission()) {
            fetch(container, onLoading = { loading = it }, onResult = { track = it; error = null }, onError = { error = it })
        } else {
            locationLauncher.launch(arrayOf(Manifest.permission.ACCESS_FINE_LOCATION, Manifest.permission.ACCESS_COARSE_LOCATION))
        }
    }

    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text("Lähin rata") },
                navigationIcon = {
                    IconButton(onClick = {
                        if (container.location.hasPermission()) {
                            scope.launch { fetch(container, onLoading = { loading = it }, onResult = { track = it; error = null }, onError = { error = it }) }
                        } else {
                            locationLauncher.launch(arrayOf(Manifest.permission.ACCESS_FINE_LOCATION, Manifest.permission.ACCESS_COARSE_LOCATION))
                        }
                    }) { Icon(Icons.Filled.LocationOn, contentDescription = "Päivitä sijainti") }
                },
                actions = {
                    IconButton(onClick = {
                        if (authState.isLoggedIn) container.auth.signOut() else onLogin()
                    }) {
                        Icon(
                            if (authState.isLoggedIn) Icons.Filled.Person else Icons.Filled.PersonOutline,
                            contentDescription = if (authState.isLoggedIn) "Kirjaudu ulos" else "Kirjaudu sisään",
                        )
                    }
                },
            )
        },
    ) { padding ->
        Body(
            padding = padding,
            loading = loading,
            error = error,
            track = track,
            permissionDenied = permissionDenied,
            isLoggedIn = authState.isLoggedIn,
            onLogRun = onLogRun,
            onLeaderboard = onLeaderboard,
            onLogin = onLogin,
            onRetry = {
                if (container.location.hasPermission()) {
                    scope.launch { fetch(container, onLoading = { loading = it }, onResult = { track = it; error = null }, onError = { error = it }) }
                } else {
                    locationLauncher.launch(arrayOf(Manifest.permission.ACCESS_FINE_LOCATION, Manifest.permission.ACCESS_COARSE_LOCATION))
                }
            },
        )
    }
}

private suspend fun fetch(
    container: AppContainer,
    onLoading: (Boolean) -> Unit,
    onResult: (Track?) -> Unit,
    onError: (String) -> Unit,
) {
    onLoading(true)
    try {
        val coord = container.location.current()
        if (coord == null) {
            onError("Sijaintia ei saatu.")
            return
        }
        val list = container.apiClient.tracks(coord.lat, coord.lon)
        onResult(list.firstOrNull())
        if (list.isEmpty()) onError("Ei ratoja löytynyt.")
    } catch (t: Throwable) {
        onError(t.message ?: "Pyyntö epäonnistui.")
    } finally {
        onLoading(false)
    }
}

@Composable
private fun Body(
    padding: PaddingValues,
    loading: Boolean,
    error: String?,
    track: Track?,
    permissionDenied: Boolean,
    isLoggedIn: Boolean,
    onLogRun: (Long) -> Unit,
    onLeaderboard: (Long) -> Unit,
    onLogin: () -> Unit,
    onRetry: () -> Unit,
) {
    Column(
        modifier = Modifier
            .fillMaxSize()
            .padding(padding)
            .padding(16.dp),
        verticalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        when {
            permissionDenied -> {
                Text("Sijaintilupa tarvitaan lähimmän radan löytämiseen.")
                Button(onClick = onRetry) { Text("Salli sijainti") }
            }
            loading -> {
                CircularProgressIndicator(modifier = Modifier.wrapContentSize(Alignment.Center))
            }
            error != null && track == null -> {
                Text(error)
                Button(onClick = onRetry) { Text("Yritä uudelleen") }
            }
            track != null -> {
                TrackCard(track)
                Row(modifier = Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                    Button(
                        modifier = Modifier.fillMaxWidth(0.5f),
                        onClick = { if (isLoggedIn) onLogRun(track.id) else onLogin() },
                    ) { Text("Kirjaa aika") }
                    OutlinedButton(
                        modifier = Modifier.fillMaxWidth(),
                        onClick = { onLeaderboard(track.id) },
                    ) { Text("Tulokset") }
                }
            }
            else -> {
                Text("Haetaan sijaintia…")
            }
        }
    }
}

@Composable
private fun TrackCard(track: Track) {
    Card(modifier = Modifier.fillMaxWidth()) {
        Column(modifier = Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
            Text(track.displayName, style = MaterialTheme.typography.titleLarge, fontWeight = FontWeight.Bold)
            track.city?.let { Text(it, style = MaterialTheme.typography.bodyMedium) }
            track.distanceLabel?.let { Text("Etäisyys: $it", style = MaterialTheme.typography.bodySmall) }
            track.lanes?.let { Text("Ratoja: $it", style = MaterialTheme.typography.bodySmall) }
            track.surface?.let { Text("Pinta: $it", style = MaterialTheme.typography.bodySmall) }
            track.recordLabel?.let { Text("Ennätys: $it", style = MaterialTheme.typography.bodySmall) }
        }
    }
}
