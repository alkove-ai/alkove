plugins {
    alias(libs.plugins.android.application)
}

android {
    namespace = "ai.alkove.example"
    compileSdk = 36
    defaultConfig {
        applicationId = "ai.alkove.example"
        minSdk = 24
        targetSdk = 36
        versionCode = 1
        versionName = "0.3.2"
    }
    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
    buildTypes {
        release {
            isMinifyEnabled = true
            signingConfig = signingConfigs.getByName("debug")
        }
    }
}

dependencies {
    implementation(project(":alkove"))
    implementation(libs.coroutines.android)
}
