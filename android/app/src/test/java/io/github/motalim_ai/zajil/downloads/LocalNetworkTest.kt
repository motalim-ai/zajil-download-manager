package io.github.motalim_ai.zajil.downloads

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class LocalNetworkTest {
    @Test
    fun private_and_link_local_addresses_are_local() {
        listOf("10.0.2.2", "172.16.0.1", "172.31.255.254", "192.168.1.5", "169.254.10.1", "fd00::1", "fe80::1")
            .forEach { assertTrue(it, LocalNetwork.isLocalHost(it)) }
    }

    @Test
    fun public_and_loopback_addresses_are_not() {
        listOf("8.8.8.8", "172.15.0.1", "172.32.0.1", "127.0.0.1", "::1", "2001:4860:4860::8888")
            .forEach { assertFalse(it, LocalNetwork.isLocalHost(it)) }
    }

    @Test
    fun home_network_names_are_local() {
        listOf("nas", "printer.local", "router.lan", "box.home.arpa", "media.internal")
            .forEach { assertTrue(it, LocalNetwork.isLocalHost(it)) }
    }

    @Test
    fun internet_names_and_localhost_are_not_by_name_alone() {
        listOf("example.com", "zajil.by-wow.com", "localhost", "LOCALHOST")
            .forEach { assertFalse(it, LocalNetwork.isLocalHost(it)) }
    }
}
