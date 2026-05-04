package fi.ratakierros.model

import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable

@Serializable
data class Track(
    val id: Long,
    @SerialName("lipas_id") val lipasId: Long? = null,
    val name: String? = null,
    val lat: Double,
    val lon: Double,
    val city: String? = null,
    val suburb: String? = null,
    val address: String? = null,
    @SerialName("postal_code") val postalCode: String? = null,
    val surface: String? = null,
    @SerialName("track_length_m") val trackLengthM: Long? = null,
    val lanes: Long? = null,
    val status: String? = null,
    @SerialName("type_code") val typeCode: Long? = null,
    @SerialName("distance_m") val distanceM: Double? = null,
    val record: Double? = null,
) {
    val displayName: String get() = name ?: "—"

    val distanceLabel: String? get() = distanceM?.let {
        if (it < 1000) String.format("%.0f m", it) else String.format("%.1f km", it / 1000.0)
    }

    val recordLabel: String? get() = record?.let { formatSeconds(it) }
}

fun formatSeconds(seconds: Double): String {
    val total = if (seconds < 0) 0.0 else seconds
    val minutes = total.toInt() / 60
    val remainder = total - minutes * 60
    return String.format("%d:%05.2f", minutes, remainder)
}
