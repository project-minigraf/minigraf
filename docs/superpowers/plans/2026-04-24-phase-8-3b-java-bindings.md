# Phase 8.3b: Java Desktop Bindings Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Publish `io.github.adityamukho:minigraf-jvm` to Maven Central as a fat JAR with embedded platform natives — Java/Kotlin desktop bindings generated from the existing `minigraf-ffi` UniFFI crate.

**Architecture:** A Gradle project in `minigraf-ffi/java/` runs `uniffi-bindgen generate --language kotlin` to emit Kotlin sources, copies compiled platform natives into `src/main/resources/natives/<os>/<arch>/`, and produces a single fat JAR. A hand-written `NativeLoader.kt` extracts the correct native at runtime and calls `System.load()`. UniFFI's generated `System.loadLibrary("minigraf_ffi")` call is patched to `NativeLoader.load()` by a Gradle `sed` task after source generation.

**Tech Stack:** Kotlin 2.0, Gradle 8.x, JUnit Jupiter 5.x, UniFFI 0.31.1 (already in `minigraf-ffi`), Maven Central (Sonatype OSSRH), Gradle maven-publish + signing plugins.

---

## File Structure

| Action | Path | Responsibility |
|--------|------|----------------|
| CREATE | `minigraf-ffi/java/settings.gradle.kts` | Gradle project name |
| CREATE | `minigraf-ffi/java/build.gradle.kts` | Full build: bindgen, compile, fat JAR, publish |
| CREATE | `minigraf-ffi/java/src/main/kotlin/io/github/adityamukho/minigraf/NativeLoader.kt` | Runtime native extraction from JAR resources |
| CREATE | `minigraf-ffi/java/src/test/kotlin/io/github/adityamukho/minigraf/BasicTest.kt` | JUnit 5 tests |
| CREATE | `.github/workflows/java-ci.yml` | PR test matrix (4 platforms) |
| CREATE | `.github/workflows/java-release.yml` | Release: cross-compile natives + assemble fat JAR + publish |
| MODIFY | `Cargo.toml` (root) | Bump version to `0.23.0` |
| MODIFY | `minigraf-ffi/Cargo.toml` | Bump version to `0.23.0` |
| MODIFY | `CHANGELOG.md` | Add 8.3b entry |
| MODIFY | `ROADMAP.md` | Mark 8.3b complete |

---

## Task 1: Create Gradle project skeleton

**Files:**
- Create: `minigraf-ffi/java/settings.gradle.kts`
- Create: `minigraf-ffi/java/build.gradle.kts`

- [ ] **Step 1: Create `minigraf-ffi/java/settings.gradle.kts`**

```kotlin
rootProject.name = "minigraf-jvm"
```

- [ ] **Step 2: Create `minigraf-ffi/java/build.gradle.kts`**

```kotlin
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

val generateKotlinBindings by tasks.registering(Exec::class) {
    group = "codegen"
    description = "Generate Kotlin bindings from UniFFI"
    dependsOn(buildUniffiBindgen)
    workingDir = File(repoRoot)
    val outDir = "$projectDir/src/main/kotlin"
    commandLine(
        "$repoRoot/target/release/uniffi-bindgen",
        "generate", "--library", libPath,
        "--language", "kotlin",
        "--out-dir", outDir
    )
    // After generation, patch System.loadLibrary → NativeLoader.load()
    doLast {
        val generatedDir = File("$projectDir/src/main/kotlin/uniffi/minigraf_ffi")
        generatedDir.walkTopDown().filter { it.extension == "kt" }.forEach { file ->
            val patched = file.readText()
                .replace(
                    """System.loadLibrary("minigraf_ffi")""",
                    "io.github.adityamukho.minigraf.NativeLoader.load()"
                )
            file.writeText(patched)
        }
    }
}

tasks.compileKotlin {
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
```

- [ ] **Step 3: Commit**

```bash
git add minigraf-ffi/java/
git commit -m "feat(java): add Gradle project skeleton for Maven Central JAR"
```

---

## Task 2: Write NativeLoader

**Files:**
- Create: `minigraf-ffi/java/src/main/kotlin/io/github/adityamukho/minigraf/NativeLoader.kt`

- [ ] **Step 1: Create the directory structure**

```bash
mkdir -p minigraf-ffi/java/src/main/kotlin/io/github/adityamukho/minigraf
mkdir -p minigraf-ffi/java/src/main/resources/natives
mkdir -p minigraf-ffi/java/src/test/kotlin/io/github/adityamukho/minigraf
```

- [ ] **Step 2: Create `NativeLoader.kt`**

