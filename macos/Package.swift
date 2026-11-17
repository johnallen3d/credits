// swift-tools-version:6.0
import PackageDescription

let package = Package(
    name: "CreditsLauncher",
    platforms: [.macOS(.v14)],
    targets: [
        .executableTarget(
            name: "CreditsLauncher",
            path: "Sources/CreditsLauncher",
            swiftSettings: [.swiftLanguageMode(.v5)]
        )
    ]
)
