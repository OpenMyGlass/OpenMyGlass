rootProject.name = "openmyglass"

dependencyResolutionManagement {
    repositories { google(); mavenCentral() }
}
pluginManagement {
    repositories { google(); gradlePluginPortal(); mavenCentral() }
}

include(":protocol", ":core", ":tools:sim-glass", ":android:app", ":android:osmand-api")
