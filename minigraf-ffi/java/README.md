# minigraf-jvm

[![Maven Central](https://img.shields.io/maven-central/v/io.github.adityamukho/minigraf-jvm.svg)](https://central.sonatype.com/artifact/io.github.adityamukho/minigraf-jvm)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](https://github.com/project-minigraf/minigraf#license)

> Embedded bi-temporal graph database for Java/Kotlin — Datalog queries, time travel, fat JAR with embedded natives

Minigraf for Java and Kotlin on the desktop JVM. Fat JAR with embedded native libraries for Linux x86_64/aarch64, macOS universal2, and Windows x86_64. No Rust toolchain required.

## Install

### Gradle (Kotlin DSL)

```kotlin
dependencies {
    implementation("io.github.adityamukho:minigraf-jvm:1.0.0")
}
```

### Maven

```xml
<dependency>
    <groupId>io.github.adityamukho</groupId>
    <artifactId>minigraf-jvm</artifactId>
    <version>1.0.0</version>
</dependency>
```

## Quick start

```kotlin
import io.github.adityamukho.minigraf.MiniGrafDb
import org.json.JSONObject

// File-backed database
val db = MiniGrafDb.open("/path/to/myapp.graph")

// In-memory database (ephemeral / testing)
val mem = MiniGrafDb.openInMemory()

// Transact facts
db.execute("""(transact [[:alice :person/name "Alice"] [:alice :person/age 30]])""")

// Query with Datalog
val json = JSONObject(db.execute(
    "(query [:find ?name ?age :where [?e :person/name ?name] [?e :person/age ?age]])"
))
// json.getJSONArray("results").getJSONArray(0).getString(0) == "Alice"

// Time travel — state as of transaction 1
val snap = db.execute("(query [:find ?age :as-of 1 :where [:alice :person/age ?age]])")

// Flush dirty pages to disk
db.checkpoint()
```

## Links

- [Full Java/JVM integration guide](https://github.com/project-minigraf/minigraf/wiki/Use-Cases#java--jvm)
- [Repository](https://github.com/project-minigraf/minigraf)
- [Datalog Reference](https://github.com/project-minigraf/minigraf/wiki/Datalog-Reference)

## License

MIT OR Apache-2.0
