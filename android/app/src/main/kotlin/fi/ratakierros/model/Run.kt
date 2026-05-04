package fi.ratakierros.model

import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable

@Serializable
data class LogRunRequest(
    @SerialName("track_id") val trackId: Long,
    @SerialName("time_seconds") val timeSeconds: Double,
)

@Serializable
data class LogRunResponse(val ok: Boolean)

@Serializable
data class RecordEntry(
    val rank: Int,
    @SerialName("display_name") val displayName: String,
    @SerialName("time_seconds") val timeSeconds: Double,
    @SerialName("logged_at") val loggedAt: String,
)

@Serializable
data class RecordsResponse(
    val track: Track,
    val records: List<RecordEntry>,
    @SerialName("personal_best") val personalBest: Double? = null,
)