```kotlin
package io.github.adityamukho.minigraf

import java.io.File
import java.nio.file.Files

/**
 * Extracts the platform-appropriate native library from JAR resources and loads it.
 * Call [load] before using any Minigraf class. It is idempotent.
 */
object NativeLoader {
    @Volatile private var loaded = false

    @Synchronized
    fun load() {
        if (loaded) return

        val os = System.getProperty("os.name").lowercase()
        val arch = System.getProperty("os.arch").lowercase()

        val (osKey, libName) = when {
            "linux" in os && ("aarch64" in arch || "arm64" in arch) ->
                "linux/aarch64" to "libminigraf_ffi.so"
            "linux" in os ->
                "linux/x86_64" to "libminigraf_ffi.so"
            "mac" in os ->
                "macos/universal" to "libminigraf_ffi.dylib"
            "windows" in os ->
                "windows/x86_64" to "minigraf_ffi.dll"
            else -> throw UnsupportedOperationException(
                "Unsupported platform: $os / $arch. " +
                "Please file an issue at https://github.com/adityamukho/minigraf"
            )
        }

        val resourcePath = "/natives/$osKey/$libName"
        val stream = NativeLoader::class.java.getResourceAsStream(resourcePath)
            ?: throw UnsatisfiedLinkError(
                "Native library not found in JAR: $resourcePath. " +
                "Ensure you are using the correct platform JAR."
            )

        val tmpDir = Files.createTempDirectory("minigraf_native")
        tmpDir.toFile().deleteOnExit()
        val dest = tmpDir.resolve(libName).toFile()
        dest.deleteOnExit()

        stream.use { src ->
            dest.outputStream().use { dst -> src.copyTo(dst) }
        }

        System.load(dest.absolutePath)
        loaded = true
    }
}
```

- [ ] **Step 3: Commit**

```bash
git add minigraf-ffi/java/src/main/kotlin/io/github/adityamukho/minigraf/NativeLoader.kt
git commit -m "feat(java): add NativeLoader for runtime native extraction from JAR"
```

---

## Task 3: Write Java tests

**Files:**
- Create: `minigraf-ffi/java/src/test/kotlin/io/github/adityamukho/minigraf/BasicTest.kt`

- [ ] **Step 1: Create `BasicTest.kt`**

```kotlin
package io.github.adityamukho.minigraf

import com.fasterxml.jackson.module.kotlin.jacksonObjectMapper
import com.fasterxml.jackson.module.kotlin.readValue
import org.junit.jupiter.api.Test
import org.junit.jupiter.api.assertThrows
import java.io.File
import kotlin.test.assertEquals
import kotlin.test.assertNotNull
import kotlin.test.assertTrue

// MiniGrafDb and MiniGrafException are generated by uniffi-bindgen into
// src/main/kotlin/uniffi/minigraf_ffi/. They are imported transitively.
// The exact generated package path is uniffi.minigraf_ffi — adjust if different.
import uniffi.minigraf_ffi.MiniGrafDb
import uniffi.minigraf_ffi.MiniGrafException

private val mapper = jacksonObjectMapper()

class BasicTest {

    @Test
    fun testOpenInMemory() {
        val db = MiniGrafDb.openInMemory()
        assertNotNull(db)
    }

    @Test
    fun testTransactAndQuery() {
        val db = MiniGrafDb.openInMemory()
        val txJson = db.execute("""(transact [[:alice :name "Alice"]])""")
        val tx: Map<String, Any> = mapper.readValue(txJson)
        assertTrue(tx.containsKey("transacted"), "expected transacted key")

        val queryJson = db.execute("(query [:find ?n :where [?e :name ?n]])")
        val qr: Map<String, Any> = mapper.readValue(queryJson)
        @Suppress("UNCHECKED_CAST")
        val results = qr["results"] as List<List<Any>>
        assertEquals("Alice", results[0][0])
    }

    @Test
    fun testInvalidDatalogThrows() {
        val db = MiniGrafDb.openInMemory()
        assertThrows<MiniGrafException> {
            db.execute("not valid datalog !!!")
        }
    }

    @Test
    fun testFileBacked() {
        val tmp = File.createTempFile("minigraf_jvm_test", ".graph")
        tmp.deleteOnExit()
        val walFile = File(tmp.absolutePath + ".wal")
        walFile.deleteOnExit()

        val db = MiniGrafDb.open(tmp.absolutePath)
        db.execute("""(transact [[:bob :name "Bob"]])""")
        db.checkpoint()
        db.destroy()  // UniFFI frees the Arc reference

        val db2 = MiniGrafDb.open(tmp.absolutePath)
        val queryJson = db2.execute("(query [:find ?n :where [?e :name ?n]])")
        val qr: Map<String, Any> = mapper.readValue(queryJson)
        @Suppress("UNCHECKED_CAST")
        val results = qr["results"] as List<List<Any>>
        assertEquals("Bob", results[0][0])
    }
}
```

