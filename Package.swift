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
            url: "https://github.com/project-minigraf/minigraf/releases/download/v1.0.0/MinigrafKit-v1.0.0.xcframework.zip",
            checksum: "fe68cd03d7bd1c17d259d67dd8d951c823c96c593f1474f3a55ebc8c40ceb9da"
        ),
        .target(
            name: "MinigrafKit",
            dependencies: [.target(name: "minigrafFFI")],
            path: "minigraf-swift/Sources/MinigrafKit"
        ),
    ]
)
