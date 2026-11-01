package io.github.motalim_ai.zajil.downloads

import android.content.Context
import android.net.ConnectivityManager
import android.net.Network
import android.net.NetworkCapabilities
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow

/**
 * The phone's default network, while the app runs: whether there is one,
 * and whether it is unmetered (Wi-Fi), for «Wi-Fi only». When the app is not
 * running, [QueueJobService] waits for the network instead.
 */
class NetworkWatch(context: Context) {
    private val manager = context.getSystemService(ConnectivityManager::class.java)
    private val _state = MutableStateFlow(current())
    val state: StateFlow<NetworkState> = _state.asStateFlow()

    init {
        manager.registerDefaultNetworkCallback(object : ConnectivityManager.NetworkCallback() {
            override fun onCapabilitiesChanged(network: Network, caps: NetworkCapabilities) {
                _state.value = stateOf(caps)
            }

            override fun onLost(network: Network) {
                _state.value = NetworkState(connected = false, unmetered = false)
            }
        })
    }

    private fun current(): NetworkState =
        manager.getNetworkCapabilities(manager.activeNetwork)?.let(::stateOf)
            ?: NetworkState(connected = false, unmetered = false)

    private fun stateOf(caps: NetworkCapabilities) = NetworkState(
        connected = caps.hasCapability(NetworkCapabilities.NET_CAPABILITY_INTERNET),
        unmetered = caps.hasCapability(NetworkCapabilities.NET_CAPABILITY_NOT_METERED),
    )
}
