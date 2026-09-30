plugins {
    id("com.android.application")
    id("jacoco")
    id("org.jetbrains.kotlin.android")
    id("org.jetbrains.kotlin.plugin.compose")
}

android {
    namespace = "org.katinbroek.shuttli"
    compileSdk = 36
    defaultConfig {
        applicationId = "org.katinbroek.shuttli"
        minSdk = 29
        targetSdk = 36
        versionCode = 1
        versionName = rootProject.file("../../Cargo.toml").readText()
            .substringAfter("[workspace.package]").substringBefore("\n[")
            .lineSequence().first { it.trimStart().startsWith("version =") }
            .substringAfter('=').trim().removeSurrounding("\"")
        testInstrumentationRunner = "androidx.test.runner.AndroidJUnitRunner"
        resValue("string", "app_name", rootProject.file("../../branding/name.txt").readText().trim())
    }
    buildTypes {
        debug { enableUnitTestCoverage = true; enableAndroidTestCoverage = true }
        release { isMinifyEnabled = true; proguardFiles(getDefaultProguardFile("proguard-android-optimize.txt"), "proguard-rules.pro") }
    }
    buildFeatures { compose = true; buildConfig = true }
    compileOptions { sourceCompatibility = JavaVersion.VERSION_17; targetCompatibility = JavaVersion.VERSION_17 }
    sourceSets.getByName("main").java.srcDir("build/generated/uniffi")
    testOptions { unitTests.isReturnDefaultValues = true }
}

kotlin { compilerOptions { jvmTarget.set(org.jetbrains.kotlin.gradle.dsl.JvmTarget.JVM_17) } }

dependencies {
    val composeBom = platform("androidx.compose:compose-bom:2025.08.00")
    implementation(composeBom)
    androidTestImplementation(composeBom)
    implementation("androidx.activity:activity-compose:1.10.1")
    implementation("androidx.compose.material3:material3")
    implementation("androidx.compose.ui:ui-tooling-preview")
    implementation("org.jetbrains.kotlinx:kotlinx-coroutines-android:1.10.2")
    debugImplementation("androidx.compose.ui:ui-tooling")
    implementation("net.java.dev.jna:jna:5.17.0@aar")
    testImplementation("junit:junit:4.13.2")
    androidTestImplementation("androidx.test.ext:junit:1.2.1")
    androidTestImplementation("androidx.test:runner:1.6.2")
    androidTestImplementation("androidx.test.espresso:espresso-core:3.6.1")
    androidTestImplementation("androidx.compose.ui:ui-test-junit4")
    debugImplementation("androidx.compose.ui:ui-test-manifest")
}

// Report all app-owned Kotlin, including UI/state/platform code. Generated
// bindings live in a different package and cannot inflate application coverage.
jacoco { toolVersion = "0.8.13" }
tasks.register<JacocoReport>("mobileCoverageReport") {
    classDirectories.setFrom(fileTree(layout.buildDirectory.dir("tmp/kotlin-classes/debug")) {
        include("org/katinbroek/shuttli/**")
    })
    sourceDirectories.setFrom(files("src/main/java"))
    executionData.setFrom(fileTree(layout.buildDirectory) {
        include("outputs/unit_test_code_coverage/**/*.exec", "outputs/code_coverage/**/*.ec")
    })
    doFirst { require(executionData.files.any { it.exists() }) { "Run unit or emulator tests before generating coverage" } }
    reports { xml.required.set(true); html.required.set(true) }
}
