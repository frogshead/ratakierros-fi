package fi.ratakierros.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.MaterialTheme
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
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import fi.ratakierros.AppContainer
import fi.ratakierros.model.RecordsResponse
import fi.ratakierros.model.formatSeconds

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun LeaderboardScreen(container: AppContainer, trackId: Long, onDone: () -> Unit) {
    val authState by container.auth.state.collectAsState()
    var data by remember { mutableStateOf<RecordsResponse?>(null) }
    var loading by remember { mutableStateOf(true) }
    var error by remember { mutableStateOf<String?>(null) }

    LaunchedEffect(trackId, authState.token) {
        loading = true
        error = null
        try {
            data = container.apiClient.records(trackId, authState.token)
        } catch (t: Throwable) {
            error = t.message ?: "Pyyntö epäonnistui."
        } finally {
            loading = false
        }
    }

    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text(data?.track?.displayName ?: "Tulokset") },
                actions = { TextButton(onClick = onDone) { Text("Sulje") } },
            )
        }
    ) { padding ->
        Column(
            modifier = Modifier
                .fillMaxSize()
                .padding(padding)
                .padding(horizontal = 16.dp),
        ) {
            when {
                loading -> CircularProgressIndicator()
                error != null -> Text(error!!)
                data != null -> {
                    val d = data!!
                    d.personalBest?.let {
                        Text(
                            "Oma ennätys: ${formatSeconds(it)}",
                            style = MaterialTheme.typography.titleMedium,
                            modifier = Modifier.padding(vertical = 8.dp),
                        )
                    }
                    Text("Top 10", style = MaterialTheme.typography.titleSmall, modifier = Modifier.padding(vertical = 8.dp))
                    if (d.records.isEmpty()) {
                        Text("Ei vielä aikoja.")
                    } else {
                        LazyColumn(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                            items(d.records) { rec ->
                                Row(modifier = Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
                                    Text(rankLabel(rec.rank), modifier = Modifier.padding(end = 12.dp))
                                    Column(modifier = Modifier.weight(1f)) {
                                        Text(rec.displayName)
                                        Text(
                                            rec.loggedAt.take(10),
                                            style = MaterialTheme.typography.bodySmall,
                                        )
                                    }
                                    Text(formatSeconds(rec.timeSeconds), fontWeight = FontWeight.Bold)
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

private fun rankLabel(rank: Int): String = when (rank) {
    1 -> "🥇"
    2 -> "🥈"
    3 -> "🥉"
    else -> "$rank."
}
