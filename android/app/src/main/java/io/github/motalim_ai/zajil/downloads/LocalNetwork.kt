package io.github.motalim_ai.zajil.downloads

import android.content.Context
import android.content.pm.PackageManager
import android.os.Build
import androidx.core.content.ContextCompat
import java.net.Inet4Address
import java.net.Inet6Address
import java.net.InetAddress
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.async
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.withTimeoutOrNull

/**
 * From Android 17 an app reaches a device on the user's own network (a NAS,
 * a router) only with the "local network" permission. Zajil asks for it
 * only when a link points there, never at start: someone who downloads
 * from the internet only never sees it (ADR-0012, section 12; owner's
 * decision of 2026-10-31).
 */
object LocalNetwork {
    const val PERMISSION = "android.permission.ACCESS_LOCAL_NETWORK"

    /** The Android version that enforces it (Android 17). */
    private const val FROM_SDK = 37

    fun isGranted(context: Context): Boolean =
        Build.VERSION.SDK_INT < FROM_SDK ||
            ContextCompat.checkSelfPermission(context, PERMISSION) == PackageManager.PERMISSION_GRANTED

    /**
     * Whether [url] leads to the local network: by its host alone, or else
     * by what the host's name resolves to (a name on the home network that
     * the router answers for). The engine resolves it again when it
     * connects; nothing else is asked of anyone.
     */
    suspend fun isLocal(url: String): Boolean {
        val host = hostOf(url) ?: return false
        if (isLocalHost(host)) return true
        if (literal(host) != null || host.equals("localhost", ignoreCase = true)) return false
        return coroutineScope {
            val lookup = async(Dispatchers.IO) {
                runCatching { InetAddress.getAllByName(host).toList() }.getOrDefault(emptyList())
            }
            withTimeoutOrNull(3_000) { lookup.await() }.orEmpty().any(::isLocalAddress)
        }
    }

    /** The host of an http(s) link, without the brackets of an IPv6 one. */
    fun hostOf(url: String): String? =
        android.net.Uri.parse(url).host?.removePrefix("[")?.removeSuffix("]")?.takeIf { it.isNotEmpty() }

    /**
     * Local by its host alone: a private or link-local address, or a name
     * that only a home network answers for. Loopback (127.0.0.1, ::1,
     * localhost) is this phone itself, not the local network.
     */
    fun isLocalHost(host: String): Boolean {
        literal(host)?.let { return isLocalAddress(it) }
        val name = host.lowercase().trimEnd('.')
        if (name == "localhost") return false
        return '.' !in name || localSuffixes.any { name.endsWith(it) }
    }

    fun isLocalAddress(address: InetAddress): Boolean = when (address) {
        is Inet4Address -> {
            val (a, b) = address.address.map { it.toInt() and 0xff }
            a == 10 || (a == 172 && b in 16..31) || (a == 192 && b == 168) || (a == 169 && b == 254)
        }
        is Inet6Address -> {
            val first = address.address[0].toInt() and 0xff
            val second = address.address[1].toInt() and 0xff
            // fc00::/7 (unique local) and fe80::/10 (link-local).
            (first and 0xfe) == 0xfc || (first == 0xfe && (second and 0xc0) == 0x80)
        }
        else -> false
    }

    /** An IP address written as such; never a lookup. */
    private fun literal(host: String): InetAddress? {
        val looksLikeIp = host.contains(':') || host.all { it.isDigit() || it == '.' }
        if (!looksLikeIp) return null
        return runCatching { InetAddress.getByName(host) }.getOrNull()
    }

    private val localSuffixes = listOf(".local", ".lan", ".home", ".home.arpa", ".internal", ".localdomain")
}