> **Note:** UniFFI generates `destroy()` on Kotlin objects backed by Rust Arcs; call it when done to free the native reference. If the generated class name or package differs from `uniffi.minigraf_ffi`, adjust the import accordingly after running `generateKotlinBindings`.

- [ ] **Step 2: Commit**

```bash
git add minigraf-ffi/java/src/test/kotlin/io/github/adityamukho/minigraf/BasicTest.kt
git commit -m "test(java): add JUnit 5 suite for Maven Central JAR"
```

---

## Task 4: Add PR CI workflow

**Files:**
- Create: `.github/workflows/java-ci.yml`

- [ ] **Step 1: Create `.github/workflows/java-ci.yml`**

```yaml
name: Java CI

on:
  push:
    branches: [main]
  pull_request:
    branches: [main]

jobs:
  test:
    name: Java tests (${{ matrix.os }})
    runs-on: ${{ matrix.os }}
    strategy:
      fail-fast: false
      matrix:
        os: [ubuntu-latest, ubuntu-24.04-arm, macos-14, windows-latest]

    steps:
      - uses: actions/checkout@v4

      - name: Install Rust toolchain
        uses: dtolnay/rust-toolchain@stable

      - name: Set up JDK 11
        uses: actions/setup-java@v4
        with:
          java-version: '11'
          distribution: 'temurin'

      - name: Build Rust library (release)
        run: cargo build --release -p minigraf-ffi

      - name: Generate Kotlin bindings + copy local native
        working-directory: minigraf-ffi/java
        run: |
          ./gradlew generateKotlinBindings copyLocalNative

      - name: Run tests
        working-directory: minigraf-ffi/java
        run: ./gradlew test
```

- [ ] **Step 2: Add Gradle wrapper**

```bash
cd minigraf-ffi/java
gradle wrapper --gradle-version 8.11
```

Commit the wrapper:
```bash
git add minigraf-ffi/java/gradle/ minigraf-ffi/java/gradlew minigraf-ffi/java/gradlew.bat
git commit -m "chore(java): add Gradle wrapper"
```

- [ ] **Step 3: Commit CI workflow**

```bash
git add .github/workflows/java-ci.yml
git commit -m "ci(java): add PR test matrix for JVM JAR (4 platforms)"
```

---

## Task 5: Add release workflow

**Files:**
- Create: `.github/workflows/java-release.yml`

- [ ] **Step 1: Create `.github/workflows/java-release.yml`**

