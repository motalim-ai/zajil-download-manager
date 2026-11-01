package io.github.motalim_ai.zajil

import android.app.Application
import io.github.motalim_ai.zajil.browser.Protection
import io.github.motalim_ai.zajil.downloads.DownloadCenter
import io.github.motalim_ai.zajil.engine.Tls

class ZajilApp : Application() {
    lateinit var downloads: DownloadCenter
        private set

    /** The browser's and the downloads' lists of ads, trackers and harmful sites (0.3.0). */
    lateinit var protection: Protection
        private set

    override fun onCreate() {
        super.onCreate()
        System.loadLibrary("zajil_android")
        Tls.init(this)
        protection = Protection(this)
        downloads = DownloadCenter(this)
    }
}

val android.content.Context.downloads: DownloadCenter
    get() = (applicationContext as ZajilApp).downloads

val android.content.Context.protection: Protection
    get() = (applicationContext as ZajilApp).protection
