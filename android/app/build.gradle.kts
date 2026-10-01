import java.io.ByteArrayOutputStream
import javax.inject.Inject
import org.gradle.process.ExecOperations

plugins {
    alias(libs.plugins.android.application)
    alias(libs.plugins.kotlin.compose)
    alias(libs.plugins.roborazzi)
}

// The Cargo workspace: the repository this Android project sits in.
val cargoWorkspace: Directory = rootProject.layout.projectDirectory.dir("..")
val minSdkVersion = 26

android {
    namespace = "soy.iko.fold"
    compileSdk = 37
    ndkVersion = "28.2.13676358"

    defaultConfig {
        applicationId = "soy.iko.fold"
        minSdk = minSdkVersion
        targetSdk = 36
        versionCode = 1
        versionName = "0.1.0"
        // only the ABIs the Rust core is built for: JNA ships more
        ndk {
            abiFilters += providers.gradleProperty("fold.abis").get().split(",").map(String::trim)
        }
    }

    buildTypes {
        release {
            isMinifyEnabled = true
            isShrinkResources = true
            proguardFiles(getDefaultProguardFile("proguard-android-optimize.txt"), "proguard-rules.pro")
        }
    }

    buildFeatures {
        compose = true
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    testOptions {
        unitTests {
            isIncludeAndroidResources = true
        }
    }
}

// ------------------------------------------------------------ the Rust core

/** Builds `crates/fold-ffi` for each Android ABI with cargo-ndk. */
abstract class CargoNdk : DefaultTask() {
    @get:Inject abstract val exec: ExecOperations

    @get:Input abstract val abis: ListProperty<String>

    @get:Input abstract val platform: Property<Int>

    @get:Internal abstract val workspace: DirectoryProperty

    @get:Internal abstract val ndk: DirectoryProperty

    @get:InputFiles
    @get:PathSensitive(PathSensitivity.RELATIVE)
    abstract val sources: ConfigurableFileCollection

    @get:OutputDirectory abstract val outputDir: DirectoryProperty

    @TaskAction
    fun build() {
        val out = outputDir.get().asFile
        out.deleteRecursively()
        out.mkdirs()
        exec.exec {
            workingDir = workspace.get().asFile
            environment("ANDROID_NDK_HOME", ndk.get().asFile.absolutePath)
            commandLine(
                listOf("cargo", "ndk") +
                    abis.get().flatMap { listOf("-t", it) } +
                    listOf("--platform", platform.get().toString(), "-o", out.absolutePath) +
                    listOf("build", "--release", "--locked", "-p", "fold-ffi", "--lib"),
            )
        }
    }
}

/**
 * Builds `crates/fold-ffi` for this machine and generates its Kotlin
 * bindings from it with uniffi-bindgen. The host library is also what the
 * JVM unit tests load.
 */
abstract class UniffiBindgen : DefaultTask() {
    @get:Inject abstract val exec: ExecOperations

    @get:Internal abstract val workspace: DirectoryProperty

    @get:InputFiles
    @get:PathSensitive(PathSensitivity.RELATIVE)
    abstract val sources: ConfigurableFileCollection

    @get:OutputDirectory abstract val outputDir: DirectoryProperty

    @get:OutputDirectory abstract val hostLibDir: DirectoryProperty

    @TaskAction
    fun generate() {
        val root = workspace.get().asFile
        val os = System.getProperty("os.name").lowercase()
        val lib = when {
            os.contains("mac") -> "libfold_ffi.dylib"
            os.contains("windows") -> "fold_ffi.dll"
            else -> "libfold_ffi.so"
        }
        exec.exec {
            workingDir = root
            commandLine("cargo", "build", "--release", "--locked", "-p", "fold-ffi", "--lib")
        }
        val built = root.resolve("target/release/$lib")
        val hostDir = hostLibDir.get().asFile
        hostDir.mkdirs()
        built.copyTo(hostDir.resolve(lib), overwrite = true)
        val out = outputDir.get().asFile
        out.deleteRecursively()
        out.mkdirs()
        exec.exec {
            workingDir = root
            // its own target directory: the bindgen feature would rebuild
            // the library above with other features
            commandLine(
                "cargo", "run", "--release", "--locked", "--target-dir", "target/uniffi-bindgen",
                "-p", "fold-ffi", "--features", "bindgen", "--bin", "uniffi-bindgen", "--",
                "generate", "--library", hostDir.resolve(lib).absolutePath,
                "--language", "kotlin", "--out-dir", out.absolutePath, "--no-format",
            )
        }
    }
}

val rustSources = files(
    cargoWorkspace.file("Cargo.toml"),
    cargoWorkspace.file("Cargo.lock"),
    cargoWorkspace.dir("crates/fold-core/src"),
    cargoWorkspace.dir("crates/fold-ffi/src"),
    cargoWorkspace.file("crates/fold-core/Cargo.toml"),
    cargoWorkspace.file("crates/fold-ffi/Cargo.toml"),
    cargoWorkspace.file("crates/fold-ffi/uniffi.toml"),
)

val cargoNdk = tasks.register<CargoNdk>("cargoNdk") {
    description = "Builds the Rust core for the Android ABIs in fold.abis."
    abis.set(providers.gradleProperty("fold.abis").map { it.split(",").map(String::trim).filter(String::isNotEmpty) })
    platform.set(minSdkVersion)
    workspace.set(cargoWorkspace)
    ndk.set(androidComponents.sdkComponents.ndkDirectory)
    sources.from(rustSources)
    outputDir.set(layout.buildDirectory.dir("rust/jniLibs"))
}

val uniffiBindgen = tasks.register<UniffiBindgen>("uniffiBindgen") {
    description = "Generates the Kotlin bindings of the Rust core."
    workspace.set(cargoWorkspace)
    sources.from(rustSources)
    outputDir.set(layout.buildDirectory.dir("generated/uniffi/kotlin"))
    hostLibDir.set(layout.buildDirectory.dir("rust/host"))
}

androidComponents {
    onVariants { variant ->
        variant.sources.jniLibs?.addGeneratedSourceDirectory(cargoNdk, CargoNdk::outputDir)
        variant.sources.kotlin?.addGeneratedSourceDirectory(uniffiBindgen, UniffiBindgen::outputDir)
    }
}

tasks.withType<Test>().configureEach {
    dependsOn(uniffiBindgen)
    // JNA finds the host build of the Rust core here
    systemProperty("jna.library.path", uniffiBindgen.get().hostLibDir.get().asFile.absolutePath)
    // Robolectric downloads its Android jars on first use; with
    // -Probolectric.dependency.dir=DIR it reads them from DIR instead
    providers.gradleProperty("robolectric.dependency.dir").orNull?.let {
        systemProperty("robolectric.offline", "true")
        systemProperty("robolectric.dependency.dir", it)
    }
}

dependencies {
    implementation(libs.androidx.core)
    implementation(libs.androidx.activity.compose)
    implementation(libs.androidx.lifecycle.runtime.compose)
    implementation(libs.androidx.lifecycle.viewmodel.compose)
    implementation(platform(libs.compose.bom))
    implementation(libs.compose.ui)
    implementation(libs.compose.ui.tooling.preview)
    implementation(libs.compose.material3)
    implementation(libs.compose.material.icons)
    implementation(libs.kotlinx.coroutines.android)
    implementation("${libs.jna.get()}@aar")
    debugImplementation(libs.compose.ui.tooling)

    // the JVM tests load the host build of the core through the desktop JNA
    testImplementation(libs.jna)
    testImplementation(libs.junit)
    testImplementation(libs.robolectric)
    testImplementation(libs.roborazzi)
    testImplementation(libs.roborazzi.compose)
    testImplementation(libs.androidx.test.core)
    testImplementation(platform(libs.compose.bom))
    testImplementation(libs.compose.ui.test.junit4)
    debugImplementation(libs.compose.ui.test.manifest)
}
