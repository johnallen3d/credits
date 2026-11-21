{
  description = "Movie and TV browser";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    crane.url = "github:ipetkov/crane";
  };

  outputs = {
    nixpkgs,
    crane,
    ...
  }: let
    systems = ["x86_64-linux" "aarch64-linux" "aarch64-darwin"];
    forAllSystems = f: nixpkgs.lib.genAttrs systems (system: f nixpkgs.legacyPackages.${system});

    build = pkgs: let
      inherit (pkgs) lib;
      craneLib = crane.mkLib pkgs;
      caBundle = "${pkgs.cacert}/etc/ssl/certs/ca-bundle.crt";

      # Templates, migrations and static assets are embedded at compile time;
      # openapi.json is what the tests check the API routes against.
      src = lib.fileset.toSource {
        root = ./.;
        fileset = lib.fileset.unions [
          (craneLib.fileset.commonCargoSources ./.)
          ./openapi.json
          ./migrations
          ./static
          ./templates
        ];
      };

      commonArgs = {
        inherit src;
        strictDeps = true;
        SSL_CERT_FILE = caBundle;
      };

      # Dependencies only, keyed on Cargo.lock: app-only changes reuse this from the cache.
      cargoArtifacts = craneLib.buildDepsOnly commonArgs;
    in {
      credits = craneLib.buildPackage (commonArgs
        // {
          inherit cargoArtifacts;
          # Tests run in CI as a separate check, so deploys never run them.
          doCheck = false;
          nativeBuildInputs = [pkgs.makeBinaryWrapper];
          postInstall = ''
            wrapProgram "$out/bin/credits-web" --set-default SSL_CERT_FILE ${caBundle}
          '';
          meta = {
            description = "Movie and TV browser";
            license = lib.licenses.mit;
            # The server, which Home Manager runs.
            mainProgram = "credits-web";
          };
        });

      tests = craneLib.cargoTest (commonArgs // {inherit cargoArtifacts;});
    };

    # The macOS launcher (Option-Space search), built from macos/ alone.
    launcher = pkgs:
      pkgs.swiftPackages.stdenv.mkDerivation {
        pname = "credits-launcher";
        version = "0.1";
        src = pkgs.lib.fileset.toSource {
          root = ./macos;
          fileset = pkgs.lib.fileset.unions [
            ./macos/Package.swift
            ./macos/Sources
            ./macos/Info.plist
            ./macos/AppIcon.icns
            ./macos/Metadata.appintents
          ];
        };
        nativeBuildInputs = with pkgs; [swift swiftpm swiftPackages.swiftpmHook rcodesign];
        # App Intents' IndexedEntity and NSUserActivity.appEntityIdentifier need macOS 15.2.
        buildInputs = [pkgs.apple-sdk_15];
        # The swiftpm hook runs `swift build -c release`. The Info.plist key
        # tells the app it's started at login by the installer (launchd), so it
        # hides Open at Login, whose registered store path would go stale.
        installPhase = ''
          runHook preInstall
          app="$out/Applications/Credits Launcher.app"
          mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
          sed 's|^</dict>|\t<key>CreditsLoginItemManaged</key><true/>\n</dict>|' Info.plist > "$app/Contents/Info.plist"
          cp AppIcon.icns "$app/Contents/Resources/"
          # Siri and Shortcuts metadata, extracted with Xcode by `mise run mac:intents`.
          cp -R Metadata.appintents "$app/Contents/Resources/"
          cp "$(swiftpmBinPath)/CreditsLauncher" "$app/Contents/MacOS/"
          runHook postInstall
        '';
        # Sign last: fixup strips the binary, which would break the signature.
        postFixup = ''
          rcodesign sign "$out/Applications/Credits Launcher.app"
        '';
        meta = {
          description = "Option-Space search for Credits";
          license = pkgs.lib.licenses.mit;
          platforms = ["aarch64-darwin"];
        };
      };
  in {
    packages = forAllSystems (pkgs: let
      inherit (build pkgs) credits;
    in
      {
        inherit credits;
        default = credits;
      }
      // nixpkgs.lib.optionalAttrs pkgs.stdenv.hostPlatform.isDarwin {launcher = launcher pkgs;});

    checks = forAllSystems (pkgs: {inherit (build pkgs) tests;});
  };
}
