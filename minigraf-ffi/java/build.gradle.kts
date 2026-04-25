import java.io.File

plugins {
    kotlin("jvm") version "2.0.21"
    `maven-publish`
    signing
    `java-library`
}

group = "io.github.adityamukho"
version = "0.23.0"

repositories {
    mavenCentral()
}

dependencies {
    // JNA is required at runtime by UniFFI-generated Kotlin bindings (com.sun.jna.*)
    implementation("net.java.dev.jna:jna:5.14.0")
    testImplementation(kotlin("test"))
    testImplementation("org.junit.jupiter:junit-jupiter:5.11.0")
    testRuntimeOnly("org.junit.platform:junit-platform-launcher")
    testImplementation("com.fasterxml.jackson.module:jackson-module-kotlin:2.17.0")
}

tasks.test {
    useJUnitPlatform()
}

kotlin {
    jvmToolchain(11)
}

// ── uniffi-bindgen codegen ─────────────────────────────────────────────────

val repoRoot = rootProject.projectDir.parentFile.parentFile.absolutePath

val buildUniffiBindgen by tasks.registering(Exec::class) {
    group = "codegen"
    description = "Compile the uniffi-bindgen binary"
    workingDir = File(repoRoot)
    commandLine("cargo", "build", "--release", "--bin", "uniffi-bindgen",
                "--manifest-path", "$repoRoot/minigraf-ffi/Cargo.toml")
}

val libExt = when {
    System.getProperty("os.name").lowercase().contains("windows") -> "dll"
    System.getProperty("os.name").lowercase().contains("mac") -> "dylib"
    else -> "so"
}
val libPrefix = if (System.getProperty("os.name").lowercase().contains("windows")) "" else "lib"
val libPath = "$repoRoot/target/release/${libPrefix}minigraf_ffi.$libExt"

// Generated sources go to build/generated/uniffi so Gradle can track them
// as task outputs and wire them into the compile classpath automatically.
val generatedSourcesDir = layout.buildDirectory.dir("generated/uniffi")

val generateKotlinBindings by tasks.registering(Exec::class) {
    group = "codegen"
    description = "Generate Kotlin bindings from UniFFI"
    dependsOn(buildUniffiBindgen)
    workingDir = File(repoRoot)
    inputs.file(libPath)
    outputs.dir(generatedSourcesDir)
    commandLine(
        "$repoRoot/target/release/uniffi-bindgen",
        "generate", "--library", libPath,
        "--language", "kotlin",
        "--no-format",
        "--out-dir", generatedSourcesDir.get().asFile.absolutePath
    )
    // After generation, patch findLibraryName() to extract the native and set the
    // libraryOverride property before JNA tries to resolve the library by name.
    // findLibraryName() is called from every Native.register() invocation, so this
    // fires regardless of which object (UniffiLib / IntegrityCheckingUniffiLib) is
    // initialised first.  NativeLoader.load() is idempotent.
    doLast {
        generatedSourcesDir.get().asFile
            .walkTopDown().filter { it.extension == "kt" }.forEach { file ->
                val patched = file.readText()
                    .replace(
                        "private fun findLibraryName(componentName: String): String {",
                        "private fun findLibraryName(componentName: String): String {\n    io.github.adityamukho.minigraf.NativeLoader.load()"
                    )
                file.writeText(patched)
            }
    }
}

// Register the generated dir as a Kotlin source set so that both
// compileKotlin and compileTestKotlin automatically depend on the task.
sourceSets.main {
    kotlin.srcDir(generatedSourcesDir)
}

// ── native resources ───────────────────────────────────────────────────────
// In CI the natives are copied in by the release workflow before Gradle runs.
// Locally, copy the current platform's native from target/release/.

val copyLocalNative by tasks.registering(Copy::class) {
    group = "codegen"
    description = "Copy local platform native into resources (dev only)"
    val os = System.getProperty("os.name").lowercase()
    val arch = System.getProperty("os.arch").lowercase()
    val (osKey, nativeName) = when {
        "linux" in os && ("aarch64" in arch || "arm64" in arch) ->
            "linux/aarch64" to "libminigraf_ffi.so"
        "linux" in os -> "linux/x86_64" to "libminigraf_ffi.so"
        "mac" in os -> "macos/universal" to "libminigraf_ffi.dylib"
        "windows" in os -> "windows/x86_64" to "minigraf_ffi.dll"
        else -> throw GradleException("Unsupported platform: $os $arch")
    }
    from(File("$repoRoot/target/release/$nativeName"))
    into(File("$projectDir/src/main/resources/natives/$osKey"))
}

// ── publishing ─────────────────────────────────────────────────────────────

java {
    withSourcesJar()
    withJavadocJar()
}

publishing {
    publications {
        create<MavenPublication>("release") {
            from(components["java"])
            groupId = "io.github.adityamukho"
            artifactId = "minigraf-jvm"
            version = project.version.toString()

            pom {
                name.set("Minigraf JVM")
                description.set("Zero-config, single-file, embedded graph database with bi-temporal Datalog queries — JVM bindings")
                url.set("https://github.com/adityamukho/minigraf")
                licenses {
                    license {
                        name.set("MIT OR Apache-2.0")
                        url.set("https://github.com/adityamukho/minigraf/blob/main/LICENSE-MIT")
                    }
                }
                developers {
                    developer {
                        id.set("adityamukho")
                        name.set("Aditya Mukhopadhyay")
                    }
                }
                scm {
                    connection.set("scm:git:git://github.com/adityamukho/minigraf.git")
                    developerConnection.set("scm:git:ssh://github.com/adityamukho/minigraf.git")
                    url.set("https://github.com/adityamukho/minigraf")
                }
            }
        }
    }
    repositories {
        maven {
            name = "OSSRH"
            url = uri(
                if (version.toString().endsWith("SNAPSHOT"))
                    "https://s01.oss.sonatype.org/content/repositories/snapshots/"
                else
                    "https://s01.oss.sonatype.org/service/local/staging/deploy/maven2/"
            )
            credentials {
                username = System.getenv("OSSRH_USERNAME")
                password = System.getenv("OSSRH_PASSWORD")
            }
        }
    }
}

signing {
    val signingKey = System.getenv("GPG_SIGNING_KEY")
    val signingPassword = System.getenv("GPG_SIGNING_PASSWORD")
    if (signingKey != null && signingPassword != null) {
        useInMemoryPgpKeys(signingKey, signingPassword)
        sign(publishing.publications["release"])
    }
}
