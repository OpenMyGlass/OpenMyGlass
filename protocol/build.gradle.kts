plugins {
    alias(libs.plugins.kotlin.jvm)
    alias(libs.plugins.protobuf)
}

kotlin { jvmToolchain(17) }

sourceSets {
    main { proto { srcDir("companion"); srcDir("openmyglass") } }
}

dependencies {
    api(libs.protobuf.javalite)
    testImplementation(platform(libs.junit.bom))
    testImplementation(libs.junit.jupiter)
    testRuntimeOnly(libs.junit.launcher)
}

protobuf {
    protoc { artifact = libs.protoc.get().toString() }
    generateProtoTasks {
        all().forEach { task ->
            task.builtins { named("java") { option("lite") } }
        }
    }
}

tasks.test { useJUnitPlatform() }
