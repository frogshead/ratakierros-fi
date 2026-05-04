package fi.ratakierros.network

import com.jakewharton.retrofit2.converter.kotlinx.serialization.asConverterFactory
import fi.ratakierros.auth.AuthRepository
import fi.ratakierros.model.AuthResponse
import fi.ratakierros.model.LogRunRequest
import fi.ratakierros.model.LogRunResponse
import fi.ratakierros.model.LoginRequest
import fi.ratakierros.model.RecordsResponse
import fi.ratakierros.model.RegisterRequest
import fi.ratakierros.model.Track
import kotlinx.serialization.json.Json
import okhttp3.MediaType.Companion.toMediaType
import okhttp3.OkHttpClient
import retrofit2.Retrofit
import retrofit2.http.Body
import retrofit2.http.GET
import retrofit2.http.Header
import retrofit2.http.POST
import retrofit2.http.Path
import retrofit2.http.Query

class ApiClient(baseUrl: String) {
    private val json = Json {
        ignoreUnknownKeys = true
        explicitNulls = false
    }

    private val client = OkHttpClient.Builder().build()

    private val service: Service = Retrofit.Builder()
        .baseUrl(baseUrl.trimEnd('/') + "/")
        .client(client)
        .addConverterFactory(json.asConverterFactory("application/json".toMediaType()))
        .build()
        .create(Service::class.java)

    suspend fun tracks(lat: Double?, lon: Double?, query: String? = null): List<Track> =
        service.tracks(lat, lon, query?.takeIf { it.isNotBlank() })

    suspend fun records(trackId: Long, token: String?): RecordsResponse =
        service.records(trackId, token?.let { "Bearer $it" })

    suspend fun logRun(trackId: Long, seconds: Double, token: String): LogRunResponse =
        service.logRun(LogRunRequest(trackId, seconds), "Bearer $token")

    suspend fun login(email: String, password: String): AuthResponse =
        service.login(LoginRequest(email, password))

    suspend fun register(email: String, displayName: String, password: String): AuthResponse =
        service.register(RegisterRequest(email, displayName, password))

    interface Service {
        @GET("api/tracks")
        suspend fun tracks(
            @Query("lat") lat: Double?,
            @Query("lon") lon: Double?,
            @Query("q") q: String?,
        ): List<Track>

        @GET("api/tracks/{id}/records")
        suspend fun records(
            @Path("id") id: Long,
            @Header("Authorization") authorization: String?,
        ): RecordsResponse

        @POST("api/runs")
        suspend fun logRun(
            @Body body: LogRunRequest,
            @Header("Authorization") authorization: String,
        ): LogRunResponse

        @POST("api/auth/login")
        suspend fun login(@Body body: LoginRequest): AuthResponse

        @POST("api/auth/register")
        suspend fun register(@Body body: RegisterRequest): AuthResponse
    }
}
