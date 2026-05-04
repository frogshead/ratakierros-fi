package fi.ratakierros.location

import android.Manifest
import android.annotation.SuppressLint
import android.content.Context
import android.content.pm.PackageManager
import androidx.core.content.ContextCompat
import com.google.android.gms.location.LocationServices
import com.google.android.gms.location.Priority
import kotlinx.coroutines.suspendCancellableCoroutine
import kotlin.coroutines.resume
import kotlin.coroutines.resumeWithException

data class LatLon(val lat: Double, val lon: Double)

class LocationProvider(private val appContext: Context) {
    private val client by lazy { LocationServices.getFusedLocationProviderClient(appContext) }

    fun hasPermission(): Boolean =
        ContextCompat.checkSelfPermission(appContext, Manifest.permission.ACCESS_FINE_LOCATION) ==
            PackageManager.PERMISSION_GRANTED ||
            ContextCompat.checkSelfPermission(appContext, Manifest.permission.ACCESS_COARSE_LOCATION) ==
            PackageManager.PERMISSION_GRANTED

    @SuppressLint("MissingPermission")
    suspend fun current(): LatLon? {
        if (!hasPermission()) return null
        return suspendCancellableCoroutine { cont ->
            client.getCurrentLocation(Priority.PRIORITY_BALANCED_POWER_ACCURACY, null)
                .addOnSuccessListener { loc ->
                    cont.resume(loc?.let { LatLon(it.latitude, it.longitude) })
                }
                .addOnFailureListener { cont.resumeWithException(it) }
        }
    }
}
