import org.jetbrains.kotlin.gradle.dsl.JvmTarget

// bitwire for Kotlin/JVM: plain JVM library, no Android SDK dependency, no
// runtime dependencies beyond the Kotlin standard library.
plugins {
    kotlin("jvm") version "2.4.10"
}

group = "bitwire"
version = "0.1.0"

repositories {
    mavenCentral()
}

dependencies {
    testImplementation(platform("org.junit:junit-bom:5.13.4"))
    testImplementation("org.junit.jupiter:junit-jupiter")
    testRuntimeOnly("org.junit.platform:junit-platform-launcher")
}

// Java 11 bytecode: loadable on Android (D8) and any desktop JVM >= 11.
java {
    sourceCompatibility = JavaVersion.VERSION_11
    targetCompatibility = JavaVersion.VERSION_11
}

kotlin {
    compilerOptions {
        jvmTarget.set(JvmTarget.JVM_11)
    }
}

tasks.test {
    useJUnitPlatform()
    // The shared golden vectors live next to this directory (../vectors).
    systemProperty("bitwire.vectors", file("../vectors").absolutePath)
    inputs.dir("../vectors").withPropertyName("vectors")
    testLogging {
        events("failed")
        exceptionFormat = org.gradle.api.tasks.testing.logging.TestExceptionFormat.FULL
    }
}
