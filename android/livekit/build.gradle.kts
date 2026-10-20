plugins {
    alias(libs.plugins.android.library)
}

android {
    namespace = "ai.alkove.livekit"
    compileSdk = 36
    defaultConfig {
        minSdk = 24
    }
    testOptions {
        unitTests.isReturnDefaultValues = true
    }
    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
}

tasks.withType<Test>().configureEach {
    dependsOn(":alkove:cargoHost")
    systemProperty("java.library.path", File(rootDir.parentFile, "target/android").path)
    systemProperty("alkove.repo", rootDir.parentFile.path)
}

dependencies {
    api(project(":alkove"))
    implementation(libs.livekit)
    testImplementation(libs.junit)
}
