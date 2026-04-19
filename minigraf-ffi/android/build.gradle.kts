plugins {
    id("com.android.library") version "8.2.2"
    id("maven-publish")
}

android {
    namespace = "io.github.adityamukho.minigraf"
    compileSdk = 34
    defaultConfig {
        minSdk = 24
        targetSdk = 34
    }
    sourceSets {
        getByName("main") {
            jniLibs.srcDirs("jniLibs")
            java.srcDirs("src/main/java")
        }
    }
    publishing {
        singleVariant("release")
    }
}

afterEvaluate {
    publishing {
        publications {
            create<MavenPublication>("release") {
                from(components["release"])
                groupId = "io.github.adityamukho"
                artifactId = "minigraf-android"
                version = System.getenv("VERSION") ?: "0.0.0-local"
            }
        }
        repositories {
            maven {
                name = "GitHubPackages"
                url = uri("https://maven.pkg.github.com/adityamukho/minigraf")
                credentials {
                    username = System.getenv("GITHUB_ACTOR") ?: ""
                    password = System.getenv("GITHUB_TOKEN") ?: ""
                }
            }
        }
    }
}
