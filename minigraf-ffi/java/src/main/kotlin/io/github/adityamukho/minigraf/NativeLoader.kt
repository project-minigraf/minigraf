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

        // JNA's Native.register() uses findLibraryName() which reads this property.
        // Providing an absolute path causes JNA to dlopen/LoadLibrary directly from
        // the temp file, bypassing the system library search path.
        System.setProperty("uniffi.component.minigraf_ffi.libraryOverride", dest.absolutePath)
        loaded = true
    }
}
