# JNA and the UniFFI bindings reach Kotlin from native code by name and by
# reflection: keep them whole.
-keep class com.sun.jna.** { *; }
-keep class * implements com.sun.jna.** { *; }
-keep class io.github.motalim_ai.zajil.engine.** { *; }
-dontwarn java.awt.**

# rustls-platform-verifier's Kotlin half (copied in org/rustls/platformverifier) is called from Rust through JNI only.
-keep, includedescriptorclasses class org.rustls.platformverifier.** { *; }

# What yt-dlp's Python (zajil_video.py, through Chaquopy) calls by name: R8 must keep
# those names, or the call fails in the release build only.
-keep class io.github.motalim_ai.zajil.video.JsBox$Runner { public *; }
-keep class io.github.motalim_ai.zajil.video.JsRun { public *; }
-keep interface io.github.motalim_ai.zajil.video.YtDlp$HlsListener { *; }
-keep class * implements io.github.motalim_ai.zajil.video.YtDlp$HlsListener { *; }
