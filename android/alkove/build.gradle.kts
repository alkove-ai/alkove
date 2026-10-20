import java.util.Properties

plugins {
    alias(libs.plugins.android.library)
    `maven-publish`
}

val repo: File = rootDir.parentFile
val local = Properties().apply { rootProject.file("local.properties").takeIf { it.exists() }?.reader()?.use { load(it) } }
val cargo = listOfNotNull(System.getenv("CARGO"), "${System.getProperty("user.home")}/.cargo/bin/cargo").firstOrNull { File(it).canExecute() } ?: "cargo"
val abis = listOf("arm64-v8a", "x86_64")
val rustLibs = layout.buildDirectory.dir("rust/jniLibs")
val testAssets = layout.buildDirectory.dir("testAssets")

// cargo decides what is up to date: these run on every build
val cargoAndroid = tasks.register<Exec>("cargoAndroid") {
    workingDir = repo
    (local.getProperty("alkove.ndk.dir") ?: System.getenv("ANDROID_NDK_HOME"))?.let { environment("ANDROID_NDK_HOME", it) }
    commandLine(listOf(cargo, "ndk") + abis.flatMap { listOf("-t", it) } + listOf("-P", "24", "-o", rustLibs.get().asFile.path, "build", "--profile", "android", "-p", "alkove-android"))
}

val cargoHost = tasks.register<Exec>("cargoHost") {
    workingDir = repo
    commandLine(cargo, "build", "--profile", "android", "-p", "alkove-android")
}

android {
    namespace = "ai.alkove"
    compileSdk = 36
    defaultConfig {
        minSdk = 24
        consumerProguardFiles("consumer-rules.pro")
        testInstrumentationRunner = "ai.alkove.TestRunner"
        buildConfigField("String", "VERSION", "\"$version\"")
    }
    buildFeatures {
        buildConfig = true
    }
    publishing {
        singleVariant("release") { withSourcesJar() }
    }
    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
    sourceSets {
        getByName("main").jniLibs.directories.add(rustLibs.get().asFile.path)
        // the host's tests run on the device too, on the fixtures and models packed as test assets
        getByName("androidTest").kotlin.directories.add("src/test/kotlin")
        getByName("androidTest").assets.directories.add(testAssets.get().asFile.path)
    }
    testOptions {
        unitTests.isReturnDefaultValues = true
    }
}

val packTestAssets = tasks.register<Sync>("packTestAssets") {
    from(File(repo, "packages/alkove/test/fixtures")) { into("repo/packages/alkove/test/fixtures") }
    // aapt drops folders whose name starts with a dot: the runner puts them back under .cache
    from(File(repo, "packages/alkove/.cache/models")) {
        include("*.alk")
        into("models")
    }
    into(testAssets)
}

tasks.named("preBuild") { dependsOn(cargoAndroid, packTestAssets) }

tasks.withType<Test>().configureEach {
    dependsOn(cargoHost)
    systemProperty("java.library.path", File(repo, "target/android").path)
    systemProperty("alkove.repo", repo.path)
    maxHeapSize = "2g"
}

dependencies {
    api(libs.coroutines.android)
    testImplementation(libs.junit)
    androidTestImplementation(libs.junit)
    androidTestImplementation(libs.androidx.test.runner)
}

publishing {
    publications {
        register<MavenPublication>("release") {
            artifactId = "alkove"
            afterEvaluate { from(components["release"]) }
            pom {
                name = "Alkove"
                description = "On-device voice input for Android: keep the user's voice, find speech, hear words, know a voice; the same to the bit as the web and Python packages"
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
