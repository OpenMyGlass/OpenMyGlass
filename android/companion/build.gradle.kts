plugins {
    alias(libs.plugins.android.application)
}

// The new companion: a thin Android shell over the shared Rust core (rust/crates/omg-ffi).
// See docs/companion-mobile.md. The older app in android/app is a separate, Kotlin-core track.

android {
    namespace = "org.openmyglass.companion"
    compileSdk = 36
    ndkVersion = "27.3.13750724"
    defaultConfig {
        applicationId = "org.openmyglass.companion"
        minSdk = 29
        targetSdk = 36
        versionCode = 1
        versionName = "0.1.0"
        ndk { abiFilters += RustAbi.all.map { it.android } }
    }
    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
    sourceSets["main"].jniLibs.directories.add(layout.buildDirectory.dir("rustJniLibs").get().asFile.path)
    sourceSets["main"].kotlin.directories.add(layout.buildDirectory.dir("generated/uniffi").get().asFile.path)
    testOptions { unitTests.isReturnDefaultValues = true }
}

kotlin { jvmToolchain(17) }

dependencies {
    implementation(libs.jna) { artifact { type = "aar" } }
    testImplementation(libs.junit4)
}

object RustAbi {
    data class Abi(val android: String, val triple: String, val clang: String)

    // The phone under test is arm64. Add armv7 and x86_64 here when a device needs them.
    val all = listOf(Abi("arm64-v8a", "aarch64-linux-android", "aarch64-linux-android29-clang"))
}

val rustDir = rootProject.layout.projectDirectory.dir("rust")
val rustTarget = System.getenv("CARGO_TARGET_DIR")?.let { file(it) } ?: rustDir.dir("target").asFile
val rustSources = fileTree(rustDir) {
    include("Cargo.toml", "Cargo.lock", "crates/**/*.rs", "crates/**/Cargo.toml", "crates/omg-ffi/uniffi.toml")
}

val cargoAndroid = tasks.register("cargoBuildAndroid") {
    val ndk = androidComponents.sdkComponents.ndkDirectory
    val outDir = layout.buildDirectory.dir("rustJniLibs")
    inputs.files(rustSources)
    outputs.dir(outDir)
    val execOps = project.objects.newInstance<ExecHelper>().execOps
    doLast {
        val bin = ndk.get().asFile.resolve("toolchains/llvm/prebuilt/linux-x86_64/bin")
        RustAbi.all.forEach { abi ->
            val envTriple = abi.triple.uppercase().replace('-', '_')
            execOps.exec {
                workingDir = rustDir.asFile
                environment("CARGO_TARGET_${envTriple}_LINKER", bin.resolve(abi.clang).path)
                commandLine("cargo", "build", "--release", "-p", "omg-ffi", "--lib", "--target", abi.triple)
            }
            copy {
                from(rustTarget.resolve("${abi.triple}/release/libomg_ffi.so"))
                into(outDir.get().asFile.resolve(abi.android))
            }
        }
    }
}

val uniffiBindings = tasks.register("uniffiBindings") {
    val outDir = layout.buildDirectory.dir("generated/uniffi")
    inputs.files(rustSources)
    outputs.dir(outDir)
    val execOps = project.objects.newInstance<ExecHelper>().execOps
    doLast {
        execOps.exec {
            workingDir = rustDir.asFile
            commandLine("cargo", "build", "-p", "omg-ffi", "--lib")
        }
        execOps.exec {
            workingDir = rustDir.asFile
            commandLine(
                "cargo", "run", "-p", "omg-ffi", "--features", "cli", "--bin", "uniffi-bindgen", "--",
                "generate", "--library", rustTarget.resolve("debug/libomg_ffi.so").path,
                "--language", "kotlin", "--no-format", "--out-dir", outDir.get().asFile.path,
            )
        }
    }
}

abstract class ExecHelper @Inject constructor(val execOps: ExecOperations)

tasks.matching { it.name.startsWith("compile") && it.name.contains("Kotlin") }
    .configureEach { dependsOn(uniffiBindings) }
tasks.matching { it.name.contains("JniLib") || it.name.startsWith("merge") && it.name.contains("Native") }
    .configureEach { dependsOn(cargoAndroid) }
