import java.io.File

plugins {
    kotlin("jvm") version "2.0.21"
    `maven-publish`
    signing
    `java-library`
    // New Sonatype Central Portal publisher (replaces EOL OSSRH / s01.oss.sonatype.org)
    id("com.gradleup.nmcp") version "0.0.8"
}

group = "io.github.project-minigraf"
version = System.getenv("RELEASE_VERSION") ?: "0.23.0"

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
    jvmToolchain(17)
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
                        "private fun findLibraryName(componentName: String): String {\n    io.github.project_minigraf.minigraf.NativeLoader.load()"
                    )
                file.writeText(patched)
            }
    }
}

// Register the generated dir as a Kotlin source set and wire explicit task dependencies
// so Gradle knows compileKotlin must wait for codegen to finish.
sourceSets.main {
    kotlin.srcDir(generatedSourcesDir)
}
tasks.named("compileKotlin") { dependsOn(generateKotlinBindings) }
tasks.named("compileTestKotlin") { dependsOn(generateKotlinBindings) }
// Wire all archive tasks (sourcesJar, kotlinSourcesJar, javadocJar, etc.) to codegen.
// Using withType<AbstractArchiveTask> avoids a hard-coded task name that may differ
// across Kotlin / java-library plugin versions.
tasks.withType<org.gradle.api.tasks.bundling.AbstractArchiveTask>().configureEach {
    dependsOn(generateKotlinBindings)
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
            groupId = "io.github.project-minigraf"
            artifactId = "minigraf-jvm"
            version = project.version.toString()

            pom {
                name.set("Minigraf JVM")
                description.set("Zero-config, single-file, embedded graph database with bi-temporal Datalog queries — JVM bindings")
                url.set("https://github.com/project-minigraf/minigraf")
                licenses {
                    license {
                        name.set("MIT OR Apache-2.0")
                        url.set("https://github.com/project-minigraf/minigraf/blob/main/LICENSE-MIT")
                    }
                }
                developers {
                    developer {
                        id.set("adityamukho")
                        name.set("Aditya Mukhopadhyay")
                    }
                }
                scm {
                    connection.set("scm:git:git://github.com/project-minigraf/minigraf.git")
                    developerConnection.set("scm:git:ssh://github.com/project-minigraf/minigraf.git")
                    url.set("https://github.com/project-minigraf/minigraf")
                }
            }
        }
    }
}

// ── Sonatype Central Portal publishing ────────────────────────────────────
// Uses com.gradleup.nmcp which POSTs a bundle to central.sonatype.com/api/v1/publisher/upload.
// Credentials are a *token* pair generated at https://central.sonatype.com → Account → Generate User Token.
// Set CENTRAL_TOKEN_USERNAME and CENTRAL_TOKEN_PASSWORD as GitHub Actions secrets.
// publishingType = "USER_MANAGED" means you click "Publish" in the portal UI after validation.
// Change to "AUTOMATIC" to publish without that manual step.
nmcp {
    // "release" must match the publication name in publishing { publications { create("release") { } } }
    // publicationType = "USER_MANAGED" means you click Publish in the portal UI after validation;
    // change to "AUTOMATIC" to release without that manual step.
    publish("release") {
        username = System.getenv("CENTRAL_TOKEN_USERNAME") ?: ""
        password = System.getenv("CENTRAL_TOKEN_PASSWORD") ?: ""
        publicationType = "AUTOMATIC"
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
