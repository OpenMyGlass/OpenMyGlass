// Original compile-only stubs of the few GDK classes OpenMyGlass uses, written from the public
// GDK reference documentation (not from Google's gdk.jar). Never packaged into an APK.
plugins { `java-library` }

java {
    sourceCompatibility = JavaVersion.VERSION_1_8
    targetCompatibility = JavaVersion.VERSION_1_8
}

val sdkDir = providers.environmentVariable("ANDROID_HOME").orElse("${System.getProperty("user.home")}/Library/Android/sdk")

dependencies {
    compileOnly(files(sdkDir.map { "$it/platforms/android-19/android.jar" }))
}
