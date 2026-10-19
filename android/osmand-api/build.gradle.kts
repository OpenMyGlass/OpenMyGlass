// Vendored OsmAnd AIDL client API (GPLv3, see NOTICE.md). Sources are copied unchanged from upstream.
plugins {
    alias(libs.plugins.android.library)
}

android {
    namespace = "net.osmand.aidlapi"
    compileSdk = 36
    defaultConfig { minSdk = 29 }
    buildFeatures { aidl = true }
    sourceSets {
        getByName("main") {
            manifest.srcFile("AndroidManifest.xml")
            aidl.directories += "src"
            java.directories += "src"
        }
    }
    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
    lint { abortOnError = false }
}

dependencies {
    implementation(libs.androidx.annotation)
}
