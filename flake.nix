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
            wrapProgram "$out/bin/credits" --set-default SSL_CERT_FILE ${caBundle}
          '';
          meta = {
            description = "Movie and TV browser";
            license = lib.licenses.mit;
            mainProgram = "credits";
          };
        });

      tests = craneLib.cargoTest (commonArgs // {inherit cargoArtifacts;});
    };
  in {
    packages = forAllSystems (pkgs: let
      inherit (build pkgs) credits;
    in {
      inherit credits;
      default = credits;
    });

    checks = forAllSystems (pkgs: {inherit (build pkgs) tests;});
  };
}
