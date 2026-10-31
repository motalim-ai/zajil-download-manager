import com.android.build.api.artifact.SingleArtifact
import java.net.URI
import java.security.MessageDigest
import javax.inject.Inject
import javax.xml.parsers.DocumentBuilderFactory
import org.w3c.dom.Element

plugins {
    alias(libs.plugins.android.application)
    alias(libs.plugins.kotlin.compose)
    alias(libs.plugins.chaquopy)
}

/**
 * A tool's pin from tools/<tool>/pins.txt, the desktop's own (ADR-0009):
 * its version, and for [target] its release file and SHA-256. One place for
 * both apps' versions.
 */
fun pin(tool: String, target: String? = null): List<String> {
    val lines = rootProject.file("../tools/$tool/pins.txt").readLines()
        .map(String::trim).filter { it.isNotEmpty() && !it.startsWith("#") }
    val version = lines.first { it.startsWith("version ") }.substringAfter(' ')
    if (target == null) return listOf(version)
    val (_, file, sha256) = lines.first { it.startsWith("$target ") }.split(Regex("\\s+"))
    return listOf(version, file, sha256)
}

// Zajil for Android has its own version (ADR-0012, owner's decision 4):
// tags android-vX.Y.Z. The version code only ever grows.
val zajilVersion = "0.1.0"
val zajilVersionCode = zajilVersion.split('.').map(String::toInt).let { (major, minor, patch) ->
    major * 1_000_000 + minor * 1_000 + patch
}

