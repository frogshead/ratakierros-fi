package fi.ratakierros

import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.ui.Modifier
import androidx.navigation.compose.NavHost
import androidx.navigation.compose.composable
import androidx.navigation.compose.rememberNavController
import fi.ratakierros.auth.AuthRepository
import fi.ratakierros.auth.LoginScreen
import fi.ratakierros.location.LocationProvider
import fi.ratakierros.network.ApiClient
import fi.ratakierros.ui.ClosestTrackScreen
import fi.ratakierros.ui.LeaderboardScreen
import fi.ratakierros.ui.LogRunScreen

class MainActivity : ComponentActivity() {
    private lateinit var container: AppContainer

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        container = AppContainer(applicationContext)

        setContent {
            MaterialTheme {
                Surface(modifier = Modifier.fillMaxSize(), color = MaterialTheme.colorScheme.background) {
                    val nav = rememberNavController()
                    NavHost(navController = nav, startDestination = "closest") {
                        composable("closest") {
                            ClosestTrackScreen(
                                container = container,
                                onLogRun = { trackId -> nav.navigate("log/$trackId") },
                                onLeaderboard = { trackId -> nav.navigate("leaderboard/$trackId") },
                                onLogin = { nav.navigate("login") },
                            )
                        }
                        composable("login") {
                            LoginScreen(
                                container = container,
                                onDone = { nav.popBackStack() },
                            )
                        }
                        composable("log/{trackId}") { entry ->
                            val tid = entry.arguments?.getString("trackId")?.toLongOrNull() ?: return@composable
                            LogRunScreen(
                                container = container,
                                trackId = tid,
                                onDone = { nav.popBackStack() },
                                onLoginNeeded = { nav.navigate("login") },
                            )
                        }
                        composable("leaderboard/{trackId}") { entry ->
                            val tid = entry.arguments?.getString("trackId")?.toLongOrNull() ?: return@composable
                            LeaderboardScreen(
                                container = container,
                                trackId = tid,
                                onDone = { nav.popBackStack() },
                            )
                        }
                    }
                }
            }
        }
    }
}

class AppContainer(appContext: android.content.Context) {
    val apiClient: ApiClient = ApiClient(BuildConfig.API_BASE)
    val auth: AuthRepository = AuthRepository(appContext)
    val location: LocationProvider = LocationProvider(appContext)
}
