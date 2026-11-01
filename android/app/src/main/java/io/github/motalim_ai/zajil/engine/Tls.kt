package io.github.motalim_ai.zajil.engine

import android.content.Context

/**
 * Hands the app's [Context] to the engine's TLS verifier, which checks
 * certificates against Android's own trust store (ADR-0012, section 2).
 * Called once, before any download; until then every HTTPS connection fails.
 */
object Tls {
    @JvmStatic
    external fun init(context: Context)
}
