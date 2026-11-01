package io.github.motalim_ai.zajil.browser

import java.io.InputStream
import java.util.zip.GZIPInputStream

/** Zajil's protection lists (ADR-0012, section 17): what each one is for. */
enum class ListKind(val file: String) {
    Ads("ads"),
    Trackers("trackers"),
    Malware("malware"),
}

/**
 * One compiled list (`tools/blocklists/compile.py`): its hosts, kept as
 * sorted 64-bit hashes (FNV-1a) and searched in halves, so 50,000 hosts
 * take 400 KB, not megabytes of strings, and a request is matched at once.
 * A host counts if it or a domain it is under is listed, unless it or that
 * domain is kept («@@»). All on the phone: nothing is sent anywhere.
 */
class HostList private constructor(private val blocked: LongArray, private val kept: LongArray) {
    val size get() = blocked.size

    /** Whether [host] (lower case) is listed: it, or a domain it is under. */
    fun matches(host: String): Boolean {
        var at = 0
        var found = false
        while (true) {
            val part = host.substring(at)
            val hash = fnv(part)
            if (kept.binarySearch(hash) >= 0) return false
            if (!found && blocked.binarySearch(hash) >= 0) found = true
            val dot = host.indexOf('.', at)
            // Never the last label alone («com»).
            if (dot < 0 || host.indexOf('.', dot + 1) < 0) return found
            at = dot + 1
        }
    }

    companion object {
        val EMPTY = HostList(LongArray(0), LongArray(0))

        /** A list from its gzip file; at most [MAX_HOSTS] hosts are read. */
        fun read(gzip: InputStream): HostList {
            val blocked = LongArray(MAX_HOSTS)
            var nb = 0
            val kept = ArrayList<Long>()
            GZIPInputStream(gzip).bufferedReader().useLines { lines ->
                for (line in lines) {
                    if (line.isEmpty() || line.startsWith("#")) continue
                    if (line.startsWith("@@")) {
                        if (kept.size < MAX_HOSTS) kept += fnv(line.substring(2))
                    } else if (nb < MAX_HOSTS) {
                        blocked[nb++] = fnv(line)
                    }
                }
            }
            val b = blocked.copyOf(nb).also { it.sort() }
            val k = kept.toLongArray().also { it.sort() }
            return HostList(b, k)
        }

        /** The most hosts read from one list. */
        const val MAX_HOSTS = 400_000

        /** FNV-1a, 64 bits, of [text]'s characters: no two of a list's hosts share one in practice. */
        fun fnv(text: String): Long {
            var hash = -0x340d631b7bdddcdbL
            for (c in text) {
                hash = hash xor c.code.toLong()
                hash *= 0x100000001b3L
            }
            return hash
        }
    }
}
