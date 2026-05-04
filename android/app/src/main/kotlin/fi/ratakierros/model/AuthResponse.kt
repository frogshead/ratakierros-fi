package fi.ratakierros.model

import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable

@Serializable
data class AuthResponse(
    val token: String,
    @SerialName("user_id") val userId: Long,
    @SerialName("display_name") val displayName: String,
)

@Serializable
data class LoginRequest(val email: String, val password: String)

@Serializable
data class RegisterRequest(
    val email: String,
    @SerialName("display_name") val displayName: String,
    val password: String,
)

@Serializable
data class ApiErrorPayload(val error: String? = null)