android {
    namespace = "io.github.motalim_ai.zajil"
    compileSdk = 37

    defaultConfig {
        applicationId = "io.github.motalim_ai.zajil"
        minSdk = 26
        targetSdk = 37
        versionCode = zajilVersionCode
        versionName = zajilVersion
        // The engine is built for arm64-v8a only for now (ADR-0012,
        // section 9; armeabi-v7a comes before A5).
        ndk { abiFilters += listOf("arm64-v8a") }
    }

    packaging {
        // The tools run as programs (QuickJS-NG for yt-dlp), which Android
        // allows only from the folder it unpacks native libraries into.
        jniLibs.useLegacyPackaging = true
        // QuickJS-NG exactly as pinned: its SHA-256 can be checked in the APK.
        jniLibs.keepDebugSymbols += "**/libqjs.so"
    }

    buildTypes {
        debug {
            // Lives beside an installed Zajil instead of replacing it.
            applicationIdSuffix = ".debug"
            versionNameSuffix = "-debug"
        }
        release {
            // Signed in CI only (A4); a local release build is unsigned.
            isMinifyEnabled = true
            isShrinkResources = true
            proguardFiles(getDefaultProguardFile("proguard-android-optimize.txt"), "proguard-rules.pro")
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    ndkVersion = "29.0.14206865"

    buildFeatures {
        compose = true
    }
}

/** Builds the engine and its Kotlin bindings with build-engine.sh. */
abstract class BuildEngine @Inject constructor(private val exec: ExecOperations) : DefaultTask() {
    @get:InputFiles
    @get:PathSensitive(PathSensitivity.RELATIVE)
    abstract val sources: ConfigurableFileCollection

    @get:InputFile
    @get:PathSensitive(PathSensitivity.NONE)
    abstract val script: RegularFileProperty

    @get:Input
    abstract val ndk: Property<String>

    @get:OutputDirectory
    abstract val jniLibs: DirectoryProperty

    @get:OutputDirectory
    abstract val kotlin: DirectoryProperty

    @TaskAction
    fun build() {
        exec.exec {
            commandLine(
                "sh",
                script.get().asFile.path,
                jniLibs.get().asFile.path,
                kotlin.get().asFile.path,
            )
            environment("ANDROID_NDK_HOME", ndk.get())
        }
    }
}

/**
 * The QuickJS-NG that yt-dlp needs for YouTube's JavaScript challenges
 * (ADR-0009, ADR-0012): the desktop's pinned linux-aarch64 release, a
 * static program that runs on Android as it is. Downloaded once over HTTPS
 * (checked as always), its SHA-256 checked against the pin, and shipped as
 * libqjs.so so that Android unpacks it where a program may run.
 */
abstract class FetchQuickJs : DefaultTask() {
    @get:Input
    abstract val url: Property<String>

    @get:Input
    abstract val sha256: Property<String>

    @get:Internal
    abstract val cache: DirectoryProperty

    @get:OutputDirectory
    abstract val jniLibs: DirectoryProperty

    @TaskAction
    fun fetch() {
        val cached = cache.get().file("qjs-${sha256.get()}").asFile
        if (!cached.isFile || digest(cached.readBytes()) != sha256.get()) {
            cached.parentFile.mkdirs()
            val bytes = URI(url.get()).toURL().openStream().use { it.readBytes() }
            val got = digest(bytes)
            if (got != sha256.get()) throw GradleException("QuickJS-NG: SHA-256 $got, not the pinned ${sha256.get()}")
            cached.writeBytes(bytes)
        }
        val out = jniLibs.get().dir("arm64-v8a").asFile.apply { mkdirs() }
        cached.copyTo(File(out, "libqjs.so"), overwrite = true)
    }

    private fun digest(bytes: ByteArray) =
        MessageDigest.getInstance("SHA-256").digest(bytes).joinToString("") { "%02x".format(it) }
}

val fetchQuickJs = tasks.register<FetchQuickJs>("fetchQuickJs") {
    val (version, file, sha) = pin("quickjs", "linux-aarch64")
    url = "https://github.com/quickjs-ng/quickjs/releases/download/$version/$file"
    sha256 = sha
    cache = rootProject.layout.projectDirectory.dir(".gradle/zajil-tools")
    jniLibs = layout.buildDirectory.dir("quickjs/jniLibs")
}

// yt-dlp on the device (ADR-0012, section 6): the desktop's pinned version,
// with the scripts it runs in QuickJS for YouTube, and certifi so that it
// checks TLS certificates.
chaquopy {
    defaultConfig {
        version = "3.13"
        providers.gradleProperty("zajil.buildPython").orNull?.let { buildPython(it) }
        pip {
            install("yt-dlp==${pin("yt-dlp")[0]}")
            install("yt-dlp-ejs==0.8.0")
            install("certifi==2026.10.23")
        }
    }
}

val workspace = rootProject.layout.projectDirectory.dir("..")
val buildEngine = tasks.register<BuildEngine>("buildEngine") {
    sources.from(
        workspace.dir("crates").asFileTree.matching { exclude("**/target/**") },
        workspace.file("Cargo.toml"),
        workspace.file("Cargo.lock"),
    )
    script = rootProject.layout.projectDirectory.file("build-engine.sh")
    ndk = androidComponents.sdkComponents.ndkDirectory.map { it.asFile.path }
    jniLibs = layout.buildDirectory.dir("rust/jniLibs")
    kotlin = layout.buildDirectory.dir("rust/kotlin")
}

/**
 * Fails the build if the merged manifest (with everything the libraries add)
 * asks for a permission, or exports a component, that ADR-0012 (section 11)
 * does not list. Any change to these lists changes the ADR first.
 */
abstract class CheckManifest : DefaultTask() {
    @get:InputFile
    abstract val manifest: RegularFileProperty

    @get:Input
    abstract val allowedPermissions: SetProperty<String>

    @get:Input
    abstract val allowedExported: SetProperty<String>

    @TaskAction
    fun check() {
        val android = "http://schemas.android.com/apk/res/android"
        val document = DocumentBuilderFactory.newInstance()
            .apply { isNamespaceAware = true }
            .newDocumentBuilder()
            .parse(manifest.get().asFile)
        fun elements(tag: String) = document.getElementsByTagName(tag).let { list ->
            (0 until list.length).map { list.item(it) as Element }
        }
        val permissions = elements("uses-permission").map { it.getAttributeNS(android, "name") } +
            elements("uses-permission-sdk-23").map { it.getAttributeNS(android, "name") }
        val exported = listOf("activity", "activity-alias", "service", "receiver", "provider")
            .flatMap(::elements)
            .filter { it.getAttributeNS(android, "exported") == "true" }
            .map { it.getAttributeNS(android, "name") }
        val problems = permissions.filterNot { it in allowedPermissions.get() }
            .map { "permission not in ADR-0012: $it" } +
            exported.filterNot { it in allowedExported.get() }
                .map { "exported component not in ADR-0012: $it" }
        if (problems.isNotEmpty()) throw GradleException(problems.joinToString("\n"))
    }
}

androidComponents {
    onVariants { variant ->
        variant.sources.jniLibs?.addGeneratedSourceDirectory(buildEngine, BuildEngine::jniLibs)
        variant.sources.kotlin?.addGeneratedSourceDirectory(buildEngine, BuildEngine::kotlin)
        variant.sources.jniLibs?.addGeneratedSourceDirectory(fetchQuickJs, FetchQuickJs::jniLibs)

        val name = variant.name.replaceFirstChar(Char::uppercase)
        val check = tasks.register<CheckManifest>("check${name}Permissions") {
            manifest = variant.artifacts.get(SingleArtifact.MERGED_MANIFEST)
            allowedPermissions = setOf(
                "android.permission.INTERNET",
                "android.permission.ACCESS_NETWORK_STATE",
                "android.permission.FOREGROUND_SERVICE",
                "android.permission.FOREGROUND_SERVICE_DATA_SYNC",
                "android.permission.POST_NOTIFICATIONS",
                "android.permission.RECEIVE_BOOT_COMPLETED",
                "android.permission.WRITE_EXTERNAL_STORAGE",
                // Asked only for a link on the local network (Android 17).
                "android.permission.ACCESS_LOCAL_NETWORK",
                // AndroidX's own signature permission for receivers it
                // registers unexported (core-ktx): no other app gets it.
                "${variant.applicationId.get()}.DYNAMIC_RECEIVER_NOT_EXPORTED_PERMISSION",
            )
            allowedExported = setOf("io.github.motalim_ai.zajil.MainActivity")
        }
        tasks.matching { it.name == "assemble$name" }.configureEach { dependsOn(check) }
    }
}

// rustls-platform-verifier's Kotlin half must be the version its Rust half
// expects: the one in Cargo.lock, read at configuration time.
val verifierVersion = providers.fileContents(workspace.file("Cargo.lock")).asText.map { lock ->
    val lines = lock.lines()
    val at = lines.indexOfFirst { it.trim() == "name = \"rustls-platform-verifier-android\"" }
    check(at >= 0) { "rustls-platform-verifier-android is not in Cargo.lock" }
    lines[at + 1].substringAfter('"').substringBefore('"')
}
configurations.configureEach {
    resolutionStrategy.eachDependency {
        if (requested.group == "org.rustls" && requested.name == "rustls-platform-verifier") {
            useVersion(verifierVersion.get())
            because("the version of rustls-platform-verifier-android in Cargo.lock")
        }
    }
}

dependencies {
    implementation(libs.androidx.core.ktx)
    implementation(libs.androidx.activity.compose)
    implementation(libs.androidx.lifecycle.runtime.compose)
    implementation(libs.androidx.lifecycle.service)
    implementation(libs.kotlinx.coroutines.android)
    implementation(platform(libs.compose.bom))
    implementation(libs.compose.ui)
    implementation(libs.compose.foundation)
    implementation(libs.compose.material3)
    implementation("${libs.jna.get()}@aar")
    implementation(libs.rustls.platform.verifier)
    testImplementation(libs.junit)
}
