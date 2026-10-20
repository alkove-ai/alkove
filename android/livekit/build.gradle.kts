plugins {
    alias(libs.plugins.android.library)
    `maven-publish`
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
    publishing {
        singleVariant("release") { withSourcesJar() }
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
    api(libs.livekit)
    testImplementation(libs.junit)
}

publishing {
    publications {
        register<MavenPublication>("release") {
            artifactId = "alkove-livekit"
            afterEvaluate { from(components["release"]) }
            pom {
                name = "Alkove for LiveKit"
                description = "Alkove's Focus in LiveKit Android's microphone path"
                url = "https://alkove.ai"
                licenses {
                    license {
                        name = "Apache-2.0"
                        url = "https://www.apache.org/licenses/LICENSE-2.0"
                    }
                }
                scm {
                    url = "https://github.com/alkove-ai/alkove"
                    connection = "scm:git:https://github.com/alkove-ai/alkove.git"
                }
            }
        }
    }
    // the folder that goes to maven.alkove.ai as it is
    repositories {
        maven {
            name = "staging"
            url = uri(rootProject.layout.buildDirectory.dir("maven"))
        }
    }
}
