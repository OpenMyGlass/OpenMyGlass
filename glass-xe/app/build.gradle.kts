// OpenMyGlass bridge app for Glass Explorer Edition XE24 (API 19).
// Flavors: "glass" (GDK LiveCard, installed on the glasses) and "emulator" (plain Activity, no GDK).
plugins {
    alias(libs.plugins.android.application)
}

val sdkDir: String = providers.environmentVariable("ANDROID_HOME")
    .orElse("${System.getProperty("user.home")}/Library/Android/sdk").get()
/** Google's GDK jar, installed via sdkmanager under the GDK license; never committed or packaged. */
val gdkJar = file(providers.gradleProperty("gdkJar").getOrElse("$sdkDir/add-ons/addon-google_gdk-google-19/libs/gdk.jar"))
val useGdkStubs = providers.gradleProperty("gdkStubs").map { it.toBoolean() }.getOrElse(false)

android {
    namespace = "org.openmyglass.glass"
    compileSdk = 36
    defaultConfig {
        applicationId = "org.openmyglass.glass"
        minSdk = 19
        //noinspection ExpiredTargetSdkVersion — Glass XE24 is API 19
        targetSdk = 19
        versionCode = 1
        versionName = "0.1.0"
    }
    flavorDimensions += "target"
    productFlavors {
        create("glass") { dimension = "target" }
        // The emulator build runs on modern AVDs, which refuse to install apps targeting API < 23.
        create("emulator") { dimension = "target"; applicationIdSuffix = ".emulator"; minSdk = 21; targetSdk = 36 }
    }
    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
    lint { abortOnError = false }
    testOptions { unitTests.isReturnDefaultValues = true }
}

dependencies {
    implementation(project(":protocol"))
    if (useGdkStubs) {
        "glassCompileOnly"(project(":glass-xe:gdk-stubs"))
    } else {
        "glassCompileOnly"(files(gdkJar))
    }
    testImplementation(libs.junit4)
    testImplementation(project(":core"))
    testImplementation(libs.coroutines.core)
}

gradle.taskGraph.whenReady {
    if (!useGdkStubs && allTasks.any { it.path.startsWith(":glass-xe:app:") && it.name.contains("Glass") } && !gdkJar.exists()) {
        throw GradleException(
            "GDK jar not found at $gdkJar. Install it with sdkmanager --include_obsolete " +
                "\"add-ons;addon-google_gdk-google-19\" (accepting Google's GDK license), pass -PgdkJar=<path>, " +
                "or build with -PgdkStubs=true to use OpenMyGlass's own stubs."
        )
    }
}
