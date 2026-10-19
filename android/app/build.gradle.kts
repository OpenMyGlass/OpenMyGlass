plugins {
    alias(libs.plugins.android.application)
}

android {
    namespace = "org.openmyglass.android"
    compileSdk = 36
    defaultConfig {
        applicationId = "org.openmyglass.android"
        minSdk = 29
        targetSdk = 36
        versionCode = 1
        versionName = "0.1.0"
    }
    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
    testOptions { unitTests.isReturnDefaultValues = true }
}

kotlin { jvmToolchain(17) }

dependencies {
    implementation(project(":core"))
    implementation(project(":android:osmand-api"))
    implementation(libs.coroutines.android)
    implementation(libs.vosk.android)
    testImplementation(libs.junit4)
    testImplementation(libs.coroutines.test)
}
