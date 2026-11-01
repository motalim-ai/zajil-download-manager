# JNA and the UniFFI bindings reach Kotlin from native code by name and by
# reflection: keep them whole.
-keep class com.sun.jna.** { *; }
-keep class * implements com.sun.jna.** { *; }
-keep class io.github.motalim_ai.zajil.engine.** { *; }
-dontwarn java.awt.**

# rustls-platform-verifier's Kotlin half (copied in org/rustls/platformverifier) is called from Rust through JNI only.
-keep, includedescriptorclasses class org.rustls.platformverifier.** { *; }
