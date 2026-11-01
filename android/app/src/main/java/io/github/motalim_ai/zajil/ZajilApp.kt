package io.github.motalim_ai.zajil

import android.app.Application
import android.os.Build
import android.os.Process
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
        // QuickJS's isolated process (JsBoxService): none of the app, its engine or its files.
        if (isolated()) return
        System.loadLibrary("zajil_android")
        Tls.init(this)
        protection = Protection(this)
        downloads = DownloadCenter(this)
    }
}

/** Whether this process is an isolated one: its own user, from 90000 on (Android's own test before 9). */
private fun isolated(): Boolean =
    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.P) Process.isIsolated() else Process.myUid() % 100_000 >= 90_000

val android.content.Context.downloads: DownloadCenter
    get() = (applicationContext as ZajilApp).downloads

val android.content.Context.protection: Protection
    get() = (applicationContext as ZajilApp).protection