```yaml
name: Java Release

on:
  workflow_call:
    inputs:
      tag:
        required: true
        type: string
  workflow_dispatch:
    inputs:
      tag:
        required: true
        type: string

jobs:
  build-natives:
    name: Build native (${{ matrix.target }})
    runs-on: ${{ matrix.os }}
    strategy:
      fail-fast: false
      matrix:
        include:
          - os: ubuntu-latest
            target: x86_64-unknown-linux-gnu
            native-dir: linux/x86_64
            lib-name: libminigraf_ffi.so
          - os: ubuntu-24.04-arm
            target: aarch64-unknown-linux-gnu
            native-dir: linux/aarch64
            lib-name: libminigraf_ffi.so
          - os: macos-14
            target: universal2
            native-dir: macos/universal
            lib-name: libminigraf_ffi.dylib
          - os: windows-latest
            target: x86_64-pc-windows-msvc
            native-dir: windows/x86_64
            lib-name: minigraf_ffi.dll

    steps:
      - uses: actions/checkout@v4

      - name: Install Rust toolchain
        uses: dtolnay/rust-toolchain@stable
        with:
          targets: ${{ matrix.target == 'universal2' && 'aarch64-apple-darwin x86_64-apple-darwin' || matrix.target }}

      - name: Build native (universal2)
        if: matrix.target == 'universal2'
        run: |
          cargo build --release -p minigraf-ffi --target aarch64-apple-darwin
          cargo build --release -p minigraf-ffi --target x86_64-apple-darwin
          lipo -create \
            target/aarch64-apple-darwin/release/libminigraf_ffi.dylib \
            target/x86_64-apple-darwin/release/libminigraf_ffi.dylib \
            -output libminigraf_ffi.dylib

      - name: Build native (other)
        if: matrix.target != 'universal2'
        run: cargo build --release -p minigraf-ffi --target ${{ matrix.target }}

      - name: Upload native
        uses: actions/upload-artifact@v4
        with:
          name: native-${{ matrix.native-dir == 'linux/x86_64' && 'linux-x64' || matrix.native-dir == 'linux/aarch64' && 'linux-arm64' || matrix.native-dir == 'macos/universal' && 'macos' || 'windows' }}
          path: |
            ${{ matrix.target == 'universal2' && 'libminigraf_ffi.dylib' || format('target/{0}/release/{1}', matrix.target, matrix.lib-name) }}

  assemble-and-publish:
    name: Assemble fat JAR and publish to Maven Central
    needs: build-natives
    runs-on: ubuntu-latest

    steps:
      - uses: actions/checkout@v4

      - name: Install Rust toolchain
        uses: dtolnay/rust-toolchain@stable

      - name: Set up JDK 11
        uses: actions/setup-java@v4
        with:
          java-version: '11'
          distribution: 'temurin'

      - name: Download all natives
        uses: actions/download-artifact@v4
        with:
          pattern: native-*
          path: natives-staging
          merge-multiple: false

      - name: Copy natives into resources
        run: |
          RESOURCES=minigraf-ffi/java/src/main/resources/natives
          mkdir -p "$RESOURCES/linux/x86_64" "$RESOURCES/linux/aarch64" \
                   "$RESOURCES/macos/universal" "$RESOURCES/windows/x86_64"
          cp natives-staging/native-linux-x64/libminigraf_ffi.so  "$RESOURCES/linux/x86_64/"
          cp natives-staging/native-linux-arm64/libminigraf_ffi.so "$RESOURCES/linux/aarch64/"
          cp natives-staging/native-macos/libminigraf_ffi.dylib    "$RESOURCES/macos/universal/"
          cp natives-staging/native-windows/minigraf_ffi.dll       "$RESOURCES/windows/x86_64/"

      - name: Build Rust library for bindgen (Linux x86_64)
        run: cargo build --release -p minigraf-ffi

      - name: Generate Kotlin bindings
        working-directory: minigraf-ffi/java
        run: ./gradlew generateKotlinBindings

      - name: Publish to Maven Central
        working-directory: minigraf-ffi/java
        env:
          OSSRH_USERNAME: ${{ secrets.OSSRH_USERNAME }}
          OSSRH_PASSWORD: ${{ secrets.OSSRH_PASSWORD }}
          GPG_SIGNING_KEY: ${{ secrets.GPG_SIGNING_KEY }}
          GPG_SIGNING_PASSWORD: ${{ secrets.GPG_SIGNING_PASSWORD }}
        run: ./gradlew publishReleasePublicationToOSSRHRepository
```

- [ ] **Step 2: Commit**

```bash
git add .github/workflows/java-release.yml
git commit -m "ci(java): add release workflow — fat JAR assembly + Maven Central publish"
```

---

## Task 6: Bump version and update docs

**Files:**
- Modify: `Cargo.toml` (root)
- Modify: `minigraf-ffi/Cargo.toml`
- Modify: `CHANGELOG.md`
- Modify: `ROADMAP.md`

- [ ] **Step 1: Bump crate versions to `0.23.0`**

In `Cargo.toml` (root):
```toml
version = "0.23.0"
```

In `minigraf-ffi/Cargo.toml`:
```toml
version = "0.23.0"
```

- [ ] **Step 2: Run `cargo check`**

```bash
cargo check --workspace
```

Expected: no errors.

- [ ] **Step 3: Add CHANGELOG entry**

```markdown
## [0.23.0] — 2026-04-XX

### Added
- **Phase 8.3b**: Java desktop JVM bindings published to Maven Central as
  `io.github.adityamukho:minigraf-jvm:0.23.0`. Add to Gradle:
  `implementation("io.github.adityamukho:minigraf-jvm:0.23.0")`.
  Fat JAR with embedded natives for Linux x86_64/aarch64, macOS universal2,
  Windows x86_64. API: `MiniGrafDb.open(path)`, `MiniGrafDb.openInMemory()`,
  `.execute(datalog)`, `.checkpoint()`.
```

- [ ] **Step 4: Mark 8.3b complete in ROADMAP.md**

Find the Phase 8.3b section and update status to `✅ COMPLETE`.

- [ ] **Step 5: Commit and tag**

```bash
git add Cargo.toml minigraf-ffi/Cargo.toml CHANGELOG.md ROADMAP.md
git commit -m "chore(release): bump version to v0.23.0 — Phase 8.3b Java desktop bindings"
git tag -a v0.23.0 -m "Phase 8.3b complete — Java desktop bindings published to Maven Central"
git push origin v0.23.0
```
