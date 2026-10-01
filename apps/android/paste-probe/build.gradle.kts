plugins { id("com.android.application"); id("org.jetbrains.kotlin.android") }
android {
    namespace = "org.katinbroek.shuttli.pasteprobe"
    compileSdk = 36
    defaultConfig { applicationId = "org.katinbroek.shuttli.pasteprobe"; minSdk = 29; targetSdk = 36 }
    compileOptions { sourceCompatibility = JavaVersion.VERSION_17; targetCompatibility = JavaVersion.VERSION_17 }
}
kotlin { compilerOptions { jvmTarget.set(org.jetbrains.kotlin.gradle.dsl.JvmTarget.JVM_17) } }
