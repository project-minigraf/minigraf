// swift-tools-version: 5.9
import PackageDescription

// This file is automatically updated by CI after each release.
// The URL and checksum below are updated to point to the latest .xcframework.zip.
let package = Package(
    name: "MinigrafKit",
    platforms: [
        .iOS(.v16),
    ],
    products: [
        .library(
            name: "MinigrafKit",
            targets: ["minigrafFFI", "MinigrafKit"]
        ),
    ],
    targets: [
        .binaryTarget(
            name: "minigrafFFI",
            // Updated by CI: release-upload-mobile job
            url: "https://github.com/adityamukho/minigraf/releases/download/v0.21.1/MinigrafKit-v0.21.1.xcframework.zip",
            checksum: "7e662e65d1d7ce3ca182cfb3d56b4ea0790a3e084868db82b6922b30fe247d3e"
        ),
        .target(
            name: "MinigrafKit",
            dependencies: [.target(name: "minigrafFFI")],
            path: "swift/Sources/MinigrafKit"
        ),
    ]
)
