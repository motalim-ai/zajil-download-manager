package io.github.motalim_ai.zajil.update

import java.io.ByteArrayOutputStream
import java.io.File
import java.io.IOException
import java.net.HttpURLConnection
import java.net.URL
import java.security.MessageDigest

/**
 * Small HTTPS fetches for Zajil's own updates (the app's and yt-dlp's), and
 * a video's thumbnail for the format sheet, with Android's TLS checks as
 * always. Never anything else: downloads are the engine's.
 */
object Https {
    /** More bytes than allowed: for a signed size, not the file it names. */
    class TooLong : IOException("longer than allowed")

    /** At most [max] bytes; null if there is no such file (404). */
    fun bytes(url: String, max: Int): ByteArray? = open(url) { connection ->
        connection.inputStream.use { input ->
            val out = ByteArrayOutputStream()
            val buffer = ByteArray(64 * 1024)
            while (true) {
                val n = input.read(buffer)
                if (n < 0) break
                out.write(buffer, 0, n)
                if (out.size() > max) throw TooLong()
            }
            out.toByteArray()
        }
    }

    /**
     * Writes at most [max] bytes to [target] and returns their SHA-256 (hex);
     * null if there is no such file. [progress] gets the bytes so far.
     */
    fun toFile(url: String, target: File, max: Long, progress: (Long) -> Unit = {}): String? = open(url) { connection ->
        val digest = MessageDigest.getInstance("SHA-256")
        var done = 0L
        connection.inputStream.use { input ->
            target.outputStream().use { out ->
                val buffer = ByteArray(256 * 1024)
                while (true) {
                    val n = input.read(buffer)
                    if (n < 0) break
                    done += n
                    if (done > max) throw TooLong()
                    digest.update(buffer, 0, n)
                    out.write(buffer, 0, n)
                    progress(done)
                }
            }
        }
        hex(digest.digest())
    }

    fun sha256(bytes: ByteArray) = hex(MessageDigest.getInstance("SHA-256").digest(bytes))

    private fun hex(bytes: ByteArray) = bytes.joinToString("") { "%02x".format(it) }

    private fun <T> open(url: String, read: (HttpURLConnection) -> T): T? {
        val connection = URL(url).openConnection() as HttpURLConnection
        try {
            connection.connectTimeout = 20_000
            connection.readTimeout = 30_000
            if (connection.responseCode == HttpURLConnection.HTTP_NOT_FOUND) return null
            if (connection.responseCode != HttpURLConnection.HTTP_OK) throw IOException("HTTP ${connection.responseCode}")
            return read(connection)
        } finally {
            connection.disconnect()
        }
    }
}
