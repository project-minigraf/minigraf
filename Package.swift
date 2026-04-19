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
            url: "https://github.com/adityamukho/minigraf/releases/download/v0.21.0/MinigrafKit-v0.21.0.xcframework.zip",
            checksum: "f0cf7b15c1b9d341b51a022048521f2443b49b9338afa7eced1f8bbdec1759ba"
        ),
        .target(
            name: "MinigrafKit",
            dependencies: [.target(name: "minigrafFFI")],
            path: "swift/Sources/MinigrafKit"
        ),
    ]
)
