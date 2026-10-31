package io.github.motalim_ai.zajil

import android.app.Application
import io.github.motalim_ai.zajil.downloads.DownloadCenter
import io.github.motalim_ai.zajil.engine.Tls

class ZajilApp : Application() {
    lateinit var downloads: DownloadCenter
        private set

    override fun onCreate() {
        super.onCreate()
        System.loadLibrary("zajil_android")
        Tls.init(this)
        downloads = DownloadCenter(this)
    }
}

val android.content.Context.downloads: DownloadCenter
    get() = (applicationContext as ZajilApp).downloads
