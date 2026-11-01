package io.github.motalim_ai.zajil

import android.os.Bundle
import android.widget.Toast
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.compose.runtime.getValue
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import io.github.motalim_ai.zajil.downloads.CopiedLinks
import io.github.motalim_ai.zajil.ui.Offer
import io.github.motalim_ai.zajil.ui.Source
import io.github.motalim_ai.zajil.ui.ZajilTheme
import io.github.motalim_ai.zajil.video.VideoPages

/**
 * The Quick Settings tile's window («زاجِل: حمّل الرابط المنسوخ»): transparent
 * over whatever is on the screen. Once it has the focus it reads the
 * clipboard (Android allows that only then), and shows the new-download
 * sheet, or the video sheet for a video page; with no link, a short notice.
 * It closes when the sheet is answered.
 */
class CopiedLinkActivity : ZajilActivity() {
    private var looked = false

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        looked = savedInstanceState?.getBoolean(LOOKED) ?: false
        enableEdgeToEdge()
        setContent {
            val prefs by downloads.prefs.prefs.collectAsStateWithLifecycle()
            ZajilTheme(prefs.look) { Sheets() }
        }
    }

    override fun onSaveInstanceState(outState: Bundle) {
        super.onSaveInstanceState(outState)
        outState.putBoolean(LOOKED, looked)
    }

    override fun onWindowFocusChanged(hasFocus: Boolean) {
        super.onWindowFocusChanged(hasFocus)
        if (hasFocus && !looked) {
            looked = true
            // A moment after the focus, as in the main window: some phones answer nothing at the instant itself.
            window.decorView.postDelayed({ look(retry = true) }, 250)
        }
    }

    private fun look(retry: Boolean) {
        when (val copied = CopiedLinks.read(this, onlyNew = false) { false }) {
            is CopiedLinks.Copied.Link ->
                if (VideoPages.isVideoPage(copied.url)) readVideo(copied.url) else offer.value = Offer(copied.url, Source.Tile)
            CopiedLinks.Copied.Unavailable -> if (retry) window.decorView.postDelayed({ look(retry = false) }, 750) else nothing()
            CopiedLinks.Copied.NoLink, CopiedLinks.Copied.Seen -> nothing()
        }
    }

    private fun nothing() {
        Toast.makeText(this, R.string.tile_no_link, Toast.LENGTH_LONG).show()
        finish()
    }

    override fun added() {
        Toast.makeText(this, R.string.tile_added, Toast.LENGTH_SHORT).show()
    }

    override fun settled() = finish()

    private companion object {
        const val LOOKED = "looked"
    }
}
