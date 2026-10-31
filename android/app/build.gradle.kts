import com.android.build.api.artifact.SingleArtifact
import javax.inject.Inject
import javax.xml.parsers.DocumentBuilderFactory
import org.w3c.dom.Element

plugins {
    alias(libs.plugins.android.application)
    alias(libs.plugins.kotlin.compose)
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
