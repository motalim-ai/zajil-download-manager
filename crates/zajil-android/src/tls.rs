//! Android only: hands the app's `Context` to `rustls-platform-verifier`,
//! which checks TLS certificates against Android's own trust store and fails
//! every connection until it has one (ADR-0012, section 2). The one JNI
//! entry point of the app: UniFFI cannot pass a JNI environment.
//!
//! `cfg(target_os)` outside `zajil-platform`: this crate is Android's own,
//! and the module only builds there (exception recorded in ADR-0012).

use jni::EnvUnowned;
use jni::errors::ThrowRuntimeExAndDefault;
use jni::objects::{JClass, JObject};

/// `io.github.motalim_ai.zajil.engine.Tls.init(context)`, called once at
/// start, before any download.
#[allow(unsafe_code)] // `no_mangle`: the name the JVM looks up.
#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_motalim_1ai_zajil_engine_Tls_init<'caller>(
    mut env: EnvUnowned<'caller>,
    _class: JClass<'caller>,
    context: JObject<'caller>,
) {
    env.with_env(|env| rustls_platform_verifier::android::init_with_env(env, context))
        .resolve::<ThrowRuntimeExAndDefault>()
}
