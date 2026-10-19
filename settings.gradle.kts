rootProject.name = "openmyglass"

dependencyResolutionManagement {
    repositories { mavenCentral() }
}
pluginManagement {
    repositories { gradlePluginPortal(); mavenCentral() }
}

include(":protocol", ":core", ":tools:sim-glass")
