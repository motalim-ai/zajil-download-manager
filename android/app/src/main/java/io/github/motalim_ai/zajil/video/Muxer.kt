package io.github.motalim_ai.zajil.video

import android.media.MediaCodec
import android.media.MediaExtractor
import android.media.MediaMuxer
import android.os.StatFs
import java.io.File
import java.io.IOException
import java.nio.ByteBuffer

/**
 * Joins a video's picture and sound into one file with Android's own
 * MediaMuxer: the samples are copied as they are, nothing is converted
 * (ADR-0012, section 6). FFmpeg is not needed for what [VideoChoice] picks.
 */
object Muxer {
    /** Room for the joined file next to its parts. */
    fun hasRoom(inputs: List<File>, folder: File): Boolean {
        val needed = inputs.sumOf { it.length() }
        return StatFs(folder.path).availableBytes > needed + 16L * 1024 * 1024
    }

    /** Writes [output] from the first track of each of [inputs]. */
    @Throws(IOException::class)
    fun merge(inputs: List<File>, output: File, container: Container) {
        output.delete()
        val format = if (container == Container.Webm) {
            MediaMuxer.OutputFormat.MUXER_OUTPUT_WEBM
        } else {
            MediaMuxer.OutputFormat.MUXER_OUTPUT_MPEG_4
        }
        val muxer = MediaMuxer(output.path, format)
        val extractors = inputs.map { input -> MediaExtractor().apply { setDataSource(input.path) } }
        try {
            val tracks = extractors.map { extractor ->
                if (extractor.trackCount < 1) throw IOException("a stream with no track")
                extractor.selectTrack(0)
                muxer.addTrack(extractor.getTrackFormat(0))
            }
            muxer.start()
            val buffer = ByteBuffer.allocate(4 * 1024 * 1024)
            val info = MediaCodec.BufferInfo()
            // Interleaved by time, as a player reads them: the stream that is
            // furthest behind gives the next sample.
            val ended = BooleanArray(extractors.size)
            while (!ended.all { it }) {
                val i = extractors.indices.filter { !ended[it] }.minBy { extractors[it].sampleTime }
                val extractor = extractors[i]
                val size = extractor.readSampleData(buffer, 0)
                if (size < 0) {
                    ended[i] = true
                    continue
                }
                info.set(0, size, extractor.sampleTime, flagsOf(extractor.sampleFlags))
                muxer.writeSampleData(tracks[i], buffer, info)
                if (!extractor.advance()) ended[i] = true
            }
            muxer.stop()
        } catch (e: IllegalStateException) {
            output.delete()
            throw IOException("MediaMuxer: ${e.message}", e)
        } finally {
            extractors.forEach { it.release() }
            runCatching { muxer.release() }
        }
    }

    /**
     * Writes [output] (MP4) from every track of [input]: HLS's MPEG-TS, as
     * yt-dlp joined its fragments (A6), with no conversion.
     */
    @Throws(IOException::class)
    fun remux(input: File, output: File) {
        output.delete()
        val muxer = MediaMuxer(output.path, MediaMuxer.OutputFormat.MUXER_OUTPUT_MPEG_4)
        val extractor = MediaExtractor()
        try {
            extractor.setDataSource(input.path)
            if (extractor.trackCount < 1) throw IOException("a stream with no track")
            val tracks = IntArray(extractor.trackCount) { i ->
                extractor.selectTrack(i)
                muxer.addTrack(extractor.getTrackFormat(i))
            }
            muxer.start()
            val buffer = ByteBuffer.allocate(4 * 1024 * 1024)
            val info = MediaCodec.BufferInfo()
            while (true) {
                val track = extractor.sampleTrackIndex
                if (track < 0) break
                val size = extractor.readSampleData(buffer, 0)
                if (size < 0) break
                info.set(0, size, extractor.sampleTime, flagsOf(extractor.sampleFlags))
                muxer.writeSampleData(tracks[track], buffer, info)
                if (!extractor.advance()) break
            }
            muxer.stop()
        } catch (e: IllegalStateException) {
            output.delete()
            throw IOException("MediaMuxer: ${e.message}", e)
        } catch (e: IllegalArgumentException) {
            output.delete()
            throw IOException("MediaMuxer: ${e.message}", e)
        } finally {
            extractor.release()
            runCatching { muxer.release() }
        }
    }

    private fun flagsOf(sampleFlags: Int): Int {
        var flags = 0
        if (sampleFlags and MediaExtractor.SAMPLE_FLAG_SYNC != 0) flags = flags or MediaCodec.BUFFER_FLAG_KEY_FRAME
        return flags
    }
}
